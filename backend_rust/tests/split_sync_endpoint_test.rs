//! Split sync is an explicit action, and the write is guarded by a constraint (#50).
//!
//! Two separate defects lived here. `GET /tax-summary` and `GET /performance` wrote
//! `SPLIT` transactions as a side effect of being read, which meant a page view was a
//! mutation and a provider outage had no status code of its own. And the dedupe inside
//! `apply_split_events` was a `SELECT COUNT(*)` followed by an `INSERT` with no
//! transaction and nothing behind it, so two concurrent callers could both see zero
//! and both write — doubling a split adjustment and silently corrupting cost basis.
//!
//! The first is fixed by moving the write to a POST. The second is fixed by a unique
//! index, because a check-then-write cannot be made safe by tightening the check; it is
//! safe only when a second writer physically cannot land.
//!
//! All hermetic: the provider stand-in serves a canned split event.

mod common;

use backend_rust::services::currency_service::apply_split_events;
use common::{
    MockYahoo,
    build_rocket_pricing,
    db_with_user,
    seed_asset,
    seed_portfolio,
    seed_split_sync,
    seed_transaction_at,
};
use rocket::http::{ContentType, Status};
use rocket::local::asynchronous::Client;
use backend_rust::db::DbPool;

/// A 2:1 split on 2024-06-01, in Yahoo's chart envelope.
const SPLIT_ME: &str = r#"{"chart":{"result":[{"timestamp":[],"indicators":{"quote":[{"close":[]}]},"events":{"splits":{"1717200000":{"date":1717200000,"numerator":2.0,"denominator":1.0,"splitRatio":"2:1"}}}}],"error":null}}"#;

const SYMBOL: &str = "SPLITME";

async fn split_rows(pool: &DbPool, asset_id: i64) -> Vec<String> {
    sqlx::query_as::<_, (String,)>(
        "SELECT ((date AT TIME ZONE 'UTC')::date)::text \
         FROM transactions WHERE asset_id = $1 AND type = 'SPLIT' \
         ORDER BY ((date AT TIME ZONE 'UTC')::date)",
    )
    .bind(asset_id)
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .map(|(d,)| d)
    .collect()
}

async fn portfolio_payload(client: &Client, id: i64) -> serde_json::Value {
    let resp = client
        .get(format!("/api/portfolios/{id}"))
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Ok);
    serde_json::from_str(&resp.into_string().await.unwrap()).unwrap()
}

/// The headline claim of #50: reading a portfolio no longer writes to it.
///
/// Before this change this was the *only* way splits got recorded in normal use — the
/// undated tax summary is what `App.tsx` fetches on every portfolio open. Asserting
/// the GET writes nothing is what stops the write creeping back onto a read path
/// because it is convenient.
#[tokio::test]
async fn a_get_no_longer_writes_split_rows() {
    let (pool, user) = db_with_user("a_get_no_longer_writes_split_rows").await;
    let port = seed_portfolio(&pool, user, "Read only", "USD").await;
    let asset = seed_asset(&pool, port, SYMBOL, "Split me").await;
    seed_transaction_at(&pool, asset, "BUY", 10.0, 60.0, 0.0, "2024-01-01T00:00:00Z").await;

    let yahoo = MockYahoo::serving([(SYMBOL, SPLIT_ME)]);
    let client = Client::tracked(build_rocket_pricing(pool.clone(), user, yahoo.base_url()))
        .await
        .unwrap();

    for path in [
        format!("/api/portfolios/{port}/tax-summary?strategy=FIFO&threshold_days=30"),
        format!("/api/portfolios/{port}/performance"),
    ] {
        let resp = client.get(&path).dispatch().await;
        assert_eq!(resp.status(), Status::Ok, "{path} should still succeed");
    }

    assert!(
        split_rows(&pool, asset).await.is_empty(),
        "a GET must not insert split rows; the write belongs to POST /sync-splits"
    );
    // Scoped to split calls, not to the provider in general. A GET reading prices is
    // fine and expected — it is the *write* that had to leave the read path. The split
    // lookup is the one request shape that proves the sync ran (`events=splits`, see
    // `CurrencyService::fetch_split_events`), so that is what must be absent.
    let split_calls: Vec<String> = yahoo
        .paths()
        .into_iter()
        .filter(|p| p.contains("events=splits"))
        .collect();
    assert!(
        split_calls.is_empty(),
        "a GET must not look up splits, got {split_calls:?}"
    );
}

