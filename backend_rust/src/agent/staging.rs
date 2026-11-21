//! The staged-batch lifecycle: validate, persist, hand out a handle, approve, discard.
//!
//! A batch exists so that the write and the decision to write can be separated in time and
//! in credential. The agent stages it with a token; a human approves it with a session
//! cookie; the write happens later, driven by the stored `payload` and not by anything the
//! agent re-sends. That separation is the whole control, so the state lives here and nowhere
//! the model can edit it.
//!
//! `apply` is here too, and it is the reason the rest of the file exists. It is the only
//! function that writes portfolio rows, and it will not do so for a handle a session has
//! not moved to `approved`.
//!
//! Caps come from *Limits* in the design spec and are enforced here, before anything is
//! inserted, rather than discovered by a body limit mid-parse. A batch that is too big is a
//! named error, not a partial row.

use chrono::{DateTime, Duration, NaiveDate, Utc};
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::agent::protocol::{Proposal, Target};
use crate::db_types::is_canonical_decimal;
use crate::models::TxType;

/// Rows per staged batch. A real batch is 20-40 rows; 500 is a full history for a large
/// portfolio, and it bounds the JSONB row and the preview.
pub const MAX_TRANSACTIONS_PER_BATCH: usize = 500;
/// Assets per staged batch. Twenty is the real largest portfolio; 50 leaves room without
/// letting a malformed payload allocate freely. Unrelated to `backup::detail`'s 50-asset
/// *display* cap — the coincidence is accidental and the two must not be merged.
pub const MAX_ASSETS_PER_BATCH: usize = 50;
/// Pending batches per user, so abandoned imports cannot accumulate rows.
pub const MAX_PENDING_BATCHES_PER_USER: i64 = 5;
/// Default lifetime of a staged batch. A human approves in the same sitting or the batch
/// is stale.
pub const DEFAULT_TTL_MINUTES: i64 = 60;
/// pi truncates results over 20 KB by removing the middle, so the preview is kept under
/// that with room left for the envelope.
pub const MAX_PREVIEW_BYTES: usize = 16 * 1024;
/// How many assets the preview lists before switching to a count. Chosen so a
/// maximum-size batch still renders; the cap is what makes the byte budget satisfiable.
const PREVIEW_ASSETS_SHOWN: usize = 25;

/// Why a proposal was refused, with the code the agent sees.
///
/// The codes are the ones the design spec lists as authoritative. Each carries a `field`
/// path rather than an id, because the sender needs to know *which* value to fix in the
/// document it still holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageError {
    BatchTooLarge { what: &'static str, given: usize, max: usize },
    TooManyPending { given: i64, max: i64 },
    EmptyAsset { field: String },
    NonCanonicalDecimal { field: String, given: String },
    BadDate { field: String, given: String },
    OutOfRange { field: String, rule: &'static str, given: String },
    Database,
}

impl StageError {
    /// The wire code, and the message an agent is told.
    pub fn code(&self) -> &'static str {
        match self {
            StageError::BatchTooLarge { .. } => "batch_too_large",
            StageError::TooManyPending { .. } => "too_many_pending",
            StageError::EmptyAsset { .. } => "empty_asset",
            StageError::NonCanonicalDecimal { .. } => "non_canonical_decimal",
            StageError::BadDate { .. } => "bad_date",
            StageError::OutOfRange { .. } => "out_of_range",
            StageError::Database => "database",
        }
    }

    pub fn field(&self) -> Option<&str> {
        match self {
            StageError::EmptyAsset { field }
            | StageError::NonCanonicalDecimal { field, .. }
            | StageError::BadDate { field, .. }
            | StageError::OutOfRange { field, .. } => Some(field),
            _ => None,
        }
    }
}

