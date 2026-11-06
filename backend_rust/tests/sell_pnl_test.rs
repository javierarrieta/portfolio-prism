mod common;

use common::{build_rocket, setup_db, seed_portfolio, seed_asset, seed_transaction_at};
use rocket::http::Status;
use rocket::local::asynchronous::Client;
use rocket::{Build, Rocket};
use rust_decimal::Decimal;

/// Money fields are full-precision Decimals whose serialized scale is a by-product of the
/// arithmetic (spec: New schemas → numeric precision). Compare values, never strings —
/// `"2000"`, `"2000.0"` and `"2000.000"` are the same number and any of them may appear.
fn dec(body: &serde_json::Value, key: &str) -> Decimal {
    body[key]
        .as_str()
        .unwrap_or_default()
        .parse::<Decimal>()
        .unwrap_or_else(|_| panic!("{key} is not a decimal string: {}", body[key]))
}

fn d(s: &str) -> Decimal {
    s.parse::<Decimal>().unwrap()
}

// FIFO fixture: two buys, one sell of 80 units split across both lots.
// Returns the unlaunched Rocket; each test wraps it in its own tracked Client.
async fn fifo_fixture() -> (Rocket<Build>, i32, i32, i32) {
    let pool = setup_db().await;
    let port_id = seed_portfolio(&pool, "Sell PnL", "USD").await;
    let asset_id = seed_asset(&pool, port_id, "AAPL", "Apple").await;
    let buy1 = seed_transaction_at(&pool, asset_id, "BUY", 60.0, 100.0, 0.0, "2024-01-01T00:00:00Z").await;
    let buy2 = seed_transaction_at(&pool, asset_id, "BUY", 40.0, 120.0, 0.0, "2024-06-01T00:00:00Z").await;
    let sell = seed_transaction_at(&pool, asset_id, "SELL", 80.0, 130.0, 0.0, "2024-12-01T00:00:00Z").await;
    let _ = buy2;
    (build_rocket(pool), sell, buy1, asset_id)
}

#[tokio::test]
async fn test_sell_pnl_unknown_transaction_is_404() {
    let (rocket, _, _, _) = fifo_fixture().await;
    let client = Client::tracked(rocket).await.unwrap();
    let resp = client.get("/api/transactions/999999/pnl?strategy=FIFO&threshold_days=30").dispatch().await;
    assert_eq!(resp.status(), Status::NotFound);
}

#[tokio::test]
async fn test_sell_pnl_rejects_non_sell_transaction() {
    let (rocket, _, buy1, _) = fifo_fixture().await;
    let client = Client::tracked(rocket).await.unwrap();
    let resp = client.get(format!("/api/transactions/{}/pnl?strategy=FIFO", buy1)).dispatch().await;
    assert_eq!(resp.status(), Status::BadRequest);
}

#[tokio::test]
async fn test_sell_pnl_fifo_breakdown_shape() {
    let (rocket, sell, buy1, _) = fifo_fixture().await;
    let client = Client::tracked(rocket).await.unwrap();
    let resp = client.get(format!("/api/transactions/{}/pnl?strategy=FIFO&threshold_days=30", sell)).dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();

    assert_eq!(body["transaction_id"], sell);
    assert_eq!(body["symbol"], "AAPL");
    assert_eq!(body["strategy"], "FIFO");
    assert_eq!(body["threshold_days"], 30);
    assert_eq!(body["currency"], "USD");
    assert_eq!(body["asset_currency"], "USD");
    assert_eq!(body["fx_applied"], false);
    // This fixture has no SPLIT row, so the server-side flag must be false. The
    // split footnote is a response field now, not something the browser guesses.
    assert_eq!(body["split_applied"], false);
    assert_eq!(dec(&body, "sold_qty"), d("80"));
    assert_eq!(dec(&body, "price"), d("130"));
    assert_eq!(dec(&body, "realized_pnl"), d("2000"));
    assert_eq!(dec(&body, "matched_qty"), d("80"));
    assert_eq!(dec(&body, "unmatched_qty"), d("0"));
    assert_eq!(body["matches"].as_array().unwrap().len(), 2);
    assert_eq!(body["matches"][0]["buy_transaction_id"], buy1);
    assert_eq!(dec(&body["matches"][0], "matched_qty"), d("60"));
    assert_eq!(body["matches"][0]["holding_days"], 335);
    assert!(body["matches"][0]["term"].is_null(), "term is HYBRID-only");
    assert_eq!(dec(&body["matches"][1], "matched_qty"), d("20"));
    // Compare parsed Decimals, never serialized strings (review record: "Decimal
    // scale -- full precision, no quantisation"). This pins the actual quotient
    // instead of hedging on the leading digits of a 28-digit division tail.
    assert_eq!(dec(&body, "roi"), d("2000") / d("8400"));
}

