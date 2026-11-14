//! A private, freshly migrated database for one test.
//!
//! SQLite gave every test isolation for free: `SqlitePool::connect(":memory:")`
//! made a database no other test could see. Postgres has no equivalent, so
//! isolation is built here — one real database per test, created and migrated for
//! that test alone. Without it, the tests inside one test binary (which `cargo test`
//! runs on parallel threads) would share one database and truncate each other's rows.
//!
//! * **Per-test migration, not a shared template.** A template database was tried
//!   first — `CREATE DATABASE … TEMPLATE prism_template` instead of migrating per
//!   test, on the assumption that migrating 462 times would dominate the suite. It
//!   does not: migrating `0001_init.sql` takes ~15 ms, and the measured suite is
//!   *faster* without a template. The template also cost a global sweep of every
//!   harness database at process start, which made two concurrent `cargo test` runs
//!   destroy each other. Both reasons for it are gone, so it is gone.
//! * **`caller` must be unique within the process.** The name is
//!   `prism_t_<pid>_<caller>`; the pid separates concurrent test binaries and
//!   nothing inside one. A process-wide registry turns a duplicated `caller` into a
//!   panic that names it, rather than two tests quietly sharing one database.
//! * **The normal path leaves nothing behind.** When a test's pool is dropped its
//!   connections close, and the next `create_database` in that process drops the
//!   now-unreferenced database. That is the ordinary cleanup, not a sweep.
//! * **A crash is cleaned up by the next run.** A run that is killed leaves the
//!   databases its last few tests were holding. The first `create_database` of a
//!   later run drops any harness database whose embedded pid is no longer a live
//!   process *and* which has no backends attached, and prints one line saying what it
//!   took. Liveness is the test; a live run's pid exists, so its databases are
//!   untouchable no matter how briefly one of them has no backends.
//! * **Nothing here decides a deletion from a failed query.** `has_backends` returns
//!   `Result`, and a reaper that could not read `pg_stat_activity` reports up instead
//!   of assuming "no backends" — see the comment on [`has_backends`].
//!
//! This file is compiled twice by design, from `src/lib.rs` (`#[cfg(test)]`) and
//! from `tests/common/mod.rs`, exactly like `test_provider.rs`, so the unit suite
//! and the integration suite share one implementation and neither ships it. Its own
//! tests are in `test_db_tests.rs`, included from `src/lib.rs` alone — including
//! them here would run them once per integration binary.
#![allow(dead_code)]

use sqlx::postgres::PgPoolOptions;
use sqlx::Connection as _;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// See the module comment. Panics rather than returning a `Result`: every caller is
/// a test, and a failure here means the harness is broken, not the thing under test.
pub async fn test_database(caller: &str) -> sqlx::PgPool {
    let (name, url) = create_database(caller).await;
    let pool = connect(&url)
        .await
        .unwrap_or_else(|e| panic!("could not connect to {name}: {e}"));
    // `min_connections(0)` means `connect` does not have to open anything, and a
    // database with no backend is indistinguishable from one whose test has finished.
    // Take a connection now so that from `mark_live` onwards there is always one, and
    // the backend count means what the reaper needs it to mean. It goes back to the
    // pool's idle queue when dropped, which is only safe because [`pool_options`]
    // turns off `idle_timeout`.
    let _ = pool
        .acquire()
        .await
        .unwrap_or_else(|e| panic!("could not acquire from {name}: {e}"));
    mark_live(&name).await;
    pool
}

/// The URL of a private database rather than a pool for one.
///
/// Two tests need a *second* pool against the same database — one to hold a lock, one
/// to be kept out of it — and a pool cannot be cloned into a different shape. They
/// take this and build their own options; the isolation guarantee is the name, not the
/// pool.
///
/// Because this hands out a URL instead of a pool, the harness never sees a connection
/// appear and so never counts the database as live: it is reclaimed by the next run's
/// crash sweep rather than by the in-run cleanup.
pub async fn test_database_url(caller: &str) -> String {
    create_database(caller).await.1
}