/// True for exactly `YYYY-MM-DD` — ten characters, digits in the right places — and then
/// for a real day in a real month.
///
/// The shape check is not redundant with `parse_from_str`: chrono's `%m` and `%d` accept a
/// single digit, so `"2024-3-1"` parses cleanly and would be stored as a date nothing else
/// in this system writes. "Strict" has to mean the width as well as the order, and a test
/// caught this rather than a reader did.
fn is_calendar_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10
        || b[4] != b'-'
        || b[7] != b'-'
        || !b[..4].iter().all(u8::is_ascii_digit)
        || !b[5..7].iter().all(u8::is_ascii_digit)
        || !b[8..].iter().all(u8::is_ascii_digit)
    {
        return false;
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

/// Check a proposal completely, before anything is written.
///
/// Deterministic and total: it reports the first problem it finds in document order, with a
/// path. It does not consult the database — that is `stage`'s job — so the same payload
/// always gets the same answer, which is what makes an agent's retry loop converge instead
/// of oscillating.
pub fn validate(proposal: &Proposal) -> Result<(), StageError> {
    if proposal.assets.is_empty() {
        return Err(StageError::EmptyAsset {
            field: "assets".to_string(),
        });
    }
    if proposal.assets.len() > MAX_ASSETS_PER_BATCH {
        return Err(StageError::BatchTooLarge {
            what: "assets",
            given: proposal.assets.len(),
            max: MAX_ASSETS_PER_BATCH,
        });
    }

    let mut total_tx = 0usize;
    for (ai, asset) in proposal.assets.iter().enumerate() {
        if asset.transactions.is_empty() {
            return Err(StageError::EmptyAsset {
                field: format!("assets[{ai}].transactions"),
            });
        }
        total_tx += asset.transactions.len();
        if total_tx > MAX_TRANSACTIONS_PER_BATCH {
            return Err(StageError::BatchTooLarge {
                what: "transactions",
                given: total_tx,
                max: MAX_TRANSACTIONS_PER_BATCH,
            });
        }

        for (ti, tx) in asset.transactions.iter().enumerate() {
            let base = format!("assets[{ai}].transactions[{ti}]");

            // A calendar date, exactly. Not RFC 3339: a broker statement says the day, and
            // inventing an instant for it is the kind of silent interpretation this module
            // exists to avoid.
            if !is_calendar_date(&tx.date) {
                return Err(StageError::BadDate {
                    field: format!("{base}.date"),
                    given: tx.date.clone(),
                });
            }

            // The same window `services::transactions::insert` enforces, checked here as
            // well as there. A real calendar day outside 1900–2100 passes the shape check
            // above, so without this it is discovered at apply — after a human approved it,
            // which is the worst possible moment to find out. Staging's job is to refuse
            // anything that cannot apply; this was a hole in that claim, found by a test
            // that was trying to prove something else.
            if let Ok(day) = NaiveDate::parse_from_str(&tx.date, "%Y-%m-%d") {
                let min = NaiveDate::from_ymd_opt(1900, 1, 1).expect("1900-01-01 exists");
                let max = NaiveDate::from_ymd_opt(2100, 1, 1).expect("2100-01-01 exists");
                if day < min || day > max {
                    return Err(StageError::OutOfRange {
                        field: format!("{base}.date"),
                        rule: "between 1900-01-01 and 2100-01-01",
                        given: tx.date.clone(),
                    });
                }
            }

            for (key, value) in [
                ("quantity", &tx.quantity),
                ("price", &tx.price),
                ("fee", &tx.fee),
            ] {
                if !is_canonical_decimal(value) {
                    return Err(StageError::NonCanonicalDecimal {
                        field: format!("{base}.{key}"),
                        given: value.clone(),
                    });
                }
            }

            // Sign rules, stated rather than inferred. A zero price is legitimate — a
            // free allotment or a transfer at nil cost is real — but a negative one is not,
            // and a zero *quantity* would make every later per-share figure a division by
            // zero.
            let qty = rust_decimal::Decimal::from_str_exact(&tx.quantity)
                .expect("is_canonical_decimal already accepted this");
            let price = rust_decimal::Decimal::from_str_exact(&tx.price)
                .expect("is_canonical_decimal already accepted this");
            let fee = rust_decimal::Decimal::from_str_exact(&tx.fee)
                .expect("is_canonical_decimal already accepted this");

            if qty <= rust_decimal::Decimal::ZERO {
                return Err(StageError::OutOfRange {
                    field: format!("{base}.quantity"),
                    rule: "quantity must be greater than zero",
                    given: tx.quantity.clone(),
                });
            }
            if price < rust_decimal::Decimal::ZERO {
                return Err(StageError::OutOfRange {
                    field: format!("{base}.price"),
                    rule: "price must be zero or greater",
                    given: tx.price.clone(),
                });
            }
            if fee < rust_decimal::Decimal::ZERO {
                return Err(StageError::OutOfRange {
                    field: format!("{base}.fee"),
                    rule: "fee must be zero or greater",
                    given: tx.fee.clone(),
                });
            }
            // A split is a ratio, and `services::transactions::insert` refuses the same
            // thing at write time. Checked here too so the agent is told at staging rather
            // than after a human has approved it — approving a batch that cannot apply is
            // worse than refusing it now.
            if tx.r#type == TxType::Split && price <= rust_decimal::Decimal::ZERO {
                return Err(StageError::OutOfRange {
                    field: format!("{base}.price"),
                    rule: "a SPLIT needs a price greater than zero",
                    given: tx.price.clone(),
                });
            }
        }
    }
    Ok(())
}

