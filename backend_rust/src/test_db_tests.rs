//! Tests for the test harness itself.
//!
//! `src/test_db.rs` is the only component in this repo that deletes things, and until
//! now it had no test of its own. It earned one: an early version asked
//! `pg_stat_activity` for a column called `database` instead of `datname`, the query
//! errored, `unwrap_or((0,))` reported "no backends", and the harness dropped live test
//! databases mid-run — which surfaced as an unrelated backup test failing with
//! `terminating connection due to administrator command`. The repo's own standard is
//! that a gate is only as good as the mutation that proves it can fail, so each test
//! here is written as the mutation for a specific way the harness can destroy a run.
//!
//! Included from `src/lib.rs` only, so it runs once rather than once per binary.

use crate::test_db::{
    database_name, database_url, harness_pid, has_backends, next_caller, test_pool,
};
use sqlx::Connection as _;

// ---------------------------------------------------------------------------
// harness_pid: which databases the harness is allowed to delete.
// ---------------------------------------------------------------------------

#[test]
fn a_harness_database_reports_its_pid() {
    assert_eq!(harness_pid("prism_t_4242_split_sync"), Some(4242));
    // The shape a `next_caller` label actually produces today (`access_0` in
    // `src/access.rs`), because a test of the name shape should name a real one.
    assert_eq!(harness_pid("prism_t_1_access_0"), Some(1));
}

#[test]
fn a_name_that_is_not_the_harness_shape_is_not_harness_property() {
    // The mutation for `LIKE 'prism_t_%'`: `_` is a single-character wildcard in a
    // LIKE pattern, so that pattern matches all of these and the sweep would delete
    // databases it does not own.
    assert_eq!(
        harness_pid("prism_testdb"),
        None,
        "`_` must not be a wildcard"
    );
    assert_eq!(harness_pid("prism_t_"), None, "no pid, no label");
    assert_eq!(
        harness_pid("prism_t_2024"),
        None,
        "pid with no label is not ours"
    );
    // `prism_t_2024_archive` is indistinguishable from a harness database by shape —
    // pid 2024, caller "archive" — and no name filter can fix that. What protects it is
    // that the sweep also requires the pid to be dead and the backend count to be zero.
    assert_eq!(harness_pid("prism_t_2024_archive"), Some(2024));
    assert_eq!(harness_pid("prism_t_abcd_x"), None, "non-numeric pid");
    assert_eq!(harness_pid("prism_template"), None);
    assert_eq!(harness_pid("portfolio"), None);
    assert_eq!(harness_pid(""), None);
}

#[test]
fn the_name_the_harness_makes_is_a_name_the_harness_recognises() {
    // The two halves have to agree, or the sweep would skip its own leftovers.
    let made = database_name("some_caller");
    assert!(
        harness_pid(&made).is_some(),
        "{made} was created by the harness but harness_pid does not claim it"
    );
}

// ---------------------------------------------------------------------------
// database_name: sanitisation, truncation, and collapsing.
// ---------------------------------------------------------------------------

#[test]
fn a_caller_is_sanitised_to_a_valid_unquoted_identifier() {
    // `startup_test` once labelled a case with `{raw:?}`; the `"` went straight into
    // `CREATE DATABASE` and Postgres answered `zero-length delimited identifier`.
    let name = database_name("blank_bootstrap_subject_\"\"");
    assert!(
        name.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
        "{name} would be rejected by an unquoted identifier"
    );
    assert_eq!(
        name.to_lowercase(),
        name,
        "must already be folded, as Postgres would"
    );
}

#[test]
fn a_long_caller_is_truncated_to_what_postgres_will_store() {
    let long = database_name(&"x".repeat(200));
    assert_eq!(
        long.chars().count(),
        63,
        "Postgres truncates an identifier at 63 bytes"
    );
    assert!(
        harness_pid(&long).is_some(),
        "truncation must not cost the name its pid"
    );
}

