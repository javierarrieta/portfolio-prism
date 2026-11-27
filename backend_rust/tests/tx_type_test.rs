//! The transaction-type vocabulary has exactly one authority, and two things that must
//! keep agreeing with it.
//!
//! `models::TxType` is the authority: `#[serde(rename_all = "UPPERCASE")]` means the
//! accepted wire values are the style names and nothing else, so there is no `to_uppercase`
//! left to forget and no table of accepted strings to update.
//!
//! Two neighbours could drift from it, and both are pinned here rather than left to
//! someone noticing during a restore:
//!
//! - `backup::KNOWN_TRANSACTION_TYPES` is what a bundle's JSON Schema enumerates and what
//!   `validate()` refuses. A fourth variant added to the enum and not to that constant
//!   would produce a bundle the app can write and refuses to read back.
//! - the `CHECK` in `0002_transaction_type_check.sql` is what the database enforces. It is
//!   a string in a `.sql` file, so nothing but a test like this one connects it to the enum.

use backend_rust::backup::KNOWN_TRANSACTION_TYPES;
use backend_rust::models::TxType;
use backend_rust::schemas::TransactionCreate;

/// A body that differs only in the `type` field, so each case below is about the type and
/// nothing else. `date` is RFC 3339 because a bare date is refused for its own reason
/// (`tests/date_boundary_test.rs`) and would mask these.
fn body_with_type(ty: &str) -> String {
    format!(
        r#"{{"type":{ty},"quantity":"1","price":"1.0","fee":"0","date":"2024-06-15T12:00:00Z"}}"#
    )
}

fn parse(ty: &str) -> Result<TransactionCreate, serde_json::Error> {
    serde_json::from_str::<TransactionCreate>(&body_with_type(&format!("\"{ty}\"")))
}

#[test]
fn the_three_spellings_the_engines_handle_are_the_ones_the_api_accepts() {
    for spelling in ["BUY", "SELL", "SPLIT"] {
        parse(spelling)
            .unwrap_or_else(|e| panic!("{spelling} must be accepted on the wire: {e}"));
    }
}

#[test]
fn a_lowercase_spelling_is_refused_rather_than_uppercased() {
    // This is the behaviour change. Before the enum, `"buy"` was uppercased and stored as
    // `BUY`, which was forgiving in a way that also let `"purchase"` through: the same
    // `to_uppercase()` could not tell a case difference from an unknown word. Refusing is
    // the point, and the frontend already refuses these before sending
    // (`csvValidator.ts:98-100`), so nothing in the app relied on the forgiveness.
    for spelling in ["buy", "sell", "split", "Buy", "bUy"] {
        assert!(
            parse(spelling).is_err(),
            "{spelling} must be refused, not normalized — serde matches the style name exactly"
        );
    }
}

#[test]
fn a_type_outside_the_set_is_refused_and_named() {
    // `DIVIDEND` is the real one: it used to be listed in `KNOWN_TRANSACTION_TYPES`, the
    // docs rule dividends out, and before this change it was stored and then silently
    // ignored by both engines.
    for spelling in ["DIVIDEND", "purchase", "buy ", "BUY2", ""] {
        let err = parse(spelling).expect_err(&format!("{spelling:?} must be refused"));
        // The message has to name what was wrong, or a 422 arrives at the client as
        // "invalid input" and the sender guesses.
        assert!(
            !err.to_string().is_empty(),
            "the serde error for {spelling:?} must say something"
        );
    }
}

#[test]
fn the_enum_and_the_bundle_constant_are_the_same_set() {
    // The drift this pins: `KNOWN_TRANSACTION_TYPES` feeds `docs/backup-schema/v1.json`,
    // so a variant added here and not there yields a bundle the writer emits and the
    // reader rejects — a failure that only appears on restore, which is the worst possible
    // moment to discover a schema disagreement.
    let from_enum: Vec<&str> = TxType::ALL.iter().map(|t| t.as_str()).collect();
    assert_eq!(
        from_enum,
        KNOWN_TRANSACTION_TYPES,
        "`TxType::ALL` and `backup::KNOWN_TRANSACTION_TYPES` disagree; the bundle schema \
         is generated from the latter and the API accepts only the former"
    );
}

#[test]
fn display_and_as_str_agree_because_the_database_stores_that_spelling() {
    // The stored value is `as_str()`, and the engines compare against the same spelling.
    // A `Display` that drifted from `as_str()` would make log lines and SQL disagree about
    // which type a row was.
    for t in TxType::ALL {
        assert_eq!(t.to_string(), t.as_str());
    }
    assert_eq!(TxType::Split.to_string(), "SPLIT");
}

#[test]
fn the_check_in_the_migration_names_the_same_three_values() {
    // A string in a `.sql` file cannot be imported, so it is read from disk and compared.
    // This is the only thing connecting the database constraint to the enum; without it,
    // adding a fourth variant would compile, pass every Rust test, and be refused by
    // Postgres at the first insert.
    let sql = include_str!("../migrations/0002_transaction_type_check.sql");
    for t in TxType::ALL {
        let needle = format!("'{}'", t.as_str());
        assert!(
            sql.contains(&needle),
            "the CHECK in 0002 must list {}, or the enum can write a row the database rejects",
            t.as_str()
        );
    }
    // And nothing looser: a CHECK that also allowed a fourth value would let the enum's
    // guarantee be the only real one.
    assert!(
        !sql.contains("'DIVIDEND'"),
        "0002 must not admit a type outside the set"
    );
}
