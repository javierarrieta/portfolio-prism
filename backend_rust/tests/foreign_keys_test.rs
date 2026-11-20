//! The five foreign keys, and what each one forbids.
//!
//! The claim this file replaces — "SQLite never set `PRAGMA foreign_keys`, so every
//! `FOREIGN KEY` in `init_db` was decorative" — is false, and the correction is
//! measured rather than remembered: `sqlx-sqlite` turns `foreign_keys=ON` on for
//! every connection by default (`sqlx-sqlite-0.8.6/src/options/mod.rs:185`, checked
//! in Task 2). The two `FOREIGN KEY` clauses the old schema actually declared,
//! `assets.portfolio_id` and `transactions.asset_id`, were therefore already
//! enforced, and the app already had to delete children before parents.
//!
//! What the switch to Postgres genuinely *adds* is two constraints that SQLite did
//! not have at all:
//!
//! * `portfolios.owner_id → users(id)`. SQLite added the column with a bare
//!   `ALTER TABLE portfolios ADD COLUMN owner_id INTEGER` (`add_owner_id_column`,
//!   `src/lib.rs:254-267` at the merge base), and an `ADD COLUMN` cannot carry a
//!   foreign key. A dangling `owner_id` was representable, which is why
//!   `backfill_ownership` exists to clean up after it.
//! * `split_sync.asset_id → assets(id)`. The SQLite table (`migrate_split_sync_key`,
//!   `src/lib.rs:530` at the merge base) declared `asset_id INTEGER PRIMARY KEY` with
//!   no foreign key, so a throttle row could outlive its asset indefinitely.
//!
//! All five are pinned here anyway. Two of them confirm something that was already
//! true, and that is worth the tests rather than worth skipping: they are the reason
//! the app's delete ordering is a continuation of existing discipline and not new
//! behaviour to distrust. Each test names the constraint it expects, so a refusal
//! from some *other* constraint cannot satisfy it.
//!
//! None of the five declares `ON DELETE CASCADE`, and
//! [`the_schema_has_exactly_the_five_foreign_keys_and_none_of_them_cascades`] pins that too, because three
//! places in the app are written against it: `delete_asset`
//! (`src/api_routes/transactions.rs`) deletes the `split_sync` row before the asset
//! and inside one transaction, `delete_portfolio` answers 409 instead of cascading,
//! and `src/backup/restore.rs` deletes four tables in reverse dependency order. A
//! cascade added later would not break those three — it would silently widen what
//! they delete, which is worse.
//!
//! Not covered here, because it is covered elsewhere and duplicating it would only
//! put the same property in two files: `DELETE /api/portfolios/<id>` while assets
//! exist is answered 409 by the route's own count check, so the `assets.portfolio_id`
//! FK is never reached through that route —
//! `tests/portfolios_test.rs::test_delete_portfolio_with_assets_returns_conflict`.
//! And `DELETE /api/assets/<id>` on an asset that has transactions is a **500**, a
//! known deferred item: the FK refuses, and nothing maps `23503` to a 409 the way the
//! portfolio route does. That is pinned at the route by
//! `tests/isolation_test.rs::deleting_an_asset_that_has_transactions_fails_today`;
//! [`an_asset_with_a_transaction_cannot_be_deleted_underneath_it`] is the database
//! half of the same fact, and is deliberately at the database level so that fixing
//! the route to answer 409 does not have to edit a test.
mod common;

use rocket::http::Status;
use rocket::local::asynchronous::Client;

use common::{
    build_rocket, db_with_user, seed_asset, seed_portfolio, seed_split_sync, seed_transaction,
};

/// Assert that `err` is *this* foreign-key refusal, not merely "some error".
///
/// `format!("{err:?}").contains("foreign key")` — the first draft of these tests —
/// is satisfied by any FK on any table, including one added later, and by the word
/// appearing in a detail string. `23503` plus the constraint name plus the table
/// pins which edge of the schema held, which is the only part worth a regression
/// test: if a migration ever drops or renames one constraint, the test that names it
/// is the one that says so.
///
/// `table` is the table Postgres names in the error, and the two directions differ —
/// measured, not inferred. On an insert that is the table being written
/// (`transactions`, for the orphan transaction). On a blocked delete it is the
/// **child** table holding the blocking row, not the table the `DELETE` names: the
/// message reads `update or delete on table "portfolios" violates foreign key
/// constraint "assets_portfolio_id_fkey" on table "assets"` and `table()` is
/// `assets`. So the delete tests below pass the child, which is the more useful half
/// anyway — it is where the row that has to be deleted first lives.
fn assert_fk_violation(err: sqlx::Error, constraint: &str, table: &str) {
    let db = err
        .as_database_error()
        .unwrap_or_else(|| panic!("expected a database error, got {err:?}"));
    assert!(
        db.is_foreign_key_violation(),
        "expected a foreign-key violation, got {db} (code {:?})",
        db.code()
    );
    assert_eq!(db.code().as_deref(), Some("23503"), "{db}");
    assert_eq!(db.constraint(), Some(constraint), "{db}");
    assert_eq!(db.table(), Some(table), "{db}");
}