#[tokio::test]
async fn post_sync_splits_records_the_split_and_reports_it() {
    let (pool, user) = db_with_user("post_sync_splits_records_the_split_and_reports_it").await;
    let port = seed_portfolio(&pool, user, "Explicit sync", "USD").await;
    let asset = seed_asset(&pool, port, SYMBOL, "Split me").await;
    seed_transaction_at(&pool, asset, "BUY", 10.0, 60.0, 0.0, "2024-01-01T00:00:00Z").await;

    let yahoo = MockYahoo::serving([(SYMBOL, SPLIT_ME)]);
    let client = Client::tracked(build_rocket_pricing(pool.clone(), user, yahoo.base_url()))
        .await
        .unwrap();

    let resp = client
        .post(format!("/api/portfolios/{port}/sync-splits"))
        .header(ContentType::JSON)
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::Ok);
    let body: serde_json::Value = serde_json::from_str(&resp.into_string().await.unwrap()).unwrap();
    assert_eq!(body["assets_checked"], 1);
    assert_eq!(body["splits_inserted"], 1);
    assert_eq!(body["failures"].as_array().unwrap().len(), 0);

    assert_eq!(
        split_rows(&pool, asset).await,
        vec!["2024-06-01".to_string()],
        "the POST must actually write the row the GETs refused to"
    );
}

/// The POST is cheap to call repeatedly, so the UI can offer it without hammering the
/// provider. This is the throttle from #38 doing its job on the new path.
#[tokio::test]
async fn a_second_sync_is_throttled_and_costs_no_provider_call() {
    let (pool, user) = db_with_user("a_second_sync_is_throttled_and_costs_no_provider_call").await;
    let port = seed_portfolio(&pool, user, "Twice", "USD").await;
    let asset = seed_asset(&pool, port, SYMBOL, "Split me").await;
    seed_transaction_at(&pool, asset, "BUY", 10.0, 60.0, 0.0, "2024-01-01T00:00:00Z").await;

    let yahoo = MockYahoo::serving([(SYMBOL, SPLIT_ME)]);
    let client = Client::tracked(build_rocket_pricing(pool.clone(), user, yahoo.base_url()))
        .await
        .unwrap();

    let url = format!("/api/portfolios/{port}/sync-splits");
    let first = client.post(&url).header(ContentType::JSON).dispatch().await;
    assert_eq!(first.status(), Status::Ok);
    let calls_after_first = yahoo.paths().len();
    assert_eq!(calls_after_first, 1, "first sync should hit the provider once");

    let second = client.post(&url).header(ContentType::JSON).dispatch().await;
    assert_eq!(second.status(), Status::Ok);
    let body: serde_json::Value = serde_json::from_str(&second.into_string().await.unwrap()).unwrap();
    assert_eq!(body["splits_inserted"], 0);
    assert_eq!(
        yahoo.paths().len(),
        calls_after_first,
        "a throttled sync must not reach the provider again"
    );
    assert_eq!(split_rows(&pool, asset).await, vec!["2024-06-01".to_string()]);
}

/// The race, directly. Two callers both see "no split recorded" and both write; the
/// unique index is what turns the second write into a no-op instead of a doubled
/// adjustment. Without `uq_split_asset_date` this inserts two rows and the asset's
/// cost basis is off by the split ratio, with nothing reporting it.
#[tokio::test]
async fn concurrent_syncs_cannot_double_a_split() {
    let (pool, user) = db_with_user("concurrent_syncs_cannot_double_a_split").await;
    let port = seed_portfolio(&pool, user, "Race", "USD").await;
    let asset = seed_asset(&pool, port, SYMBOL, "Split me").await;

    let events = vec![(
        chrono::NaiveDate::from_ymd_opt(2024, 6, 1).unwrap(),
        2u32,
        1u32,
    )];

    let a = apply_split_events(asset, None, &events, &pool);
    let b = apply_split_events(asset, None, &events, &pool);
    let (ra, rb) = tokio::join!(a, b);
    let inserted = ra.unwrap() + rb.unwrap();

    assert_eq!(inserted, 1, "exactly one caller should have written a row");
    assert_eq!(
        split_rows(&pool, asset).await,
        vec!["2024-06-01".to_string()],
        "two concurrent syncs must leave one split row, not two"
    );
}

#[tokio::test]
async fn another_user_cannot_sync_someone_elses_portfolio() {
    let (pool, user) = db_with_user("another_user_cannot_sync_someone_elses_portfolio").await;
    let other = common::seed_user(&pool, "other@example.com").await;
    let port = seed_portfolio(&pool, other, "Not yours", "USD").await;

    let yahoo = MockYahoo::serving([(SYMBOL, SPLIT_ME)]);
    // The client is authenticated as user, who owns nothing here.
    let client = Client::tracked(build_rocket_pricing(pool.clone(), user, yahoo.base_url()))
        .await
        .unwrap();

    let resp = client
        .post(format!("/api/portfolios/{port}/sync-splits"))
        .header(ContentType::JSON)
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::NotFound);
    assert!(
        yahoo.paths().is_empty(),
        "a denied request must not spend a provider call"
    );
}

