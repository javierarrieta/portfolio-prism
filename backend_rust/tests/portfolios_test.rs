mod common;

use common::{build_rocket, db_with_user, seed_asset, seed_portfolio};
use rocket::http::Status;
use rocket::local::asynchronous::Client;
use rocket::serde::json::Json;

#[tokio::test]
async fn test_create_portfolio() {
    let (pool, user) = db_with_user("test_create_portfolio").await;
    let rocket = build_rocket(pool, user);
    let client = Client::tracked(rocket).await.unwrap();

    let body = Json(serde_json::json!({
        "name": "Test Portfolio",
        "description": null,
        "currency": "USD"
    }));

    let resp = client.post("/api/portfolios/")
        .header(rocket::http::ContentType::JSON)
        .body(body.to_string())
        .dispatch()
        .await;

    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    assert_eq!(parsed["name"], "Test Portfolio");
    assert_eq!(parsed["currency"], "USD");
    assert!(parsed["assets"].is_array());
    assert_eq!(parsed["assets"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_list_portfolios() {
    let (pool, user) = db_with_user("test_list_portfolios").await;
    let id = seed_portfolio(&pool, user, "List Test", "USD").await;
    let rocket = build_rocket(pool, user);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.get("/api/portfolios/").dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&body_str).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0]["id"], id);
    assert!(parsed[0]["assets"].is_array());
}

#[tokio::test]
async fn test_create_portfolio_with_description() {
    let (pool, user) = db_with_user("test_create_portfolio_with_description").await;
    let rocket = build_rocket(pool, user);
    let client = Client::tracked(rocket).await.unwrap();

    let body = Json(serde_json::json!({
        "name": "Desc Test",
        "description": "A portfolio with description",
        "currency": "USD"
    }));

    let resp = client.post("/api/portfolios/")
        .header(rocket::http::ContentType::JSON)
        .body(body.to_string())
        .dispatch()
        .await;

    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    assert_eq!(parsed["name"], "Desc Test");
    assert_eq!(parsed["description"], "A portfolio with description");
    assert!(parsed["assets"].is_array());
}

#[tokio::test]
async fn test_list_portfolios_empty_db() {
    let (pool, user) = db_with_user("test_list_portfolios_empty_db").await;
    let rocket = build_rocket(pool, user);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.get("/api/portfolios/").dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&body_str).unwrap();
    assert_eq!(parsed.len(), 0);
}

#[tokio::test]
async fn test_list_portfolios_multi_asset() {
    let (pool, user) = db_with_user("test_list_portfolios_multi_asset").await;
    let id = seed_portfolio(&pool, user, "Multi Asset", "USD").await;
    seed_asset(&pool, id, "AAPL", "Apple").await;
    seed_asset(&pool, id, "GOOGL", "Google").await;
    seed_asset(&pool, id, "MSFT", "Microsoft").await;
    let rocket = build_rocket(pool, user);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.get("/api/portfolios/").dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&body_str).unwrap();
    let portfolio = parsed.iter().find(|p| p["id"].as_i64().unwrap() == id).unwrap();
    let assets = portfolio["assets"].as_array().unwrap();
    assert_eq!(assets.len(), 3);
    let symbols: Vec<&serde_json::Value> = assets.iter().map(|a| &a["symbol"]).collect();
    assert!(symbols.contains(&&serde_json::json!("AAPL")));
    assert!(symbols.contains(&&serde_json::json!("GOOGL")));
    assert!(symbols.contains(&&serde_json::json!("MSFT")));
}

#[tokio::test]
async fn test_get_portfolio_by_id() {
    let (pool, user) = db_with_user("test_get_portfolio_by_id").await;
    let id = seed_portfolio(&pool, user, "Get Test", "EUR").await;
    let rocket = build_rocket(pool, user);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.get(format!("/api/portfolios/{}", id)).dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    assert_eq!(parsed["name"], "Get Test");
    assert_eq!(parsed["currency"], "EUR");
    assert!(parsed["assets"].is_array());
}

#[tokio::test]
async fn test_get_portfolio_not_found() {
    let (pool, user) = db_with_user("test_get_portfolio_not_found").await;
    let rocket = build_rocket(pool, user);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.get("/api/portfolios/9999").dispatch().await;
    assert_eq!(resp.status(), Status::NotFound);
}

