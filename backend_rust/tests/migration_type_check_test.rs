//! That migration `0002` can refuse, and that refusing leaves nothing behind.
//!
//! `check_migrations.sh` inspects a *database*, so it cannot observe a migration
//! aborting — it would only see the schema the aborted migration failed to produce, and
//! it passes just as happily against a stale one. And `test_db::test_pool` runs the
//! embedded `sqlx::migrate!` before the test body, so by the time a test using it starts
//! `0002` has already either succeeded or taken the whole suite down. Neither is a place
//! to assert "this migration raises and names the value".
//!
//! So this file does the thing directly: create an empty database, apply `0001` from the
//! file on disk, write the row that must be refused, apply `0002` from the file on disk,
//! and read the error. Reading the files rather than `sqlx::migrate!` matters — the point
//! is to test the SQL that ships, and `include_str!` is the same bytes the migrator would
//! run.
//!
//! Four claims, one per test:
//! - an out-of-set type aborts, and the error names the value;
//! - the abort rolls the normalization back, so the database is exactly as found;
//! - a legacy lowercase `buy` survives normalization and satisfies the `CHECK`, which is
//!   the whole reason the constraint is `upper(btrim(type))` and not `type IN (...)`;
//! - two `SPLIT` rows differing only in case abort *before* the `UPDATE`, because
//!   collapsing them would violate the case-sensitive partial index `uq_split_asset_date`.

use sqlx::postgres::PgConnection;
use sqlx::Connection;

// Included the way `tests/common/mod.rs` includes it: `src/test_db.rs` is compiled into
// the lib only under `cfg(test)`, so the integration binary pulls the same file in by
// path rather than reaching for a private module. One implementation of "a database this
// test owns alone", shared by both suites.
#[path = "../src/test_db.rs"]
mod test_db;

use test_db::{database_name, database_url};

const MIG_0001: &str = include_str!("../migrations/0001_init.sql");
const MIG_0002: &str = include_str!("../migrations/0002_transaction_type_check.sql");

/// A freshly created, completely empty database — no migrations at all.
///
/// Deliberately not `test_db::test_pool`: that one hands back a *migrated* database,
/// which is the opposite of what this file needs.
async fn empty_db(tag: &str) -> (String, PgConnection) {
    let base = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set; these tests run real migrations on real databases");
    let name = database_name(&format!("mig_{tag}"));
    let url = database_url(&base, &name);

    let mut admin = PgConnection::connect(&base)
        .await
        .unwrap_or_else(|e| panic!("could not reach {base}: {e}"));
    // `IF EXISTS` because a previous run that panicked mid-test leaves the name behind,
    // and `database_name` is deterministic per caller.
    sqlx::raw_sql(&format!("DROP DATABASE IF EXISTS {name}"))
        .execute(&mut admin)
        .await
        .unwrap_or_else(|e| panic!("could not clear stale {name}: {e}"));
    sqlx::raw_sql(&format!("CREATE DATABASE {name}"))
        .execute(&mut admin)
        .await
        .unwrap_or_else(|e| panic!("could not create {name}: {e}"));
    drop(admin);

    let conn = PgConnection::connect(&url)
        .await
        .unwrap_or_else(|e| panic!("could not connect to fresh {name}: {e}"));
    (name, conn)
}

/// Apply `0001` and put one asset in place, so a transaction row has something to point
/// at. Returns the asset id.
///
/// The chain is `transactions -> assets -> portfolios -> users`, and every column the
/// schema declares `NOT NULL` is written here, because `0001` is applied verbatim and
/// there is no template with defaults to lean on.
async fn migrated_with_asset(conn: &mut PgConnection) -> i64 {
    sqlx::raw_sql(MIG_0001)
        .execute(&mut *conn)
        .await
        .expect("0001 must apply to an empty database");

    sqlx::raw_sql(
        "INSERT INTO users (issuer, subj, is_active, token_version, created_at)
         VALUES ('iss', 'sub', true, 0, now())",
    )
    .execute(&mut *conn)
    .await
    .unwrap();
    sqlx::raw_sql(
        "INSERT INTO portfolios (owner_id, name, currency, base_currency)
         VALUES (1, 'p', 'USD', 'USD')",
    )
    .execute(&mut *conn)
    .await
    .unwrap();
    sqlx::raw_sql(
        "INSERT INTO assets (portfolio_id, symbol, name, asset_type, currency)
         VALUES (1, 'ACME', 'Acme', 'stock', 'USD')",
    )
    .execute(&mut *conn)
    .await
    .unwrap();

    let (asset_id,): (i64,) = sqlx::query_as("SELECT id FROM assets WHERE symbol = 'ACME'")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    asset_id
}

