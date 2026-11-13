//! Reading a caller's rows into a `Bundle`. All export SQL lives here.
//!
//! The queries are transitive by design: only `portfolios` carries `owner_id`, so
//! assets and transactions are reached through `portfolio_id` and `asset_id`. The
//! caller's ids are checked by `access::require` in the handler *before* anything
//! here runs; nothing in this file decides who may see a row.

use chrono::{NaiveDate, Utc};
use sqlx::SqlitePool;
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
    Other,
}

impl DbFailure {
    pub fn status(&self) -> rocket::http::Status {
        match self {
            DbFailure::Lock => rocket::http::Status::ServiceUnavailable,
            DbFailure::Other => rocket::http::Status::InternalServerError,
        }
    }
}

/// Classify a sqlx error as a lock or not.
///
/// sqlx has no `is_locked()` on `DatabaseError`, so this matches on what the
/// drivers actually surface: SQLite reports `SQLITE_BUSY` as code `5` and
/// `SQLITE_LOCKED` as code `6` (`sqlx-sqlite`'s `DatabaseError::code()` formats the
/// raw code), and Postgres — later — reports `40001` serialisation failure and
/// `40P01` deadlock detected. The message check is the belt: it is what a bundled
/// SQLite build actually prints.
///
/// Pool exhaustion counts as busy too, and not only by analogy. Under SQLite a reader
/// rarely lives long enough to meet `SQLITE_BUSY` — the likelier way to be kept out is
/// every connection in the pool being checked out — and `PoolTimedOut` is exactly as
/// retryable, so it must not answer a bare `500` where the caller was told to retry.
pub fn classify(e: sqlx::Error) -> DbFailure {
    if matches!(e, sqlx::Error::PoolTimedOut) {
        // Said out loud because this branch returns before the `ERROR:` line below, and
        // an exhausted pool is the one lock an operator can act on: a `SQLITE_BUSY` that
        // clears in milliseconds is not, but a pool with nothing free stays that way.
        eprintln!("WARN: backup: no database connection free within the pool timeout; answering 503");
        return DbFailure::Lock;
    }
    if let sqlx::Error::Database(db) = &e {
        let code = db.code().map(|c| c.to_string()).unwrap_or_default();
        let message = db.message().to_lowercase();
        if matches!(code.as_str(), "5" | "6" | "40001" | "40P01")
            || message.contains("database is locked")
            || message.contains("database table is locked")
        {
            return DbFailure::Lock;
        }
    }
    eprintln!("ERROR: backup database failure: {e}");
    DbFailure::Other
}

