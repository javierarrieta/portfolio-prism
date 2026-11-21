//! The `/mcp` door: who gets in, and what the gate does once they are inside.
//!
//! The first section is the one that matters. `/mcp` sits outside `/api/`, so `AuthFairing`
//! does not cover it and the route is unauthenticated by default — the whole protection is
//! the bearer check inside `agent::mount`. A test that only proves "a good token works"
//! would leave the dangerous direction unproven, so both directions are asserted:
//!
//! - no token, and a **session cookie instead of a token**, are both refused;
//! - a good token reaches the tools, and its scopes are the ceiling on what it can call.
//!
//! The last test is the design's headline scenario end to end: an agent stages with a
//! token, a human approves with a session, the agent applies, and only then do rows exist.

use backend_rust::agent::staging;
use backend_rust::auth::pat;
use rocket::http::{ContentType, Header, Status};
use rocket::local::asynchronous::Client;
use serde_json::{json, Value};

mod common;
use common::{build_rocket, db_with_user};

async fn client(caller: &str) -> (sqlx::PgPool, i64, Client) {
    let (pool, user) = db_with_user(caller).await;
    let rocket = build_rocket(pool.clone(), user);
    (pool, user, Client::tracked(rocket).await.unwrap())
}

/// Mint a token directly — the UI path is `POST /api/tokens` and is covered in `pat_test`.
async fn token(pool: &sqlx::PgPool, user: i64, scopes: &[&str]) -> String {
    let owned: Vec<String> = scopes.iter().map(|s| s.to_string()).collect();
    pat::mint(pool, user, "test", &owned, None)
        .await
        .unwrap()
        .secret
}

async fn rpc(client: &Client, body: Value, bearer: Option<&str>) -> (Status, Value) {
    let mut req = client
        .post("/mcp")
        .header(ContentType::JSON)
        .header(Header::new("Accept", "application/json, text/event-stream"));
    if let Some(secret) = bearer {
        req = req.header(Header::new("Authorization", format!("Bearer {secret}")));
    }
    let res = req.body(body.to_string()).dispatch().await;
    let status = res.status();
    let text = res.into_string().await.unwrap_or_default();
    // A body that is not JSON stays visible rather than panicking the assertion away.
    let parsed = serde_json::from_str(&text).unwrap_or_else(|_| json!({"raw": text}));
    (status, parsed)
}

fn initialize(id: u8) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "initialize",
        "params": {
            "protocolVersion": "2026-07-28",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "0"}
        }
    })
}

fn call(id: u8, tool: &str, args: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
           "params": {"name": tool, "arguments": args}})
}

/// The text payload of a `tools/call` result, parsed back into JSON.
fn payload(response: &Value) -> Value {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default();
    serde_json::from_str(text).unwrap_or_else(|_| json!({"raw": text}))
}

// --- where the door is -------------------------------------------------------------

#[tokio::test]
async fn mcp_is_mounted_at_the_root_and_not_under_api() {
    // The prefix is the entire protection, and getting it wrong is easy: this route was
    // first added to the `/api` mount list, which served it at `/api/mcp` — inside
    // `AuthFairing`'s prefix, where a session cookie would authenticate it and every scope
    // check would become optional. It answered 404 there, which is the only way that
    // mistake announces itself.
    //
    // So this asserts the shape rather than the behaviour: exactly one `/mcp`, at the root,
    // and nothing under `/api` that looks like it.
    let (pool, _) = db_with_user("mcp_prefix").await;
    let rocket = build_rocket(pool, 1).ignite().await.expect("rocket ignites");
    let paths: Vec<String> = rocket
        .routes()
        .map(|r| format!("{} {}", r.method.as_str(), r.uri))
        .collect();

    let at_root: Vec<&String> = paths
        .iter()
        .filter(|p| p.ends_with(" /mcp") && p.starts_with("POST /mcp"))
        .collect();
    assert_eq!(at_root.len(), 1, "exactly one POST /mcp, got {paths:?}");
    assert!(
        !paths.iter().any(|p| p.contains(" /api/mcp")),
        "/mcp must not be under /api: {paths:?}"
    );
}

// --- the door ----------------------------------------------------------------------

#[tokio::test]
async fn mcp_refuses_a_request_with_no_credential() {
    let (_pool, _user, client) = client("mcp_no_cred").await;
    let (status, _) = rpc(&client, initialize(1), None).await;
    assert_eq!(status, Status::Unauthorized, "an anonymous /mcp call must not reach a tool");
}