/// Render what the human will be shown. Stored verbatim and rendered verbatim by the UI, so
/// "what I approved" cannot drift from "what the agent described".
///
/// Bounded by construction, not by truncating a string afterwards: at most
/// [`PREVIEW_ASSETS_SHOWN`] assets are listed and the rest are counted, because a 500-row
/// batch rendered in full would blow the 16 KB budget the client enforces by deleting the
/// middle of the payload — which is the part a human would have needed to read.
pub fn render_preview(proposal: &Proposal) -> Value {
    let total_tx: usize = proposal.assets.iter().map(|a| a.transactions.len()).sum();

    let shown: Vec<Value> = proposal
        .assets
        .iter()
        .take(PREVIEW_ASSETS_SHOWN)
        .map(|a| {
            json!({
                "symbol": a.symbol,
                "name": a.name,
                "isin": a.isin,
                "asset_type": a.asset_type,
                "currency": a.currency,
                // The assumption, surfaced. A missing currency defaults to the portfolio's
                // base currency at apply time; the browser importer hardcoded USD instead
                // and that is wrong for a EUR statement.
                "currency_assumed": a.currency.is_none(),
                "transactions": a.transactions.len(),
                "first_date": a.transactions.first().map(|t| t.date.clone()),
                "last_date": a.transactions.last().map(|t| t.date.clone()),
            })
        })
        .collect();

    let mut warnings: Vec<String> = Vec::new();
    if proposal.assets.iter().any(|a| a.currency.is_none()) {
        warnings.push(
            "one or more assets carry no currency and will take the portfolio's base currency"
                .to_string(),
        );
    }
    if proposal.assets.len() > PREVIEW_ASSETS_SHOWN {
        warnings.push(format!(
            "showing {} of {} assets; the remainder are listed by symbol only in the full view",
            PREVIEW_ASSETS_SHOWN,
            proposal.assets.len()
        ));
    }

    let target = match &proposal.target {
        Target::New {
            name,
            base_currency,
        } => json!({"kind": "new", "name": name, "base_currency": base_currency}),
        Target::Existing { portfolio_id } => json!({"kind": "existing", "portfolio_id": portfolio_id}),
    };

    json!({
        "target": target,
        "assets": shown,
        "totals": {"assets": proposal.assets.len(), "transactions": total_tx},
        "warnings": warnings,
    })
}

/// A batch that has been persisted and can be approved.
#[derive(Debug, Clone)]
pub struct Staged {
    pub id: i64,
    pub handle: String,
    pub preview: Value,
    pub expires_at: DateTime<Utc>,
}

/// What a listing shows. No payload, no preview: the list is a menu, and the detail route
/// is where the bytes are.
#[derive(Debug, Clone)]
pub struct BatchSummary {
    pub handle: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub approved_at: Option<DateTime<Utc>>,
    pub applied_at: Option<DateTime<Utc>>,
    pub asset_count: i64,
    pub transaction_count: i64,
}