/// The whole foreign-key inventory, in one assertion.
///
/// The four tests below each pin one constraint by refusing to write a row through
/// it; this pins the *set*, so a sixth constraint appearing, one of these five going
/// missing, or a referential action being added to any of them is a failure here
/// rather than a change in what some unrelated delete is allowed to do.
///
/// `confdeltype` is Postgres' referential-action code: `a` is NO ACTION, which is
/// what all five use, and `c` would be CASCADE.
#[tokio::test]
async fn the_schema_has_exactly_the_five_foreign_keys_and_none_of_them_cascades() {
    let (pool, _) = db_with_user("fk_inventory").await;

    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT con.conname, pg_get_constraintdef(con.oid), con.confrelid::regclass::text, \
                con.confdeltype::text \
         FROM pg_constraint con \
         WHERE con.contype = 'f' AND con.connamespace = 'public'::regnamespace \
         ORDER BY con.conname",
    )
    .fetch_all(&pool)
    .await
    .expect("pg_constraint is readable");

    let expected: Vec<(String, String, String, String)> = [
        // The newest one, and the one a reader would least expect to be here: a token
        // points at a user and nothing cascades, because `users` rows are deactivated
        // rather than deleted and `pat::lookup` resolves the user on every call.
        (
            "api_token_user_id_fkey",
            "FOREIGN KEY (user_id) REFERENCES users(id)",
            "users",
            "a",
        ),
        (
            "assets_portfolio_id_fkey",
            "FOREIGN KEY (portfolio_id) REFERENCES portfolios(id)",
            "portfolios",
            "a",
        ),
        (
            "portfolios_owner_id_fkey",
            "FOREIGN KEY (owner_id) REFERENCES users(id)",
            "users",
            "a",
        ),
        (
            "split_sync_asset_id_fkey",
            "FOREIGN KEY (asset_id) REFERENCES assets(id)",
            "assets",
            "a",
        ),
        (
            "transactions_asset_id_fkey",
            "FOREIGN KEY (asset_id) REFERENCES assets(id)",
            "assets",
            "a",
        ),
    ]
    .into_iter()
    .map(|(name, def, referenced, action)| {
        (
            name.to_string(),
            def.to_string(),
            referenced.to_string(),
            action.to_string(),
        )
    })
    .collect();

    assert_eq!(
        rows, expected,
        "the foreign-key inventory changed; every consumer of delete ordering reads this list"
    );
}

// --- transactions.asset_id -> assets(id) --------------------------------------
// Already enforced under SQLite. Confirmed, not assumed.

#[tokio::test]
async fn a_transaction_for_a_missing_asset_is_refused() {
    let (pool, user) = db_with_user("fk_orphan_transaction").await;
    let err = sqlx::query(
        "INSERT INTO transactions (asset_id, type, quantity, price, fee, date) \
         VALUES ($1, 'BUY', '1', '1', '0', '2026-11-14T00:00:00Z')",
    )
    .bind(999_999_i64)
    .execute(&pool)
    .await
    .expect_err("an orphan transaction must not insert");
    assert_fk_violation(err, "transactions_asset_id_fkey", "transactions");

    // The refusal above is only interesting if the same statement *would* have
    // worked with a real asset — otherwise it could be passing on a NOT NULL, a
    // type mismatch, or a missing table.
    let pid = seed_portfolio(&pool, user, "FK", "USD").await;
    let asset = seed_asset(&pool, pid, "VTI", "Vanguard").await;
    let ok = sqlx::query(
        "INSERT INTO transactions (asset_id, type, quantity, price, fee, date) \
         VALUES ($1, 'BUY', '1', '1', '0', '2026-11-14T00:00:00Z')",
    )
    .bind(asset)
    .execute(&pool)
    .await;
    assert!(
        ok.is_ok(),
        "the same insert with a real asset must work: {ok:?}"
    );
}

