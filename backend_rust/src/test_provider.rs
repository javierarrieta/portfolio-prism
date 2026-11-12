//! A stand-in for the Yahoo chart API, compiled into both the library's unit-test
//! build and — via `#[path]` in `tests/common/mod.rs` — every integration-test
//! binary. One source, two consumers, nothing shipped in the release binary.
//!
//! It is a socket rather than a trait mock, deliberately. The defect that made every
//! foreign-currency number wrong in `v0.0.6-M5` was URL construction — the service
//! asked Yahoo for `EUR=USD` where Yahoo wants `EURUSD=X`. A trait mock would
//! bypass URL building and response parsing, which is the code that actually
//! failed. This walks both.
//!
//! Kept free of `crate::` paths on purpose: the integration-test include is a
//! different crate, so anything here that needed the app's own types could not be
//! shared. `offline_pricing_service()` in `lib.rs` is where the two meet.
//!
//! `allow(dead_code)`: this file is compiled into every test binary and the library's
//! own test build, and each consumer uses a subset. A subset is not dead code.
#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;

/// Yahoo's own 404 envelope, copied from a live response so the failure path under
/// test is the one production sees. `chart.result` is a `Vec` in the deserialising
/// struct, so a real 404 never reaches the parser — the status check returns first,
/// exactly as it does against the live host.
const YAHOO_NOT_FOUND: &str = r#"{"chart":{"result":null,"error":{"code":"Not Found","description":"No data found, symbol may be delisted"}}}"#;

/// A stand-in for the Yahoo chart API, on an ephemeral loopback port.
///
/// Four tests used to reach `query1.finance.yahoo.com` for real. One of them was a
/// provider-outage test whose own comment said the outage "is real whichever way
/// the request fails" — true, and also the suite depending on the public internet
/// to pass. This answers the same two shapes without leaving the process: a 404 in
/// Yahoo's own envelope for any symbol not registered with
/// [`MockYahoo::serving`], and a canned chart for one that is.
///
/// A socket rather than a trait mock, deliberately. The defect that made every
/// foreign-currency number wrong in `v0.0.6-M5` was URL construction — the service
/// asked Yahoo for `EUR=USD` where Yahoo wants `EURUSD=X`. A trait mock would
/// bypass URL building and response parsing, which is the code that actually
/// failed. This walks both.
pub struct MockYahoo {
    base_url: String,
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
    paths: Arc<Mutex<Vec<String>>>,
}

impl MockYahoo {
    /// Every symbol answers 404, as Yahoo answers one it does not know.
    pub fn start() -> Self {
        Self::serving([])
    }

    /// Each `(symbol, body)` answers 200 with `body`; anything else answers 404.
    /// The body must be Yahoo's chart envelope, not a bare price.
    pub fn serving<const N: usize>(responses: [(&str, &str); N]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("free loopback port");
        let port = listener.local_addr().expect("loopback addr").port();
        listener.set_nonblocking(true).expect("nonblocking accept");
        let stop = Arc::new(AtomicBool::new(false));
        let paths = Arc::new(Mutex::new(Vec::new()));
        let table: Arc<Vec<(String, String)>> = Arc::new(
            responses
                .into_iter()
                .map(|(symbol, body)| (symbol.to_string(), body.to_string()))
                .collect(),
        );

        let (stop_loop, paths_seen) = (stop.clone(), paths.clone());
        let accept = std::thread::spawn(move || loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    let (table, paths_seen) = (table.clone(), paths_seen.clone());
                    // One thread per connection: the engine issues price and rate
                    // requests concurrently, and a serial accept loop would make
                    // two in-flight requests wait on each other.
                    std::thread::spawn(move || {
                        let mut stream = stream;
                        let line = {
                            let mut buf = String::new();
                            if BufReader::new(&stream).read_line(&mut buf).is_err() {
                                return;
                            }
                            buf
                        };
                        paths_seen.lock().unwrap().push(line.clone());
                        let (status, body) = route_chart_request(&line, &table);
                        let _ = write!(
                            stream,
                            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = stream.flush();
                    });
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if stop_loop.load(Ordering::Relaxed) {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(_) => break,
            }
        });

        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            stop,
            accept: Some(accept),
            paths,
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Request lines answered so far. Lets a test assert the seam was walked rather
    /// than silently bypassed.
    #[allow(dead_code)]
    pub fn paths(&self) -> Vec<String> {
        self.paths.lock().unwrap().clone()
    }
}

/// The URL of a process-wide [`MockYahoo`] answering 404 for every symbol.
///
/// For files that build several rockets: the mock has to outlive each one, and a
/// per-test mock dropped while a rocket still holds it would leave the port closed.
/// One per integration-test binary is enough, and statics are never dropped, so the
/// accept thread simply ends with the process.
pub fn mock_pricing_url() -> &'static str {
    static MOCK: OnceLock<MockYahoo> = OnceLock::new();
    MOCK.get_or_init(MockYahoo::start).base_url()
}

impl Drop for MockYahoo {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.accept.take() {
            let _ = handle.join();
        }
    }
}

/// Every outbound pricing call is `/v8/finance/chart/<symbol>[?query]`, so the
/// symbol alone decides the response.
fn route_chart_request(line: &str, table: &[(String, String)]) -> (&'static str, String) {
    let symbol = line
        .split("/chart/")
        .nth(1)
        .map(|rest| rest.split(['?', ' ']).next().unwrap_or(rest))
        .unwrap_or("");
    match table.iter().find(|(registered, _)| registered == symbol) {
        Some((_, body)) => ("200 OK", body.clone()),
        None => ("404 Not Found", YAHOO_NOT_FOUND.to_string()),
    }
}
