//! Money crosses the boundary as a string, or it does not cross it.
//!
//! `transactions.quantity`, `price` and `fee` are `TEXT` columns holding
//! `Decimal::to_string()`, which is exact. The exactness is worthless if the value arrived
//! as a JSON number: by the time serde hands it to `Decimal` it has been an `f64`, and
//! `0.1 + 0.2 != 0.3`. A cost basis that drifts by a rounding unit per row is wrong on tax
//! figures, and nothing on screen would look wrong while it drifted.
//!
//! So the write codec refuses. These tests are the refusal, in both directions:
//!
//! - a JSON number is a 422 that says what to send instead;
//! - the *response* codec still accepts numbers, unchanged, because roughly 55 sites in
//!   `schemas.rs` depend on its lenient arm and tightening it there would break every tax
//!   figure the app answers with. The two codecs are deliberately different and one test
//!   below exists only to keep someone from "unifying" them.

mod common;

use backend_rust::db_types::is_canonical_decimal;
use backend_rust::schemas::{TransactionCreate, TransactionOut};
use rust_decimal::Decimal;

fn body(quantity: &str) -> String {
    format!(
        r#"{{"type":"BUY","quantity":{quantity},"price":"1.0","fee":"0","date":"2024-06-15T12:00:00Z"}}"#
    )
}

fn parse_quantity(quantity: &str) -> Result<TransactionCreate, serde_json::Error> {
    serde_json::from_str::<TransactionCreate>(&body(quantity))
}

#[test]
fn a_json_number_is_refused_and_the_error_names_the_fix() {
    for raw in ["10", "10.5", "0.1", "-3"] {
        let err = parse_quantity(raw)
            .expect_err(&format!("{raw} as a JSON number must be refused, not absorbed"));
        let msg = err.to_string();
        assert!(
            msg.contains("decimal string") || msg.contains("invalid type"),
            "the error has to say what to send instead of 'invalid input'; got: {msg}"
        );
    }
}

#[test]
fn a_decimal_string_is_accepted_and_keeps_every_digit_it_was_given() {
    let tx = parse_quantity("\"9.99\"").expect("9.99 as a string must be accepted");
    assert_eq!(tx.quantity, Decimal::from_str_exact("9.99").unwrap());
    // The reason the whole exercise exists: this is the value, not a float's nearest
    // neighbour. `9.99` is not representable in binary floating point at all.
    assert_eq!(tx.quantity.to_string(), "9.99");
}

#[test]
fn the_scale_cap_is_a_stated_rule_and_not_a_column_type() {
    // 12 places is the cap in `MAX_DECIMAL_SCALE`; the columns are TEXT and would hold
    // anything, so nothing but this rule bounds it.
    assert!(parse_quantity("\"0.000000000001\"").is_ok(), "12 dp");
    assert!(
        parse_quantity("\"0.0000000000001\"").is_err(),
        "13 dp must be refused"
    );
}

#[test]
fn forms_the_storage_parser_would_accept_are_still_refused_on_the_wire() {
    // `Decimal::from_str` is the storage parser and is more permissive than this. The
    // narrower wire rule is deliberate: a value that enters as `1e3` and one that enters
    // as `1000` would be stored differently and then treated as different transactions by
    // anything that dedupes on the stored string.
    for raw in [
        "\"1e3\"",
        "\"1E3\"",
        "\"+10\"",
        "\" 10\"",
        "\"10 \"",
        "\"10.\"",
        "\".5\"",
        "\"1,50\"",
        "\"\"",
        "\"NaN\"",
        "\"inf\"",
        "\"0x10\"",
    ] {
        assert!(
            parse_quantity(raw).is_err(),
            "{raw} must be refused on the wire even though some parser downstream would take it"
        );
    }
    // And the plain forms are fine, including the negative and the zero the frontend
    // canonicalizes to.
    for raw in ["\"0\"", "\"-0\"", "\"0.0\"", "\"10\"", "\"-10.50\""] {
        assert!(parse_quantity(raw).is_ok(), "{raw} must be accepted");
    }
}

