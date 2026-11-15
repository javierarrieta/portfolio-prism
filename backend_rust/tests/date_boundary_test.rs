//! `transactions.date` became TIMESTAMPTZ. What TEXT accepted, a real date
//! column can reject — and the CSV import is where odd shapes arrive. The
//! expected behaviour is a 422, never a 500. Naming the offending row is the
//! client's job (`frontend/src/utils/csvValidator.ts`) and the bundle
//! validator's job (`src/backup/mod.rs::validate`); the HTTP body is Rocket's
//! generic deserialization error.
//!
//! The answer this file pins, measured rather than guessed:
//!
//! * A bare `"2026-11-14"` is **rejected**, at deserialization, before any SQL is
//!   prepared. chrono deserializes `DateTime<Utc>` as RFC 3339 and a date with no
//!   time component fails with `premature end of input`. It does **not** silently
//!   become midnight UTC.
//! * That is not a change from the engine switch. `TransactionCreate::date` was
//!   already `DateTime<Utc>` when the column was TEXT (`git show 508b738` — the
//!   merge base — `src/schemas.rs:18`), so the API has never accepted a bare date
//!   here. TEXT storage made the *column* permissive; the DTO never was.
//! * Where a bare date *does* become midnight UTC is the client: the CSV importer
//!   normalizes `YYYY-MM-DD` to `YYYY-MM-DDT00:00:00Z` before it posts
//!   (`frontend/src/utils/csvParser.ts`, `dateToISO`). So the convention is real,
//!   it is stated in one place, and it is not something the server infers from a
//!   truncated timestamp.
//! * A non-UTC offset is accepted and normalized to the same instant
//!   (`2026-11-14T12:30:00+02:00` == `2026-11-14T10:30:00Z`), which is what a
//!   `TIMESTAMPTZ` column stores anyway — an instant, not a wall-clock reading.
//!
//! None of this touches what counts as *the same day* for split dedupe. That is
//! `uq_split_asset_date` — the partial unique index on
//! `(asset_id, ((date at time zone 'UTC')::date)) WHERE type = 'SPLIT'` — and the
//! identical expression in `currency_service.rs`. Both are pinned by
//! `scripts/check_migrations.sh` and neither is in scope here.
mod common;

use backend_rust::schemas::TransactionCreate;
use rocket::http::{ContentType, Status};
use rocket::local::asynchronous::Client;

use common::{build_rocket, db_with_user, seed_asset, seed_portfolio};

/// The body the API takes for a transaction. `asset_id` is deliberately absent:
/// the asset comes from the URL path, not the body.
fn body_with_date(date: &str) -> String {
    format!(r#"{{"type":"BUY","quantity":"1","price":"1.0","fee":"0","date":"{date}"}}"#)
}

#[test]
fn a_date_only_string_is_rejected_before_it_reaches_the_database() {
    // Pinned by running it, not by guessing: chrono's serde impl parses RFC 3339,
    // and a date with no time component is `premature end of input`. The point of
    // the assertion is that no `DateTime` comes back at all — a bare date must not
    // silently become midnight UTC on the server, where nobody asked for it.
    let parsed = serde_json::from_str::<TransactionCreate>(&body_with_date("2026-11-14"));
    assert!(
        parsed.is_err(),
        "a bare date must not silently become midnight UTC"
    );
}

#[test]
fn a_full_rfc3339_timestamp_is_accepted() {
    let ok = serde_json::from_str::<TransactionCreate>(&body_with_date("2026-11-14T00:00:00Z"));
    assert!(ok.is_ok(), "{:?}", ok.err());
}

#[test]
fn a_non_utc_offset_is_normalized_to_the_same_instant() {
    // `transactions.date` is `TIMESTAMPTZ`: it stores an instant. Accepting an
    // offset and converting it is therefore the only honest reading of the column,
    // and it is what SQLite's TEXT storage did too (the string went in as typed,
    // but every comparison went through the same UTC conversion).
    let offset =
        serde_json::from_str::<TransactionCreate>(&body_with_date("2026-11-14T12:30:00+02:00"))
            .expect("an RFC 3339 timestamp with an offset is valid input");
    let utc = serde_json::from_str::<TransactionCreate>(&body_with_date("2026-11-14T10:30:00Z"))
        .expect("the equivalent UTC instant is valid input");
    assert_eq!(offset.date, utc.date);
}

#[tokio::test]
async fn a_date_only_string_is_a_422_from_the_api_and_never_a_500() {
    // The deserialization failure above has to surface as the client's error. A
    // `500` here would mean the bad string reached the database and Postgres
    // rejected it, which would put a parse error in the server's log and a
    // meaningless body in front of the user.
    let (pool, user) = db_with_user("a_date_only_string_is_a_422_from_the_api_and_never_a_500").await;
    let port_id = seed_portfolio(&pool, user, "Date Boundary", "USD").await;
    let asset_id = seed_asset(&pool, port_id, "AAPL", "Apple").await;
    let client = Client::tracked(build_rocket(pool.clone(), user))
        .await
        .unwrap();

    let resp = client
        .post(format!(
            "/api/portfolios/{port_id}/assets/{asset_id}/transactions"
        ))
        .header(ContentType::JSON)
        .body(body_with_date("2026-11-14"))
        .dispatch()
        .await;

    assert_eq!(resp.status(), Status::UnprocessableEntity);

    // "Rejected before it reaches the database", stated as a fact about the
    // database rather than about the response code.
    let rows: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM transactions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        rows.0, 0,
        "a rejected transaction must not have been written"
    );
}

#[tokio::test]
async fn a_full_rfc3339_timestamp_is_written_and_echoed_back() {
    // The other half of the pin: the accepted shape is not merely parseable, it
    // round-trips through the real column. The response renders UTC as `Z`, the
    // same string that went in.
    let (pool, user) = db_with_user("a_full_rfc3339_timestamp_is_written_and_echoed_back").await;
    let port_id = seed_portfolio(&pool, user, "Date Boundary", "USD").await;
    let asset_id = seed_asset(&pool, port_id, "AAPL", "Apple").await;
    let client = Client::tracked(build_rocket(pool.clone(), user))
        .await
        .unwrap();

    let resp = client
        .post(format!(
            "/api/portfolios/{port_id}/assets/{asset_id}/transactions"
        ))
        .header(ContentType::JSON)
        .body(body_with_date("2026-11-14T00:00:00Z"))
        .dispatch()
        .await;

    assert_eq!(resp.status(), Status::Ok);
    let out: serde_json::Value = resp.into_json().await.unwrap();
    assert_eq!(out["date"], "2026-11-14T00:00:00Z");
    assert!(out["id"].is_number());
    assert_eq!(out["asset_id"], asset_id);
}