/// The pool options a test starts from: production's, plus exactly one change.
///
/// Derived from [`backend_rust::db::pool_options`] rather than retyped, because
/// `tests/backup_test.rs::an_exhausted_pool_is_a_lock_not_an_outage` only proves what
/// production does while the two agree on `max_connections` and `acquire_timeout`. A
/// hand-typed copy drifts silently and the test keeps passing while measuring a pool
/// production no longer has.
///
/// The addition is `idle_timeout(None)`. sqlx defaults to closing a connection that has
/// been idle for 600 s, and this harness decides whether a test is still running by
/// counting backends — so a test that idled past ten minutes would lose its only
/// backend, look finished, and have its database `DROP … WITH (FORCE)`d under it by the
/// next `create_database`. Keeping idle connections alive is what makes "zero backends"
/// mean "this test is over" and nothing else.
pub fn pool_options() -> PgPoolOptions {
    backend_rust::db::pool_options().idle_timeout(None)
}

/// What one process knows about the databases it made.
#[derive(Default)]
struct Harness {
    /// Every name this process has been handed, for the duplicate-`caller` check.
    seen: HashSet<String>,
    /// Connected, with the moment it became so. See [`GRACE`].
    live: HashMap<String, Instant>,
    /// Whether the crash sweep has *completed* in this process.
    swept: bool,
}

static HARNESS: tokio::sync::Mutex<Option<Harness>> = tokio::sync::Mutex::const_new(None);

/// Provision the database and return its name and URL.
async fn create_database(caller: &str) -> (String, String) {
    let base = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set for tests; `bash scripts/dev-db.sh` prints one");
    let name = database_name(caller);
    let url = database_url(&base, &name);

    // The reapers run under the lock so two tests cannot sweep the same database at
    // each other, but a failure is raised only once the guard is dropped. A transient
    // `pg_stat_activity` error must not panic while the lock is held: the panic
    // unwinds through whichever test happened to be creating a database at that
    // moment, blaming it for a server hiccup.
    let problems = {
        let mut guard = HARNESS.lock().await;
        let harness = guard.get_or_insert_with(Harness::default);
        assert!(
            harness.seen.insert(name.clone()),
            "two tests asked for the same database name {name}: `caller` must be a \
             distinct string per test, or the two tests share one database and \
             truncate each other's rows"
        );
        let mut problems = Vec::new();
        if !harness.swept {
            let sweep = reap_crash_leftovers(&base).await;
            // Recorded as done only if it finished cleanly. A sweep that died halfway
            // left databases behind and has to be tried again, not filed as complete.
            harness.swept = sweep.is_empty();
            problems.extend(sweep);
        }
        problems.extend(reap_finished(&base, &mut harness.live).await);
        problems
    };
    if let Some(first) = problems.first() {
        panic!(
            "the test database harness could not do its cleanup: {first}{}{}",
            if problems.len() > 1 { " (and " } else { "" },
            if problems.len() > 1 {
                format!("{} more)", problems.len() - 1)
            } else {
                String::new()
            }
        );
    }

    let mut admin = sqlx::postgres::PgConnection::connect(&base)
        .await
        .unwrap_or_else(|e| panic!("could not connect to DATABASE_URL {base}: {e}"));
    // Dropped only when it exists. `DROP DATABASE IF EXISTS` on a missing database is
    // a *notice*, and sqlx forwards notices to stderr — so the polite form would print
    // one line per test in a clean run, and a clean run has to be silent to be readable.
    let exists: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM pg_database WHERE datname = $1")
        .bind(&name)
        .fetch_one(&mut admin)
        .await
        .unwrap();
    if exists.0 > 0 {
        // `WITH (FORCE)` disconnects whoever left a pool behind — a test that panicked
        // before its pool was dropped.
        drop_with_retries(&mut admin, &name)
            .await
            .unwrap_or_else(|e| panic!("could not drop stale {name}: {e}"));
    }
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&mut admin)
        .await
        .unwrap_or_else(|e| panic!("could not create {name}: {e}"));
    drop(admin);

    // Migrated on its own connection, because `CREATE DATABASE` cannot run inside a
    // transaction and the migration runs several statements that must.
    let mut fresh = sqlx::postgres::PgConnection::connect(&url)
        .await
        .unwrap_or_else(|e| panic!("could not connect to the fresh database {name}: {e}"));
    sqlx::migrate!("./migrations")
        .run(&mut fresh)
        .await
        .unwrap_or_else(|e| panic!("migrations failed against {name}: {e}"));
    drop(fresh);

    (name, url)
}