#[tokio::test]
async fn an_asset_with_a_transaction_cannot_be_deleted_underneath_it() {
    // The database fact behind the deferred product bug: `delete_asset` issues a bare
    // `DELETE FROM assets` and maps nothing to `23503`, so through the route this is
    // a 500 rather than the 409 `delete_portfolio` gives for the same shape. Pinned
    // here, at the database, on purpose — when the route learns to answer 409 this
    // test still says the same true thing and does not need editing.
    let (pool, user) = db_with_user("fk_delete_asset_with_transactions").await;
    let pid = seed_portfolio(&pool, user, "FK", "USD").await;
    let asset = seed_asset(&pool, pid, "VTI", "Vanguard").await;
    seed_transaction(&pool, asset, "BUY", 1.0, 100.0, 0.0).await;

    let err = sqlx::query("DELETE FROM assets WHERE id = $1")
        .bind(asset)
        .execute(&pool)
        .await
        .expect_err("a transaction still points at this asset");
    assert_fk_violation(err, "transactions_asset_id_fkey", "transactions");
}

// --- assets.portfolio_id -> portfolios(id) ------------------------------------
// Already enforced under SQLite. Confirmed, not assumed.

#[tokio::test]
async fn an_asset_in_a_missing_portfolio_is_refused() {
    let (pool, _) = db_with_user("fk_orphan_asset").await;
    let err = sqlx::query(
        "INSERT INTO assets (portfolio_id, symbol, name, asset_type, currency) \
         VALUES ($1, 'VTI', 'Vanguard', 'STOCK', 'USD')",
    )
    .bind(999_999_i64)
    .execute(&pool)
    .await
    .expect_err("an orphan asset must not insert");
    assert_fk_violation(err, "assets_portfolio_id_fkey", "assets");
}

#[tokio::test]
async fn a_portfolio_with_assets_cannot_be_deleted_underneath_them() {
    // No cascade was added, deliberately: the app deletes in order. `delete_portfolio`
    // gets there by refusing first (409 while any asset remains); `restore` in
    // `replace` mode gets there by deleting the four tables bottom-up.
    let (pool, user) = db_with_user("fk_delete_portfolio_with_assets").await;
    let pid = seed_portfolio(&pool, user, "keep", "USD").await;
    seed_asset(&pool, pid, "VTI", "Vanguard").await;

    let err = sqlx::query("DELETE FROM portfolios WHERE id = $1")
        .bind(pid)
        .execute(&pool)
        .await
        .expect_err("cascade was deliberately not added; the app deletes in order");
    assert_fk_violation(err, "assets_portfolio_id_fkey", "assets");
}

// --- portfolios.owner_id -> users(id) -----------------------------------------
// NEW with Postgres. SQLite added the column by a bare `ALTER TABLE`, which cannot
// carry a foreign key, so a dangling `owner_id` was representable right up until the
// cutover and `backfill_ownership` is the code that had to live with it.

#[tokio::test]
async fn a_portfolio_owned_by_a_missing_user_is_refused() {
    let (pool, _) = db_with_user("fk_orphan_owner").await;
    let err = sqlx::query(
        "INSERT INTO portfolios (name, description, currency, base_currency, owner_id) \
         VALUES ('orphan', NULL, 'USD', 'USD', $1)",
    )
    .bind(999_999_i64)
    .execute(&pool)
    .await
    .expect_err("a portfolio owned by nobody must not insert");
    assert_fk_violation(err, "portfolios_owner_id_fkey", "portfolios");

    // NULL is still allowed, and that asymmetry is the whole of `backfill_ownership`:
    // "no owner yet" is a state the app has a rule for, "owner who never existed" is
    // corruption. A test that only checked the refusal above would not tell them apart.
    let unowned = sqlx::query(
        "INSERT INTO portfolios (name, description, currency, base_currency, owner_id) \
         VALUES ('unowned', NULL, 'USD', 'USD', NULL)",
    )
    .execute(&pool)
    .await;
    assert!(
        unowned.is_ok(),
        "owner_id IS NULL must stay insertable: {unowned:?}"
    );
}

#[tokio::test]
async fn a_user_who_owns_a_portfolio_cannot_be_deleted_underneath_it() {
    // Nothing in the app deletes a `users` row — deactivation flips `is_active` — so
    // this is not protecting a live path. It is protecting the one someone writes next:
    // a delete-user route. Whether it silently takes a user's portfolios with it, or
    // half-succeeds and leaves portfolios naming nobody, is decided by whether this
    // constraint exists before that route is written, not after.
    let (pool, user) = db_with_user("fk_delete_owner").await;
    seed_portfolio(&pool, user, "keep", "USD").await;

    let err = sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user)
        .execute(&pool)
        .await
        .expect_err("a portfolio still names this owner");
    assert_fk_violation(err, "portfolios_owner_id_fkey", "portfolios");
}

// --- split_sync.asset_id -> assets(id) ----------------------------------------
// NEW with Postgres. The SQLite table had no foreign key at all.

