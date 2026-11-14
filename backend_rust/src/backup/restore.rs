//! Counting, planning, and applying a bundle. All restore SQL lives here.
//!
//! Two rules hold throughout. **A bundle ref is a bundle-local label and nothing
//! else.** It names a row inside one bundle, so validation can refuse two rows that
//! claim the same label; it is never a database id, it is never persisted, and nothing
//! here matches one against a stored row. It does not even carry the parent/child
//! link — the bundle's nesting does, by putting an asset inside its portfolio. The
//! write path goes further and never reads a ref at all: a ref cannot reach a column
//! and an empty one cannot reach an insert. Every link comes from the freshly
//! allocated id (`RETURNING id`) of the insert that made the row, so a restore always
//! mints new ids rather than reusing anything the bundle names. **The whole restore is
//! one transaction** — a bundle that fails anywhere writes nothing.
//!
//! [`plan_for`] is the read half: it is what the user is shown before a restore is
//! allowed, so its numbers are the promise [`apply`] has to keep. [`apply`] is the
//! write half, and it keeps that promise inside a single `pool.begin()` — every
//! insert and every delete commits together or not at all.

use serde::Serialize;
use sqlx::SqlitePool;
use crate::backup::{BackupConfig, Bundle};
use crate::backup::export::{classify, DbFailure};

/// What to do with what the caller already owns. Merge is deliberately absent:
/// "which transaction is a duplicate?" has no clean answer, and a wrong guess
/// silently corrupts a tax ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Copy,
    Replace,
}