#[tokio::test]
async fn test_delete_portfolio() {
    let (pool, user) = db_with_user("test_delete_portfolio").await;
    let id = seed_portfolio(&pool, user, "Delete Test", "USD").await;
    let rocket = build_rocket(pool, user);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.delete(format!("/api/portfolios/{}", id)).dispatch().await;
    assert_eq!(resp.status(), Status::NoContent);

    // Verify it's gone
    let resp = client.get(format!("/api/portfolios/{}", id)).dispatch().await;
    assert_eq!(resp.status(), Status::NotFound);
}

#[tokio::test]
async fn test_delete_portfolio_with_assets_returns_conflict() {
    let (pool, user) = db_with_user("test_delete_portfolio_with_assets_returns_conflict").await;
    let id = seed_portfolio(&pool, user, "Delete With Assets", "USD").await;
    seed_asset(&pool, id, "AAPL", "Apple").await;
    let rocket = build_rocket(pool, user);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.delete(format!("/api/portfolios/{}", id)).dispatch().await;
    assert_eq!(resp.status(), Status::Conflict);

    // Verify portfolio still exists
    let resp = client.get(format!("/api/portfolios/{}", id)).dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
}

#[tokio::test]
async fn test_delete_portfolio_not_found() {
    let (pool, user) = db_with_user("test_delete_portfolio_not_found").await;
    let rocket = build_rocket(pool, user);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.delete("/api/portfolios/9999").dispatch().await;
    assert_eq!(resp.status(), Status::NotFound);
}

#[tokio::test]
async fn test_update_portfolio_currency() {
    let (pool, user) = db_with_user("test_update_portfolio_currency").await;
    let id = seed_portfolio(&pool, user, "Update Cur", "USD").await;
    let rocket = build_rocket(pool, user);
    let client = Client::tracked(rocket).await.unwrap();

    let body = Json(serde_json::json!({
        "currency": "EUR"
    }));

    let resp = client.patch(format!("/api/portfolios/{}", id))
        .header(rocket::http::ContentType::JSON)
        .body(body.to_string())
        .dispatch()
        .await;

    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    assert_eq!(parsed["currency"], "EUR");
    assert!(parsed["assets"].is_array());

    // Verify the change persisted
    let resp = client.get(format!("/api/portfolios/{}", id)).dispatch().await;
    let body_str = resp.into_string().await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    assert_eq!(parsed["currency"], "EUR");
    assert!(parsed["assets"].is_array());
}

#[tokio::test]
async fn test_update_portfolio_not_found() {
    let (pool, user) = db_with_user("test_update_portfolio_not_found").await;
    let rocket = build_rocket(pool, user);
    let client = Client::tracked(rocket).await.unwrap();

    let body = Json(serde_json::json!({
        "currency": "EUR"
    }));

    let resp = client.patch("/api/portfolios/9999")
        .header(rocket::http::ContentType::JSON)
        .body(body.to_string())
        .dispatch()
        .await;

    assert_eq!(resp.status(), Status::NotFound);
}

#[tokio::test]
async fn test_list_portfolios_includes_assets() {
    let (pool, user) = db_with_user("test_list_portfolios_includes_assets").await;
    let id = seed_portfolio(&pool, user, "Asset Test", "USD").await;
    seed_asset(&pool, id, "AAPL", "Apple Inc").await;
    seed_asset(&pool, id, "BTC-USD", "Bitcoin").await;
    let rocket = build_rocket(pool, user);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.get("/api/portfolios/").dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&body_str).unwrap();
    let portfolio = parsed.iter().find(|p| p["id"].as_i64().unwrap() == id).unwrap();
    assert_eq!(portfolio["assets"].as_array().unwrap().len(), 2);
    assert_eq!(portfolio["assets"][0]["symbol"], "AAPL");
    assert_eq!(portfolio["assets"][1]["symbol"], "BTC-USD");
}

#[tokio::test]
async fn test_get_portfolio_includes_assets() {
    let (pool, user) = db_with_user("test_get_portfolio_includes_assets").await;
    let id = seed_portfolio(&pool, user, "Single Asset Test", "USD").await;
    seed_asset(&pool, id, "SPY", "SPDR S&P 500").await;
    let rocket = build_rocket(pool, user);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.get(format!("/api/portfolios/{}", id)).dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body_str = resp.into_string().await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    let assets = parsed["assets"].as_array().unwrap();
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0]["symbol"], "SPY");
}