/// How long a live database must have been idle before the reaper may take it.
///
/// The backend count is the liveness signal, and it is only trustworthy once the test
/// has had a chance to use its pool. A connection is acquired in [`test_database`]
/// before a database is marked live, and [`pool_options`] keeps idle connections open,
/// which together close the obvious gap; this covers the rest, because "no backends" is
/// a statement about one instant and a test that is about to run its first query also
/// has none.
const GRACE: Duration = Duration::from_secs(2);

/// Start counting a database's idle time, once something is actually connected.
async fn mark_live(name: &str) {
    let mut guard = HARNESS.lock().await;
    if let Some(harness) = guard.as_mut() {
        harness.live.insert(name.to_string(), Instant::now());
    }
}

/// Drop the databases this process connected earlier and which now have no backends,
/// meaning the test that owned them has finished and dropped its pool.
///
/// This is the ordinary cleanup, so it is silent: a line per reclaimed database would
/// be one line per test. The two kinds of failure are treated differently, on purpose.
///
/// A *drop* that fails is printed and left in `live` for the next call to retry. The
/// test that owned that database already finished and passed, and a database nobody is
/// connected to is a disk-usage problem, not a correctness one — so a test must not
/// fail because the janitor was late.
///
/// A *read* that fails is different and comes back as a problem for the caller to
/// raise: not knowing whether a database is in use is the one state from which deleting
/// something would be wrong, so the harness stops rather than guesses.
async fn reap_finished(base: &str, live: &mut HashMap<String, Instant>) -> Vec<String> {
    let mut problems = Vec::new();
    if live.is_empty() {
        return problems;
    }
    let mut admin = match sqlx::postgres::PgConnection::connect(base).await {
        Ok(admin) => admin,
        Err(e) => {
            problems.push(format!(
                "could not connect to {base} to reap finished databases: {e}"
            ));
            return problems;
        }
    };
    let mut reaped = Vec::new();
    for (name, since) in live.iter() {
        if since.elapsed() < GRACE {
            continue;
        }
        match has_backends(&mut admin, name).await {
            Ok(true) => continue,
            Ok(false) => {}
            Err(e) => {
                problems.push(format!("could not read backends for {name}: {e}"));
                continue;
            }
        }
        match drop_with_retries(&mut admin, name).await {
            Ok(()) => reaped.push(name.clone()),
            Err(e) => {
                // Loud, not fatal, and left in `live` so the next call retries it. The
                // test that owned this database has already finished and passed; failing
                // it now would blame a passed test for a janitor that could not keep up.
                eprintln!(
                    "harness: could not drop finished database {name}: {e}; retrying next call"
                );
            }
        }
    }
    for name in reaped {
        live.remove(&name);
    }
    problems
}

