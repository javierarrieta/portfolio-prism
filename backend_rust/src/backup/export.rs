//! Reading a caller's rows into a `Bundle`. All export SQL lives here.
//!
//! The queries are transitive by design: only `portfolios` carries `owner_id`, so
//! assets and transactions are reached through `portfolio_id` and `asset_id`. The
//! caller's ids are checked by `access::require` in the handler *before* anything
//! here runs; nothing in this file decides who may see a row.

use chrono::Utc;
use crate::db::DbPool;
use crate::backup::{
    Bundle, BundleAsset, BundlePortfolio, BundlePrice, BundleTransaction, CURRENT_SCHEMA_VERSION,
    FORMAT_TAG, MAX_BUNDLE_BYTES, MAX_BUNDLE_ROWS, row_count,
};

/// A database failure, kept distinct from a client error so the handler can answer
/// `503` for a lock and `500` for anything else. Collapsing the two is what
/// `src/access.rs` warns against: an outage would look like a bad client.
#[derive(Debug)]
pub enum DbFailure {
    Lock,
    /// A row the caller was authorised for is no longer there: `access::require`
    /// cleared the id and the read that followed found nothing, so a delete landed in
    /// between. Not retryable — the portfolio is not coming back — so it answers `404`,
    /// not a `503` with a `Retry-After` promising otherwise. Only `export::build`
    /// produces it; nothing that reads a bundle can name a server-side row.
    Missing,
    Other,
}

impl DbFailure {
    pub fn status(&self) -> rocket::http::Status {
        match self {
            DbFailure::Lock => rocket::http::Status::ServiceUnavailable,
            DbFailure::Missing => rocket::http::Status::NotFound,
            DbFailure::Other => rocket::http::Status::InternalServerError,
        }
    }
}

/// Classify a sqlx error as retryable or not.
///
/// Postgres has no `SQLITE_BUSY`. Three conditions mean "try later", and they are the
/// only things that produce a `503` here (spec §5.4):
///
/// * the pool had no connection within `acquire_timeout` (`db::pool_options`) —
///   [`sqlx::Error::PoolTimedOut`], which carries no SQLSTATE and so is matched by
///   variant, not by code;
/// * `statement_timeout` fired: `57014 query_canceled`. It is the code Postgres
///   reports for *any* cancelled statement, and `db::pool_options`' 30 s limit is the
///   only thing in this application that cancels one of its own accord — an operator
///   calling `pg_cancel_backend()` produces the same SQLSTATE and the same answer;
/// * the transaction was asked to give up: `40001` serialisation failure, `40P01`
///   deadlock detected, or `55P03` lock_not_available.
///
/// `55P03` is named rather than swallowed by the `500` catch-all, and it is worth saying
/// plainly that nothing here can currently raise it: no `lock_timeout` is set at any
/// level this app controls, so that arm only fires if an operator sets one at role,
/// database or server level. There is no unbounded wait either way, because the statement
/// timeout cancels a lock-blocked statement at 30 s as `57014` above.
///
/// Everything else is a `500`, because it is a bug and a retry will not help. The
/// SQLite version of this function also matched `SQLITE_BUSY`/`SQLITE_LOCKED` — the
/// primary codes and their extended forms — and grepped the message for "database is
/// locked"; all of that is gone, because a message match would keep answering `503`
/// for text the Postgres driver has no reason to produce, and because SQLSTATEs are
/// the engine's contract while prose in an error message is not.
pub fn classify(e: sqlx::Error) -> DbFailure {
    if matches!(e, sqlx::Error::PoolTimedOut) {
        // Said out loud because this branch returns before the `ERROR:` line below, and
        // an exhausted pool is the one retryable condition an operator can act on: a
        // serialisation failure that clears in milliseconds is not, but a pool with
        // nothing free stays that way.
        eprintln!("WARN: backup: no database connection free within the pool timeout; answering 503");
        return DbFailure::Lock;
    }
    if let sqlx::Error::Database(db) = &e {
        // `code()` is the SQLSTATE for `sqlx::postgres::PgDatabaseError`, and it is the
        // only thing here worth matching: `db.message()` is English prose Postgres is
        // free to reword, and `db.kind()` has no variant for any of the four codes above
        // (they all map to `ErrorKind::Other`), so the code is the whole signal.
        if matches!(
            db.code().as_deref(),
            Some("40001") | Some("40P01") | Some("55P03") | Some("57014")
        ) {
            eprintln!("WARN: backup: retryable database failure ({e}); answering 503");
            return DbFailure::Lock;
        }
    }
    eprintln!("ERROR: backup database failure: {e}");
    DbFailure::Other
}