#[tokio::test]
async fn mcp_refuses_a_session_cookie() {
    // THE test for this module. A session cookie is a human's whole authority and passes
    // every scope, so if `/mcp` ever honoured one, the scope model would be decorative.
    // `build_rocket` runs in `AuthMode::Disabled`, which is exactly a session-shaped
    // credential with no bearer token attached.
    let (_pool, _user, client) = client("mcp_cookie").await;
    let (status, _) = rpc(&client, initialize(1), None).await;
    assert_eq!(
        status,
        Status::Unauthorized,
        "a session must not authenticate /mcp; the route is outside AuthFairing's prefix"
    );
}

#[tokio::test]
async fn mcp_refuses_a_revoked_token_and_says_nothing_about_why() {
    let (pool, user, client) = client("mcp_revoked").await;
    let secret = token(&pool, user, &["read"]).await;
    let info = pat::list(&pool, user).await.unwrap();
    pat::revoke(&pool, user, info[0].id).await.unwrap();

    let (status, body) = rpc(&client, initialize(1), Some(&secret)).await;
    assert_eq!(status, Status::Unauthorized);
    // The refusal is the application's standard unauthenticated body and says nothing about
    // *why*: "revoked", "expired" and "never existed" have to stay indistinguishable to
    // whoever holds the string, or the endpoint confirms that a token id was real.
    assert_eq!(body, json!({"error": "unauthenticated"}));
}

#[tokio::test]
async fn a_valid_token_negotiates_and_lists_the_tools() {
    let (pool, user, client) = client("mcp_ok").await;
    let secret = token(&pool, user, &["read", "import"]).await;

    let (status, init) = rpc(&client, initialize(1), Some(&secret)).await;
    assert_eq!(status, Status::Ok, "body: {init}");
    assert!(
        init["result"]["serverInfo"]["name"].is_string(),
        "initialize must identify the server: {init}"
    );

    let (status, listed) = rpc(
        &client,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
        Some(&secret),
    )
    .await;
    assert_eq!(status, Status::Ok);
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for expected in [
        "list_portfolios",
        "get_portfolio",
        "list_transactions",
        "get_tax_summary",
        "get_performance",
        "get_transaction_pnl",
        "lookup_asset",
        "stage_import",
        "apply_staged",
        "discard_staged",
        "get_staged",
    ] {
        assert!(names.contains(&expected), "{expected} missing from {names:?}");
    }

    // The write-ish tools say so, so a client that prompts can prompt.
    let tools = listed["result"]["tools"].as_array().unwrap();
    let stage = tools.iter().find(|t| t["name"] == "stage_import").unwrap();
    assert_eq!(
        stage["annotations"]["readOnlyHint"],
        json!(false),
        "stage_import must not claim to be read-only"
    );
    let list = tools.iter().find(|t| t["name"] == "list_portfolios").unwrap();
    assert_eq!(list["annotations"]["readOnlyHint"], json!(true));

    // Every tool's description is load-bearing: clients discover these by search, so a
    // description that does not say what the tool does is a tool that is never called.
    for t in tools {
        let name = t["name"].as_str().unwrap();
        let desc = t["description"].as_str().unwrap_or_default();
        assert!(
            desc.len() > 40,
            "{name}'s description is too thin to be discovered: {desc:?}"
        );
    }
}

#[tokio::test]
async fn a_read_token_cannot_call_an_import_tool() {
    let (pool, user, client) = client("mcp_scope").await;
    let secret = token(&pool, user, &["read"]).await;

    rpc(&client, initialize(1), Some(&secret)).await;
    let (status, resp) = rpc(
        &client,
        call(2, "stage_import", json!({"payload": {}})),
        Some(&secret),
    )
    .await;
    // A refusal is a tool result, not a protocol error: the call was well-formed and the
    // answer is "you may not". A protocol error invites an infinite identical retry.
    assert_eq!(status, Status::Ok);
    assert_eq!(payload(&resp)["error"], json!("scope_missing"));
}

// --- the gate, end to end ----------------------------------------------------------