#[tokio::test]
async fn test_portfolio_id_above_i32_max_round_trips() {
    // `BIGINT GENERATED ALWAYS AS IDENTITY` reaches this without anyone doing anything
    // exotic, and every id in the API is `i64` on purpose. This is the runtime tripwire
    // behind that width: grep and the type system say nothing is narrowed, and if a
    // future change narrows an id on the way in or out, this test is what notices.
    const BIG_ID: i64 = 2_147_483_648; // i32::MAX + 1

    let (pool, user) = db_with_user("test_portfolio_id_above_i32_max_round_trips").await;
    // `GENERATED ALWAYS` refuses a supplied id, and `OVERRIDING SYSTEM VALUE` is the
    // documented way to name one, so no sequence has to be touched to get here.
    sqlx::query(
        "INSERT INTO portfolios (id, name, description, currency, base_currency, owner_id) \
         OVERRIDING SYSTEM VALUE VALUES ($1, 'Big Id', NULL, 'USD', 'USD', $2)",
    )
    .bind(BIG_ID)
    .bind(user)
    .execute(&pool)
    .await
    .unwrap();

    let rocket = build_rocket(pool, user);
    let client = Client::tracked(rocket).await.unwrap();

    let resp = client.get(format!("/api/portfolios/{}", BIG_ID)).dispatch().await;
    assert_eq!(resp.status(), Status::Ok, "a path parameter past i32::MAX must route and resolve");
    let body: serde_json::Value = resp.into_json().await.unwrap();
    assert_eq!(body["id"].as_i64(), Some(BIG_ID), "the id must come back undiminished");

    // The list route reaches the same id through a different query and a `HashMap<i64,_>`
    // grouping, so it is pinned on the same claim.
    let resp = client.get("/api/portfolios/").dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let list: serde_json::Value = resp.into_json().await.unwrap();
    let hits = list.as_array().unwrap()
        .iter()
        .filter(|p| p["id"].as_i64() == Some(BIG_ID))
        .count();
    assert_eq!(hits, 1, "the list route must carry the same id");
}

#[tokio::test]
async fn editing_an_asset_does_not_move_it_in_the_list() {
    // The regression this pins: under SQLite an unordered scan returned rows in rowid
    // order, so an asset stayed where it was however often it was edited. Postgres writes
    // an UPDATE as a new tuple at the end of the heap, so an unordered scan returns the
    // edited row last — every edit moved an asset to the bottom of the user's list. The
    // list routes now say `ORDER BY id`, which is insertion order, which is what the old
    // order was.
    //
    // The middle asset is the one under test on purpose: first and last are indistinguishable
    // between "stayed put" and "the sort happens to agree", and moving the middle one to the
    // end is the visible breakage.
    let (pool, user) = db_with_user("editing_an_asset_does_not_move_it").await;
    let pid = seed_portfolio(&pool, user, "Order Test", "USD").await;
    let first = seed_asset(&pool, pid, "AAA", "First").await;
    let second = seed_asset(&pool, pid, "BBB", "Second").await;
    let third = seed_asset(&pool, pid, "CCC", "Third").await;
    let rocket = build_rocket(pool.clone(), user);
    let client = Client::tracked(rocket).await.unwrap();

    let res = client
        .patch(format!("/api/portfolios/{pid}/assets/{second}"))
        .header(rocket::http::ContentType::JSON)
        .body(r#"{"currency":"EUR"}"#)
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok, "the edit itself must succeed");

    // The edit landed, so the tuple really moved and the assertions below are about the
    // ordering rather than about an UPDATE that did nothing.
    let edited = sqlx::query_scalar::<_, String>("SELECT currency FROM assets WHERE id = $1")
        .bind(second)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(edited, "EUR");

    let expected = [first, second, third];

    let resp = client.get(format!("/api/portfolios/{pid}")).dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body: serde_json::Value = resp.into_json().await.unwrap();
    assert_eq!(
        body["assets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["symbol"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["AAA", "BBB", "CCC"],
        "the edited asset must sit between its neighbours, not after both of them"
    );
    assert_eq!(
        body["assets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["id"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        expected
    );

    // The dashboard list is the other route that groups assets by portfolio, and it
    // reaches them through a different statement, so it needs pinning separately.
    let resp = client.get("/api/portfolios/").dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body: Vec<serde_json::Value> = resp.into_json().await.unwrap();
    let portfolio = body
        .iter()
        .find(|p| p["id"].as_i64().unwrap() == pid)
        .expect("the seeded portfolio is listed");
    assert_eq!(
        portfolio["assets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["id"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        expected,
        "GET /api/portfolios/ must order assets the same way GET /api/portfolios/<id> does"
    );
}