/// Build a bundle for exactly `portfolio_ids`, which the caller has already been
/// authorised for. Order follows the order requested, so the file matches what the
/// user ticked.
///
/// **Every read `build` itself issues runs inside one `READ ONLY REPEATABLE READ`
/// transaction** ([`begin_read_tx`]), because the product is a point-in-time copy. Scope
/// matters here: the route does other reads before this function is reached — the auth
/// guard's read of `users`, and `require`'s per-id authorisation check in
/// `api_routes::backup` — and those run outside it, at the server default. What is one
/// snapshot is the bundle's own data, which is the thing that must not straddle two
/// states. On the pool each query
/// is its own implicit transaction, so a commit landing between the portfolio read and
/// the per-asset transaction reads yields a bundle that straddles two database states —
/// a portfolio row from one moment and its transactions from a later one.
/// `restore::apply` has always been one transaction; export being one too is the same
/// promise seen from the other side.
///
/// `REPEATABLE READ` and not the server default, and that is the whole point of the
/// transaction: at READ COMMITTED Postgres takes a **fresh snapshot per statement**, so
/// merely wrapping the reads in a transaction would not prevent the straddle above — it
/// would only make it less likely. `REPEATABLE READ` takes one snapshot per transaction,
/// which is the guarantee the previous paragraph claims.
/// `tests/export_snapshot_test.rs` reads the isolation level back from the server on the
/// transaction [`begin_read_tx`] opens, and shows a commit landing between two reads
/// staying invisible to the second one.
///
/// Not `SERIALIZABLE`, which the write path needs and a read does not: predicate locking
/// on an export that writes nothing would make a read-only route pay for a serialiser it
/// has nothing to serialise against. And `READ ONLY` is sent without `FOR UPDATE`, which
/// is the half that matters for concurrency: a read transaction that takes no row lock is
/// what lets a writer keep working while an export runs —
/// `a_read_only_export_does_not_queue_behind_a_writer` is what fails if someone
/// "hardens" it into a lock. `READ ONLY` on its own takes no lock of any kind; it only
/// forbids writing, which is what [`EXPORT_ISOLATION`] is for.
pub async fn build(
    pool: &DbPool,
    portfolio_ids: &[i64],
    include_prices: bool,
) -> Result<Bundle, DbFailure> {
    let mut tx = begin_read_tx(pool).await?;
    let bundle = build_in(&mut tx, portfolio_ids, include_prices).await?;
    // A read-only commit: nothing was written, and ending the transaction is what
    // releases the one snapshot every read above shared.
    tx.commit().await.map_err(classify)?;
    Ok(bundle)
}

/// The transaction [`build`] reads inside, opened in one place so that every test of the
/// isolation level goes through the same call `build` makes, and so that the portability
/// gate has one spelling to look for. `build` is its only caller, and the gate rule is
/// what keeps a second caller from appearing with a bare `pool.begin()` instead — see
/// Rule 3 of `scripts/check_backup_portability.sh`.
pub async fn begin_read_tx(
    pool: &DbPool,
) -> Result<sqlx::Transaction<'_, sqlx::Postgres>, DbFailure> {
    pool.begin_with(EXPORT_ISOLATION).await.map_err(classify)
}

/// The statement [`begin_read_tx`] sends, verbatim, through `Pool::begin_with`. Named so
/// the isolation level is stated once, where it takes effect, and so that grepping
/// `REPEATABLE READ` finds the code rather than only the docs.
///
/// `READ ONLY` is not decoration. `REPEATABLE READ` alone buys the snapshot but still
/// lets a statement write, so "this transaction writes nothing" was a property of the
/// Rust code — true while `build_in` contained only reads, and silently false the moment
/// someone added one. With `READ ONLY` the server enforces it: a write attempt inside the
/// export raises `25006 read_only_sql_transaction`, which `classify` maps to a `500`
/// rather than letting the export mutate the tenant's rows from a route that documents
/// itself as a read. `tests/export_snapshot_test.rs` reads `transaction_read_only` back
/// from the server for the same reason it reads `transaction_isolation`.
///
/// The two clauses are different kinds of thing and only one of them is a lock: `READ
/// ONLY` is the transaction's access mode, `FOR UPDATE` is a lock strength, and sending
/// the first without the second is the whole point — see the note on [`build`] about
/// `a_read_only_export_does_not_queue_behind_a_writer`. `BEGIN READ ONLY FOR UPDATE` is
/// not even valid syntax, so nobody can typo one into the other by accident.
pub const EXPORT_ISOLATION: &str = "BEGIN READ ONLY ISOLATION LEVEL REPEATABLE READ";