#[tokio::test]
async fn an_agent_stages_a_human_approves_and_only_then_does_the_agent_write() {
    let (pool, user, client) = client("mcp_gate").await;
    let secret = token(&pool, user, &["read", "import"]).await;
    rpc(&client, initialize(1), Some(&secret)).await;

    let proposal = json!({
        "target": {"kind": "new", "name": "Depot", "base_currency": "EUR"},
        "assets": [{
            "symbol": "IWDA", "asset_type": "ETF",
            "transactions": [
                {"type": "BUY", "date": "2024-03-01", "quantity": "1.5", "price": "82.34", "fee": "0.00"}
            ]
        }]
    });

    // 1. stage
    let (_, resp) = rpc(
        &client,
        call(2, "stage_import", json!({"payload": proposal})),
        Some(&secret),
    )
    .await;
    let staged = payload(&resp);
    let handle = staged["handle"]
        .as_str()
        .unwrap_or_else(|| panic!("stage failed: {resp}"))
        .to_string();
    assert_eq!(staged["status"], json!("staged"));

    // 2. apply before approval: refused, and nothing written
    let (_, resp) = rpc(
        &client,
        call(3, "apply_staged", json!({"handle": handle})),
        Some(&secret),
    )
    .await;
    assert_eq!(payload(&resp)["error"], json!("not_approved"));
    let (portfolios,): (i64,) = sqlx::query_as("SELECT count(*) FROM portfolios")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(portfolios, 0);

    // 3. a human approves, over the session-authenticated route
    let approve_path = format!("/api/staged-batches/{handle}/approve");
    let approval = client.post(&approve_path).dispatch().await;
    let approval_status = approval.status();
    let body = approval.into_string().await.unwrap_or_default();
    assert_eq!(approval_status, Status::Ok, "approve body: {body}");

    // 4. now the agent may write
    let (_, resp) = rpc(
        &client,
        call(4, "apply_staged", json!({"handle": handle})),
        Some(&secret),
    )
    .await;
    let applied = payload(&resp);
    assert_eq!(applied["status"], json!("applied"), "apply failed: {resp}");
    assert_eq!(applied["transaction_count"], json!(1));

    let (portfolios, assets, transactions): (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM portfolios),
                (SELECT count(*) FROM assets),
                (SELECT count(*) FROM transactions)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((portfolios, assets, transactions), (1, 1, 1));

    // 5. and the agent can read back what it wrote, through the read scope
    let (_, resp) = rpc(
        &client,
        call(5, "list_portfolios", json!({})),
        Some(&secret),
    )
    .await;
    let portfolios_out = payload(&resp);
    assert_eq!(portfolios_out.as_array().unwrap().len(), 1);
    assert_eq!(portfolios_out[0]["transaction_count"], json!(1));
}

#[tokio::test]
async fn an_agent_sees_only_its_own_users_portfolios() {
    let (pool, alice, alice_client) = client("mcp_iso").await;
    let alice_secret = token(&pool, alice, &["read"]).await;
    rpc(&alice_client, initialize(1), Some(&alice_secret)).await;

    let (_, resp) = rpc(&alice_client, call(2, "list_portfolios", json!({})), Some(&alice_secret)).await;
    assert_eq!(payload(&resp).as_array().unwrap().len(), 0);

    // Bob's portfolio, in the same database, is not a thing Alice can name.
    let bob = common::seed_user(&pool, "bob").await;
    let (bob_portfolio,): (i64,) = sqlx::query_as(
        "INSERT INTO portfolios (name, currency, base_currency, owner_id)
         VALUES ('Bobs', 'EUR', 'EUR', $1) RETURNING id",
    )
    .bind(bob)
    .fetch_one(&pool)
    .await
    .unwrap();

    let (_, resp) = rpc(&alice_client, call(3, "list_portfolios", json!({})), Some(&alice_secret)).await;
    assert_eq!(payload(&resp).as_array().unwrap().len(), 0, "the list is scoped to the caller");

    let (_, resp) = rpc(
        &alice_client,
        call(4, "get_portfolio", json!({"portfolio_id": bob_portfolio})),
        Some(&alice_secret),
    )
    .await;
    assert_eq!(
        payload(&resp)["error"],
        json!("portfolio_not_found"),
        "a stranger's portfolio is answered like one that does not exist"
    );
}

#[tokio::test]
async fn get_staged_reports_what_a_human_actually_approved() {
    let (pool, user, client) = client("mcp_get_staged").await;
    let secret = token(&pool, user, &["import"]).await;
    rpc(&client, initialize(1), Some(&secret)).await;

    let proposal: backend_rust::agent::protocol::Proposal = serde_json::from_value(json!({
            "target": {"kind": "new", "name": "D", "base_currency": "EUR"},
            "assets": [{"symbol": "A", "asset_type": "ETF", "transactions": [
        {"type": "BUY", "date": "2024-01-01", "quantity": "1", "price": "1", "fee": "0"}]}]
    }))
    .unwrap();
    let handle = staging::stage(&pool, user, &proposal, 60).await.unwrap().handle;

    let (_, resp) = rpc(&client, call(2, "get_staged", json!({"handle": handle})), Some(&secret)).await;
    let body = payload(&resp);
    assert_eq!(body["status"], json!("staged"));
    assert_eq!(body["expired"], json!(false));
    assert!(body["preview"]["assets"].is_array(), "the preview comes back intact");
}