#[test]
fn is_canonical_decimal_agrees_with_the_codec_it_guards() {
    // The predicate is public because the frontend needs the same rule; if the two ever
    // disagree, a client validates a value the server then refuses.
    assert!(is_canonical_decimal("10"));
    assert!(is_canonical_decimal("-10.50"));
    assert!(!is_canonical_decimal("1e3"));
    assert!(!is_canonical_decimal("-"));
    assert!(!is_canonical_decimal("--1"));
    assert!(!is_canonical_decimal("1.2.3"));
}

#[test]
fn the_response_codec_still_accepts_a_number_and_that_is_not_an_oversight() {
    // This test exists to fail if someone "unifies" the two codecs. `decimal_json` is the
    // response serializer at ~55 sites and its `visit_f64` arm is load-bearing for
    // `TaxLot` and `AssetTaxSummary`; making the response side strict would turn every tax
    // figure the app has ever answered into a deserialization failure.
    let json = r#"{"id":1,"asset_id":1,"type":"BUY","quantity":10,"price":1.5,"fee":0,"date":"2024-06-15T12:00:00Z"}"#;
    let out: TransactionOut = serde_json::from_str(json)
        .expect("the read side stays lenient on purpose — see strict_decimal_json's comment");
    assert_eq!(out.quantity, Decimal::from(10));
}

#[test]
fn every_money_field_on_the_write_dto_is_strict_not_just_the_first_one() {
    // A `#[serde(with)]` applied to `quantity` and forgotten on `price` is exactly the
    // kind of half-fix that reads as done. Each field is exercised on its own.
    let cases = [
        (
            "price",
            r#"{"type":"BUY","quantity":"1","price":1.5,"fee":"0","date":"2024-06-15T12:00:00Z"}"#,
        ),
        (
            "fee",
            r#"{"type":"BUY","quantity":"1","price":"1","fee":0,"date":"2024-06-15T12:00:00Z"}"#,
        ),
    ];
    for (field, raw) in cases {
        // Each case makes exactly one field a number and leaves the others valid, so a
        // refusal here can only be about that field. serde's own message does not name the
        // field for a `from_str` parse, so the isolation is what carries the claim — and
        // "invalid type" is what distinguishes a strictness refusal from a missing field.
        let err = serde_json::from_str::<TransactionCreate>(raw)
            .expect_err(&format!("{field} as a JSON number must be refused"));
        assert!(
            err.to_string().contains("invalid type"),
            "{field} should be refused as a wrong type, not as a missing field; got {err}"
        );
    }
}

#[tokio::test]
async fn the_route_answers_422_rather_than_storing_a_float_rounding_artifact() {
    use rocket::http::{ContentType, Status};
    use rocket::local::asynchronous::Client;

    let (pool, user) = common::db_with_user("strict_route").await;
    let pid = common::seed_portfolio(&pool, user, "Strict", "USD").await;
    let asset_id = common::seed_asset(&pool, pid, "ACME", "Acme").await;
    let rocket = common::build_rocket(pool.clone(), user);
    let client = Client::tracked(rocket).await.unwrap();

    let number =
        r#"{"type":"BUY","quantity":10,"price":"1.0","fee":"0","date":"2024-06-15T12:00:00Z"}"#;
    let res = client
        .post(format!("/api/portfolios/{pid}/assets/{asset_id}/transactions"))
        .header(ContentType::JSON)
        .body(number)
        .dispatch()
        .await;
    assert_eq!(
        res.status(),
        Status::UnprocessableEntity,
        "a JSON number must be a 422 at the route, not a stored value"
    );

    let string =
        r#"{"type":"BUY","quantity":"10","price":"1.0","fee":"0","date":"2024-06-15T12:00:00Z"}"#;
    let ok = client
        .post(format!("/api/portfolios/{pid}/assets/{asset_id}/transactions"))
        .header(ContentType::JSON)
        .body(string)
        .dispatch()
        .await;
    assert_eq!(ok.status(), Status::Ok, "the same value as a string is accepted");

    // And what got stored is the string, not a float's nearest neighbour.
    let stored: (String,) =
        sqlx::query_as("SELECT quantity FROM transactions WHERE asset_id = $1")
        .bind(asset_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored.0, "10", "the stored text is what was sent, not a float's neighbour");
}