/// The reads themselves, on a transaction [`build`] opened. Separate from `build` so
/// the transaction's scope is three lines and impossible to widen by accident.
async fn build_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    portfolio_ids: &[i64],
    include_prices: bool,
) -> Result<Bundle, DbFailure> {
    let mut portfolios = Vec::with_capacity(portfolio_ids.len());
    let mut symbols = std::collections::BTreeSet::new();

    for id in portfolio_ids {
        let row = sqlx::query!(
            "SELECT name, description, currency, base_currency FROM portfolios WHERE id = $1",
            *id
        )
        .fetch_optional(&mut **tx)
        .await
        .map_err(classify)?;

        let Some(row) = row else {
            // `require` already cleared this id; a row vanishing underneath us is a
            // concurrent delete, not a denial, and not something a retry fixes.
            return Err(DbFailure::Missing);
        };

        let assets = assets_for(&mut *tx, *id).await?;
        for a in &assets {
            symbols.insert(a.symbol.clone());
        }
        portfolios.push(BundlePortfolio {
            bundle_ref: format!("p{id}"),
            name: row.name,
            description: row.description,
            currency: row.currency,
            base_currency: row.base_currency,
            assets,
        });
    }

    let prices = if include_prices {
        prices_for(&mut *tx, &symbols).await?
    } else {
        Vec::new()
    };

    Ok(Bundle {
        format: FORMAT_TAG.to_string(),
        schema_version: CURRENT_SCHEMA_VERSION,
        created_at: Utc::now().to_rfc3339(),
        portfolios,
        prices,
    })
}

/// Why a built bundle is too large to hand back, or `None` when it fits.
///
/// Both ceilings are the ones **restore** enforces, so a bundle over either is a file
/// its own restore will refuse — discovered by the user, after the download. Checked
/// here instead, where the caller can still do something about it: export fewer
/// portfolios. Rows are checked first because it is a counter, while the byte check
/// has to serialise.
///
/// One asymmetry between the two ends, so a surprise here is not a mystery: this byte
/// ceiling measures `serde_json::to_vec(&bundle)` — the compact form this build writes —
/// while the upload ceiling measures the **raw request bytes** the caller sent. A bundle
/// that leaves here just under the cap and is then re-saved pretty-printed by an editor is
/// over the ceiling on the way back in and comes back `413`. Size is measured at both ends
/// and the two measurements are not the same measurement.
///
/// The byte ceiling is checked here as well as on the way in because the bundle is
/// already fully in memory by this point, so its serialised size is knowable — which is
/// not true of a stream being uploaded. It is a genuinely separate axis from the row
/// ceiling rather than a restatement of it: `row_count` counts rows, and a single long
/// text field — a description, an asset name — is megabytes of bundle carried by three
/// rows. A function rather than an inline `if` so both boundaries are testable without
/// seeding tens of thousands of rows.
pub fn cap_error(bundle: &Bundle) -> Option<String> {
    let rows = row_count(bundle);
    if rows > MAX_BUNDLE_ROWS {
        return Some(format!(
            "that selection is {rows} rows, over the {MAX_BUNDLE_ROWS} row limit; \
             export fewer portfolios, or leave out the price history"
        ));
    }

    // A `Bundle` is strings and sequences, so this cannot fail in practice; if it ever
    // did, the byte ceiling is simply not the thing that stops this response, and the
    // row ceiling above still stands.
    let bytes = serde_json::to_vec(bundle).ok()?.len();
    (bytes > MAX_BUNDLE_BYTES).then(|| {
        format!(
            "that selection is {bytes} bytes, over the {MAX_BUNDLE_BYTES} byte limit; \
             export fewer portfolios"
        )
    })
}