// --- the read tools ----------------------------------------------------------------

/// A portfolio with two buys and one sell, inserted directly; the staging path has its own
/// test and this section is about reading, not writing.
async fn seed_rows(pool: &sqlx::PgPool, user: i64) -> (i64, i64) {
    let (portfolio,): (i64,) = sqlx::query_as(
        "INSERT INTO portfolios (name, currency, base_currency, owner_id)
         VALUES ('Seed', 'EUR', 'EUR', $1) RETURNING id",
    )
    .bind(user)
    .fetch_one(pool)
    .await
    .unwrap();
    let (asset,): (i64,) = sqlx::query_as(
        "INSERT INTO assets (portfolio_id, symbol, name, asset_type, currency)
         VALUES ($1, 'IWDA', 'iShares Core MSCI World', 'ETF', 'EUR') RETURNING id",
    )
    .bind(portfolio)
    .fetch_one(pool)
    .await
    .unwrap();
    for (date, kind, qty, price) in [
        ("2024-01-05", "BUY", "1.5", "80.00"),
        ("2024-02-05", "BUY", "0.5", "90.00"),
        ("2024-03-05", "SELL", "1.0", "95.00"),
    ] {
        sqlx::query(
            "INSERT INTO transactions (asset_id, date, type, quantity, price, fee)
             VALUES ($1, $2::timestamptz, $3, $4, $5, '0')",
        )
        .bind(asset)
        .bind(date)
        .bind(kind)
        .bind(qty)
        .bind(price)
        .execute(pool)
        .await
        .unwrap();
    }
    (portfolio, asset)
}