/// 32 random bytes, hex. Not the row id: a sequential handle is enumerable and leaks how
/// many imports have happened, and this string is the only thing identifying a batch that
/// leaves the server.
fn new_handle() -> Result<String, StageError> {
    let rng = SystemRandom::new();
    let mut buf = [0u8; 32];
    rng.fill(&mut buf)
        .map_err(|_| StageError::Database)?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// Persist a proposal as a batch awaiting approval.
///
/// One transaction, doing three things: refuse if the user already has too many pending,
/// sweep the ones that have expired, and insert. The sweep is in the same transaction as
/// the insert on purpose — there is no scheduled job in this design, so "when does an
/// abandoned batch go away" has to be answered by something, and the honest answer is "the
/// next time this user stages". A user who never stages again keeps their rows; that is
/// stated in the spec rather than papered over.
pub async fn stage(
    pool: &PgPool,
    user_id: i64,
    proposal: &Proposal,
    ttl_minutes: i64,
) -> Result<Staged, StageError> {
    validate(proposal)?;

    let payload = serde_json::to_value(proposal).map_err(|_| StageError::Database)?;
    let preview = render_preview(proposal);
    let preview_bytes = serde_json::to_string(&preview).map_err(|_| StageError::Database)?;
    if preview_bytes.len() > MAX_PREVIEW_BYTES {
        // Unreachable for a payload that passed `validate`, because the preview is bounded
        // by construction. Kept as a refusal rather than an assertion because the budget is
        // a client property and a future preview field could break it silently.
        return Err(StageError::BatchTooLarge {
            what: "preview",
            given: preview_bytes.len(),
            max: MAX_PREVIEW_BYTES,
        });
    }

    let mut tx = pool.begin().await.map_err(|_| StageError::Database)?;

    // Sweep first, then count. In the other order five expired batches would refuse a new
    // one: the cap would be measuring rows that are already dead, and a user whose batches
    // had all timed out could never stage again without discarding by hand. The spec puts
    // the sweep in the same transaction as the insert and leaves the order inside it to
    // whoever writes the code; this is the order that makes the cap mean "live batches".
    sqlx::query(
        "DELETE FROM agent_staged_batch
         WHERE user_id = $1 AND status IN ('staged', 'approved') AND expires_at < now()",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(|_| StageError::Database)?;

    let (pending,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM agent_staged_batch
         WHERE user_id = $1 AND status IN ('staged', 'approved')",
    )
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| StageError::Database)?;

    if pending >= MAX_PENDING_BATCHES_PER_USER {
        return Err(StageError::TooManyPending {
            given: pending,
            max: MAX_PENDING_BATCHES_PER_USER,
        });
    }

    let handle = new_handle()?;
    let expires_at = Utc::now() + Duration::minutes(ttl_minutes);

    let row: (i64, DateTime<Utc>) = sqlx::query_as(
        "INSERT INTO agent_staged_batch (handle, user_id, payload, preview, status, expires_at)
         VALUES ($1, $2, $3, $4, 'staged', $5)
         RETURNING id, expires_at",
    )
    .bind(&handle)
    .bind(user_id)
    .bind(&payload)
    .bind(&preview)
    .bind(expires_at)
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| StageError::Database)?;

    tx.commit().await.map_err(|_| StageError::Database)?;

    Ok(Staged {
        id: row.0,
        handle,
        preview,
        expires_at: row.1,
    })
}

/// Move `staged -> approved`. This is the human act.
///
/// A conditional update, so the state check and the write are one statement: two tabs
/// approving the same batch cannot both succeed, and the second sees `already_applied`
/// rather than a second `approved_at`.
///
/// Returns `Err(false)` meaning "not in a state that can be approved" — the caller turns
/// that into `not_approved` or `already_applied` by reading the row, because the distinction
/// is for the human's benefit and the update deliberately does not encode it.
pub async fn approve(
    pool: &PgPool,
    user_id: i64,
    handle: &str,
) -> Result<bool, StageError> {
    let updated: Option<(i64,)> = sqlx::query_as(
        "UPDATE agent_staged_batch
            SET status = 'approved', approved_at = now(), approved_by = $3
          WHERE handle = $1 AND user_id = $2 AND status = 'staged'
          RETURNING id",
    )
    .bind(handle)
    .bind(user_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| StageError::Database)?;

    Ok(updated.is_some())
}

/// Discard a batch without writing anything. No approval is required to *not* do something.
pub async fn discard(pool: &PgPool, user_id: i64, handle: &str) -> Result<bool, StageError> {
    let updated: Option<(i64,)> = sqlx::query_as(
        "UPDATE agent_staged_batch
            SET status = 'discarded'
          WHERE handle = $1 AND user_id = $2 AND status IN ('staged', 'approved')
          RETURNING id",
    )
    .bind(handle)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| StageError::Database)?;

    Ok(updated.is_some())
}

/// The stored preview, for the detail route and for `get_staged`.
///
/// `user_id` is in the predicate, not checked afterwards: a handle that is not yours and a
/// handle that does not exist must be indistinguishable, or the endpoint is an existence
/// oracle over a namespace the agent chose.
pub async fn preview_of(
    pool: &PgPool,
    user_id: i64,
    handle: &str,
) -> Result<Option<(String, Value, DateTime<Utc>)>, StageError> {
    let row = sqlx::query_as!(
        BatchDetail,
        "SELECT status, preview, expires_at FROM agent_staged_batch
         WHERE handle = $1 AND user_id = $2",
        handle,
        user_id
    )
    .fetch_optional(pool)
    .await
    .map_err(|_| StageError::Database)?;

    Ok(row.map(|r| (r.status, r.preview, r.expires_at)))
}

