//! The `Host` guard on `/mcp`, and the configuration that drives it.
//!
//! rmcp validates `Host` inside `StreamableHttpService::handle`, against a list that
//! defaults to loopback only, and it does that **after** this route has already accepted the
//! bearer. The combination is the whole reason this file exists: with the default left in
//! place, a deployment reached by its real hostname answers every MCP call
//! `403 Forbidden: Host header is not allowed` while holding a perfectly valid token, which
//! reads as a broken credential and says nothing about the missing setting. Nothing in the
//! existing suite could see it, because Rocket's local client sends no `Host` header at all
//! and `mount.rs` falls back to the literal `localhost` — which is on the default list.
//! Every test here therefore names the header explicitly.
//!
//! Reading order: the refusal, then the proof that the same token passes on an allowed host
//! (the differential that separates this from a token bug), then what the operator can write
//! and what it means.

use backend_rust::agent::AgentConfig;
use backend_rust::auth::pat;
use rocket::http::{ContentType, Header, Status};
use rocket::local::asynchronous::Client;
use serde_json::{json, Value};

mod common;
use common::{build_rocket_with_agent_config, db_with_user};

/// The hostname a deployment would actually be reached by, and the one nginx forwards as
/// `Host` because `frontend/nginx.conf` proxies with `proxy_set_header Host $host`.
const PUBLIC_HOST: &str = "portfolio.example.com";

fn initialize() -> Value {
    json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2026-07-28",
            "capabilities": {},
            "clientInfo": {"name": "host-test", "version": "0"}
        }
    })
}

/// One `/mcp` call, with the `Host` header named rather than left to Rocket's fallback.
async fn rpc(client: &Client, host: Option<&str>, bearer: &str) -> (Status, String) {
    let mut req = client
        .post("/mcp")
        .header(ContentType::JSON)
        .header(Header::new(
            "Accept",
            "application/json, text/event-stream",
        ))
        .header(Header::new("Authorization", format!("Bearer {bearer}")));
    if let Some(h) = host {
        req = req.header(Header::new("Host", h.to_string()));
    }
    let res = req.body(initialize().to_string()).dispatch().await;
    let status = res.status();
    (status, res.into_string().await.unwrap_or_default())
}

/// A rocket built with `hosts` as the allowlist, and a live token for it.
async fn harness(caller: &str, hosts: Vec<String>) -> (Client, String) {
    let (pool, user) = db_with_user(caller).await;
    let cfg = AgentConfig {
        mcp_allowed_hosts: hosts,
        ..Default::default()
    };
    let client = Client::tracked(build_rocket_with_agent_config(pool.clone(), user, cfg))
        .await
        .expect("valid rocket");
    let secret = pat::mint(&pool, user, "host-test", &["read".to_string()], None)
        .await
        .unwrap()
        .secret;
    (client, secret)
}

#[tokio::test]
async fn the_default_allowlist_refuses_the_public_host() {
    // The defect, pinned. `AgentConfig::default()` carries rmcp's loopback list, so the
    // hostname the app is actually deployed under is refused.
    let (client, secret) = harness("mcp_host_default", AgentConfig::default().mcp_allowed_hosts).await;
    let (status, body) = rpc(&client, Some(PUBLIC_HOST), &secret).await;
    assert_eq!(
        status,
        Status::Forbidden,
        "an allowlist nobody configured must not answer for the public host; body: {body}"
    );
    assert!(
        body.contains("Host header is not allowed"),
        "the refusal must name the Host, since that is the only hint an operator gets: {body}"
    );
}

#[tokio::test]
async fn the_same_token_passes_on_an_allowed_host() {
    // The other half of the differential, and the reason the test above is not simply a
    // token test wearing a costume: same client, same credential, one header changed. If
    // this ever fails the 403 above is not about `Host`.
    let (client, secret) = harness("mcp_host_allowed", vec![PUBLIC_HOST.to_string()]).await;
    let (status, body) = rpc(&client, Some(PUBLIC_HOST), &secret).await;
    assert_eq!(status, Status::Ok, "allowed host must serve the handshake: {body}");
    let parsed: Value = serde_json::from_str(&body).expect("initialize answers JSON");
    assert!(
        parsed["result"]["serverInfo"]["name"].is_string(),
        "and it must be a real handshake, not an empty 200: {parsed}"
    );
}

#[tokio::test]
async fn a_missing_host_header_falls_back_to_localhost_and_is_judged_like_any_other_host() {
    // Rocket's local client sends no `Host`, and `mount.rs` synthesizes `localhost`. That
    // fallback is on the default list, which is why the whole suite stayed green while the
    // deployment was 403 — so the fallback is asserted as behaviour rather than trusted:
    // with a real allowlist configured, the synthesized host is refused like any other.
    let (client, secret) = harness("mcp_host_fallback", vec![PUBLIC_HOST.to_string()]).await;
    let (status, body) = rpc(&client, None, &secret).await;
    assert_eq!(
        status,
        Status::Forbidden,
        "no Host header means the synthesized localhost, which is not on this list: {body}"
    );

    // And on the default list it passes, which is the half the rest of the suite relies on.
    let (client, secret) =
        harness("mcp_host_fallback_default", AgentConfig::default().mcp_allowed_hosts).await;
    let (status, body) = rpc(&client, None, &secret).await;
    assert_eq!(status, Status::Ok, "localhost is on the default list: {body}");
}

#[tokio::test]
async fn a_portless_entry_matches_any_port() {
    // Documented rather than discovered: rmcp compares the port only when the entry names
    // one, so `portfolio.example.com` covers `:443`, `:8443` and no port at all. This is
    // why the k8s value is the bare hostname and not a copy of `ALLOWED_ORIGINS`.
    let (client, secret) = harness("mcp_host_port", vec![PUBLIC_HOST.to_string()]).await;
    for host in [
        PUBLIC_HOST.to_string(),
        format!("{PUBLIC_HOST}:443"),
        format!("{PUBLIC_HOST}:8443"),
    ] {
        let (status, body) = rpc(&client, Some(&host), &secret).await;
        assert_eq!(status, Status::Ok, "portless entry must match {host}: {body}");
    }
}

#[tokio::test]
async fn an_entry_with_a_port_refuses_that_host_on_another_port() {
    // The converse, because a list that only ever matches is as uninformative as one that
    // only ever refuses.
    let (client, secret) = harness(
        "mcp_host_pinned_port",
        vec![format!("{PUBLIC_HOST}:8443")],
    )
    .await;
    let (status, _) = rpc(&client, Some(&format!("{PUBLIC_HOST}:8443")), &secret).await;
    assert_eq!(status, Status::Ok, "the pinned port must match itself");
    let (status, _) = rpc(&client, Some(PUBLIC_HOST), &secret).await;
    assert_eq!(
        status,
        Status::Forbidden,
        "a pinned entry pins the port too; a bare Host is not a wildcard match"
    );
}

#[tokio::test]
async fn an_empty_allowlist_is_the_documented_off_switch_and_accepts_any_host() {
    // What `MCP_ALLOWED_HOSTS=*` produces. Asserted so the empty list is never mistaken for
    // a closed door: in rmcp it means the opposite, which is also why the parser refuses to
    // produce it from a blank value.
    let (client, secret) = harness("mcp_host_star", Vec::new()).await;
    for host in ["anything.invalid", PUBLIC_HOST, "localhost:8000"] {
        let (status, body) = rpc(&client, Some(host), &secret).await;
        assert_eq!(status, Status::Ok, "an empty allowlist accepts {host}: {body}");
    }
}