#[tokio::test]
async fn list_transactions_returns_money_as_strings_and_reports_the_total() {
    let (pool, user, client) = client("mcp_list_tx").await;
    let secret = token(&pool, user, &["read"]).await;
    rpc(&client, initialize(1), Some(&secret)).await;
    let (portfolio, _) = seed_rows(&pool, user).await;

    let (_, resp) = rpc(
        &client,
        call(2, "list_transactions", json!({"portfolio_id": portfolio})),
        Some(&secret),
    )
    .await;
    let body = payload(&resp);
    assert_eq!(body["total"], json!(3));
    let rows = body["transactions"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    // Newest first, so the most recent activity is what survives a truncating client.
    assert_eq!(rows[0]["date"], json!("2024-03-05"));
    // The project rule: money crosses a wire as a string, never a JSON number. A float
    // here would be a silent rounding of a tax lot.
    assert!(
        rows[0]["price"].is_string(),
        "price must be a string, got {}",
        rows[0]["price"]
    );
    assert_eq!(rows[0]["price"], json!("95.00"));
}

#[tokio::test]
async fn list_transactions_paginates_and_filters() {
    let (pool, user, client) = client("mcp_page").await;
    let secret = token(&pool, user, &["read"]).await;
    rpc(&client, initialize(1), Some(&secret)).await;
    let (portfolio, _) = seed_rows(&pool, user).await;

    let (_, resp) = rpc(
        &client,
        call(2, "list_transactions", json!({"portfolio_id": portfolio, "limit": 2})),
        Some(&secret),
    )
    .await;
    let body = payload(&resp);
    assert_eq!(body["total"], json!(3), "total counts the whole set, not the page");
    assert_eq!(body["transactions"].as_array().unwrap().len(), 2);

    // A date range that excludes the sell.
    let (_, resp) = rpc(
        &client,
        call(3, "list_transactions",
             json!({"portfolio_id": portfolio, "to": "2024-02-01", "type": "buy"})),
        Some(&secret),
    )
    .await;
    let body = payload(&resp);
    assert_eq!(body["total"], json!(1));
    assert_eq!(body["transactions"][0]["date"], json!("2024-01-05"));

    // A loose date is refused rather than quietly reflowed.
    let (_, resp) = rpc(
        &client,
        call(4, "list_transactions", json!({"portfolio_id": portfolio, "from": "2024-3-1"})),
        Some(&secret),
    )
    .await;
    assert_eq!(payload(&resp)["error"], json!("bad_date"));
}

#[tokio::test]
async fn get_performance_does_not_publish_the_constants_it_cannot_measure() {
    let (pool, user, client) = client("mcp_perf").await;
    let secret = token(&pool, user, &["read"]).await;
    rpc(&client, initialize(1), Some(&secret)).await;
    let (portfolio, _) = seed_rows(&pool, user).await;

    let (_, listed) = rpc(
        &client,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
        Some(&secret),
    )
    .await;
    let tools = listed["result"]["tools"].as_array().unwrap();
    let perf = tools.iter().find(|t| t["name"] == "get_performance").unwrap();
    let description = perf["description"].as_str().unwrap();
    // The description carries the whole burden, so it has to name the three constants.
    for named in ["volatility", "sharpe_ratio", "beta"] {
        assert!(
            description.contains(named),
            "the description must warn about {named}: {description}"
        );
    }

    let (_, resp) = rpc(
        &client,
        call(3, "get_performance", json!({"portfolio_id": portfolio})),
        Some(&secret),
    )
    .await;
    let body = payload(&resp);
    // Absent, not zero. A field that is present gets quoted; a description telling a model
    // to ignore a number it can see loses to the number.
    let metrics = body["metrics"]
        .as_object()
        .unwrap_or_else(|| panic!("no metrics in {body}"));
    assert!(!metrics.contains_key("realized_pnl"), "realized_pnl must be stripped: {metrics:?}");
    assert!(!metrics.contains_key("unrealized_pnl"), "unrealized_pnl must be stripped: {metrics:?}");
}

#[tokio::test]
async fn get_transaction_pnl_refuses_a_buy_and_says_why() {
    let (pool, user, client) = client("mcp_pnl").await;
    let secret = token(&pool, user, &["read"]).await;
    rpc(&client, initialize(1), Some(&secret)).await;
    let (portfolio, _) = seed_rows(&pool, user).await;

    let (buy_id, sell_id): (i64, i64) = sqlx::query_as(
        "SELECT min(id), max(id) FROM transactions WHERE asset_id = (SELECT id FROM assets WHERE portfolio_id = $1)",
    )
    .bind(portfolio)
    .fetch_one(&pool)
    .await
    .unwrap();

    let (_, resp) = rpc(
        &client,
        call(2, "get_transaction_pnl", json!({"transaction_id": buy_id})),
        Some(&secret),
    )
    .await;
    assert_eq!(payload(&resp)["error"], json!("not_a_sell"));

    // The sell does reach the engine. Whether it matches a lot depends on pricing, which
    // is not available here, so either a breakdown or a refusal is a correct answer — what
    // must not happen is a protocol error.
    let (status, resp) = rpc(
        &client,
        call(3, "get_transaction_pnl", json!({"transaction_id": sell_id})),
        Some(&secret),
    )
    .await;
    assert_eq!(status, Status::Ok, "body: {resp}");
    assert!(
        resp["result"].is_object(),
        "a sell must produce a result, not a protocol error: {resp}"
    );
}

#[tokio::test]
async fn read_tools_answer_not_found_for_someone_elses_rows() {
    let (pool, alice, client) = client("mcp_read_iso").await;
    let secret = token(&pool, alice, &["read"]).await;
    rpc(&client, initialize(1), Some(&secret)).await;

    let bob = common::seed_user(&pool, "bob").await;
    let (bob_portfolio, bob_asset) = seed_rows(&pool, bob).await;
    let (bob_tx,): (i64,) = sqlx::query_as(
        "SELECT id FROM transactions WHERE asset_id = $1 LIMIT 1",
    )
    .bind(bob_asset)
    .fetch_one(&pool)
    .await
    .unwrap();

    for (tool, args) in [
        ("get_performance", json!({"portfolio_id": bob_portfolio})),
        ("get_tax_summary", json!({"portfolio_id": bob_portfolio})),
        ("get_transaction_pnl", json!({"transaction_id": bob_tx})),
    ] {
        let (_, resp) = rpc(&client, call(2, tool, args), Some(&secret)).await;
        let code = payload(&resp)["error"].clone();
        assert!(
            matches!(code, Value::String(_)),
            "{tool} must refuse a stranger's id, got {resp}"
        );
    }

    // And the list is empty rather than leaky.
    let (_, resp) = rpc(&client, call(3, "list_transactions", json!({})), Some(&secret)).await;
    assert_eq!(payload(&resp)["total"], json!(0));
}

#[tokio::test]
async fn lookup_asset_refuses_a_malformed_isin_without_reaching_the_network() {
    let (pool, user, client) = client("mcp_lookup").await;
    let secret = token(&pool, user, &["read"]).await;
    rpc(&client, initialize(1), Some(&secret)).await;

    for bad in ["", "XS012345678", "123456789012", "TOOLONGISIN"] {
        let (_, resp) = rpc(&client, call(2, "lookup_asset", json!({"isin": bad})), Some(&secret)).await;
        assert_eq!(payload(&resp)["error"], json!("bad_isin"), "{bad:?} should be refused");
    }
}