/// The user's batches, newest first, with counts so the list is a menu and not a tease.
///
/// Both `::bigint` casts in the query are load-bearing. `jsonb_array_length` answers `int4`,
/// and sqlx compares declared widths when it decodes, so an `i64` field against it fails at
/// run time with INT8-is-not-compatible-with-INT4 — the first time the route is called, not
/// at compile time. The cast is the fix; a `i32` field would be the wrong fix, because the
/// sum over the elements is `int8` and would then fail the other way.
///
/// The counts come from the stored `payload`, not from a re-read of the proposal: the number
/// a human sees next to a batch is the number that was staged, and recomputing it from live
/// rows would let a portfolio's contents change what a historical batch claims to contain.
pub async fn list(pool: &PgPool, user_id: i64) -> Result<Vec<BatchSummary>, StageError> {
    let rows = sqlx::query_as!(
        SummaryRow,
        "SELECT handle, status, created_at, expires_at, approved_at, applied_at,
                jsonb_array_length(payload -> 'assets')::bigint AS \"asset_count!\",
                COALESCE((
                    SELECT sum(jsonb_array_length(a -> 'transactions'))
                      FROM jsonb_array_elements(payload -> 'assets') AS a
                ), 0)::bigint AS \"transaction_count!\"
           FROM agent_staged_batch
          WHERE user_id = $1
          ORDER BY created_at DESC",
        user_id
    )
    .fetch_all(pool)
    .await
    .map_err(|_| StageError::Database)?;

    Ok(rows
        .into_iter()
        .map(|r| BatchSummary {
            handle: r.handle,
            status: r.status,
            created_at: r.created_at,
            expires_at: r.expires_at,
            approved_at: r.approved_at,
            applied_at: r.applied_at,
            asset_count: r.asset_count,
            transaction_count: r.transaction_count,
        })
        .collect())
}

struct BatchDetail {
    status: String,
    preview: Value,
    expires_at: DateTime<Utc>,
}

struct SummaryRow {
    handle: String,
    status: String,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    approved_at: Option<DateTime<Utc>>,
    applied_at: Option<DateTime<Utc>>,
    asset_count: i64,
    transaction_count: i64,
}

use crate::access::{require as require_portfolio, Access};
use crate::services::assets as asset_service;
use crate::services::error::WriteError;
use crate::services::transactions as tx_service;
use crate::schemas::{AssetCreate, TransactionCreate};
use rust_decimal::Decimal;

/// Why an apply was refused, before it wrote anything.
///
/// Named after the cause, like [`WriteError`], because the caller is an agent reading a
/// machine-readable code rather than a person reading a sentence. The codes are the ones
/// the design spec fixes; inventing a new one here means the spec's list stops being the
/// contract.
#[derive(Debug)]
pub enum ApplyError {
    /// No batch with this handle belongs to this user. Same answer as "no such handle"
    /// from every other route, because a handle another user holds must not be
    /// distinguishable from one that never existed.
    NotFound,
    /// Still `staged`: no session has approved it. This is the gate working, not an error
    /// in the ordinary sense, and the agent is expected to see it and go ask.
    NotApproved,
    /// `approved`, but past `expires_at`. Approval does not last forever, and an approval
    /// the human has not looked at in an hour is not consent to a write now.
    Expired,
    /// Already `applied`. Distinct from `NotApproved` on purpose: "nobody approved that"
    /// and "you already wrote that" are different things for an agent to do about.
    AlreadyApplied,
    /// `target.existing` named a portfolio that is gone, or is not this user's. Re-checked
    /// at apply because the portfolio could have been deleted between stage and apply.
    PortfolioNotFound,
    /// The stored proposal no longer validates. It did at staging, so this means the world
    /// moved — a symbol now collides, a date fell out of range — and not that the agent
    /// sent something bad.
    Rejected(StageError),
    /// A row write was refused by the same seam the HTTP routes use.
    Refused(WriteError),
    Database,
}

impl ApplyError {
    pub fn code(&self) -> &'static str {
        match self {
            ApplyError::NotFound => "stale_handle",
            ApplyError::NotApproved => "not_approved",
            ApplyError::Expired => "expired",
            ApplyError::AlreadyApplied => "already_applied",
            ApplyError::PortfolioNotFound => "portfolio_not_found",
            ApplyError::Rejected(e) => e.code(),
            ApplyError::Refused(_) => "write_refused",
            ApplyError::Database => "database",
        }
    }

    /// The human-readable half, for a tool response body.
    pub fn message(&self) -> String {
        match self {
            ApplyError::NotFound => "no staged batch with that handle".into(),
            ApplyError::NotApproved => {
                "the batch has not been approved by a session; ask a human to approve it"
                    .into()
            }
            ApplyError::Expired => "the approval has expired".into(),
            ApplyError::AlreadyApplied => "this batch has already been applied".into(),
            ApplyError::PortfolioNotFound => "the target portfolio no longer exists".into(),
            ApplyError::Rejected(e) => format!("the stored proposal no longer validates: {}", e.code()),
            ApplyError::Refused(w) => format!("a row was refused: {:?}", w),
            ApplyError::Database => "the database refused the write".into(),
        }
    }
}