// `assets.id` is `BIGINT`. The hand-written tuple aliases this function used to read
// into were the only place that said so, and getting it wrong was a runtime
// mismatched-types error under sqlx-postgres, not a compile error. `query!` now infers
// both row shapes from the named columns, so the column list is the row type.
async fn assets_for(
    conn: &mut sqlx::PgConnection,
    portfolio_id: i64,
) -> Result<Vec<BundleAsset>, DbFailure> {
    let rows = sqlx::query!(
        "SELECT id, symbol, name, asset_type, sector, currency, isin \
         FROM assets WHERE portfolio_id = $1 ORDER BY id",
        portfolio_id
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(classify)?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let id = row.id;
        let txs = sqlx::query!(
            "SELECT type, quantity, price, fee, date FROM transactions \
             WHERE asset_id = $1 ORDER BY date, id",
            id
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(classify)?;

        out.push(BundleAsset {
            bundle_ref: format!("a{id}"),
            symbol: row.symbol,
            name: row.name,
            asset_type: row.asset_type,
            sector: row.sector,
            currency: row.currency,
            isin: row.isin,
            transactions: txs
                .into_iter()
                .map(|tx| BundleTransaction {
                    r#type: tx.r#type,
                    quantity: tx.quantity,
                    price: tx.price,
                    fee: tx.fee,
                    date: tx.date.to_rfc3339(),
                })
                .collect(),
        });
    }
    Ok(out)
}

async fn prices_for(
    conn: &mut sqlx::PgConnection,
    symbols: &std::collections::BTreeSet<String>,
) -> Result<Vec<BundlePrice>, DbFailure> {
    let mut out = Vec::new();
    // One query per symbol rather than a constructed IN list: the selection is
    // bounded by the portfolio's holdings, and a placeholder built by `format!`
    // is invisible to the access gate and to a reader alike.
    for symbol in symbols {
        let rows = sqlx::query!(
            "SELECT symbol, date, close_price FROM historical_prices \
             WHERE symbol = $1 ORDER BY date",
            symbol
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(classify)?;

        out.extend(rows.into_iter().map(|row| BundlePrice {
            symbol: row.symbol,
            date: row.date.format("%Y-%m-%d").to_string(),
            close_price: row.close_price,
        }));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::ACCEPTED_VERSIONS;
    use rocket::http::Status;
    use sqlx::error::DatabaseError;

    /// A database error carrying exactly the SQLSTATE `code` — or no code at all, when
    /// `code` is `None` — and a message that matches nothing.
    ///
    /// `sqlx::postgres::PgDatabaseError` cannot be constructed from outside the driver,
    /// so the four codes `classify` matches are exercised through this instead. The
    /// three near-identical fakes this replaces (`FakeLock`, `FakeOther`, `FakeCode`)
    /// existed because the old classifier read the message *and* the code; only the
    /// code is read now, so one fake with a field for it is enough.
    #[derive(Debug)]
    struct FakeDb {
        message: &'static str,
        code: Option<&'static str>,
    }

    impl FakeDb {
        fn error(self) -> sqlx::Error {
            sqlx::Error::Database(Box::new(self))
        }
    }

    /// A database error whose only interesting property is its SQLSTATE.
    fn error_with_code(code: Option<&'static str>) -> sqlx::Error {
        FakeDb {
            message: "nothing in this message matches anything classify reads",
            code,
        }
        .error()
    }

    /// A database error carrying `message` and no SQLSTATE at all, which is the shape
    /// the deleted message-matching used to accept.
    fn error_with_message(message: &'static str) -> sqlx::Error {
        FakeDb {
            message,
            code: None,
        }
        .error()
    }

    impl std::fmt::Display for FakeDb {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.message)
        }
    }
    impl std::error::Error for FakeDb {}

    impl DatabaseError for FakeDb {
        fn message(&self) -> &str {
            self.message
        }
        fn code(&self) -> Option<std::borrow::Cow<'_, str>> {
            self.code.map(std::borrow::Cow::Borrowed)
        }
        // The rest is plumbing: a `DatabaseError` must be able to hand out `self` as a
        // `dyn Error` because `sqlx::Error::Database` stores it as a boxed trait object,
        // and `kind()` is this sqlx version's fourth required method (mapped violations
        // only; none of the codes under test map to one).
        fn kind(&self) -> sqlx::error::ErrorKind {
            sqlx::error::ErrorKind::Other
        }
        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }
        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }
        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }
    }

    /// The four conditions spec §5.4 allows to be a `503`, by the SQLSTATE each one
    /// arrives as: serialisation failure, deadlock, `lock_timeout`, `statement_timeout`.
    #[test]
    fn the_retryable_sqlstates_are_locks() {
        for code in ["40001", "40P01", "55P03", "57014"] {
            assert!(
                matches!(classify(error_with_code(Some(code))), DbFailure::Lock),
                "code {code}"
            );
        }
    }

    /// The SQLite result codes the deleted classifier matched — `SQLITE_BUSY` (5),
    /// `SQLITE_LOCKED` (6) and the extended forms 261, 517, 262 — must now be outages.
    /// Without this the old arms could come back as "harmless" belt-and-braces matching
    /// and a `500` that should be a `500` would quietly become a retryable `503`.
    #[test]
    fn the_sqlite_busy_codes_are_no_longer_retryable() {
        for code in ["5", "6", "261", "517", "262"] {
            assert!(
                matches!(classify(error_with_code(Some(code))), DbFailure::Other),
                "code {code}"
            );
        }
    }

    /// The message half of the old classifier is gone, and this is what it would still
    /// match: SQLite's wording with no SQLSTATE behind it. Prose is not the engine's
    /// contract, so a message no longer buys a `Retry-After`.
    #[test]
    fn a_lock_shaped_message_without_a_sqlstate_is_an_outage() {
        assert!(matches!(
            classify(error_with_message("database is locked")),
            DbFailure::Other
        ));
    }

    #[tokio::test]
    async fn a_real_database_error_is_not_retryable() {
        // The false case driven through the driver rather than a fake, because a fake
        // proves the match arm and this proves the *shape*: a real Postgres failure
        // really does arrive as `Error::Database` with a SQLSTATE, which is what the
        // arm reads. A missing table is `42P01 undefined_table` — permanent, so a retry
        // would be an outage with a `Retry-After` header on it.
        let pool = crate::test_db::test_pool(&crate::test_db::next_caller("classify")).await;
        // macro-exempt: the whole point is a table that does not exist, so no macro can
        // describe it — `query!` would fail at compile time, not produce 42P01.
        let err = sqlx::query("SELECT * FROM a_table_no_test_created")
            .fetch_one(&pool)
            .await
            .expect_err("the table does not exist");
        let sqlx::Error::Database(db) = &err else {
            panic!("a missing table must arrive as a database error, got {err:?}");
        };
        assert_eq!(
            db.code().as_deref(),
            Some("42P01"),
            "the arm reads the SQLSTATE, so the fixture has to carry one"
        );
        assert!(matches!(classify(err), DbFailure::Other));
    }

    #[test]
    fn anything_else_is_a_500_not_a_503() {
        // The other half of the rule in `src/access.rs`: an outage must not be
        // reported as a client error, and a client error must not be reported as an
        // outage the client should retry. Without this arm, every schema failure in
        // the application would answer `503` with a `Retry-After`.
        // `42601 syntax_error`: retried forever that is an outage with a `Retry-After`
        // on it, and it is the code the plan named for this exact case.
        let other = error_with_code(Some("42601"));
        assert!(matches!(classify(other), DbFailure::Other));
        assert_eq!(DbFailure::Other.status(), Status::InternalServerError);
    }

    /// A bundle of `1 + assets` counted rows: one portfolio plus that many empty
    /// assets, which is the cheapest way to sit exactly on either side of the cap.
    fn bundle_with_assets(assets: usize) -> Bundle {
        Bundle {
            format: FORMAT_TAG.to_string(),
            schema_version: CURRENT_SCHEMA_VERSION,
            created_at: "2026-11-13T00:00:00Z".to_string(),
            portfolios: vec![BundlePortfolio {
                bundle_ref: "p1".into(),
                name: "P".into(),
                description: None,
                currency: "USD".into(),
                base_currency: "USD".into(),
                assets: (0..assets)
                    .map(|i| BundleAsset {
                        bundle_ref: format!("a{i}"),
                        symbol: "S".into(),
                        name: "S".into(),
                        asset_type: "STOCK".into(),
                        sector: None,
                        currency: "USD".into(),
                        isin: None,
                        transactions: vec![],
                    })
                    .collect(),
            }],
            prices: vec![],
        }
    }

    /// The two statuses are what keeps a busy database distinguishable from an
    /// outage for the caller, so they are pinned rather than implied by the handler.
    #[test]
    fn a_lock_is_retryable_and_anything_else_is_an_outage() {
        assert_eq!(DbFailure::Lock.status(), Status::ServiceUnavailable);
        assert_eq!(DbFailure::Other.status(), Status::InternalServerError);
    }

    /// A portfolio that vanished between `access::require` and the read that follows is
    /// permanent, so answering it as a `Lock` — which this did — tells the caller to
    /// retry something that will never succeed, with a `Retry-After` header to match.
    #[test]
    fn a_row_that_vanished_is_gone_not_busy() {
        assert_eq!(DbFailure::Missing.status(), Status::NotFound);
        assert_ne!(
            DbFailure::Missing.status(),
            DbFailure::Lock.status(),
            "a permanent condition must not share a status with a retryable one"
        );
    }

    #[test]
    fn pool_exhaustion_is_busy_and_a_plain_error_is_not() {
        assert!(matches!(classify(sqlx::Error::PoolTimedOut), DbFailure::Lock));
        assert!(matches!(classify(sqlx::Error::RowNotFound), DbFailure::Other));
    }

    /// The writer's version and the accepted set cannot be edited apart: a bundle
    /// this build writes is a bundle this build can read back.
    #[test]
    fn the_written_schema_version_is_the_newest_accepted() {
        assert_eq!(
            CURRENT_SCHEMA_VERSION,
            *ACCEPTED_VERSIONS.last().expect("there is always a newest accepted version"),
            "bumping the version means appending it to ACCEPTED_VERSIONS, never replacing \
             the version before it: a released version stays restorable forever"
        );
    }

    /// A bundle whose serialised size is `base + pad` bytes: one portfolio whose
    /// description is `pad` copies of `x`. That character needs no JSON escaping, so
    /// every padded byte is exactly one serialised byte, which is what makes both sides
    /// of the byte ceiling pinnable exactly rather than approximately.
    fn bundle_with_padding(pad: usize) -> Bundle {
        let mut b = bundle_with_assets(0);
        b.portfolios[0].description = Some("x".repeat(pad));
        b
    }

    fn serialised_bytes(bundle: &Bundle) -> usize {
        serde_json::to_vec(bundle).expect("a bundle serialises").len()
    }

    #[test]
    fn a_bundle_exactly_at_the_row_ceiling_is_still_exportable() {
        let b = bundle_with_assets(MAX_BUNDLE_ROWS - 1);
        assert_eq!(row_count(&b), MAX_BUNDLE_ROWS);
        assert_eq!(cap_error(&b), None);
    }

    #[test]
    fn one_row_over_the_ceiling_is_refused_naming_the_cap_and_the_remedy() {
        let b = bundle_with_assets(MAX_BUNDLE_ROWS);
        let err = cap_error(&b).expect("one row over the cap must be refused");
        assert!(err.contains(&MAX_BUNDLE_ROWS.to_string()), "{err}");
        assert!(err.contains("export fewer portfolios"), "{err}");
    }

    #[test]
    fn a_bundle_exactly_at_the_byte_ceiling_is_still_exportable() {
        let pad = MAX_BUNDLE_BYTES - serialised_bytes(&bundle_with_padding(0));
        let b = bundle_with_padding(pad);
        assert_eq!(serialised_bytes(&b), MAX_BUNDLE_BYTES);
        assert_eq!(cap_error(&b), None);
    }

    #[test]
    fn one_byte_over_the_byte_ceiling_is_refused_naming_the_cap_and_the_remedy() {
        let pad = MAX_BUNDLE_BYTES - serialised_bytes(&bundle_with_padding(0)) + 1;
        let b = bundle_with_padding(pad);
        assert_eq!(serialised_bytes(&b), MAX_BUNDLE_BYTES + 1);
        // The point of the byte check on its own axis: rows say nothing about size, so
        // three rows carrying one enormous text field are far under the row ceiling and
        // still too big to restore. Without this arm that bundle is exported and refused
        // later, by the upload, after the user has downloaded it.
        assert!(row_count(&b) < 10, "the row ceiling was never going to see this");
        let err = cap_error(&b).expect("one byte over the cap must be refused");
        assert!(err.contains("byte"), "{err}");
        assert!(err.contains(&MAX_BUNDLE_BYTES.to_string()), "{err}");
        assert!(err.contains("export fewer portfolios"), "{err}");
    }
}