#[tokio::test]
async fn test_sell_pnl_split_before_every_buy_is_not_claimed() {
    // The replay rescales lots that already exist, so a split older than every buy
    // adjusted nothing. Claiming otherwise would tell the user the table shows
    // split-adjusted prices when it shows exactly what they typed.
    let pool = setup_db().await;
    let port_id = seed_portfolio(&pool, "Split Too Early", "USD").await;
    let asset_id = seed_asset(&pool, port_id, "EARLY", "Early Split").await;
    seed_transaction_at(&pool, asset_id, "SPLIT", 2.0, 1.0, 0.0, "2023-01-01T00:00:00Z").await;
    seed_transaction_at(&pool, asset_id, "BUY", 100.0, 100.0, 0.0, "2024-01-01T00:00:00Z").await;
    let sell = seed_transaction_at(&pool, asset_id, "SELL", 100.0, 60.0, 0.0, "2024-12-01T00:00:00Z").await;
    let client = Client::tracked(build_rocket(pool)).await.unwrap();

    let resp = client.get(format!("/api/transactions/{}/pnl?strategy=FIFO", sell)).dispatch().await;
    let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();
    assert_eq!(body["split_applied"], false, "a split older than every buy adjusted no lot");
    assert_eq!(dec(&body["matches"][0], "buy_price"), d("100"));
}

#[tokio::test]
async fn test_sell_pnl_lifo_attributes_different_lots() {
    let (rocket, sell, _, _) = fifo_fixture().await;
    let client = Client::tracked(rocket).await.unwrap();
    let resp = client.get(format!("/api/transactions/{}/pnl?strategy=LIFO&threshold_days=30", sell)).dispatch().await;
    let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();
    assert_eq!(body["strategy"], "LIFO");
    assert_eq!(dec(&body, "realized_pnl"), d("1600"));
}

#[tokio::test]
async fn test_sell_pnl_defaults_to_fifo_when_params_omitted() {
    let (rocket, sell, _, _) = fifo_fixture().await;
    let client = Client::tracked(rocket).await.unwrap();
    let resp = client.get(format!("/api/transactions/{}/pnl", sell)).dispatch().await;
    let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();
    assert_eq!(body["strategy"], "FIFO");
    assert_eq!(body["threshold_days"], 30);
    assert_eq!(dec(&body, "realized_pnl"), d("2000"));
}

#[tokio::test]
async fn test_sell_pnl_unmatched_quantity_is_declared_not_dropped() {
    let pool = setup_db().await;
    let port_id = seed_portfolio(&pool, "Unmatched", "USD").await;
    let asset_id = seed_asset(&pool, port_id, "TSLA", "Tesla").await;
    seed_transaction_at(&pool, asset_id, "BUY", 50.0, 100.0, 0.0, "2024-01-01T00:00:00Z").await;
    let sell = seed_transaction_at(&pool, asset_id, "SELL", 100.0, 120.0, 0.0, "2024-12-01T00:00:00Z").await;
    let client = Client::tracked(build_rocket(pool)).await.unwrap();

    let resp = client.get(format!("/api/transactions/{}/pnl?strategy=FIFO", sell)).dispatch().await;
    let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();
    assert_eq!(dec(&body, "sold_qty"), d("100"));
    assert_eq!(dec(&body, "matched_qty"), d("50"));
    assert_eq!(dec(&body, "unmatched_qty"), d("50"));
    assert_eq!(dec(&body, "realized_pnl"), d("1000"));
}