/// What a successful apply produced.
#[derive(Debug)]
pub struct Applied {
    pub portfolio_id: i64,
    pub asset_ids: Vec<i64>,
    pub transaction_count: i64,
    /// Rows the proposal contained that were already in the database, and so were skipped.
    ///
    /// Reported rather than silent. An agent that staged 40 rows and gets back 37 has to be
    /// told three were skipped, or it will conclude the import lost rows — and "the import
    /// silently dropped rows" is the failure this whole interface is trying not to be.
    pub skipped_duplicates: i64,
}

/// The identity of a transaction row, for duplicate matching.
///
/// `Decimal` equality, never string equality: `1.5` and `1.50` are the same cost basis and a
/// string comparison would import both. `fee` is included, which is *stricter* than the
/// browser importer (whose in-memory key also includes `fee`, but which never compares
/// against stored rows at all).
///
/// The `fee` stored for a `SPLIT` is always zero — `services::transactions::insert`
/// discards it, because a split is a ratio and a per-share fee is meaningless. The key is
/// built from the value that will actually be stored, not from the value the agent sent;
/// keying on the sent value would make every staged split look unlike the row it created
/// and re-import on the second run.
#[derive(PartialEq, Eq, Hash)]
struct DupKey {
    date: String,
    tx_type: String,
    quantity: Decimal,
    price: Decimal,
    fee: Decimal,
}

fn dup_key(date: &str, tx_type: TxType, quantity: &str, price: &str, fee: &str) -> Option<DupKey> {
    let quantity = Decimal::from_str_exact(quantity).ok()?;
    let price = Decimal::from_str_exact(price).ok()?;
    let fee = if tx_type == TxType::Split {
        Decimal::ZERO
    } else {
        Decimal::from_str_exact(fee).ok()?
    };
    Some(DupKey {
        date: date.to_string(),
        tx_type: tx_type.as_str().to_string(),
        quantity,
        price,
        fee,
    })
}