impl Mode {
    /// `None` for anything other than exactly `copy` or `replace`, so an
    /// unrecognised mode is a `422` rather than a default.
    pub fn from_query(raw: Option<&str>) -> Option<Mode> {
        match raw.map(str::trim) {
            Some("copy") => Some(Mode::Copy),
            Some("replace") => Some(Mode::Replace),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Copy => "copy",
            Mode::Replace => "replace",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Counts {
    pub portfolios: usize,
    pub assets: usize,
    pub transactions: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub schema_version: i64,
    pub mode: String,
    pub delete: Counts,
    pub insert: Counts,
    pub prices: usize,
    pub warnings: Vec<String>,
}

/// Rows the caller owns today — the exact set `replace` would delete.
///
/// Scoped by `owner_id` and by nothing else: every count a foreign bundle can
/// provoke is a count of the caller's own rows, because the caller's id is the
/// only filter any of these three queries has.
pub async fn owned_counts(pool: &SqlitePool, user_id: i32) -> Result<Counts, DbFailure> {
    let portfolios: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM portfolios WHERE owner_id = ?")
        .bind(user_id as i64)
        .fetch_one(pool)
        .await
        .map_err(classify)?;

    let assets: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM assets WHERE portfolio_id IN \
         (SELECT id FROM portfolios WHERE owner_id = ?)",
    )
    .bind(user_id as i64)
    .fetch_one(pool)
    .await
    .map_err(classify)?;

    let transactions: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM transactions WHERE asset_id IN \
         (SELECT id FROM assets WHERE portfolio_id IN \
          (SELECT id FROM portfolios WHERE owner_id = ?))",
    )
    .bind(user_id as i64)
    .fetch_one(pool)
    .await
    .map_err(classify)?;

    Ok(Counts {
        portfolios: portfolios.0 as usize,
        assets: assets.0 as usize,
        transactions: transactions.0 as usize,
    })
}

/// Counts in the bundle, which is what the UI shows as "will insert".
pub fn bundle_counts(bundle: &Bundle) -> Counts {
    let mut c = Counts {
        portfolios: bundle.portfolios.len(),
        assets: 0,
        transactions: 0,
    };
    for p in &bundle.portfolios {
        c.assets += p.assets.len();
        for a in &p.assets {
            c.transactions += a.transactions.len();
        }
    }
    c
}

/// The plan the user must see before restoring. Reads only; writes nothing.
pub async fn plan_for(
    pool: &SqlitePool,
    user_id: i32,
    bundle: &Bundle,
    mode: Mode,
) -> Result<Plan, DbFailure> {
    let delete = match mode {
        Mode::Copy => Counts::default(),
        Mode::Replace => owned_counts(pool, user_id).await?,
    };

    let mut warnings = Vec::new();
    if mode == Mode::Replace && delete.portfolios > 0 {
        warnings.push(format!(
            "replace will delete all {} portfolio(s) you own, including any not in this bundle",
            delete.portfolios
        ));
    }
    if !bundle.prices.is_empty() {
        warnings.push(
            "this bundle carries price history; it is shared market data, not your data".into(),
        );
    }

    Ok(Plan {
        schema_version: bundle.schema_version,
        mode: mode.as_str().to_string(),
        delete,
        insert: bundle_counts(bundle),
        prices: bundle.prices.len(),
        warnings,
    })
}

/// What a restore wrote, and what it cost the caller in new ids.
#[derive(Debug, Clone, Serialize)]
pub struct RestoreOutcome {
    pub inserted: Counts,
    /// The database ids the restore allocated, in bundle order. Returned so the UI
    /// can link straight to what appeared — and as proof that the ids are new.
    pub portfolio_ids: Vec<i32>,
    pub prices_inserted: usize,
}

/// Why a restore stopped after validation passed.
#[derive(Debug)]
pub enum ApplyError {
    /// `apply_prices` was asked for while the server flag is off. A `422`, never a
    /// silently ignored request.
    PricesDisabled,
    Db(DbFailure),
}

impl ApplyError {
    pub fn status(&self) -> rocket::http::Status {
        match self {
            ApplyError::PricesDisabled => rocket::http::Status::UnprocessableEntity,
            ApplyError::Db(d) => d.status(),
        }
    }
}

/// So every database statement in [`apply`] can end in `.map_err(classify)?` rather
/// than re-wrapping by hand: a `DbFailure` is one of this enum's variants, and the
/// conversion has exactly one possible shape.
impl From<DbFailure> for ApplyError {
    fn from(d: DbFailure) -> Self {
        ApplyError::Db(d)
    }
}

/// Apply a bundle.
///
/// **One transaction**: every insert and, for `replace`, every delete happens inside
/// it, so a bundle that fails on row 201 leaves the database exactly as it was.
/// Partial application is not a state this feature can produce.
///
/// Validation belongs to the caller and the route does it: `parse_and_validate` is
/// the only path to this function over HTTP, and it refuses an unsupported version,
/// a duplicate ref, a non-decimal amount and an out-of-range date before a
/// transaction is opened. `apply` does not re-validate — but it does not trust
/// either. Every statement is parameterised, every parent id comes from the
/// `RETURNING id` of the insert that made it, and a value that still cannot be
/// written fails the whole transaction rather than writing half a bundle.
pub async fn apply(
    pool: &SqlitePool,
    user_id: i32,
    bundle: &Bundle,
    mode: Mode,
    apply_prices: bool,
    cfg: &BackupConfig,
) -> Result<RestoreOutcome, ApplyError> {
    // Checked before anything else, and only when there is a price section to write:
    // asking for prices the server forbids is a refusal, never a silent ignore, but a
    // bundle with no prices asks for nothing the flag could forbid.
    if !bundle.prices.is_empty() && apply_prices && !cfg.allow_price_restore {
        return Err(ApplyError::PricesDisabled);
    }

    let mut tx = pool.begin().await.map_err(classify)?;

    if mode == Mode::Replace {
        // Reverse dependency order, scoped by `owner_id` and by nothing else: no
        // caller-supplied id is involved, so no bundle value can widen the delete.
        //
        // `split_sync` goes first and explicitly. Its `asset_id` is a plain INTEGER
        // PRIMARY KEY with **no** foreign key to `assets`, so `PRAGMA foreign_keys`
        // does not reach it: a delete that skipped this statement would strand one
        // orphan row per deleted asset permanently, and nothing would ever notice.
        // Then the three real foreign keys, children before parents — `foreign_keys`
        // is ON in this app and none of these FKs declare `ON DELETE CASCADE`, so
        // deleting a parent with children still attached is an error, not a cascade.
        sqlx::query(
            "DELETE FROM split_sync WHERE asset_id IN \
             (SELECT id FROM assets WHERE portfolio_id IN \
              (SELECT id FROM portfolios WHERE owner_id = ?))",
        )
        .bind(user_id as i64)
        .execute(&mut *tx)
        .await
        .map_err(classify)?;

        sqlx::query(
            "DELETE FROM transactions WHERE asset_id IN \
             (SELECT id FROM assets WHERE portfolio_id IN \
              (SELECT id FROM portfolios WHERE owner_id = ?))",
        )
        .bind(user_id as i64)
        .execute(&mut *tx)
        .await
        .map_err(classify)?;

        sqlx::query(
            "DELETE FROM assets WHERE portfolio_id IN \
             (SELECT id FROM portfolios WHERE owner_id = ?)",
        )
        .bind(user_id as i64)
        .execute(&mut *tx)
        .await
        .map_err(classify)?;

        sqlx::query("DELETE FROM portfolios WHERE owner_id = ?")
            .bind(user_id as i64)
            .execute(&mut *tx)
            .await
            .map_err(classify)?;
    }

    let mut portfolio_ids = Vec::with_capacity(bundle.portfolios.len());
    let mut inserted = Counts::default();

    for p in &bundle.portfolios {
        // `RETURNING id` is the house pattern (see `create_portfolio`) and is one
        // of the few constructs SQLite and Postgres already agree on.
        let (new_portfolio,) = sqlx::query_as::<_, (i32,)>(
            "INSERT INTO portfolios (name, description, currency, base_currency, owner_id) \
             VALUES (?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(&p.name)
        .bind(&p.description)
        .bind(&p.currency)
        .bind(&p.base_currency)
        .bind(user_id as i64)
        .fetch_one(&mut *tx)
        .await
        .map_err(classify)?;
        portfolio_ids.push(new_portfolio);
        inserted.portfolios += 1;

        for a in &p.assets {
            let (new_asset,) = sqlx::query_as::<_, (i32,)>(
                "INSERT INTO assets (portfolio_id, symbol, name, asset_type, sector, currency, isin) \
                 VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING id",
            )
            .bind(new_portfolio)
            .bind(&a.symbol)
            .bind(&a.name)
            .bind(&a.asset_type)
            .bind(&a.sector)
            .bind(&a.currency)
            .bind(&a.isin)
            .fetch_one(&mut *tx)
            .await
            .map_err(classify)?;
            inserted.assets += 1;

            for t in &a.transactions {
                // Validation already proved this parses; parsing again here is what
                // keeps a caller that skipped validation from writing a row the
                // ordinary write path would refuse. It cannot half-write: the `?`
                // abandons the transaction, and the rollback takes every row above.
                let when = chrono::DateTime::parse_from_rfc3339(&t.date)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .map_err(|_| ApplyError::Db(DbFailure::Other))?;
                sqlx::query(
                    "INSERT INTO transactions (asset_id, type, quantity, price, fee, date) \
                     VALUES (?, ?, ?, ?, ?, ?)",
                )
                .bind(new_asset)
                .bind(&t.r#type)
                .bind(&t.quantity)
                .bind(&t.price)
                .bind(&t.fee)
                .bind(when)
                .execute(&mut *tx)
                .await
                .map_err(classify)?;
                inserted.transactions += 1;
            }
        }
    }

    let mut prices_inserted = 0usize;
    if apply_prices && cfg.allow_price_restore {
        for price in &bundle.prices {
            let day = chrono::NaiveDate::parse_from_str(&price.date, "%Y-%m-%d")
                .map_err(|_| ApplyError::Db(DbFailure::Other))?;
            // DO NOTHING, not DO UPDATE: existing cached prices stay authoritative,
            // so a bundle can fill gaps but cannot overwrite what is already there.
            // The unique index `uq_historical_prices_symbol_date` backs this.
            let res = sqlx::query(
                "INSERT INTO historical_prices (symbol, date, close_price) \
                 VALUES (?, ?, ?) ON CONFLICT (symbol, date) DO NOTHING",
            )
            .bind(&price.symbol)
            .bind(day)
            .bind(&price.close_price)
            .execute(&mut *tx)
            .await
            .map_err(classify)?;
            prices_inserted += res.rows_affected() as usize;
        }
    }

    tx.commit().await.map_err(classify)?;

    Ok(RestoreOutcome {
        inserted,
        portfolio_ids,
        prices_inserted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::{
        BundleAsset, BundlePortfolio, BundleTransaction, CURRENT_SCHEMA_VERSION, FORMAT_TAG,
        row_count,
    };

    #[test]
    fn only_the_two_modes_parse_and_nothing_else_does() {
        // Including `None`: there is no default mode, so a request that named none
        // gets a 422 rather than the mode this build happens to prefer.
        assert_eq!(Mode::from_query(Some("copy")), Some(Mode::Copy));
        assert_eq!(Mode::from_query(Some("replace")), Some(Mode::Replace));
        assert_eq!(Mode::from_query(Some(" copy ")), Some(Mode::Copy));
        for raw in ["", " ", "merge", "COPY", "Replace", "copying"] {
            assert_eq!(Mode::from_query(Some(raw)), None, "{raw:?}");
        }
        assert_eq!(Mode::from_query(None), None);
        assert_eq!(Mode::Copy.as_str(), "copy");
        assert_eq!(Mode::Replace.as_str(), "replace");
    }

    /// One portfolio, two assets, one transaction on the first of them.
    fn bundle() -> Bundle {
        let tx = BundleTransaction {
            r#type: "BUY".into(),
            quantity: "1".into(),
            price: "1".into(),
            fee: "0".into(),
            date: "2024-01-01T00:00:00Z".into(),
        };
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
                assets: vec![
                    BundleAsset {
                        bundle_ref: "a1".into(),
                        symbol: "S".into(),
                        name: "S".into(),
                        asset_type: "STOCK".into(),
                        sector: None,
                        currency: "USD".into(),
                        isin: None,
                        transactions: vec![tx],
                    },
                    BundleAsset {
                        bundle_ref: "a2".into(),
                        symbol: "T".into(),
                        name: "T".into(),
                        asset_type: "STOCK".into(),
                        sector: None,
                        currency: "USD".into(),
                        isin: None,
                        transactions: vec![],
                    },
                ],
            }],
            prices: vec![],
        }
    }

    #[test]
    fn bundle_counts_counts_every_level_of_the_bundle() {
        // The same three levels `row_count` walks on the way in. If the two ever
        // disagree, the plan promises a different number of rows than validation
        // counted, and the user is shown one file and told about another.
        let bundle = bundle();
        let c = bundle_counts(&bundle);
        assert_eq!((c.portfolios, c.assets, c.transactions), (1, 2, 1));
        assert_eq!(
            row_count(&bundle),
            c.portfolios + c.assets + c.transactions,
            "row_count counts a portfolio row for each portfolio; these counts must agree \
             (on a bundle with no price section — the ceiling also counts price rows, which \
             belong to none of the three tables counted here)"
        );
    }
}