async fn insert_txn(conn: &mut PgConnection, asset_id: i64, ty: &str, day: &str) {
    let ts = chrono::DateTime::parse_from_rfc3339(&format!("{day}T00:00:00Z"))
        .expect("test day literal")
        .with_timezone(&chrono::Utc);
    // Runtime-bound, not the macro: these tests run against a database that is
    // deliberately *not* migrated, so a compile-time-checked query would be checked
    // against a schema the database does not have yet.
    sqlx::query(
        "INSERT INTO transactions (asset_id, type, quantity, price, fee, date)
         VALUES ($1, $2, '1', '1', '0', $3)",
    )
    .bind(asset_id)
    .bind(ty)
    .bind(ts)
    .execute(&mut *conn)
    .await
    .unwrap();
}

async fn apply_0002(conn: &mut PgConnection) -> Result<(), sqlx::Error> {
    sqlx::raw_sql(MIG_0002)
        .execute(&mut *conn)
        .await
        .map(|_| ())
}

async fn has_type_check(conn: &mut PgConnection) -> bool {
    let (n,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM pg_constraint
         WHERE conrelid = 'transactions'::regclass AND conname = 'transactions_type_check'",
    )
    .fetch_one(&mut *conn)
    .await
    .unwrap();
    n == 1
}