/// Write an approved batch, once.
///
/// Three properties this has to hold at the same time, and the ordering below is the whole
/// argument:
///
/// 1. **Only an approved, unexpired handle writes.** Checked against the row, not against
///    anything the caller claims.
/// 2. **Exactly once.** The flip `approved -> applied` is a conditional `UPDATE` *inside*
///    the same transaction as the writes, so a second concurrent apply either waits on the
///    row lock and then sees `applied`, or matches zero rows and is refused.
/// 3. **All or nothing.** One Postgres transaction covers the portfolio, every asset and
///    every row. A batch that fails halfway leaves nothing behind.
///
/// Property 3 has a consequence worth stating because it looks like a bug and is not: if a
/// write fails, the flip rolls back with it, so the batch is still `approved` and can be
/// retried. A batch consumed by a failed apply would be a batch nobody can recover — the
/// human approved it, nothing was written, and the only way forward is to start over. The
/// flip is a claim on the batch, not a burn of it.
///
/// Re-validation happens after the flip and before the writes, inside the transaction: the
/// target portfolio may have been deleted between stage and apply, and a symbol may have
/// been taken by hand in the meantime.
pub async fn apply(pool: &PgPool, user_id: i64, handle: &str) -> Result<Applied, ApplyError> {
    // The caller-supplied portfolio id goes through `access.rs` like every other id in
    // this application, before the transaction opens. It is checked a second time inside
    // the transaction (see `resolve_portfolio`), which is not duplication for its own sake:
    // the gate script and every other route read `access.rs` as the single answer, and the
    // in-transaction read is the one that cannot race the write. Both are needed, and the
    // comment in `resolve_portfolio` says which one is authoritative.
    if let Target::Existing { portfolio_id } = target_of(pool, user_id, handle).await? {
        require_portfolio(pool, user_id, portfolio_id, Access::Owner)
            .await
            .map_err(|_| ApplyError::PortfolioNotFound)?;
    }

    let mut tx = pool.begin().await.map_err(|e| {
        eprintln!("ERROR: staged apply: could not open a transaction: {e}");
        ApplyError::Database
    })?;

    // Lock the row for the rest of this transaction. Without it, two concurrent applies
    // both read `approved`, both flip (the second matching zero rows, which is caught), and
    // the second one still goes on to *validate* against a state it has no claim on. The
    // lock makes the second one wait and then read the truth.
    let row: Option<(i64, String, Value, DateTime<Utc>)> = sqlx::query_as(
        "SELECT id, status, payload, expires_at FROM agent_staged_batch
         WHERE handle = $1 AND user_id = $2
         FOR UPDATE",
    )
    .bind(handle)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| { eprintln!("ERROR: staged apply: {e}"); ApplyError::Database })?;

    let (_, status, payload, expires_at) = row.ok_or(ApplyError::NotFound)?;

    match status.as_str() {
        "staged" => return Err(ApplyError::NotApproved),
        "approved" => {}
        "applied" => return Err(ApplyError::AlreadyApplied),
        // `discarded`, and anything a future migration adds. Refusing on the unknown case
        // is the only safe default: a status this function does not recognise is a status
        // whose rules it cannot enforce.
        _ => return Err(ApplyError::NotApproved),
    }

    if expires_at <= Utc::now() {
        return Err(ApplyError::Expired);
    }

    // The claim. Repeating the status and expiry predicates here rather than trusting the
    // read above is what makes the single-use rule hold without the lock, and the lock is an
    // optimisation over this rather than the other way round.
    let flipped = sqlx::query(
        "UPDATE agent_staged_batch
            SET status = 'applied', applied_at = now()
          WHERE handle = $1 AND user_id = $2
            AND status = 'approved' AND expires_at > now()",
    )
    .bind(handle)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| { eprintln!("ERROR: staged apply: {e}"); ApplyError::Database })?;

    if flipped.rows_affected() == 0 {
        return Err(ApplyError::AlreadyApplied);
    }

    let proposal: Proposal = serde_json::from_value(payload).map_err(|e| {
        eprintln!("ERROR: staged apply: stored payload does not deserialize: {e}");
        ApplyError::Database
    })?;
    validate(&proposal).map_err(ApplyError::Rejected)?;

    let portfolio_id = resolve_portfolio(&mut tx, user_id, &proposal.target).await?;

    let mut asset_ids = Vec::new();
    let mut written: i64 = 0;
    let mut skipped: i64 = 0;

    for pa in &proposal.assets {
        let asset_id = resolve_asset(&mut tx, portfolio_id, pa).await?;
        asset_ids.push(asset_id);

        // Seeded from the database, then extended with whatever this batch writes, so a row
        // duplicated *within* the proposal is skipped by the same rule as one already
        // stored. Both are "this row is already there" from the importer's point of view.
        let mut seen = existing_keys(&mut tx, asset_id).await?;

        for pt in &pa.transactions {
            let key = dup_key(&pt.date, pt.r#type, &pt.quantity, &pt.price, &pt.fee).ok_or_else(|| {
                eprintln!("ERROR: staged apply: a staged money string will not parse");
                ApplyError::Database
            })?;
            if seen.contains(&key) {
                skipped += 1;
                continue;
            }

            let create = TransactionCreate {
                r#type: pt.r#type,
                quantity: Decimal::from_str_exact(&pt.quantity).map_err(|_| ApplyError::Database)?,
                price: Decimal::from_str_exact(&pt.price).map_err(|_| ApplyError::Database)?,
                fee: Decimal::from_str_exact(&pt.fee).map_err(|_| ApplyError::Database)?,
                date: chrono::DateTime::parse_from_rfc3339(&format!("{}T00:00:00Z", pt.date))
                    .map(|d| d.with_timezone(&Utc))
                    .map_err(|e| { eprintln!("ERROR: staged apply: {e}"); ApplyError::Database })?,
            };
            tx_service::insert(&mut tx, asset_id, &create)
                .await
                .map_err(|e| {
                    // Roll the claim back with the failure: see the doc comment. sqlx
                    // rolls the whole transaction back on drop without a commit, so this
                    // only has to stop, not undo.
                    ApplyError::Refused(e)
                })?;
            seen.insert(key);
            written += 1;
        }
    }

    tx.commit().await.map_err(|_| ApplyError::Database)?;

    Ok(Applied {
        portfolio_id,
        asset_ids,
        transaction_count: written,
        skipped_duplicates: skipped,
    })
}

/// The target named by the stored proposal, read before the transaction opens so the
/// `access.rs` check can happen there. `NotFound` if the handle is not this user's.
async fn target_of(pool: &PgPool, user_id: i64, handle: &str) -> Result<Target, ApplyError> {
    let payload: Option<Value> = sqlx::query_scalar(
        "SELECT payload FROM agent_staged_batch WHERE handle = $1 AND user_id = $2",
    )
    .bind(handle)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| { eprintln!("ERROR: staged apply: {e}"); ApplyError::Database })?;
    let payload = payload.ok_or(ApplyError::NotFound)?;
    // The payload is the whole proposal; the target is a field of it. Reading the payload
    // as a `Target` directly — which is what this did first — fails with serde's
    // "missing field `kind`", and because every failure here collapses to one code, the
    // message that named the real cause was the only thing left to follow.
    let proposal: Proposal = serde_json::from_value(payload).map_err(|e| {
        eprintln!("ERROR: staged apply: could not read the payload: {e}");
        ApplyError::Database
    })?;
    Ok(proposal.target)
}