#[tokio::test]
async fn test_sell_pnl_roi_is_null_when_cost_basis_is_zero() {
    let pool = setup_db().await;
    let port_id = seed_portfolio(&pool, "No Buys", "USD").await;
    let asset_id = seed_asset(&pool, port_id, "NOBUYS", "No Buys").await;
    let sell = seed_transaction_at(&pool, asset_id, "SELL", 50.0, 120.0, 0.0, "2024-12-01T00:00:00Z").await;
    let client = Client::tracked(build_rocket(pool)).await.unwrap();

    let resp = client.get(format!("/api/transactions/{}/pnl?strategy=FIFO", sell)).dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();
    assert!(body["roi"].is_null(), "roi must be null, never a division by zero");
    assert!(body["matches"].as_array().unwrap().is_empty());
    assert_eq!(dec(&body, "unmatched_qty"), d("50"));
    // An empty `matches` sums to `Decimal::ZERO` at scale 0, which serializes as "0" —
    // exactly what `analytics_test.rs` already asserts for `/tax-summary`. Comparing as
    // Decimals means the assertion holds whichever scale the arithmetic produces.
    assert_eq!(dec(&body, "realized_pnl"), d("0"));
}

#[tokio::test]
async fn test_sell_pnl_sums_to_tax_summary_realized_pnl() {
    // The central invariant as the user experiences it: two separate HTTP calls that must
    // agree on transaction set, base-currency resolution and strategy defaults. The
    // engine-level test cannot catch a divergence here, because within one engine call the
    // equality holds by construction.
    //
    // The order below is load-bearing, not cosmetic: /tax-summary inserts auto-detected
    // SPLIT rows and this endpoint is read-only, so the breakdown only agrees when the
    // summary has run first -- which is what the UI guarantees. Do not reorder this test
    // as a "cleanup"; that is the ordering this pins.
    let pool = setup_db().await;
    let port_id = seed_portfolio(&pool, "Reconcile", "USD").await;
    let asset_id = seed_asset(&pool, port_id, "RECON", "Reconcile").await;
    seed_transaction_at(&pool, asset_id, "BUY", 60.0, 100.0, 0.0, "2024-01-01T00:00:00Z").await;
    seed_transaction_at(&pool, asset_id, "BUY", 40.0, 120.0, 0.0, "2024-06-01T00:00:00Z").await;
    let sell1 = seed_transaction_at(&pool, asset_id, "SELL", 50.0, 130.0, 0.0, "2024-08-01T00:00:00Z").await;
    let sell2 = seed_transaction_at(&pool, asset_id, "SELL", 40.0, 150.0, 0.0, "2024-12-01T00:00:00Z").await;
    let client = Client::tracked(build_rocket(pool)).await.unwrap();

    for strategy in ["FIFO", "LIFO", "HYBRID"] {
        let tax_resp = client
            .get(format!("/api/portfolios/{}/tax-summary?strategy={}&threshold_days=30", port_id, strategy))
            .dispatch()
            .await;
        assert_eq!(tax_resp.status(), Status::Ok);
        let tax: serde_json::Value = serde_json::from_str(&tax_resp.into_string().await.unwrap()).unwrap();
        let asset = tax["assets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["symbol"] == "RECON")
            .expect("asset present in tax summary");
        // realized_pnl is not price-derived, so unlike market_value it is stable here.
        let asset_realized = asset["realized_pnl"]
            .as_str()
            .unwrap()
            .parse::<Decimal>()
            .unwrap();

        let mut sum = Decimal::ZERO;
        for sell in [sell1, sell2] {
            let resp = client
                .get(format!("/api/transactions/{}/pnl?strategy={}&threshold_days=30", sell, strategy))
                .dispatch()
                .await;
            assert_eq!(resp.status(), Status::Ok, "{} sell {}", strategy, sell);
            let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();
            sum += dec(&body, "realized_pnl");
        }
        assert_eq!(
            sum, asset_realized,
            "{strategy}: per-sell realized P&L must sum to the asset total the Holdings tab shows"
        );
    }
}
#[tokio::test]
async fn test_sell_pnl_unrecognized_strategy_is_applied_as_fifo() {
    // Done Criterion 4. The response's `strategy` is what the modal renders as the
    // attribution method, so it must name the method actually used -- not echo back
    // whatever the caller typed. FIFO and LIFO differ on this fixture (2000 vs
    // 1600), so the figure proves which one ran.
    let pool = setup_db().await;
    let port_id = seed_portfolio(&pool, "Strategy Echo", "USD").await;
    let asset_id = seed_asset(&pool, port_id, "AAPL", "Apple").await;
    seed_transaction_at(&pool, asset_id, "BUY", 60.0, 100.0, 0.0, "2024-01-01T00:00:00Z").await;
    seed_transaction_at(&pool, asset_id, "BUY", 40.0, 120.0, 0.0, "2024-06-01T00:00:00Z").await;
    let sell = seed_transaction_at(&pool, asset_id, "SELL", 80.0, 130.0, 0.0, "2024-12-01T00:00:00Z").await;
    let client = Client::tracked(build_rocket(pool)).await.unwrap();

    let resp = client.get(format!("/api/transactions/{}/pnl?strategy=moon-time&threshold_days=30", sell)).dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();
    assert_eq!(body["strategy"], "FIFO", "an unrecognized value must not come back naming a method that did not run");
    assert_eq!(dec(&body, "realized_pnl"), d("2000"), "FIFO figure, not LIFO's 1600");
}

async fn split_fixture(split_date: &str) -> (Client, i32) {
    let pool = setup_db().await;
    let port_id = seed_portfolio(&pool, "Split Flag", "USD").await;
    let asset_id = seed_asset(&pool, port_id, "AAPL", "Apple").await;
    seed_transaction_at(&pool, asset_id, "BUY", 100.0, 100.0, 0.0, "2024-01-01T00:00:00Z").await;
    seed_transaction_at(&pool, asset_id, "SPLIT", 2.0, 1.0, 0.0, split_date).await;
    let sell = seed_transaction_at(&pool, asset_id, "SELL", 100.0, 60.0, 0.0, "2024-12-01T00:00:00Z").await;
    let client = Client::tracked(build_rocket(pool)).await.unwrap();
    (client, sell)
}

async fn split_applied_at(split_date: &str) -> serde_json::Value {
    let (client, sell) = split_fixture(split_date).await;
    let resp = client.get(format!("/api/transactions/{}/pnl?strategy=FIFO", sell)).dispatch().await;
    assert_eq!(resp.status(), Status::Ok);
    serde_json::from_str(&resp.into_string().await.unwrap()).unwrap()
}

#[tokio::test]
async fn test_sell_pnl_split_before_sell_sets_split_applied() {
    // The `true` branch of split_applied had no endpoint coverage: the modal's own
    // test drives the payload directly, so the server-side computation was untested.
    let body = split_applied_at("2024-06-01T00:00:00Z").await;
    assert_eq!(body["split_applied"], true);
    // 2:1 split halves the recorded buy price; the footnote exists because the
    // table then shows a price nobody typed into a form.
    assert_eq!(dec(&body["matches"][0], "buy_price"), d("50"));
}

#[tokio::test]
async fn test_sell_pnl_split_on_the_sell_date_counts() {
    // Pins the `<=` boundary. Splits sort ahead of same-day sells in the replay, so
    // a same-date split has already been applied to the lots this sell consumed.
    let body = split_applied_at("2024-12-01T00:00:00Z").await;
    assert_eq!(body["split_applied"], true);
    assert_eq!(dec(&body["matches"][0], "buy_price"), d("50"));
}

#[tokio::test]
async fn test_sell_pnl_split_after_the_sell_does_not_count() {
    // Pins the other side of `<=`: a split that happened later did not touch the
    // prices this sell was computed from, so claiming it did would be a lie.
    let body = split_applied_at("2025-01-01T00:00:00Z").await;
    assert_eq!(body["split_applied"], false);
    assert_eq!(dec(&body["matches"][0], "buy_price"), d("100"));
}