#[tokio::test]
async fn two_labels_that_collapse_onto_one_name_are_caught_as_a_duplicate_caller() {
    // The registry keys on the *sanitised* name precisely so this collision is a panic
    // rather than two tests sharing one database and truncating each other's rows.
    let a = database_name("case_one");
    let b = database_name("case-one");
    assert_eq!(
        a, b,
        "these two labels must collapse, which is what the registry is for"
    );

    let pool_a = test_pool(&next_caller("collapse_a")).await;
    let pool_b = test_pool(&next_caller("collapse_b")).await;
    assert_ne!(
        pool_a.connect_options().get_database(),
        pool_b.connect_options().get_database(),
        "distinct callers must land on distinct databases"
    );
}

// ---------------------------------------------------------------------------
// database_url: the path is replaced, not appended, and the query survives.
// ---------------------------------------------------------------------------

#[test]
fn the_database_name_replaces_the_path_rather_than_joining_onto_it() {
    // Concatenating gives `postgres://h:5432/prism/prism_t_1`, which Postgres reads as
    // database `prism` with a trailing segment and rejects with an unhelpful error.
    let url = database_url("postgres://postgres@127.0.0.1:55432/prism", "prism_t_7_x");
    assert_eq!(url, "postgres://postgres@127.0.0.1:55432/prism_t_7_x");
    assert!(
        !url.contains("/prism/"),
        "{url} appended instead of replacing"
    );
}

#[test]
fn a_query_string_on_the_base_url_survives_the_rewrite() {
    let url = database_url(
        "postgres://user:pw@host:5432/prism?sslmode=require&connect_timeout=3",
        "prism_t_7_x",
    );
    assert_eq!(
        url,
        "postgres://user:pw@host:5432/prism_t_7_x?sslmode=require&connect_timeout=3"
    );
}

#[test]
fn a_base_url_with_no_database_path_still_produces_one() {
    let url = database_url("postgres://localhost:5432", "prism_t_7_x");
    assert_eq!(url, "postgres://localhost:5432/prism_t_7_x");
}

// ---------------------------------------------------------------------------
// has_backends: the signal the reaper deletes on.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_database_this_test_is_connected_to_reads_as_in_use() {
    let pool = test_pool("harness_has_backends").await;
    let name = pool
        .connect_options()
        .get_database()
        .expect("the harness hands out a URL that names a database")
        .to_string();
    let mut admin = sqlx::postgres::PgConnection::connect(
        &std::env::var("DATABASE_URL").expect("DATABASE_URL for tests"),
    )
    .await
    .unwrap();
    assert!(
        has_backends(&mut admin, &name).await.unwrap(),
        "{name} is connected right now and must not read as reclaimable"
    );
}

#[tokio::test]
async fn a_database_nothing_is_connected_to_reads_as_free() {
    // The other half: the count has to go back to zero, or `reap_finished` would never
    // delete anything and every run would leak its databases.
    let pool = test_pool("harness_goes_idle").await;
    let name = pool
        .connect_options()
        .get_database()
        .expect("the harness hands out a URL that names a database")
        .to_string();
    drop(pool); // closes its connections, which is the whole point of this test
    let mut admin = sqlx::postgres::PgConnection::connect(
        &std::env::var("DATABASE_URL").expect("DATABASE_URL for tests"),
    )
    .await
    .unwrap();
    // Postgres notices a gone client asynchronously, so this polls rather than asserts.
    for _ in 0..40 {
        if !has_backends(&mut admin, &name).await.unwrap() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("no backends expected for {name} once its pool was dropped");
}

#[tokio::test]
async fn a_bogus_column_is_an_error_and_not_a_quiet_no_backends() {
    // The exact mutation that caused the original incident. `pg_stat_activity` has
    // `datname`; a query written against `database` must fail loudly, because the
    // caller reads `Err` as "do not delete anything" and `Ok(false)` as "delete this".
    let base = std::env::var("DATABASE_URL").expect("DATABASE_URL for tests");
    let mut admin = sqlx::postgres::PgConnection::connect(&base).await.unwrap();
    let bogus: Result<(i64,), sqlx::Error> =
        // macro-exempt: the assertion is that a wrong column name fails at *runtime*;
        // a macro would reject it at compile time and the test would test nothing.
        sqlx::query_as("SELECT COUNT(*) FROM pg_stat_activity WHERE database = $1")
            .bind("prism")
            .fetch_one(&mut admin)
            .await;
    assert!(
        bogus.is_err(),
        "if this ever returns Ok, a wrong column name is once again indistinguishable \
         from an unused database"
    );
}