/// Drop databases left by an earlier run that was killed before its tests finished.
///
/// Two conditions, both required. The embedded pid must be gone from `/proc`, and the
/// database must have no backends. The pid alone would be enough for a dead run; the
/// backend count is kept as a second lock so that a pid recycled by a *live* process
/// still cannot lose a database it is using. Without `/proc` — macOS — liveness
/// cannot be read at all, so this does nothing rather than guess.
async fn reap_crash_leftovers(base: &str) -> Vec<String> {
    let mut problems = Vec::new();
    if !std::path::Path::new("/proc").exists() {
        return problems;
    }
    let mut admin = match sqlx::postgres::PgConnection::connect(base).await {
        Ok(admin) => admin,
        Err(e) => {
            problems.push(format!(
                "could not connect to {base} to sweep crash leftovers: {e}"
            ));
            return problems;
        }
    };
    // `LIKE 'prism%'` deliberately, not `LIKE 'prism_t_%'`: in a LIKE pattern `_` is a
    // single-character wildcard, so the narrower-looking pattern also matches
    // `prism_testdb`. Ownership is decided by [`harness_pid`] in Rust, which is the
    // only place the name shape is specified.
    let rows: Vec<(String,)> =
        match sqlx::query_as("SELECT datname FROM pg_database WHERE datname LIKE 'prism%'")
            .fetch_all(&mut admin)
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                problems.push(format!(
                    "could not list databases to sweep crash leftovers: {e}"
                ));
                return problems;
            }
        };

    let mut reaped = Vec::new();
    for (name,) in rows {
        let Some(pid) = harness_pid(&name) else {
            continue;
        };
        if pid == std::process::id() {
            continue;
        }
        if std::path::Path::new(&format!("/proc/{pid}")).exists() {
            continue;
        }
        match has_backends(&mut admin, &name).await {
            Ok(true) => continue,
            Ok(false) => {}
            Err(e) => {
                problems.push(format!("could not read backends for {name}: {e}"));
                continue;
            }
        }
        match drop_with_retries(&mut admin, &name).await {
            Ok(()) => reaped.push(name),
            Err(e) => eprintln!(
                "harness: could not drop crash leftover {name}: {e}; leaving it for a later run"
            ),
        }
    }
    if !reaped.is_empty() {
        // One line, and only when something was actually taken: a sweep that quietly
        // deletes databases is the last thing a debugging engineer wants.
        eprintln!(
            "harness: reaped {} test database(s) left by a run that did not finish: {}",
            reaped.len(),
            reaped.join(", ")
        );
    }
    problems
}