#[tokio::test]
async fn an_out_of_set_type_aborts_and_the_error_names_the_value() {
    let (name, mut conn) = empty_db("out_of_set").await;
    let asset = migrated_with_asset(&mut conn).await;
    insert_txn(&mut conn, asset, "DIVIDEND", "2024-01-05").await;

    let err = apply_0002(&mut conn)
        .await
        .expect_err("0002 must refuse to rewrite DIVIDEND into something it is not");
    let msg = err.to_string();
    assert!(
        msg.contains("DIVIDEND"),
        "the error has to name the offending value or the operator cannot act on it; got: {msg}"
    );
    assert!(
        !has_type_check(&mut conn).await,
        "the CHECK must not exist after an aborted migration"
    );

    // The row it refused to rewrite is untouched — the migration invents nothing and
    // destroys nothing.
    let (kept,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM transactions WHERE type = 'DIVIDEND'")
            .fetch_one(&mut conn)
            .await
            .unwrap();
    assert_eq!(kept, 1, "the refused row must still be there, unchanged");

    drop(conn);
    cleanup(&name).await;
}

#[tokio::test]
async fn the_abort_rolls_back_the_normalization_it_already_applied()
{
    // The `UPDATE` in step (2) runs *before* the validation in step (3). Without the
    // per-file transaction, a database with both a `buy` row and a `DIVIDEND` row would
    // come out half-migrated: normalized, then refused. That is the worst of both — the
    // audit script's answer is now "some rows changed and the constraint is missing".
    let (name, mut conn) = empty_db("rollback").await;
    let asset = migrated_with_asset(&mut conn).await;
    insert_txn(&mut conn, asset, "buy", "2024-01-05").await;
    insert_txn(&mut conn, asset, "DIVIDEND", "2024-02-05").await;

    assert!(apply_0002(&mut conn).await.is_err());

    let (still_lower,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM transactions WHERE type = 'buy'")
            .fetch_one(&mut conn)
            .await
            .unwrap();
    assert_eq!(
        still_lower, 1,
        "the abort must roll the normalization back; a half-migrated database is worse \
         than an unmigrated one because the audit script's answer changes under it"
    );

    drop(conn);
    cleanup(&name).await;
}

#[tokio::test]
async fn a_clean_database_gains_the_check_and_still_refuses_junk_afterwards() {
    let (name, mut conn) = empty_db("clean").await;
    let asset = migrated_with_asset(&mut conn).await;
    // Legacy lowercase row: the case the CHECK is written to tolerate.
    insert_txn(&mut conn, asset, "buy", "2024-01-05").await;

    apply_0002(&mut conn)
        .await
        .expect("0002 must apply to a database whose only oddity is case");
    assert!(has_type_check(&mut conn).await);

    // Normalized, not rewritten into something else.
    let (upper,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM transactions WHERE type = 'BUY'")
            .fetch_one(&mut conn)
            .await
            .unwrap();
    assert_eq!(upper, 1, "the lowercase row is normalized to BUY");

    // And the constraint bites: this is the whole point of the migration.
    let after = sqlx::query(
        "INSERT INTO transactions (asset_id, type, quantity, price, fee, date)
         VALUES ($1, 'DIVIDEND', '1', '1', '0', '2024-03-05T00:00:00Z')",
    )
    .bind(asset)
    .execute(&mut conn)
    .await;
    assert!(
        after.is_err(),
        "the CHECK must refuse an out-of-set type after the migration"
    );

    // A whitespace-padded value is accepted, matching the `upper(btrim(...))` predicate.
    // Pinned because the predicate is deliberately looser than the enum, and a reader
    // should have to notice that.
    let padded = sqlx::query(
        "INSERT INTO transactions (asset_id, type, quantity, price, fee, date)
         VALUES ($1, ' sell ', '1', '1', '0', '2024-03-06T00:00:00Z')",
    )
    .bind(asset)
    .execute(&mut conn)
    .await;
    assert!(
        padded.is_ok(),
        "the CHECK is upper(btrim(type)) on purpose; see migration step (4)"
    );

    drop(conn);
    cleanup(&name).await;
}

#[tokio::test]
async fn split_rows_differing_only_in_case_abort_before_the_update_can_collide_them() {
    // `uq_split_asset_date` is `WHERE type = 'SPLIT'` — case-sensitive, and pinned that
    // way by `check_migrations.sh`. These two rows are both outside it today. The
    // normalization would put them both inside it on the same key, so the `UPDATE` would
    // fail with a bare unique violation. Step (1) exists to name them instead.
    let (name, mut conn) = empty_db("split_collision").await;
    let asset = migrated_with_asset(&mut conn).await;
    insert_txn(&mut conn, asset, "SPLIT", "2024-04-01").await;
    insert_txn(&mut conn, asset, "split", "2024-04-01").await;

    let err = apply_0002(&mut conn)
        .await
        .expect_err("collapsing these two rows would violate uq_split_asset_date");
    let msg = err.to_string();
    assert!(
        msg.contains("uq_split_asset_date"),
        "the error should name the index at stake, not just say unique violation; got: {msg}"
    );
    assert!(
        !has_type_check(&mut conn).await,
        "nothing may be committed by an aborted migration"
    );

    // Both rows survive: which duplicate is real is a question about the owner's data.
    let (both,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM transactions WHERE upper(btrim(type)) = 'SPLIT'")
            .fetch_one(&mut conn)
            .await
            .unwrap();
    assert_eq!(both, 2, "the migration must not pick a winner");

    drop(conn);
    cleanup(&name).await;
}

async fn cleanup(name: &str) {
    let base = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let mut admin = PgConnection::connect(&base).await.expect("admin");
    // `WITH (FORCE)` because this connection is the only one that could remain, and a
    // leaked database from a test is exactly the thing the reaper in `test_db` exists to
    // sweep; being tidy here keeps the sweep quiet.
    let _ = sqlx::raw_sql(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"))
        .execute(&mut admin)
        .await;
}
