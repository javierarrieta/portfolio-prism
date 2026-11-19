//! SPIKE test: drive the Rocket-mounted rmcp service end to end with no second listener.
//!
//! Proves three claims the spec currently only asserts:
//!   1. a Rocket route can serve the streamable-HTTP transport,
//!   2. in stateless mode no `Mcp-Session-Id` is minted,
//!   3. with `json_response(true)` the reply is a plain JSON body, not an SSE stream.

use backend_rust::agent_spike;
use rocket::http::{ContentType, Header, Status};
use rocket::local::asynchronous::Client;
use serde_json::{json, Value};

fn rocket() -> rocket::Rocket<rocket::Build> {
    rocket::build().mount("/", rocket::routes![agent_spike::mcp])
}

#[rocket::async_test]
async fn rocket_route_serves_mcp_statelessly_with_plain_json() {
    let client = Client::tracked(rocket()).await.expect("valid rocket");

    // --- initialize -------------------------------------------------------
    let res = client
        .post("/mcp")
        .header(ContentType::JSON)
        .header(Header::new("accept", "application/json, text/event-stream"))
        .header(Header::new("host", "localhost:8000"))
        .body(
            json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {
                    "protocolVersion": "2026-07-28",
                    "capabilities": {},
                    "clientInfo": { "name": "spike", "version": "0" }
                }
            })
            .to_string(),
        )
        .dispatch()
        .await;

    let status = res.status();
    let headers: Vec<(String, String)> = res
        .headers()
        .iter()
        .map(|h| (h.name().to_string(), h.value().to_string()))
        .collect();
    let text = res.into_string().await.unwrap();
    assert_eq!(status, Status::Ok, "initialize must succeed; body: {text}");
    assert!(
        !headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("mcp-session-id")),
        "stateless mode must not mint a session id: {headers:?}"
    );
    let ct = headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| "<none>".to_string());
    assert!(
        ct.starts_with("application/json"),
        "json_response(true) must reply with plain JSON, got {ct}"
    );
    let init: Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("body must be plain JSON, not SSE: {e}\n{text}"));
    let protocol = init["result"]["protocolVersion"]
        .as_str()
        .expect("initialize must report a protocol version")
        .to_string();
    assert!(
        init["result"]["serverInfo"].is_object(),
        "initialize must report serverInfo: {init}"
    );

    // --- tools/list -------------------------------------------------------
    let res = client
        .post("/mcp")
        .header(ContentType::JSON)
        .header(Header::new("accept", "application/json, text/event-stream"))
        .header(Header::new("host", "localhost:8000"))
        .header(Header::new("mcp-protocol-version", protocol.clone()))
        .body(
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} })
                .to_string(),
        )
        .dispatch()
        .await;
    let listed: Value =
        serde_json::from_str(&res.into_string().await.unwrap()).expect("tools/list must be plain JSON");
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .map(|a| a.iter().filter_map(|t| t["name"].as_str()).collect())
        .unwrap_or_default();
    assert_eq!(names, vec!["ping"], "tools/list should expose the spike tool");

    // --- tools/call -------------------------------------------------------
    let res = client
        .post("/mcp")
        .header(ContentType::JSON)
        .header(Header::new("accept", "application/json, text/event-stream"))
        .header(Header::new("host", "localhost:8000"))
        .header(Header::new("mcp-protocol-version", protocol))
        .body(
            json!({
                "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": { "name": "ping", "arguments": {} }
            })
            .to_string(),
        )
        .dispatch()
        .await;
    let called: Value =
        serde_json::from_str(&res.into_string().await.unwrap()).expect("tools/call must be plain JSON");
    assert_eq!(
        called["result"]["content"][0]["text"].as_str(),
        Some("pong"),
        "the tool must actually run: {called}"
    );
}