/// The staleness half of the design. `None` means "not every asset here has been
/// synced", which the UI renders as its own message rather than as an age — because
/// reporting the age of the assets that *were* synced is exactly how a newly added
/// asset would sit silently unsynced.
#[tokio::test]
async fn splits_last_synced_is_none_until_every_asset_is_synced() {
    let (pool, user) = db_with_user("splits_last_synced_is_none_until_every_asset_is_synced").await;
    let port = seed_portfolio(&pool, user, "Staleness", "USD").await;
    let a = seed_asset(&pool, port, "AAAA", "A").await;
    let b = seed_asset(&pool, port, "BBBB", "B").await;

    let yahoo = MockYahoo::start();
    let client = Client::tracked(build_rocket_pricing(pool.clone(), user, yahoo.base_url()))
        .await
        .unwrap();

    assert_eq!(
        portfolio_payload(&client, port).await["splits_last_synced_at"],
        serde_json::Value::Null,
        "nothing has been synced yet"
    );

    seed_split_sync(&pool, a).await;
    assert_eq!(
        portfolio_payload(&client, port).await["splits_last_synced_at"],
        serde_json::Value::Null,
        "one asset synced and one never has; the portfolio is not covered"
    );

    seed_split_sync(&pool, b).await;
    let value = portfolio_payload(&client, port).await["splits_last_synced_at"].clone();
    assert!(
        value.as_str().is_some(),
        "every asset has now been synced, so an age can be reported, got {value}"
    );

    // A third asset added afterwards reopens the gap rather than hiding behind the
    // two existing timestamps.
    seed_asset(&pool, port, "CCCC", "C").await;
    assert_eq!(
        portfolio_payload(&client, port).await["splits_last_synced_at"],
        serde_json::Value::Null,
        "an asset added after the last sync must make the portfolio uncovered again"
    );
}

/// `apply_split_events` has no read-based dedupe at all — it inserts and counts
/// `rows_affected()`. The unique index is the whole guarantee, so this asserts the
/// migrated schema really rejects a second split on the same UTC day, while leaving a
/// different asset and a non-split on that day alone.
///
/// It used to test a migration that deduped an existing SQLite table. Postgres installs
/// fresh and `0001_init.sql` creates the index as part of the schema, so there is
/// nothing to dedupe; what remains worth testing is that the index is there and bites.
#[tokio::test]
async fn the_schema_rejects_a_second_split_on_the_same_day() {
    let (pool, user) = db_with_user("the_schema_rejects_a_second_split_on_the_same_day").await;
    let port_a = seed_portfolio(&pool, user, "A", "USD").await;
    let port_b = seed_portfolio(&pool, user, "B", "USD").await;
    let asset_a = seed_asset(&pool, port_a, "AAA", "Aaa").await;
    let asset_b = seed_asset(&pool, port_b, "BBB", "Bbb").await;

    // Two different timestamps on one calendar day is exactly what a count-then-insert
    // check let through, because it compared the day while storing a timestamp.
    for ts in ["2024-06-01T00:00:00Z", "2024-06-01T14:30:00Z"] {
        let dup = sqlx::query(
            "INSERT INTO transactions (asset_id, type, quantity, price, fee, date) \
             VALUES ($1, 'SPLIT', '2', '1', '0', $2)",
        )
        .bind(asset_a)
        .bind(
            chrono::DateTime::parse_from_rfc3339(ts)
                .unwrap()
                .with_timezone(&chrono::Utc),
        )
        .execute(&pool)
        .await;
        if ts.ends_with("00:00:00Z") {
            dup.unwrap();
        } else {
            assert!(
                dup.is_err(),
                "the second split on the same UTC day must be rejected by uq_split_asset_date"
            );
        }
    }

    // A different asset on the same day is not a duplicate, and neither is a non-split.
    sqlx::query(
        "INSERT INTO transactions (asset_id, type, quantity, price, fee, date) \
         VALUES ($1, 'SPLIT', '2', '1', '0', '2024-06-01T00:00:00Z')",
    )
    .bind(asset_b)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO transactions (asset_id, type, quantity, price, fee, date) \
         VALUES ($1, 'BUY', '5', '10', '0', '2024-06-01T00:00:00Z')",
    )
    .bind(asset_a)
    .execute(&pool)
    .await
    .unwrap();

    let kept: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT asset_id, type, ((date AT TIME ZONE 'UTC')::date)::text \
         FROM transactions ORDER BY asset_id, type",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        kept,
        vec![
            (asset_a, "BUY".to_string(), "2024-06-01".to_string()),
            (asset_a, "SPLIT".to_string(), "2024-06-01".to_string()),
            (asset_b, "SPLIT".to_string(), "2024-06-01".to_string()),
        ],
        "one split per asset per day, other rows untouched"
    );
}