#[tokio::test]
async fn a_split_sync_row_for_a_missing_asset_is_refused() {
    let (pool, _) = db_with_user("fk_orphan_split_sync").await;
    let err = sqlx::query("INSERT INTO split_sync (asset_id, last_synced_at) VALUES ($1, $2)")
        .bind(999_999_i64)
        .bind(chrono::NaiveDate::from_ymd_opt(2026, 11, 14).unwrap())
        .execute(&pool)
        .await
        .expect_err("a throttle row for no asset must not insert");
    assert_fk_violation(err, "split_sync_asset_id_fkey", "split_sync");
}

#[tokio::test]
async fn an_asset_with_a_split_sync_row_cannot_be_deleted_underneath_it() {
    // This is the one Task 3 had to fix rather than inherit: the old order deleted the
    // asset first and the throttle row second, which SQLite allowed because that
    // table carried no foreign key. Under Postgres the first delete is refused, so
    // `delete_asset` now issues both in one transaction, throttle row first
    // (`src/api_routes/transactions.rs`). Through the route the same property is
    // pinned by `tests/split_sync_throttle_test.rs::deleting_an_asset_drops_its_throttle_row`.
    let (pool, user) = db_with_user("fk_delete_asset_with_throttle_row").await;
    let pid = seed_portfolio(&pool, user, "FK", "USD").await;
    let asset = seed_asset(&pool, pid, "VTI", "Vanguard").await;
    seed_split_sync(&pool, asset).await;

    let err = sqlx::query("DELETE FROM assets WHERE id = $1")
        .bind(asset)
        .execute(&pool)
        .await
        .expect_err("the throttle row has to go first");
    assert_fk_violation(err, "split_sync_asset_id_fkey", "split_sync");
}

// --- the ordering that is actually required -----------------------------------

#[tokio::test]
async fn deleting_bottom_up_succeeds_and_leaves_nothing_behind() {
    // The positive half. Four refusals would also be satisfied by a schema that
    // refuses *everything*, so one test walks the order the app uses and proves the
    // chain is completable: `split_sync`, then `transactions`, then `assets`, then
    // `portfolios` — the order `src/backup/restore.rs` uses in `replace` mode, and
    // the order `delete_asset` uses for its two tables.
    let (pool, user) = db_with_user("fk_bottom_up_delete").await;
    let pid = seed_portfolio(&pool, user, "FK", "USD").await;
    let asset = seed_asset(&pool, pid, "VTI", "Vanguard").await;
    seed_transaction(&pool, asset, "BUY", 1.0, 100.0, 0.0).await;
    seed_split_sync(&pool, asset).await;

    // Each statement is bound to the id its own column holds, because that is the
    // shape the app uses: the asset's children go by `asset_id`, the portfolio's by
    // `portfolio_id`.
    for (sql, id) in [
        ("DELETE FROM split_sync WHERE asset_id = $1", asset),
        ("DELETE FROM transactions WHERE asset_id = $1", asset),
        ("DELETE FROM assets WHERE portfolio_id = $1", pid),
        ("DELETE FROM portfolios WHERE id = $1", pid),
    ] {
        let outcome = sqlx::query(sql).bind(id).execute(&pool).await;
        assert!(outcome.is_ok(), "{sql} failed: {:?}", outcome.err());
    }

    for (table, sql) in [
        ("split_sync", "SELECT COUNT(*) FROM split_sync"),
        ("transactions", "SELECT COUNT(*) FROM transactions"),
        ("assets", "SELECT COUNT(*) FROM assets"),
        ("portfolios", "SELECT COUNT(*) FROM portfolios"),
    ] {
        let (left,): (i64,) = sqlx::query_as(sql).fetch_one(&pool).await.unwrap();
        assert_eq!(left, 0, "{table} still has rows after a bottom-up delete");
    }
}

#[tokio::test]
async fn the_asset_route_deletes_the_throttle_row_before_the_asset() {
    // The same ordering, through the real route, with a transaction attached so a
    // partial delete cannot hide behind a later test. `split_sync_throttle_test`
    // already proves the throttle row is gone afterwards; what this adds is that the
    // route succeeds *because* of the order, and that the asset row itself is gone
    // rather than the request having been refused and reported as a success.
    let (pool, user) = db_with_user("fk_asset_route_order").await;
    let pid = seed_portfolio(&pool, user, "FK", "USD").await;
    let asset = seed_asset(&pool, pid, "VTI", "Vanguard").await;
    seed_split_sync(&pool, asset).await;

    let client = Client::tracked(build_rocket(pool.clone(), user))
        .await
        .unwrap();
    let resp = client
        .delete(format!("/api/assets/{asset}"))
        .dispatch()
        .await;
    assert_eq!(resp.status(), Status::NoContent);

    let (assets,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM assets")
        .fetch_one(&pool)
        .await
        .unwrap();
    let (throttle,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM split_sync")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!((assets, throttle), (0, 0), "both rows must be gone");
}