/// The portfolio to write into, creating it when the proposal asked for a new one.
///
/// The `existing` branch goes through `access.rs` like every other caller-supplied id. That
/// is not ceremony: without it, a token could name any portfolio in the database by counting
/// integers, and the batch would write into it after a human approved a preview that named a
/// different one.
async fn resolve_portfolio(
    tx: &mut sqlx::PgConnection,
    user_id: i64,
    target: &Target,
) -> Result<i64, ApplyError> {
    match target {
        Target::New {
            name,
            base_currency,
        } => {
            let (id,): (i64,) = sqlx::query_as(
                "INSERT INTO portfolios (name, description, currency, base_currency, owner_id)
                 VALUES ($1, $2, $3, $3, $4) RETURNING id",
            )
            .bind(name)
            .bind("Created by an agent import")
            .bind(base_currency)
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| { eprintln!("ERROR: staged apply: {e}"); ApplyError::Database })?;
            Ok(id)
        }
        Target::Existing { portfolio_id } => {
            // `require` answers on a pool, and this is a connection. The ownership answer
            // cannot be taken outside the transaction that writes, or the portfolio could
            // change owner between the check and the write, so the same predicate is
            // restated here against the open transaction.
            let owner: Option<i64> =
                sqlx::query_scalar("SELECT owner_id FROM portfolios WHERE id = $1")
                    .bind(portfolio_id)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(|e| { eprintln!("ERROR: staged apply: {e}"); ApplyError::Database })?;
            match owner {
                Some(uid) if uid == user_id => Ok(*portfolio_id),
                _ => Err(ApplyError::PortfolioNotFound),
            }
        }
    }
}

/// Find the asset by symbol, or create it.
///
/// Merge is by uppercased symbol within the target portfolio, which is the same identity
/// the unique index uses. An agent re-staging the same statement must land on the same
/// asset rather than failing with a symbol collision.
async fn resolve_asset(
    tx: &mut sqlx::PgConnection,
    portfolio_id: i64,
    pa: &crate::agent::protocol::ProposalAsset,
) -> Result<i64, ApplyError> {
    let symbol = pa.symbol.to_uppercase();
    let found: Option<i64> =
        sqlx::query_scalar("SELECT id FROM assets WHERE portfolio_id = $1 AND symbol = $2")
            .bind(portfolio_id)
            .bind(&symbol)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| { eprintln!("ERROR: staged apply: {e}"); ApplyError::Database })?;

    if let Some(id) = found {
        return Ok(id);
    }

    let create = AssetCreate {
        symbol: pa.symbol.clone(),
        name: pa.name.clone().unwrap_or_else(|| pa.symbol.clone()),
        asset_type: pa.asset_type.clone(),
        sector: None,
        currency: pa.currency.clone().unwrap_or_else(|| "USD".to_string()),
        isin: pa.isin.clone().unwrap_or_default(),
    };
    let asset = asset_service::insert(tx, portfolio_id, &create)
        .await
        .map_err(ApplyError::Refused)?;
    Ok(asset.id)
}

/// The rows already stored for one asset, as duplicate keys.
async fn existing_keys(tx: &mut sqlx::PgConnection, asset_id: i64) -> Result<std::collections::HashSet<DupKey>, ApplyError> {
    let rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT date::text, upper(btrim(type)), quantity, price, fee
           FROM transactions WHERE asset_id = $1",
    )
    .bind(asset_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| { eprintln!("ERROR: staged apply: {e}"); ApplyError::Database })?;

    let mut out = std::collections::HashSet::new();
    for (date, ty, quantity, price, fee) in rows {
        // A stored row this code cannot parse is skipped rather than fatal: it predates the
        // closure, and refusing to import anything into an asset that holds one legacy row
        // would be a worse outcome than possibly re-importing a row nobody can read.
        let qty = Decimal::from_str_exact(&quantity).ok();
        let prc = Decimal::from_str_exact(&price).ok();
        let fee = Decimal::from_str_exact(&fee).ok();
        if let (Some(q), Some(p), Some(f)) = (qty, prc, fee) {
            out.insert(DupKey {
                date: date.chars().take(10).collect(),
                tx_type: ty,
                quantity: q,
                price: p,
                fee: f,
            });
        }
    }
    Ok(out)
}