/// Build a bundle for exactly `portfolio_ids`, which the caller has already been
/// authorised for. Order follows the order requested, so the file matches what the
/// user ticked.
pub async fn build(
    pool: &SqlitePool,
    portfolio_ids: &[i32],
    include_prices: bool,
) -> Result<Bundle, DbFailure> {
    let mut portfolios = Vec::with_capacity(portfolio_ids.len());
    let mut symbols = std::collections::BTreeSet::new();

    for id in portfolio_ids {
        let row: Option<(String, Option<String>, String, String)> = sqlx::query_as(
            "SELECT name, description, currency, base_currency FROM portfolios WHERE id = ?",
        )
        .bind(*id)
        .fetch_optional(pool)
        .await
        .map_err(classify)?;

        let Some((name, description, currency, base_currency)) = row else {
            // `require` already cleared this id; a row vanishing underneath us is a
            // concurrent delete, not a denial.
            return Err(DbFailure::Lock);
        };

        let assets = assets_for(pool, *id).await?;
        for a in &assets {
            symbols.insert(a.symbol.clone());
        }
        portfolios.push(BundlePortfolio {
            bundle_ref: format!("p{id}"),
            name,
            description,
            currency,
            base_currency,
            assets,
        });
    }

    let prices = if include_prices {
        prices_for(pool, &symbols).await?
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

/// One `assets` row, in the order `assets_for` names its columns. A named type
/// rather than the inline tuple: seven positional columns is past what a reader can
/// hold in their head at the `for` that destructures it, and clippy's
/// `type_complexity` says so too.
type AssetRow = (i32, String, String, String, Option<String>, String, Option<String>);

/// One `transactions` row, in the order `assets_for` names its columns.
type TransactionRow = (String, String, String, String, chrono::DateTime<Utc>);

async fn assets_for(pool: &SqlitePool, portfolio_id: i32) -> Result<Vec<BundleAsset>, DbFailure> {
    let rows: Vec<AssetRow> = sqlx::query_as(
        "SELECT id, symbol, name, asset_type, sector, currency, isin \
         FROM assets WHERE portfolio_id = ? ORDER BY id",
    )
    .bind(portfolio_id)
    .fetch_all(pool)
    .await
    .map_err(classify)?;

    let mut out = Vec::with_capacity(rows.len());
    for (id, symbol, name, asset_type, sector, currency, isin) in rows {
        let txs: Vec<TransactionRow> = sqlx::query_as(
            "SELECT type, quantity, price, fee, date FROM transactions \
             WHERE asset_id = ? ORDER BY date, id",
        )
        .bind(id)
        .fetch_all(pool)
        .await
        .map_err(classify)?;

        out.push(BundleAsset {
            bundle_ref: format!("a{id}"),
            symbol,
            name,
            asset_type,
            sector,
            currency,
            isin,
            transactions: txs
                .into_iter()
                .map(|(r#type, quantity, price, fee, date)| BundleTransaction {
                    r#type,
                    quantity,
                    price,
                    fee,
                    date: date.to_rfc3339(),
                })
                .collect(),
        });
    }
    Ok(out)
}

async fn prices_for(
    pool: &SqlitePool,
    symbols: &std::collections::BTreeSet<String>,
) -> Result<Vec<BundlePrice>, DbFailure> {
    let mut out = Vec::new();
    // One query per symbol rather than a constructed IN list: the selection is
    // bounded by the portfolio's holdings, and a placeholder built by `format!`
    // is invisible to the access gate and to a reader alike.
    for symbol in symbols {
        let rows: Vec<(String, NaiveDate, String)> = sqlx::query_as(
            "SELECT symbol, date, close_price FROM historical_prices \
             WHERE symbol = ? ORDER BY date",
        )
        .bind(symbol)
        .fetch_all(pool)
        .await
        .map_err(classify)?;

        out.extend(rows.into_iter().map(|(symbol, date, close_price)| BundlePrice {
            symbol,
            date: date.format("%Y-%m-%d").to_string(),
            close_price,
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

    /// A database error that reports itself as a lock, which is how SQLite surfaces
    /// `SQLITE_BUSY` once the `busy_timeout` in `main.rs` has expired. `code()` keeps
    /// the trait default of `None`, so this exercises the message half of `classify`;
    /// the code half is pinned over HTTP by
    /// `tests/backup_test.rs::a_locked_database_is_a_503_with_retry_after_from_the_restore_route`,
    /// which drives a real `SQLITE_BUSY` out of a second connection.
    #[derive(Debug)]
    struct FakeLock;

    impl std::fmt::Display for FakeLock {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("database is locked")
        }
    }
    impl std::error::Error for FakeLock {}

    impl DatabaseError for FakeLock {
        fn message(&self) -> &str {
            "database is locked"
        }
        // The trait's other required methods are plumbing: a `DatabaseError` must be
        // able to hand out `self` as a `dyn Error` because `sqlx::Error::Database`
        // stores it as a boxed trait object, and `kind()` is this sqlx version's
        // fourth required method (mapped violations only; a lock has none).
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

    #[test]
    fn a_lock_by_message_is_a_503() {
        let busy = sqlx::Error::Database(Box::new(FakeLock));
        assert!(matches!(classify(busy), DbFailure::Lock));
    }

    #[test]
    fn anything_else_is_a_500_not_a_503() {
        // The other half of the rule in `src/access.rs`: an outage must not be
        // reported as a client error, and a client error must not be reported as an
        // outage the client should retry. Without this arm, every schema failure in
        // the application would answer `503` with a `Retry-After`.
        #[derive(Debug)]
        struct FakeOther;
        impl std::fmt::Display for FakeOther {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("no such table")
            }
        }
        impl std::error::Error for FakeOther {}
        impl DatabaseError for FakeOther {
            fn message(&self) -> &str {
                "no such table"
            }
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
        let other = sqlx::Error::Database(Box::new(FakeOther));
        assert!(matches!(classify(other), DbFailure::Other));
        assert_eq!(DbFailure::Other.status(), Status::InternalServerError);
    }

    /// The SQLite codes `classify` claims to read. A `SQLITE_BUSY` carries `5` and a
    /// `SQLITE_LOCKED` carries `6`; if the match arm lost them, the message check
    /// alone would decide, and a driver that words its message differently would
    /// answer a `500` for a retryable lock.
    #[test]
    fn the_busy_and_locked_codes_are_classified_as_locks() {
        #[derive(Debug)]
        struct FakeCode(&'static str);
        impl std::fmt::Display for FakeCode {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "sqlite code {}", self.0)
            }
        }
        impl std::error::Error for FakeCode {}
        impl DatabaseError for FakeCode {
            fn message(&self) -> &str {
                "nothing in the message matches"
            }
            fn code(&self) -> Option<std::borrow::Cow<'_, str>> {
                Some(std::borrow::Cow::Borrowed(self.0))
            }
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
        for code in ["5", "6"] {
            let err = sqlx::Error::Database(Box::new(FakeCode(code)));
            assert!(matches!(classify(err), DbFailure::Lock), "code {code}");
        }
        // And a code that is not one of them is still an outage.
        let err = sqlx::Error::Database(Box::new(FakeCode("1")));
        assert!(matches!(classify(err), DbFailure::Other));
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