/// Whether anything is connected to `name` right now.
///
/// The column is `pg_stat_activity.datname`, not `database` — and this must not fall
/// back to "no backends" if the query itself fails, because "no backends" is the signal
/// to delete the database. An early version wrote `WHERE database = $1`, the query
/// errored, `unwrap_or((0,))` reported no backends, and the harness dropped live test
/// databases mid-run. The `Result` is the fix: a caller that cannot tell has to stop,
/// not guess.
pub(crate) async fn has_backends(
    admin: &mut sqlx::postgres::PgConnection,
    name: &str,
) -> Result<bool, sqlx::Error> {
    let (count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM pg_stat_activity WHERE pg_stat_activity.datname = $1")
            .bind(name)
            .fetch_one(admin)
            .await?;
    Ok(count > 0)
}

/// Refuse to run DDL against a database the harness cannot prove it owns.
///
/// The name is interpolated into DDL, where a bind parameter cannot go, so it is
/// validated first instead of being trusted because of where it came from. Both call
/// sites feed this either a [`database_name`] or a row filtered by [`harness_pid`], so
/// today a bad name could only be a bug here or a hand-made database — and the worst
/// case would be a syntax error, since the extended protocol refuses a second
/// statement. Asserting the shape anyway costs one line and turns "the driver happens
/// to save us" into a stated rule.
fn assert_harness_database(name: &str) {
    assert!(
        harness_pid(name).is_some(),
        "refusing to run DDL against {name:?}: it is not a harness database name \
         (prism_t_<pid>_<caller>). Nothing in this harness may DROP a database it \
         cannot prove it owns."
    );
}

/// `DROP DATABASE … WITH (FORCE)`, retried, because Postgres refuses while a
/// connection lingers and closing a pool is not instantaneous.
async fn drop_with_retries(
    admin: &mut sqlx::postgres::PgConnection,
    name: &str,
) -> Result<(), sqlx::Error> {
    assert_harness_database(name);
    let mut last = None;
    for attempt in 0..3 {
        match sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
            .execute(&mut *admin)
            .await
        {
            Ok(_) => return Ok(()),
            Err(e) => {
                last = Some(e);
                if attempt < 2 {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }
    }
    Err(last.expect("three attempts always record an error"))
}

/// The pid embedded in a harness database name, or `None` if the name is not one.
///
/// This is the only test of whether a database belongs to the harness, so it is exact
/// and it is in Rust: `prism_t_`, then at least one digit, then `_`, then a non-empty
/// label. A `LIKE 'prism_t_%'` filter would match `prism_testdb` (because `_` is a
/// single-character wildcard) and would match a real database named
/// `prism_t_2024_archive`, whose `2024` looks like a pid until this parses the segment
/// and finds the label does not start where a pid's must.
pub(crate) fn harness_pid(name: &str) -> Option<u32> {
    let rest = name.strip_prefix("prism_t_")?;
    let end = rest.find('_')?;
    let digits = &rest[..end];
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if end + 1 >= rest.len() {
        return None; // a pid with no caller label after it
    }
    digits.parse().ok()
}

/// The database name one `caller` gets.
///
/// `caller` is a human label written at the call site, not SQL, and it is interpolated
/// into `CREATE DATABASE` unquoted — so anything outside `[a-z0-9_]` has to go. A test
/// that labels a case with `{raw:?}` would otherwise hand Postgres a `"` and get
/// `zero-length delimited identifier at or near """"`, which names the label's
/// spelling rather than the problem. Lowercasing and the 63-byte cap are Postgres'
/// own folding and truncation for an unquoted identifier; doing them here means the
/// registry sees the *same* name the server does, so two labels that collapse onto one
/// database fail the duplicate check instead of silently sharing.
pub fn database_name(caller: &str) -> String {
    format!("prism_t_{}_{}", std::process::id(), caller)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .take(63)
        .collect()
}

/// Point a Postgres URL at a different database.
///
/// `DATABASE_URL` already names one — `postgres://host:5432/prism` — so the test
/// database's name *replaces* that path rather than being appended to it. Concatenating
/// produces `postgres://host:5432/prism/prism_t_1`, which Postgres reads as a database
/// called `prism` with a schema-ish trailing segment and rejects with a connection
/// error that names neither problem.
pub fn database_url(base: &str, name: &str) -> String {
    let after_scheme = base.find("://").map(|i| i + 3).unwrap_or(0);
    let (authority_and_db, query) = match base[after_scheme..].find('?') {
        Some(i) => (
            &base[after_scheme..after_scheme + i],
            Some(base[after_scheme + i + 1..].to_string()),
        ),
        None => (&base[after_scheme..], None),
    };
    let authority = match authority_and_db.find('/') {
        Some(i) => &authority_and_db[..i],
        None => authority_and_db,
    };
    let mut out = format!("{}{}/{}", &base[..after_scheme], authority, name);
    if let Some(q) = query {
        out.push('?');
        out.push_str(&q);
    }
    out
}

/// A unique `caller` string for a helper that has one name but many callers.
///
/// `tests/common::setup_db` is the reason this exists: it takes a `StartupConfig`,
/// not a name, and there is one call per test. The counter makes each call's
/// database distinct without every call site having to invent a string.
pub fn next_caller(prefix: &str) -> String {
    static SEQUENCE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    format!(
        "{prefix}_{}",
        SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    )
}

fn connect(url: &str) -> impl std::future::Future<Output = Result<sqlx::PgPool, sqlx::Error>> {
    pool_options().connect(url)
}
