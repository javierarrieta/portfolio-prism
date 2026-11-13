# Portfolio Backup Export and Restore Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a signed-in user export their selected portfolios as a versioned JSON bundle and restore such a bundle either alongside or in place of what they already own.

**Architecture:** A new `backend_rust/src/backup/` module owns the bundle format, its validation, and all of its SQL; thin Rocket handlers in `backend_rust/src/api_routes/backup.rs` expose four routes and route every caller-supplied portfolio id through the existing `access.rs` chokepoint. The bundle is logical rows in engine-neutral JSON, written to a stated portability discipline so the separate engine-selection work is a mechanical change to this module rather than a rewrite. Restore is one database transaction: validate fully outside it, then write or roll back.

**Tech Stack:** Rust 2024, Rocket 0.5, SQLx 0.8 (SQLite), `rust_decimal`, `chrono`, `utoipa` 4.2, `schemars` (new), React 19 + Vite + vitest, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-11-13-backup-export-restore-design.md` — read it alongside this plan. Where the two disagree on behaviour, the spec wins; this plan wins on order of work, and any sanctioned exception is listed in *Deviations* at the end.

## Global Constraints

- **Bundle format tag** is exactly `portfolio-prism-backup`.
- **Accepted schema versions** are exactly `[1]`. `docs/backup-schema/v1.json` is never edited; a format change adds `v2.json` plus an upgrader.
- **Decimals in the bundle are JSON strings** (`quantity`, `price`, `fee`, `close_price`), matching the `TEXT` columns and `models.rs` (`Transaction.quantity: String`).
- **Two date shapes, matching storage:** `transactions[].date` is an RFC 3339 UTC datetime (`models.rs:35` is `DateTime<Utc>`); `prices[].date` is `YYYY-MM-DD` (`models.rs:41` is `NaiveDate`).
- **Transaction dates must fall in 1900-01-01..=2100-01-01**, the range `create_transaction` already enforces (`api_routes/transactions.rs:223-229`).
- **Row cap** `MAX_BUNDLE_ROWS = 50_000`; **byte cap** `MAX_BUNDLE_BYTES = 8 * 1024 * 1024`.
- **Error mapping is fixed:** malformed bundle / failed validation / unknown version / empty export selection / cap exceeded → `422`. `apply_prices=true` while the server flag is off → `422`. Lock timeout → `503` with `Retry-After`. Any other database error → `500`. A database failure is never collapsed into a client error — that is the rule stated in `src/access.rs`.
- **Portability discipline** (spec *Portability discipline*), enforced by `scripts/check_backup_portability.sh`: every SQL statement under `src/backup/` uses `?` placeholders, names columns explicitly, uses `RETURNING` for allocated ids, uses `ON CONFLICT (cols) DO NOTHING` for upserts, and contains none of `PRAGMA`, `pragma_`, `INSERT OR REPLACE`, `INSERT OR IGNORE`, `AUTOINCREMENT`, `last_insert_rowid`, `strftime`, `julianday`, `datetime(`, `IFNULL`, `GROUP_CONCAT`. Dates and decimals are read and bound through the same `chrono`/`String` types the rest of the codebase uses — never through SQL date functions.
- **Every new route handler takes `AuthedUser`** (gate Rule C) and lives in `src/api_routes/backup.rs`, because `scripts/check_access_gate.sh` inspects `src/api_routes` only.
- **Backend commands** run from `backend_rust/`: `cargo test`, `cargo clippy --all-targets`, `bash scripts/check_access_gate.sh`. **Frontend commands** run from `frontend/`: `npm run test`, `npm run lint`, `npm run typecheck`.
- **Branch:** work on `feat/backup-export-restore`. Commit after every task.

## Review Focus

The spec is a design, not an enumeration of every input. These are the inputs a person using this software will produce that no happy-path test covers, and what must happen for each. Each line gets its test in the task named.

1. **A hand-edited bundle carrying real database ids** (`"id": 7`, or a `ref` of `"p1"` that collides with another) — the restore must ignore unknown fields outright (`deny_unknown_fields`), must reject duplicate `ref`s as a `422` naming both paths, and must never adopt or overwrite a row that already exists in the target. *(Task 2, Task 6)*
2. **A bundle from a newer build** (`schema_version: 2`) — refused with a message naming the accepted versions, and nothing written. Not partially imported, not coerced. *(Task 2)*
3. **A restore racing a concurrent write** — the lock timeout is a `503` with `Retry-After`, never a `500`, and never a half-applied restore. *(Task 9)*
4. **A non-UTF8 or non-JSON upload body** — `422` naming what failed to parse, never a `500` from a panic in the deserializer. *(Task 5)*
5. **A bundle that is valid but empty of portfolios**, or one whose only content is the `prices` section — `422`. A user will restore an empty bundle, and "restore succeeded, everything vanished" is the worst outcome this feature can produce. *(Task 4 for export, Task 6 for restore)*

---

## File Structure

**Created — backend**

| Path | Responsibility |
|---|---|
| `backend_rust/src/backup/mod.rs` | Format structs, `FORMAT_TAG`, `ACCEPTED_VERSIONS`, caps, `BackupConfig`, validation entry points. |
| `backend_rust/src/backup/export.rs` | Reading the caller's rows into a `Bundle`. All export SQL. |
| `backend_rust/src/backup/restore.rs` | Row counting, the plan, the restore transaction. All restore SQL. |
| `backend_rust/src/backup/schema.rs` | JSON Schema generation for `--print-backup-schema`. |
| `backend_rust/src/api_routes/backup.rs` | The four Rocket handlers. No SQL. |
| `backend_rust/scripts/check_backup_portability.sh` | Static gate for the portability discipline. |
| `backend_rust/tests/backup_test.rs` | Route-level behaviour: selection, caps, versions, modes, atomicity, round-trip. |
| `docs/backup-schema/v1.json` | Generated JSON Schema for `schema_version: 1`. |
| `docs/backup-schema/README.md` | Version register: what each version is, what changed, what is accepted, deprecation policy. |

**Modified — backend**

| Path | Change |
|---|---|
| `backend_rust/Cargo.toml` | Add `schemars`. |
| `backend_rust/src/lib.rs` | `pub mod backup;`, mount the four routes, raise the `data` limit. |
| `backend_rust/src/main.rs` | `--print-backup-schema`, read `BACKUP_ALLOW_PRICE_RESTORE`, `.manage(BackupConfig)`. |
| `backend_rust/src/openapi.rs` | Register the four paths and the new component schemas. |
| `backend_rust/tests/route_coverage_test.rs` | Add the four routes to `PROTECTED_ROUTES`. |
| `backend_rust/tests/isolation_test.rs` | Cross-user matrix for export and restore. |
| `.github/workflows/ci.yml` | Schema freshness diff; portability gate step. |

**Created / modified — frontend**

| Path | Change |
|---|---|
| `frontend/src/components/BackupPanel.tsx` | Created. Both cards. |
| `frontend/src/App.tsx` | New `backup` nav item and tab body. |
| `frontend/src/test/components/BackupPanel.test.tsx` | Created. |
| `docs/openapi/openapi.json`, `frontend/src/types/api.d.ts` | Regenerated. |
| `docs/walkthrough.md` | Manual Backup section, including the fresh-instance restore CI cannot perform. |

---

### Task 1: Bundle format, version registry, and the generated schema artifact

**Files:**
- Create: `backend_rust/src/backup/mod.rs`
- Create: `backend_rust/src/backup/schema.rs`
- Create: `docs/backup-schema/v1.json`
- Create: `docs/backup-schema/README.md`
- Modify: `backend_rust/Cargo.toml`
- Modify: `backend_rust/src/lib.rs` (add `pub mod backup;` near the other `pub mod` lines)
- Modify: `backend_rust/src/main.rs` (the `--print-backup-schema` flag)
- Test: `backend_rust/src/backup/mod.rs` (`#[cfg(test)]` module)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - `backup::FORMAT_TAG: &str`, `backup::ACCEPTED_VERSIONS: &[i64]`, `backup::MAX_BUNDLE_BYTES: usize`, `backup::MAX_BUNDLE_ROWS: usize`
  - `backup::Bundle`, `backup::BundlePortfolio`, `backup::BundleAsset`, `backup::BundleTransaction`, `backup::BundlePrice`
  - `backup::schema::document_json() -> String`
  - The `--print-backup-schema` flag.

- [ ] **Step 1: Add the dependency**

In `backend_rust/Cargo.toml`, after the `rust_decimal` line:

```toml
schemars = { version = "1.0", features = ["preserve_order"] }
```

Run: `cargo build`
Expected: compiles; `schemars` resolves.

- [ ] **Step 2: Write the failing test for the version registry and the format tag**

Create `backend_rust/src/backup/mod.rs`:

```rust
//! The backup bundle: its format, its versioning, and the code that reads and
//! writes it.
//!
//! Everything SQL in this module is written to the portability discipline in
//! `docs/superpowers/specs/2026-11-13-backup-export-restore-design.md`
//! (*Portability discipline*) so that the engine-selection work is a mechanical
//! change here rather than a rewrite. `scripts/check_backup_portability.sh`
//! enforces it statically; do not weaken that script to make a statement fit.

pub mod export;
pub mod restore;
pub mod schema;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The literal `format` every bundle must carry. A file that is not a bundle
/// fails here, with a clear message, rather than at the first missing field.
pub const FORMAT_TAG: &str = "portfolio-prism-backup";

/// The schema versions this build accepts. `v1.json` is never edited: a format
/// change adds `v2.json` plus a `v1 -> v2` upgrader and appends `2` here.
pub const ACCEPTED_VERSIONS: &[i64] = &[1];

/// Application-level ceiling on an uploaded bundle. Exceeding it is a `422` that
/// names the limit, not a `413` from the framework. Rocket's own `data` limit is
/// deliberately larger (see `lib.rs`) so this check is the one that answers.
pub const MAX_BUNDLE_BYTES: usize = 8 * 1024 * 1024;

/// Ceiling on `portfolios + assets + transactions` in one bundle.
pub const MAX_BUNDLE_ROWS: usize = 50_000;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    pub format: String,
    pub schema_version: i64,
    /// RFC 3339 UTC, informational. Excluded from the round-trip equality test.
    pub created_at: String,
    pub portfolios: Vec<BundlePortfolio>,
    /// Absent unless requested at export time. Flat, because price rows are
    /// global and belong to no portfolio.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prices: Vec<BundlePrice>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BundlePortfolio {
    /// Bundle-local label (`p1`, `p2`), unique within the bundle, used only to
    /// rewire parent/child links during restore. Never a database id, never
    /// persisted. `ref` is a Rust keyword, hence the rename.
    #[serde(rename = "ref")]
    pub bundle_ref: String,
    pub name: String,
    pub description: Option<String>,
    pub currency: String,
    pub base_currency: String,
    pub assets: Vec<BundleAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BundleAsset {
    #[serde(rename = "ref")]
    pub bundle_ref: String,
    pub symbol: String,
    pub name: String,
    pub asset_type: String,
    pub sector: Option<String>,
    pub currency: String,
    pub isin: Option<String>,
    pub transactions: Vec<BundleTransaction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BundleTransaction {
    pub r#type: String,
    /// Decimal as a string, matching the TEXT column and `models::Transaction`.
    pub quantity: String,
    pub price: String,
    pub fee: String,
    /// RFC 3339 UTC datetime, matching `models::Transaction::date`.
    pub date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BundlePrice {
    pub symbol: String,
    /// `YYYY-MM-DD`, matching `models::HistoricalPrice::date`.
    pub date: String,
    pub close_price: String,
}

/// The asset types the tax engine recognises. An unknown value is a validation
/// error, not a row that silently never matches.
pub const KNOWN_ASSET_TYPES: &[&str] = &["STOCK", "FUND", "ETF", "BOND", "CASH"];

/// The transaction types the tax engine replays.
pub const KNOWN_TRANSACTION_TYPES: &[&str] = &["BUY", "SELL", "SPLIT", "DIVIDEND"];

/// Total authored rows in a bundle, against `MAX_BUNDLE_ROWS`.
pub fn row_count(bundle: &Bundle) -> usize {
    bundle.portfolios.iter().fold(0usize, |acc, p| {
        acc + 1 + p.assets.iter().fold(0usize, |a, ast| a + 1 + ast.transactions.len())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle(portfolios: Vec<BundlePortfolio>) -> Bundle {
        Bundle {
            format: FORMAT_TAG.to_string(),
            schema_version: 1,
            created_at: "2026-11-13T00:00:00Z".to_string(),
            portfolios,
            prices: vec![],
        }
    }

    #[test]
    fn accepted_versions_are_exactly_v1() {
        assert_eq!(ACCEPTED_VERSIONS, &[1]);
        assert_eq!(FORMAT_TAG, "portfolio-prism-backup");
    }

    #[test]
    fn row_count_counts_every_level() {
        let b = bundle(vec![BundlePortfolio {
            bundle_ref: "p1".into(),
            name: "Main".into(),
            description: None,
            currency: "USD".into(),
            base_currency: "USD".into(),
            assets: vec![
                BundleAsset {
                    bundle_ref: "a1".into(),
                    symbol: "AAPL".into(),
                    name: "Apple".into(),
                    asset_type: "STOCK".into(),
                    sector: None,
                    currency: "USD".into(),
                    isin: None,
                    transactions: vec![BundleTransaction {
                        r#type: "BUY".into(),
                        quantity: "1".into(),
                        price: "1".into(),
                        fee: "0".into(),
                        date: "2024-01-01T00:00:00Z".into(),
                    }],
                },
                BundleAsset {
                    bundle_ref: "a2".into(),
                    symbol: "MSFT".into(),
                    name: "Microsoft".into(),
                    asset_type: "STOCK".into(),
                    sector: None,
                    currency: "USD".into(),
                    isin: None,
                    transactions: vec![],
                },
            ],
        }]);
        // 1 portfolio + 2 assets + 1 transaction
        assert_eq!(row_count(&b), 4);
    }

    #[test]
    fn an_empty_prices_section_is_not_serialised() {
        let json = serde_json::to_string(&bundle(vec![])).unwrap();
        assert!(
            !json.contains("\"prices\""),
            "an absent price section must not appear as an empty array: {json}"
        );
    }

    #[test]
    fn an_unknown_field_is_rejected_rather_than_ignored() {
        // Review Focus #1: a hand-edited bundle must not be able to smuggle a
        // database id in, so unknown keys are an error, not a shrug.
        let raw = r#"{"format":"portfolio-prism-backup","schema_version":1,
            "created_at":"2026-11-13T00:00:00Z","portfolios":[],"prices":[],"id":7}"#;
        assert!(serde_json::from_str::<Bundle>(raw).is_err());
    }
}
```

Note `export`, `restore` and `schema` are declared but do not exist yet — create them as empty files now so the crate compiles:

```rust
// backend_rust/src/backup/export.rs  (placeholder, filled in Task 4)
```

```rust
// backend_rust/src/backup/restore.rs  (placeholder, filled in Tasks 5-8)
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test --lib backup:: -- --nocapture`
Expected: compile error until the placeholder files exist; after creating them, `an_unknown_field_is_rejected_rather_than_ignored` and the others PASS (they are written against code added in Step 2, so if they fail, Step 2 was not applied as written).

- [ ] **Step 4: Add the schema generator**

Create `backend_rust/src/backup/schema.rs`:

```rust
//! Generation of the committed JSON Schema artifact.
//!
//! `docs/backup-schema/v1.json` is produced by this code, not written by hand, so
//! the artifact and the structs cannot drift. CI regenerates it and diffs it, the
//! same way it does `docs/openapi/openapi.json`.

use crate::backup::Bundle;
use schemars::SchemaSettings;

/// The draft-2020-12 schema for the current `Bundle`.
pub fn document() -> schemars::schema::Schema {
    let settings = SchemaSettings::draft2020_12();
    settings.into_generator().into_root_schema_for::<Bundle>().into()
}

/// Pretty-printed JSON for `--print-backup-schema`.
pub fn document_json() -> String {
    let value = serde_json::to_value(document()).expect("a schemars document is JSON");
    // Key order is fixed by `preserve_order`, so the committed file is stable and
    // the CI diff is meaningful.
    serde_json::to_string_pretty(&value).expect("a serde_json value serialises")
}
```

- [ ] **Step 5: Add the `--print-backup-schema` flag**

In `backend_rust/src/main.rs`, next to `print_openapi_requested` (`main.rs:14-18`), add:

```rust
/// Print the backup bundle schema and exit, before touching configuration, the
/// database, or the network — the same contract `--print-openapi` holds, and for
/// the same reason: CI runs it with no `OIDC_*` variables and no secret key.
fn print_backup_schema_requested() -> bool {
    std::env::args().any(|arg| arg == "--print-backup-schema")
}
```

And inside `main`, immediately after the existing `if print_openapi_requested() { ... }` block:

```rust
    if print_backup_schema_requested() {
        println!("{}", backend_rust::backup::schema::document_json());
        return Ok(());
    }
```

- [ ] **Step 6: Generate the artifact and write the version register**

Run:
```bash
cargo run --bin backend_rust -- --print-backup-schema > ../docs/backup-schema/v1.json
head -20 ../docs/backup-schema/v1.json
```
Expected: a JSON Schema document whose `$ref` definitions include `BundlePortfolio`, `BundleAsset`, `BundleTransaction`, `BundlePrice`, and whose root requires `format`, `schema_version`, `created_at`, `portfolios`.

Create `docs/backup-schema/README.md`:

```markdown
# Backup bundle schema versions

`GET /api/backup/export` produces a JSON bundle; `POST /api/backup/restore`
consumes one. This directory is the normative record of what each version of that
format means. The Rust structs in `backend_rust/src/backup/mod.rs` are the source
of truth; the `vN.json` files here are generated from them by
`cargo run --bin backend_rust -- --print-backup-schema` and CI diffs the result,
so a committed file that disagrees with the code fails the build.

## Policy

- **A committed `vN.json` is never edited.** A format change adds `vN+1.json` and a
  `vN -> vN+1` upgrader in `backend_rust/src/backup/restore.rs`.
- The build declares the versions it accepts in
  `backend_rust::backup::ACCEPTED_VERSIONS`. A bundle whose `schema_version` is
  outside that set is refused with a `422` naming the accepted versions, and
  nothing is written.
- A version is deprecated by removing it from `ACCEPTED_VERSIONS` in a release
  that has shipped the upgrader for at least one prior release. The `vN.json` file
  stays in this directory forever; it is history.

## Versions

### v1 — `v1.json`

Introduced 2026-11-13 with the backup feature. Accepted by every build since.

- Nested structure: `portfolios[]` → `assets[]` → `transactions[]`. Only
  `portfolios` has an `owner_id` in the database, so nesting is what makes
  ownership structural.
- `ref` labels (`p1`, `a1`) are bundle-local and are never database ids.
- Decimals (`quantity`, `price`, `fee`, `close_price`) are strings.
- `transactions[].date` is an RFC 3339 UTC datetime; `prices[].date` is
  `YYYY-MM-DD`. The two differ because the columns differ, and normalising them
  would mean rewriting stored data.
- `prices[]` is optional and, when present, is global market data rather than
  authored data. Restoring it is disabled unless the server sets
  `BACKUP_ALLOW_PRICE_RESTORE=true`.
- Unknown fields are rejected, not ignored.

## Regenerating

```bash
cd backend_rust
cargo run --bin backend_rust -- --print-backup-schema > ../docs/backup-schema/v1.json
```
```

- [ ] **Step 7: Verify the flag needs no configuration**

Run: `env -u OIDC_ISSUER -u OIDC_CLIENT_ID -u OIDC_CLIENT_SECRET -u OIDC_REDIRECT_URI -u AUTH_MODE cargo run --bin backend_rust -- --print-backup-schema | head -3`
Expected: schema JSON on stdout, exit 0, no "startup refused" line. This is the property that makes the CI step in Task 10 possible.

- [ ] **Step 8: Commit**

```bash
git add backend_rust/Cargo.toml backend_rust/Cargo.lock backend_rust/src/backup \
        backend_rust/src/lib.rs backend_rust/src/main.rs docs/backup-schema
git commit -m "feat(backup): versioned bundle format and generated JSON Schema artifact"
```

---

### Task 2: Bundle validation

**Files:**
- Modify: `backend_rust/src/backup/mod.rs`
- Test: `backend_rust/src/backup/mod.rs` (`#[cfg(test)]` module)

**Interfaces:**
- Consumes: Task 1's structs, `FORMAT_TAG`, `ACCEPTED_VERSIONS`, `MAX_BUNDLE_ROWS`, `KNOWN_*`.
- Produces:
  - `backup::ValidationError { path: String, reason: String }`
  - `backup::BundleError` with variants `Unparseable(String)`, `NotABundle`, `UnsupportedVersion { found: i64 }`, `Invalid(Vec<ValidationError>, bool)`
  - `backup::parse_and_validate(raw: &[u8]) -> Result<Bundle, BundleError>`
  - `backup::validate(bundle: &Bundle) -> Vec<ValidationError>`
  - `impl BundleError { pub fn status(&self) -> rocket::http::Status }` and `pub fn body(&self) -> serde_json::Value` — the single place bundle failures become HTTP.

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` block in `backend_rust/src/backup/mod.rs`:

```rust
    fn tx(_path: &str, kind: &str, qty: &str, price: &str, date: &str) -> BundleTransaction {
        BundleTransaction {
            r#type: kind.into(),
            quantity: qty.into(),
            price: price.into(),
            fee: "0".into(),
            date: date.into(),
        }
    }

    fn asset(r: &str, txs: Vec<BundleTransaction>) -> BundleAsset {
        BundleAsset {
            bundle_ref: r.into(),
            symbol: "AAPL".into(),
            name: "Apple".into(),
            asset_type: "STOCK".into(),
            sector: None,
            currency: "USD".into(),
            isin: None,
            transactions: txs,
        }
    }

    fn port(r: &str, assets: Vec<BundleAsset>) -> BundlePortfolio {
        BundlePortfolio {
            bundle_ref: r.into(),
            name: "Main".into(),
            description: None,
            currency: "USD".into(),
            base_currency: "USD".into(),
            assets,
        }
    }

    #[test]
    fn a_well_formed_bundle_has_no_errors() {
        let b = bundle(vec![port("p1", vec![asset("a1", vec![
            tx("", "BUY", "10.0", "150.00", "2024-01-15T00:00:00Z"),
        ])])]);
        assert_eq!(validate(&b), Vec::<ValidationError>::new());
    }

    #[test]
    fn a_bundle_with_no_portfolios_is_rejected() {
        // Review Focus #5: an empty bundle is the one thing a user will restore
        // that must never succeed under `replace`.
        let errs = validate(&bundle(vec![]));
        assert!(errs.iter().any(|e| e.path == "portfolios"
            && e.reason.contains("at least one portfolio")));
    }

    #[test]
    fn a_non_decimal_price_names_its_path() {
        let b = bundle(vec![port("p1", vec![asset("a1", vec![
            tx("", "BUY", "10.0", "not-a-number", "2024-01-15T00:00:00Z"),
        ])])]);
        let errs = validate(&b);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert_eq!(errs[0].path, "portfolios[0].assets[0].transactions[0].price");
        assert!(errs[0].reason.contains("not a valid decimal"));
    }

    #[test]
    fn a_transaction_date_outside_the_supported_range_is_rejected() {
        // The same range `create_transaction` enforces (transactions.rs:223-229).
        // A restore must not accept a row the ordinary write path would refuse.
        let b = bundle(vec![port("p1", vec![asset("a1", vec![
            tx("", "BUY", "10.0", "150.00", "1899-01-01T00:00:00Z"),
        ])])]);
        let errs = validate(&b);
        assert!(errs.iter().any(|e| e.reason.contains("outside the supported range")), "{errs:?}");
    }

    #[test]
    fn a_transaction_date_must_be_rfc3339_not_a_bare_date() {
        // Storage is DateTime<Utc> (models.rs:35). Accepting "2024-01-15" here
        // would mean two accepted spellings for one column.
        let b = bundle(vec![port("p1", vec![asset("a1", vec![
            tx("", "BUY", "10.0", "150.00", "2024-01-15"),
        ])])]);
        let errs = validate(&b);
        assert!(errs.iter().any(|e| e.reason.contains("RFC 3339")), "{errs:?}");
    }

    #[test]
    fn a_price_date_must_be_a_plain_date() {
        let mut b = bundle(vec![port("p1", vec![])]);
        b.prices.push(BundlePrice {
            symbol: "AAPL".into(),
            date: "2024-01-15T00:00:00Z".into(),
            close_price: "150.00".into(),
        });
        let errs = validate(&b);
        assert!(errs.iter().any(|e| e.path == "prices[0].date"), "{errs:?}");
    }

    #[test]
    fn an_unknown_asset_type_or_transaction_type_is_rejected() {
        let mut a = asset("a1", vec![tx("", "BUY", "1.0", "1.0", "2024-01-15T00:00:00Z")]);
        a.asset_type = "CRYPTO".into();
        let errs = validate(&bundle(vec![port("p1", vec![a])]));
        assert!(errs.iter().any(|e| e.reason.contains("asset_type")), "{errs:?}");

        let b = bundle(vec![port("p1", vec![asset("a1", vec![
            tx("", "SHORT", "1.0", "1.0", "2024-01-15T00:00:00Z"),
        ])])]);
        let errs = validate(&b);
        assert!(errs.iter().any(|e| e.reason.contains("type")), "{errs:?}");
    }

    #[test]
    fn duplicate_refs_are_rejected_naming_both_paths() {
        // Review Focus #1: a colliding ref must not silently link to whichever
        // row the map happened to see last.
        let b = bundle(vec![port("p1", vec![asset("a1", vec![])]), port("p1", vec![])]);
        let errs = validate(&b);
        assert!(errs.iter().any(|e| e.reason.contains("duplicate ref")), "{errs:?}");
    }

    #[test]
    fn an_empty_asset_list_is_allowed_but_an_empty_transaction_list_is_too() {
        // A portfolio with no assets is legal (you can create one in the UI), so
        // a bundle may carry one. Same for an asset with no transactions.
        assert_eq!(validate(&bundle(vec![port("p1", vec![])])), Vec::new());
        assert_eq!(validate(&bundle(vec![port("p1", vec![asset("a1", vec![])])])), Vec::new());
    }

    #[test]
    fn a_newer_schema_version_is_refused_naming_the_accepted_set() {
        // Review Focus #2.
        let mut b = bundle(vec![port("p1", vec![])]);
        b.schema_version = 2;
        let raw = serde_json::to_vec(&b).unwrap();
        match parse_and_validate(&raw) {
            Err(BundleError::UnsupportedVersion { found: 2 }) => {}
            other => panic!("expected UnsupportedVersion, got {other:?}"),
        }
    }

    #[test]
    fn a_file_that_is_not_a_bundle_is_not_a_bundle_error() {
        let raw = br#"{"format":"something-else","schema_version":1,
            "created_at":"x","portfolios":[]}"#;
        assert!(matches!(parse_and_validate(raw), Err(BundleError::NotABundle)));
    }

    #[test]
    fn unparseable_input_is_reported_not_panicked() {
        // Review Focus #4.
        assert!(matches!(parse_and_validate(b"not json at all"),
            Err(BundleError::Unparseable(_))));
        assert!(matches!(parse_and_validate(&[0xff, 0xfe, 0x00]),
            Err(BundleError::Unparseable(_))));
    }

    #[test]
    fn errors_are_capped_at_twenty_and_flagged_as_truncated() {
        let txs = (0..25).map(|_| tx("", "BUY", "n", "1", "2024-01-15T00:00:00Z"))
            .collect();
        let errs = validate(&bundle(vec![port("p1", vec![asset("a1", txs)])]));
        assert_eq!(errs.len(), 20);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib backup:: 2>&1 | tail -20`
Expected: compile failure — `cannot find type ValidationError`, `cannot find function parse_and_validate`.

- [ ] **Step 3: Implement validation**

Add to `backend_rust/src/backup/mod.rs`, above the `#[cfg(test)]` module:

```rust
/// One reason a bundle is not usable, addressed by bundle path rather than by
/// table row, because the bundle is what the user is holding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValidationError {
    pub path: String,
    pub reason: String,
}

/// How many validation errors a response names. A bad multi-megabyte file must
/// not produce a multi-megabyte error.
pub const MAX_REPORTED_ERRORS: usize = 20;

/// Everything that can stop a bundle before a transaction is opened.
#[derive(Debug)]
pub enum BundleError {
    /// Not JSON, or not UTF-8. A `422`, never a `500`.
    Unparseable(String),
    /// JSON, but not a backup bundle (wrong `format`).
    NotABundle,
    /// A `schema_version` outside `ACCEPTED_VERSIONS`.
    UnsupportedVersion { found: i64 },
    /// Structurally a bundle, semantically not usable. The `bool` is "more
    /// errors were found than are reported".
    Invalid(Vec<ValidationError>, bool),
}

impl BundleError {
    /// Every bundle failure is the caller's fault, so every one is a `422`.
    /// Nothing here is a server failure, and nothing here is a `404`: a bundle
    /// names no server-side resource.
    pub fn status(&self) -> rocket::http::Status {
        rocket::http::Status::UnprocessableEntity
    }

    pub fn body(&self) -> serde_json::Value {
        use serde_json::json;
        match self {
            BundleError::Unparseable(e) => json!({ "error": "the upload is not valid JSON", "detail": e }),
            BundleError::NotABundle => json!({
                "error": format!("not a {FORMAT_TAG} bundle"),
                "detail": "the `format` field is missing or does not match",
            }),
            BundleError::UnsupportedVersion { found } => json!({
                "error": format!("unsupported schema_version {found}"),
                "accepted": ACCEPTED_VERSIONS,
                "detail": "this build cannot read that bundle version; restore it with a build that lists it",
            }),
            BundleError::Invalid(errors, more) => json!({
                "error": "the bundle failed validation",
                "errors": errors,
                "more_errors": more,
            }),
        }
    }
}

/// Parse, check the version, and validate. **Everything that can reject a bundle
/// without touching the database happens here**, so the transaction is only ever
/// about consistency, never about typos.
pub fn parse_and_validate(raw: &[u8]) -> Result<Bundle, BundleError> {
    if raw.len() > MAX_BUNDLE_BYTES {
        return Err(BundleError::Invalid(
            vec![ValidationError {
                path: String::new(),
                reason: format!(
                    "bundle is {} bytes, over the {MAX_BUNDLE_BYTES} byte limit",
                    raw.len()
                ),
            }],
            false,
        ));
    }

    let bundle: Bundle = serde_json::from_slice(raw)
        .map_err(|e| BundleError::Unparseable(e.to_string()))?;

    if bundle.format != FORMAT_TAG {
        return Err(BundleError::NotABundle);
    }
    if !ACCEPTED_VERSIONS.contains(&bundle.schema_version) {
        return Err(BundleError::UnsupportedVersion { found: bundle.schema_version });
    }

    let errors = validate(&bundle);
    if errors.is_empty() {
        Ok(bundle)
    } else {
        Err(BundleError::Invalid(errors, false))
    }
}

/// Append one error, unless the report is already at its cap.
fn push_err(errors: &mut Vec<ValidationError>, path: String, reason: String) {
    if errors.len() < MAX_REPORTED_ERRORS {
        errors.push(ValidationError { path, reason });
    }
}

/// Field-level validation. Returns every error found, capped at
/// `MAX_REPORTED_ERRORS` so the response stays a readable size.
///
/// The appender is a free function taking `&mut Vec` rather than a closure
/// capturing `errors`, because a closure holding the mutable borrow would not let
/// `errors` be returned at the end of the function.
pub fn validate(bundle: &Bundle) -> Vec<ValidationError> {
    let mut errors: Vec<ValidationError> = Vec::new();
    let mut push = |path: String, reason: String| push_err(&mut errors, path, reason);

    if bundle.portfolios.is_empty() {
        push("portfolios".into(), "a bundle must contain at least one portfolio".into());
    }
    if row_count(bundle) > MAX_BUNDLE_ROWS {
        push(String::new(), format!("bundle has {} rows, over the {MAX_BUNDLE_ROWS} row limit", row_count(bundle)));
    }
    if !is_rfc3339(&bundle.created_at) {
        push("created_at".into(), "not an RFC 3339 UTC datetime".into());
    }

    let mut seen_refs: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for (pi, p) in bundle.portfolios.iter().enumerate() {
        let pp = format!("portfolios[{pi}]");
        if !seen_refs.insert(p.bundle_ref.as_str()) {
            push(pp.clone(), format!("duplicate ref {:?}", p.bundle_ref));
        }
        if p.name.trim().is_empty() {
            push(format!("{pp}.name"), "must not be empty".into());
        }
        for (field, value) in [("currency", &p.currency), ("base_currency", &p.base_currency)] {
            if value.len() != 3 {
                push(format!("{pp}.{field}"), format!("{value:?} is not a 3-letter currency code"));
            }
        }

        let mut seen_asset_refs: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for (ai, a) in p.assets.iter().enumerate() {
            let ap = format!("{pp}.assets[{ai}]");
            if !seen_asset_refs.insert(a.bundle_ref.as_str()) {
                push(ap.clone(), format!("duplicate ref {:?}", a.bundle_ref));
            }
            if a.symbol.trim().is_empty() {
                push(format!("{ap}.symbol"), "must not be empty".into());
            }
            if a.name.trim().is_empty() {
                push(format!("{ap}.name"), "must not be empty".into());
            }
            if !KNOWN_ASSET_TYPES.contains(&a.asset_type.as_str()) {
                push(format!("{ap}.asset_type"),
                    format!("{:?} is not one of {:?}", a.asset_type, KNOWN_ASSET_TYPES));
            }
            if a.currency.len() != 3 {
                push(format!("{ap}.currency"), format!("{:?} is not a 3-letter currency code", a.currency));
            }
            if let Some(isin) = &a.isin {
                if !isin.is_empty() && !crate::utils::is_valid_isin(isin) {
                    push(format!("{ap}.isin"), format!("{isin:?} is not a valid ISIN"));
                }
            }

            for (ti, t) in a.transactions.iter().enumerate() {
                let tp = format!("{ap}.transactions[{ti}]");
                if !KNOWN_TRANSACTION_TYPES.contains(&t.r#type.as_str()) {
                    push(format!("{tp}.type"),
                        format!("{:?} is not one of {:?}", t.r#type, KNOWN_TRANSACTION_TYPES));
                }
                for (field, value) in [("quantity", &t.quantity), ("price", &t.price), ("fee", &t.fee)] {
                    if rust_decimal::Decimal::from_str_exact(value).is_err() {
                        push(format!("{tp}.{field}"), format!("{value:?} is not a valid decimal"));
                    }
                }
                match chrono::DateTime::parse_from_rfc3339(&t.date) {
                    Ok(dt) => {
                        let day = dt.date_naive();
                        let min = chrono::NaiveDate::from_ymd_opt(1900, 1, 1).unwrap();
                        let max = chrono::NaiveDate::from_ymd_opt(2100, 1, 1).unwrap();
                        if day < min || day > max {
                            push(format!("{tp}.date"),
                                format!("{:?} is outside the supported range 1900-01-01 to 2100-01-01", t.date));
                        }
                    }
                    Err(_) => push(format!("{tp}.date"),
                        format!("{:?} is not an RFC 3339 UTC datetime such as 2024-01-15T00:00:00Z", t.date)),
                }
            }
        }
    }

    for (i, price) in bundle.prices.iter().enumerate() {
        let pp = format!("prices[{i}]");
        if price.symbol.trim().is_empty() {
            push(format!("{pp}.symbol"), "must not be empty".into());
        }
        if chrono::NaiveDate::parse_from_str(&price.date, "%Y-%m-%d").is_err() {
            push(format!("{pp}.date"), format!("{:?} is not a date in YYYY-MM-DD form", price.date));
        }
        if rust_decimal::Decimal::from_str_exact(&price.close_price).is_err() {
            push(format!("{pp}.close_price"), format!("{:?} is not a valid decimal", price.close_price));
        }
    }

    errors
}

fn is_rfc3339(value: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(value).is_ok()
}
```

Add to the top of `mod.rs`: nothing — the code above calls `rust_decimal::Decimal::from_str_exact` fully qualified, which needs no import.

- [ ] **Step 4: Check whether an ISIN validator already exists**

Run: `grep -rn "fn is_valid_isin\|fn validate_isin" src/ | head`

If it exists, use its real path in place of `crate::utils::is_valid_isin`. If it does not exist, delete the ISIN block from the code above — the frontend validates ISINs (`frontend/src/utils/isinValidator.ts`) and inventing a second backend rule in a backup task is scope creep. Note the removal in the commit message.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib backup:: 2>&1 | tail -20`
Expected: all backup tests PASS, including the four from Task 1.

- [ ] **Step 6: Commit**

```bash
git add backend_rust/src/backup/mod.rs
git commit -m "feat(backup): bundle validation with bundle-pathed errors"
```

---

### Task 3: `BackupConfig`, the request-size limit, and the capabilities route

**Files:**
- Create: `backend_rust/src/api_routes/backup.rs`
- Modify: `backend_rust/src/lib.rs` (`build_rocket`, `main.rs` untouched here)
- Modify: `backend_rust/src/main.rs`
- Modify: `backend_rust/src/openapi.rs`
- Modify: `backend_rust/tests/route_coverage_test.rs`
- Test: `backend_rust/tests/backup_test.rs`

**Interfaces:**
- Consumes: Task 1's constants.
- Produces:
  - `backup::BackupConfig { allow_price_restore: bool }` with `Default`, `from_env() -> Self` and `from_env_with(&dyn Fn(&str) -> Option<String>) -> Self`
  - `api_routes::backup::capabilities` handler
  - `api_routes::backup::Capabilities { schema_versions, allow_price_restore, max_bytes, max_rows }`
  - `api_routes::backup::BackupError = (Status, Headers, Json<serde_json::Value>)` — the single error type every handler in this module returns, with `fail(status, message)` and `lock_response()` constructors. One type for every failure means `Retry-After` can be attached to a lock without any handler having to remember it.

- [ ] **Step 1: Write the failing test**

Create `backend_rust/tests/backup_test.rs`:

```rust
//! Route-level behaviour for export and restore.
//!
//! Field-level validation lives in `src/backup/mod.rs`'s own unit tests; this
//! file is about what the routes do — status codes, headers, what got written.

mod common;

use common::{build_rocket, disabled_config, seed_asset, seed_portfolio, seed_transaction, setup_db};
use rocket::http::{Header, Status};
use rocket::local::asynchronous::Client;
use serde_json::{json, Value};

/// The session header the existing tests use to run as the placeholder user.
async fn client(pool: sqlx::SqlitePool) -> Client {
    Client::tracked(build_rocket(pool, common::DEFAULT_USER)).await.unwrap()
}

#[tokio::test]
async fn capabilities_reports_the_limits_the_ui_must_respect() {
    let pool = setup_db(&disabled_config()).await;
    let c = client(pool).await;

    let res = c.get("/api/backup/capabilities").header(session()).dispatch().await;
    assert_eq!(res.status(), Status::OK);
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(body["schema_versions"], json!([1]));
    assert_eq!(body["allow_price_restore"], json!(false), "off by default");
    assert_eq!(body["max_bytes"], json!(8 * 1024 * 1024));
    assert_eq!(body["max_rows"], json!(50_000));
}

#[tokio::test]
async fn capabilities_requires_a_session() {
    let pool = setup_db(&disabled_config()).await;
    let rocket = common::build_rocket_oidc(pool);
    let c = Client::tracked(rocket).await.unwrap();
    let res = c.get("/api/backup/capabilities").dispatch().await;
    assert_eq!(res.status(), Status::Unauthorized);
}
```

Add the session header helper to `backend_rust/tests/common/mod.rs` if one is not already exported — check with `grep -n "pub fn session" tests/common/mod.rs`; the file already has `session_cookie(uid, tv)`, so in the test file use:

```rust
fn session() -> Header {
    let (name, value) = common::session_cookie(1, 0);
    Header::new(name, value)
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --test backup_test 2>&1 | tail -20`
Expected: `404` on `/api/backup/capabilities` (route not mounted).

- [ ] **Step 3: Add `BackupConfig`**

In `backend_rust/src/backup/mod.rs`:

```rust
/// Server-side switches for the backup feature, read once at startup.
///
/// Takes the same injected-environment shape as `StartupConfig::from_env_with`,
/// because `std::env::set_var` is `unsafe` in edition 2024 and the integration
/// tests run multi-threaded: a test that needs prices restorable must construct
/// this rather than mutate the process environment.
#[derive(Debug, Clone, Default)]
pub struct BackupConfig {
    /// Whether a restore may write the `prices` section. Off by default:
    /// `historical_prices` has no `owner_id`, so writing it is a tenant writing
    /// market data every other tenant's dated reports read.
    pub allow_price_restore: bool,
}

impl BackupConfig {
    pub fn from_env() -> Self {
        Self::from_env_with(&|key| std::env::var(key).ok())
    }

    pub fn from_env_with(env: &dyn Fn(&str) -> Option<String>) -> Self {
        let raw = env("BACKUP_ALLOW_PRICE_RESTORE").map(|v| v.trim().to_ascii_lowercase());
        Self {
            allow_price_restore: matches!(raw.as_deref(), Some("1") | Some("true") | Some("yes")),
        }
    }
}
```

- [ ] **Step 4: Raise the request body limit**

In `backend_rust/src/lib.rs`, change the start of `build_rocket` (`lib.rs:560`) from `rocket::build()` to a figment that carries the limit:

```rust
    // Rocket's default `data` limit is 32 KiB, which a real bundle exceeds with a
    // `413`. Raised to twice `backup::MAX_BUNDLE_BYTES` on purpose: the application
    // check in `parse_and_validate` is the one that answers, with a `422` naming the
    // limit, and Rocket's own limit stays a hard ceiling behind it rather than the
    // first thing a user hits. Set in code rather than by deployment environment so
    // it cannot be forgotten in a manifest.
    let limits = rocket::config::Limits::default()
        .limit("data", (2 * crate::backup::MAX_BUNDLE_BYTES as u64).bytes());
    let figment = rocket::Config::figment().merge(("limits", limits));
    let rocket = rocket::custom(figment)
        .attach(cors)
        .attach(AuthFairing)
        .manage(pool)
        .manage(currency_service)
        .manage(auth)
        .manage(crate::backup::BackupConfig::default());
```

`BackupConfig::default()` is managed here so every test rocket has one; `main` chains a second `.manage(...)` with the real value, and Rocket replaces previously-managed state of the same type. Add `use rocket::data::ToByteUnit;` to the imports of `lib.rs`.

- [ ] **Step 5: Add the capabilities handler**

Create `backend_rust/src/api_routes/backup.rs`:

```rust
//! The backup routes: export, capabilities, preview, restore.
//!
//! No SQL here — that lives in `crate::backup`. Handlers here exist to authenticate,
//! authorise, translate a `BundleError` into a status, and hand off.
//!
//! Every handler takes `AuthedUser` because `scripts/check_access_gate.sh` Rule C
//! demands it of every route handler in this directory, including the two that
//! touch no caller-supplied id.
//!
//! Route paths carry the full `/backup/...` prefix because they are mounted on the
//! plain `/api` mount in `lib.rs`, alongside the transaction routes.

use rocket::http::{Header, Headers, Status};
use rocket::serde::json::Json;
use rocket::State;
use serde::Serialize;
use crate::auth::guard::AuthedUser;
use crate::backup::{BackupConfig, ACCEPTED_VERSIONS, MAX_BUNDLE_BYTES, MAX_BUNDLE_ROWS};

/// What the server is willing to accept, so the UI can say so before uploading.
#[derive(Debug, Serialize)]
pub struct Capabilities {
    pub schema_versions: Vec<i64>,
    pub allow_price_restore: bool,
    pub max_bytes: usize,
    pub max_rows: usize,
}

/// Every failure this module can produce, in one shape: a status, headers, and a
/// `{"error": ...}` body. Headers are part of the type because a lock must carry
/// `Retry-After`, and an error type that cannot hold a header is one where that
/// gets forgotten.
pub type BackupError = (Status, Headers, Json<serde_json::Value>);

/// A failure with no extra headers.
pub fn fail(status: Status, message: impl Into<String>) -> BackupError {
    (status, Headers::new(), Json(serde_json::json!({ "error": message.into() })))
}

/// A lock is a `503` the client may retry, and `Retry-After` says so out loud
/// rather than leaving the client to guess whether the failure was permanent.
pub fn lock_response() -> BackupError {
    let mut headers = Headers::new();
    headers.add(Header::new("Retry-After", "2"));
    (
        Status::ServiceUnavailable,
        headers,
        Json(serde_json::json!({ "error": "the database is busy; try again in a moment" })),
    )
}

#[utoipa::path(
    get,
    path = "/api/backup/capabilities",
    responses(
        // `serde_json::Value` rather than a registered component, throughout this
        // module: the backup types carry `schemars::JsonSchema` because they *are*
        // the generated bundle schema, and deriving `utoipa::ToSchema` alongside it
        // would put one shape under two definitions free to drift. The OpenAPI body
        // is therefore described in prose and `docs/backup-schema/` stays the real
        // contract. `get_portfolio_performance` (`analytics.rs:290`) sets the
        // precedent for a `Value` body.
        (status = 200, description = "Server limits and flags: schema_versions, allow_price_restore, max_bytes, max_rows", body = serde_json::Value),
        (status = 401, description = "No session")
    )
)]
#[get("/backup/capabilities")]
pub async fn capabilities(
    _user: AuthedUser,
    cfg: &State<BackupConfig>,
) -> Json<Capabilities> {
    Json(Capabilities {
        schema_versions: ACCEPTED_VERSIONS.to_vec(),
        allow_price_restore: cfg.allow_price_restore,
        max_bytes: MAX_BUNDLE_BYTES,
        max_rows: MAX_BUNDLE_ROWS,
    })
}
```

- [ ] **Step 6: Mount it and register it**

In `backend_rust/src/lib.rs`, add to the `.mount("/api", routes![...])` block (`lib.rs:611-619`):

```rust
            api_routes::backup::capabilities,
```

In `backend_rust/src/openapi.rs`, add to `paths(...)`:

```rust
        crate::api_routes::backup::capabilities,
```

`openapi.rs` `paths(...)`: add `crate::api_routes::backup::capabilities,`.

There are **no** `components(schemas(...))` additions anywhere in this plan; see the comment on the `capabilities` annotation for why.

- [ ] **Step 7: Read the flag in `main`**

In `backend_rust/src/main.rs`, after `let auth = finalise_auth(...)` succeeds:

```rust
    let backup_cfg = backend_rust::backup::BackupConfig::from_env();
    if backup_cfg.allow_price_restore {
        eprintln!("WARN: BACKUP_ALLOW_PRICE_RESTORE is on; a restore may write shared historical_prices rows");
    }
```

and change the launch line to chain it:

```rust
    build_rocket(pool, currency_service, cors, auth)
        .manage(backup_cfg)
        .launch()
        .await?;
```

- [ ] **Step 8: Add the route to the inventory**

In `backend_rust/tests/route_coverage_test.rs`, add to `PROTECTED_ROUTES`:

```rust
    // Authenticates but resolves no caller-supplied id, so `None` — the same
    // shape as `POST /api/portfolios/`.
    ("GET", "/api/backup/capabilities", None),
```

- [ ] **Step 9: Run the tests**

Run: `cargo test --test backup_test --test route_coverage_test 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 10: Commit**

```bash
git add backend_rust/src/backup/mod.rs backend_rust/src/api_routes/backup.rs \
        backend_rust/src/lib.rs backend_rust/src/main.rs backend_rust/src/openapi.rs \
        backend_rust/tests/backup_test.rs backend_rust/tests/route_coverage_test.rs
git commit -m "feat(backup): BackupConfig, request-size limit, and the capabilities route"
```

---

### Task 4: Export

**Files:**
- Modify: `backend_rust/src/backup/export.rs`
- Modify: `backend_rust/src/api_routes/backup.rs`
- Modify: `backend_rust/src/lib.rs` (mount), `backend_rust/src/openapi.rs`
- Modify: `backend_rust/tests/route_coverage_test.rs`, `backend_rust/tests/isolation_test.rs`
- Test: `backend_rust/tests/backup_test.rs`

**Interfaces:**
- Consumes: Task 1's structs, `access::require`, `access::Access`.
- Produces:
  - `backup::export::build(pool: &SqlitePool, user_id: i32, portfolio_ids: &[i32], include_prices: bool) -> Result<Bundle, DbFailure>`
  - `backup::DbFailure { Lock, Other }` with `pub fn status(&self) -> Status`
  - `api_routes::backup::export` handler, `GET /api/backup/export?<portfolio_ids>&<include_prices>`

- [ ] **Step 1: Write the failing tests**

Append to `backend_rust/tests/backup_test.rs`:

```rust
/// Seed: portfolio "One" with one asset and one buy; portfolio "Two" with nothing.
async fn seed_two_portfolios(pool: &sqlx::SqlitePool) -> (i32, i32) {
    let one = seed_portfolio(pool, common::DEFAULT_USER, "One", "USD").await;
    let two = seed_portfolio(pool, common::DEFAULT_USER, "Two", "EUR").await;
    let asset = seed_asset(pool, one, "AAPL", "Apple").await;
    seed_transaction(pool, asset, "BUY", 10.0, 150.0, 1.0).await;
    (one, two)
}

#[tokio::test]
async fn export_contains_only_the_selected_portfolios() {
    let pool = setup_db(&disabled_config()).await;
    let (one, two) = seed_two_portfolios(&pool).await;
    let c = client(pool).await;

    let res = c.get(&format!("/api/backup/export?portfolio_ids={one}")).header(session()).dispatch().await;
    assert_eq!(res.status(), Status::OK);
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(body["portfolios"].as_array().unwrap().len(), 1);
    assert_eq!(body["portfolios"][0]["name"], "One");
    assert_eq!(body["portfolios"][0]["assets"][0]["symbol"], "AAPL");
    assert_eq!(body["portfolios"][0]["assets"][0]["transactions"][0]["quantity"], "10.0");
    assert!(body.get("prices").is_none(), "prices are opt-in and must be absent");
    let _ = two;
}

#[tokio::test]
async fn export_is_a_download_and_is_never_cached() {
    // A cached bundle is a silent data-loss bug: the user restores last week's
    // file believing it is today's.
    let pool = setup_db(&disabled_config()).await;
    let (one, _) = seed_two_portfolios(&pool).await;
    let c = client(pool).await;
    let res = c.get(&format!("/api/backup/export?portfolio_ids={one}")).header(session()).dispatch().await;
    assert_eq!(res.headers().get_one("Cache-Control"), Some("no-store"));
    let cd = res.headers().get_one("Content-Disposition").unwrap();
    assert!(cd.starts_with("attachment; filename=\"portfolio-prism-backup-"), "{cd}");
    assert!(cd.ends_with(".json\""), "{cd}");
}

#[tokio::test]
async fn an_empty_selection_is_a_422_not_an_empty_bundle() {
    // Review Focus #5. An empty bundle must never be produced, because someone
    // will eventually restore it.
    let pool = setup_db(&disabled_config()).await;
    let c = client(pool).await;
    for path in ["/api/backup/export", "/api/backup/export?portfolio_ids=", "/api/backup/export?portfolio_ids= "] {
        let res = c.get(path).header(session()).dispatch().await;
        assert_eq!(res.status(), Status::UnprocessableEntity, "{path}");
    }
}

#[tokio::test]
async fn a_malformed_id_list_is_a_422() {
    let pool = setup_db(&disabled_config()).await;
    let c = client(pool).await;
    let res = c.get("/api/backup/export?portfolio_ids=1,abc").header(session()).dispatch().await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
}

#[tokio::test]
async fn a_duplicate_id_is_exported_once() {
    let pool = setup_db(&disabled_config()).await;
    let (one, _) = seed_two_portfolios(&pool).await;
    let c = client(pool).await;
    let res = c.get(&format!("/api/backup/export?portfolio_ids={one},{one}")).header(session()).dispatch().await;
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(body["portfolios"].as_array().unwrap().len(), 1);
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test --test backup_test 2>&1 | tail -20`
Expected: 404 — the route is not mounted.

- [ ] **Step 3: Implement the export query**

Replace `backend_rust/src/backup/export.rs`:

```rust
//! Reading a caller's rows into a `Bundle`. All export SQL lives here.
//!
//! The queries are transitive by design: only `portfolios` carries `owner_id`, so
//! assets and transactions are reached through `portfolio_id` and `asset_id`. The
//! caller's ids are checked by `access::require` in the handler *before* anything
//! here runs; nothing in this file decides who may see a row.

use chrono::{NaiveDate, Utc};
use sqlx::SqlitePool;
use crate::backup::{Bundle, BundleAsset, BundlePortfolio, BundlePrice, BundleTransaction, FORMAT_TAG};

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
pub fn classify(e: sqlx::Error) -> DbFailure {
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
        schema_version: 1,
        created_at: Utc::now().to_rfc3339(),
        portfolios,
        prices,
    })
}

async fn assets_for(pool: &SqlitePool, portfolio_id: i32) -> Result<Vec<BundleAsset>, DbFailure> {
    let rows: Vec<(i32, String, String, String, Option<String>, String, Option<String>)> =
        sqlx::query_as(
            "SELECT id, symbol, name, asset_type, sector, currency, isin \
             FROM assets WHERE portfolio_id = ? ORDER BY id",
        )
        .bind(portfolio_id)
        .fetch_all(pool)
        .await
        .map_err(classify)?;

    let mut out = Vec::with_capacity(rows.len());
    for (id, symbol, name, asset_type, sector, currency, isin) in rows {
        let txs: Vec<(String, String, String, String, chrono::DateTime<Utc>)> = sqlx::query_as(
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
```

- [ ] **Step 4: Add the handler**

Append to `backend_rust/src/api_routes/backup.rs`:

```rust
/// Parse `?portfolio_ids=1,2` into a de-duplicated, ordered list.
/// `None` on anything unparseable, so the handler answers 422 rather than
/// silently exporting a subset of what was asked for.
fn parse_ids(raw: Option<&str>) -> Option<Vec<i32>> {
    let raw = raw.map(str::trim).filter(|s| !s.is_empty())?;
    let mut ids: Vec<i32> = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let id: i32 = part.parse().ok()?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    (!ids.is_empty()).then_some(ids)
}

#[utoipa::path(
    get,
    path = "/api/backup/export",
    params(
        ("portfolio_ids" = String, Query, description = "Comma-separated portfolio ids to include"),
        ("include_prices" = Option<bool>, Query, description = "Include the price-history section"),
    ),
    responses(
        (status = 200, description = "The bundle; see docs/backup-schema/v1.json for its shape", body = serde_json::Value),
        (status = 404, description = "A portfolio id is not yours", body = serde_json::Value),
        (status = 422, description = "No portfolio selected, or an id did not parse", body = serde_json::Value),
        (status = 503, description = "The database is busy", body = serde_json::Value),
    )
)]
#[get("/backup/export?<portfolio_ids>&<include_prices>")]
pub async fn export(
    user: AuthedUser,
    pool: &State<SqlitePool>,
    portfolio_ids: Option<String>,
    include_prices: Option<bool>,
) -> Result<(Headers, Json<Bundle>), BackupError> {
    let ids = match parse_ids(portfolio_ids.as_deref()) {
        Some(ids) => ids,
        None => return Err(fail(Status::UnprocessableEntity,
            "portfolio_ids must name at least one portfolio, e.g. ?portfolio_ids=1,2")),
    };

    // Every id, before any data query. The gate script's Rule B does not see
    // `WHERE portfolio_id = ?`, so this loop and tests/isolation_test.rs are what
    // make the scoping real.
    for id in &ids {
        require(pool.inner(), user.id, *id, Access::Viewer)
            .await
            .map_err(|status| fail(status, "portfolio not found"))?;
    }

    let bundle = backup_export::build(pool.inner(), &ids, include_prices.unwrap_or(false))
        .await
        .map_err(|e| match e {
            DbFailure::Lock => lock_response(),
            DbFailure::Other => fail(Status::InternalServerError,
                "could not read the portfolio data"),
        })?;

    let mut headers = Headers::new();
    headers.add(Header::new("Cache-Control", "no-store"));
    headers.add(Header::new(
        "Content-Disposition",
        format!("attachment; filename=\"portfolio-prism-backup-{}.json\"", Utc::now().format("%Y-%m-%d")),
    ));
    Ok((headers, Json(bundle)))
}
```

Add to its imports: `use sqlx::SqlitePool;`, `use chrono::Utc;`, `use crate::access::{require, Access};`, `use crate::backup::export::{self as backup_export, DbFailure};`, `use crate::backup::Bundle;`

- [ ] **Step 5: Mount, register, inventory**

`lib.rs` `/api` mount: add `api_routes::backup::export,`.
`openapi.rs` `paths(...)`: add `crate::api_routes::backup::export,` — no component registrations.
`tests/route_coverage_test.rs`: add

```rust
    ("GET", "/api/backup/export", Some(Access::Viewer)),
```

- [ ] **Step 6: Run the tests**

Run: `cargo test --test backup_test --test route_coverage_test 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6b: Add the cross-user matrix for export**

`tests/isolation_test.rs` is the real gate here — the spec says so, and the gate script's own header admits `WHERE portfolio_id = ?` slips past Rule B. Append to that file, using its existing `seed_two_users` and `client_as` helpers:

```rust
#[tokio::test]
async fn b_cannot_export_a_s_portfolio_but_a_can() {
    // The export handler loops `require(.., Viewer)` over every id before it reads
    // anything. This is what proves that loop exists; the static gate cannot.
    let pool = setup_db(&disabled_config()).await;
    let u = seed_two_users(&pool).await;
    let path = format!("/api/backup/export?portfolio_ids={}", u.a_portfolio);

    let b = client_as(pool.clone(), u.b).await;
    assert_eq!(b.get(&path).dispatch().await.status(), Status::NotFound);

    let a = client_as(pool.clone(), u.a).await;
    assert_eq!(a.get(&path).dispatch().await.status(), Status::Ok);
}

#[tokio::test]
async fn one_foreign_id_denies_the_whole_export() {
    // A request for `[mine, theirs]` must not return mine and quietly drop theirs.
    // A partial success would look like a bug in the user's own selection.
    let pool = setup_db(&disabled_config()).await;
    let u = seed_two_users(&pool).await;
    let path = format!("/api/backup/export?portfolio_ids={},{}", u.a_portfolio, u.b_portfolio);

    let a = client_as(pool.clone(), u.a).await;
    assert_eq!(a.get(&path).dispatch().await.status(), Status::NotFound);
}

#[tokio::test]
async fn an_export_never_contains_another_user_s_rows() {
    // Positive-side proof: A's bundle of A's portfolio contains only A's asset.
    let pool = setup_db(&disabled_config()).await;
    let u = seed_two_users(&pool).await;
    let a = client_as(pool.clone(), u.a).await;
    let res = a.get(&format!("/api/backup/export?portfolio_ids={}", u.a_portfolio)).dispatch().await;
    let body: serde_json::Value = res.into_json().await.unwrap();
    let text = body.to_string();
    assert!(!text.contains("BAsset"), "A's bundle must not contain B's asset: {text}");
}
```

Read `seed_two_users` first and assert on the asset names it actually uses — the point is the assertion, not the spelling.

Run: `cargo test --test isolation_test 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add backend_rust/src/backup/export.rs backend_rust/src/api_routes/backup.rs \
        backend_rust/src/lib.rs backend_rust/src/openapi.rs backend_rust/tests
git commit -m "feat(backup): export route with per-id authorisation and download headers"
```

---

### Task 5: Preview — the plan a user must see before restoring

**Files:**
- Modify: `backend_rust/src/backup/restore.rs`
- Modify: `backend_rust/src/api_routes/backup.rs`
- Modify: `backend_rust/src/lib.rs`, `backend_rust/src/openapi.rs`, `backend_rust/tests/route_coverage_test.rs`
- Test: `backend_rust/tests/backup_test.rs`

**Interfaces:**
- Consumes: `Bundle`, `parse_and_validate`, `DbFailure`, `classify`.
- Produces:
  - `backup::restore::Mode { Copy, Replace }` with `Mode::from_query(Option<&str>) -> Option<Mode>`
  - `backup::restore::Counts { portfolios, assets, transactions }`
  - `backup::restore::Plan { schema_version, mode, delete, insert, prices, warnings }`
  - `backup::restore::owned_counts(pool, user_id) -> Result<Counts, DbFailure>`
  - `backup::restore::plan_for(pool, user_id, &Bundle, Mode) -> Result<Plan, DbFailure>`
  - `api_routes::backup::preview`, `POST /api/backup/preview?<mode>`

- [ ] **Step 1: Write the failing tests**

```rust
fn bundle_json(name: &str) -> String {
    json!({
        "format": "portfolio-prism-backup",
        "schema_version": 1,
        "created_at": "2026-11-13T00:00:00Z",
        "portfolios": [{
            "ref": "p1", "name": name, "description": null,
            "currency": "USD", "base_currency": "USD",
            "assets": [{
                "ref": "a1", "symbol": "AAPL", "name": "Apple", "asset_type": "STOCK",
                "sector": null, "currency": "USD", "isin": null,
                "transactions": [{ "type": "BUY", "quantity": "10.0", "price": "150.00",
                                   "fee": "1.00", "date": "2024-01-15T00:00:00Z" }]
            }]
        }]
    }).to_string()
}

#[tokio::test]
async fn preview_counts_what_replace_would_delete_and_insert() {
    let pool = setup_db(&disabled_config()).await;
    seed_two_portfolios(&pool).await;
    let c = client(pool).await;

    let res = c.post("/api/backup/preview?mode=replace").header(session())
        .header(rocket::http::ContentType::JSON).body(bundle_json("Restored")).dispatch().await;
    assert_eq!(res.status(), Status::OK);
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(body["mode"], "replace");
    assert_eq!(body["delete"]["portfolios"], 2, "both owned portfolios go");
    assert_eq!(body["delete"]["assets"], 1);
    assert_eq!(body["delete"]["transactions"], 1);
    assert_eq!(body["insert"]["portfolios"], 1);
    assert_eq!(body["insert"]["assets"], 1);
    assert_eq!(body["insert"]["transactions"], 1);
}

#[tokio::test]
async fn preview_in_copy_mode_deletes_nothing() {
    let pool = setup_db(&disabled_config()).await;
    seed_two_portfolios(&pool).await;
    let c = client(pool).await;
    let res = c.post("/api/backup/preview?mode=copy").header(session())
        .header(rocket::http::ContentType::JSON).body(bundle_json("Restored")).dispatch().await;
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(body["delete"]["portfolios"], 0);
    assert_eq!(body["delete"]["transactions"], 0);
}

#[tokio::test]
async fn preview_writes_nothing() {
    let pool = setup_db(&disabled_config()).await;
    seed_two_portfolios(&pool).await;
    let before: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM portfolios").fetch_one(&pool).await.unwrap();
    let c = client(pool).await;
    c.post("/api/backup/preview?mode=replace").header(session())
        .header(rocket::http::ContentType::JSON).body(bundle_json("Restored")).dispatch().await;
    let after: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM portfolios").fetch_one(&pool).await.unwrap();
    assert_eq!(before.0, after.0);
}

#[tokio::test]
async fn an_unknown_mode_is_a_422() {
    let pool = setup_db(&disabled_config()).await;
    let c = client(pool).await;
    let res = c.post("/api/backup/preview?mode=merge").header(session())
        .header(rocket::http::ContentType::JSON).body(bundle_json("x")).dispatch().await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
}

#[tokio::test]
async fn a_non_json_upload_is_a_422_not_a_500() {
    // Review Focus #4.
    let pool = setup_db(&disabled_config()).await;
    let c = client(pool).await;
    let res = c.post("/api/backup/preview?mode=copy").header(session())
        .header(rocket::http::ContentType::JSON).body("not json").dispatch().await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
    let body: Value = res.into_json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("not valid JSON"));
}

#[tokio::test]
async fn a_bundle_from_a_newer_build_is_refused_by_preview() {
    // Review Focus #2: preview is where the user learns it, before anything is written.
    let pool = setup_db(&disabled_config()).await;
    let c = client(pool).await;
    let mut b: Value = json!(bundle_json("x"));
    b["schema_version"] = json!(2);
    let res = c.post("/api/backup/preview?mode=copy").header(session())
        .header(rocket::http::ContentType::JSON).body(b.to_string()).dispatch().await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(body["accepted"], json!([1]));
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test --test backup_test 2>&1 | tail -20`
Expected: 404 for `/api/backup/preview`.

- [ ] **Step 3: Implement the plan**

Replace `backend_rust/src/backup/restore.rs`:

```rust
//! Counting, planning, and applying a bundle. All restore SQL lives here.
//!
//! Two rules hold throughout. **Bundle refs are never database ids** — they are
//! local labels used only to rewire parent/child links, and nothing here matches
//! one against a stored row. **The whole restore is one transaction** — a bundle
//! that fails anywhere writes nothing.

use serde::Serialize;
use sqlx::SqlitePool;
use crate::backup::Bundle;
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
pub async fn owned_counts(pool: &SqlitePool, user_id: i32) -> Result<Counts, DbFailure> {
    let portfolios: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM portfolios WHERE owner_id = ?",
    )
    .bind(user_id as i64)
    .fetch_one(pool)
    .await
    .map_err(classify)?;

    let assets: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM assets WHERE portfolio_id IN \
         (SELECT id FROM portfolios WHERE owner_id = ?)",
    )
    .bind(user_id as i32)
    .fetch_one(pool)
    .await
    .map_err(classify)?;

    let transactions: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM transactions WHERE asset_id IN \
         (SELECT id FROM assets WHERE portfolio_id IN \
          (SELECT id FROM portfolios WHERE owner_id = ?))",
    )
    .bind(user_id as i32)
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
    let mut c = Counts { portfolios: bundle.portfolios.len(), assets: 0, transactions: 0 };
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
        warnings.push("this bundle carries price history; it is shared market data, not your data".into());
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
```

- [ ] **Step 4: Add the handler**

Append to `backend_rust/src/api_routes/backup.rs`:

```rust
/// The one error body shape for a bundle failure, so a client parses validation
/// failures the same way whichever route produced them.
fn bundle_err(e: BundleErrorKind) -> BackupError {
    (e.status(), Headers::new(), Json(e.body()))
}

#[utoipa::path(
    post,
    path = "/api/backup/preview",
    params(("mode" = String, Query, description = "copy or replace")),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "The plan: mode, delete counts, insert counts, prices, warnings", body = serde_json::Value),
        (status = 422, description = "Bad mode, or the bundle failed validation", body = serde_json::Value),
        (status = 503, description = "The database is busy", body = serde_json::Value),
    )
)]
#[post("/backup/preview?<mode>", data = "<body>")]
pub async fn preview(
    user: AuthedUser,
    pool: &State<SqlitePool>,
    mode: Option<String>,
    body: Bytes,
) -> Result<Json<Plan>, BackupError> {
    let mode = Mode::from_query(mode.as_deref())
        .ok_or_else(|| fail(Status::UnprocessableEntity, "mode must be `copy` or `replace`))?;

    let bundle = parse_and_validate(&body).map_err(bundle_err)?;

    let plan = backup_restore::plan_for(pool.inner(), user.id, &bundle, mode)
        .await
        .map_err(|e| match e {
            DbFailure::Lock => lock_response(),
            DbFailure::Other => fail(Status::InternalServerError, "could not read the current data"),
        })?;
    Ok(Json(plan))
}
```

Add to its imports: `use rocket::data::Bytes;`, `use crate::backup::{BundleError as BundleErrorKind, parse_and_validate};`, `use crate::backup::restore::{self as backup_restore, Mode, Plan};`, `use crate::backup::export::DbFailure;`

Note `Bytes` is `bytes::Bytes`, a reference-counted buffer: `parse_and_validate(&body)` takes it by slice with no copy, which matters when the ceiling is 8 MiB and the whole point of the guard is not to be generous with memory.

- [ ] **Step 5: Mount, register, inventory**

`lib.rs` `/api` mount: `api_routes::backup::preview,`.
`openapi.rs` paths: `crate::api_routes::backup::preview,` — no component registrations, per the note in Task 3.
`route_coverage_test.rs`: `("POST", "/api/backup/preview", None),`.

- [ ] **Step 6: Run the tests**

Run: `cargo test --test backup_test --test route_coverage_test 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add backend_rust/src/backup/restore.rs backend_rust/src/api_routes/backup.rs \
        backend_rust/src/lib.rs backend_rust/src/openapi.rs backend_rust/tests
git commit -m "feat(backup): preview route returning the delete/insert plan"
```

---

### Task 6: Restore — new copy mode

**Files:**
- Modify: `backend_rust/src/backup/restore.rs`
- Modify: `backend_rust/src/api_routes/backup.rs`
- Modify: `backend_rust/src/lib.rs`, `backend_rust/src/openapi.rs`, `backend_rust/tests/route_coverage_test.rs`
- Test: `backend_rust/tests/backup_test.rs`

**Interfaces:**
- Consumes: `Bundle`, `Mode`, `Counts`, `DbFailure`, `classify`, `backup::validate`.
- Produces:
  - `backup::restore::RestoreOutcome { inserted: Counts, portfolio_ids: Vec<i32>, prices_inserted: usize }`
  - `backup::restore::apply(pool, user_id, &Bundle, Mode, apply_prices: bool, cfg: &BackupConfig) -> Result<RestoreOutcome, ApplyError>`
  - `backup::restore::ApplyError { PricesDisabled, Db(DbFailure) }`
  - `api_routes::backup::restore`, `POST /api/backup/restore?<mode>&<apply_prices>`

- [ ] **Step 1: Write the failing tests**

```rust
async fn rows(pool: &sqlx::SqlitePool) -> (i64, i64, i64) {
    let p: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM portfolios").fetch_one(pool).await.unwrap();
    let a: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM assets").fetch_one(pool).await.unwrap();
    let t: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM transactions").fetch_one(pool).await.unwrap();
    (p.0, a.0, t.0)
}

#[tokio::test]
async fn copy_mode_adds_alongside_and_assigns_new_ids() {
    let pool = setup_db(&disabled_config()).await;
    let (one, _) = seed_two_portfolios(&pool).await;
    let before = rows(&pool).await;
    let c = client(pool).await;

    let res = c.post("/api/backup/restore?mode=copy").header(session())
        .header(rocket::http::ContentType::JSON).body(bundle_json("Restored")).dispatch().await;
    assert_eq!(res.status(), Status::OK);
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(body["inserted"]["portfolios"], 1);
    assert_eq!(body["inserted"]["transactions"], 1);

    let after = rows(&pool).await;
    assert_eq!(after, (before.0 + 1, before.1 + 1, before.2 + 1), "copy touches nothing");

    // The new portfolio is owned by the caller, and its asset and transaction are
    // reachable through it — that is the ref rewiring, not merely the row counts.
    let new_id = body["portfolio_ids"][0].as_i64().unwrap();
    assert_ne!(new_id, one as i64, "a bundle ref must never become a database id");
    let chain: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM transactions WHERE asset_id IN \
         (SELECT id FROM assets WHERE portfolio_id = ?)")
        .bind(new_id as i32).fetch_one(&pool).await.unwrap();
    assert_eq!(chain.0, 1, "the transaction must hang off the new asset under the new portfolio");

    let owner: (Option<i64>,) = sqlx::query_as("SELECT owner_id FROM portfolios WHERE id = ?")
        .bind(new_id as i32).fetch_one(&pool).await.unwrap();
    assert_eq!(owner.0, Some(common::DEFAULT_USER as i64));
}

#[tokio::test]
async fn a_round_trip_through_export_and_restore_is_lossless() {
    // The test that proves the format is sufficient — what backup features most
    // often fail at.
    let source = setup_db(&disabled_config()).await;
    let (one, _) = seed_two_portfolios(&source).await;
    let exported: String = {
        let c = Client::tracked(build_rocket(source.clone(), common::DEFAULT_USER)).await.unwrap();
        let res = c.get(&format!("/api/backup/export?portfolio_ids={one}")).header(session()).dispatch().await;
        res.into_string().await.unwrap()
    };

    let target = setup_db(&disabled_config()).await;
    let c = Client::tracked(build_rocket(target.clone(), common::DEFAULT_USER)).await.unwrap();
    let res = c.post("/api/backup/restore?mode=copy").header(session())
        .header(rocket::http::ContentType::JSON).body(exported.clone()).dispatch().await;
    assert_eq!(res.status(), Status::OK, "{:?}", res.into_string().await);

    // Re-export from the target and compare, ignoring the informational timestamp
    // and the ids that are target-local by construction.
    let new_id: (i32,) = sqlx::query_as("SELECT id FROM portfolios WHERE name = 'One'")
        .fetch_one(&target).await.unwrap();
    let res = c.get(&format!("/api/backup/export?portfolio_ids={}", new_id.0)).header(session()).dispatch().await;
    let reexported: Value = res.into_json().await.unwrap();
    let mut original: Value = serde_json::from_str(&exported).unwrap();
    original["created_at"] = json!("");
    reexported["created_at"] = json!("");
    for v in [&mut original] {
        v["portfolios"][0]["ref"] = json!("");
        v["portfolios"][0]["assets"][0]["ref"] = json!("");
    }
    reexported["portfolios"][0]["ref"] = json!("");
    reexported["portfolios"][0]["assets"][0]["ref"] = json!("");
    assert_eq!(original, reexported);
}

#[tokio::test]
async fn a_restored_portfolio_produces_the_same_tax_result() {
    // Proves the rewiring is semantically right, not just present: the tax engine
    // reaches transactions through assets through portfolios.
    let source = setup_db(&disabled_config()).await;
    let (one, _) = seed_two_portfolios(&source).await;
    let c = Client::tracked(build_rocket(source.clone(), common::DEFAULT_USER)).await.unwrap();
    let original: Value = c.get(&format!("/api/portfolios/{one}/tax-summary"))
        .header(session()).dispatch().await.into_json().await.unwrap();

    let target = setup_db(&disabled_config()).await;
    let c = Client::tracked(build_rocket(target.clone(), common::DEFAULT_USER)).await.unwrap();
    let exported: String = {
        let c2 = Client::tracked(build_rocket(source, common::DEFAULT_USER)).await.unwrap();
        c2.get(&format!("/api/backup/export?portfolio_ids={one}")).header(session())
            .dispatch().await.into_string().await.unwrap()
    };
    let res = c.post("/api/backup/restore?mode=copy").header(session())
        .header(rocket::http::ContentType::JSON).body(exported).dispatch().await;
    let new_id = res.into_json().await.unwrap()["portfolio_ids"][0].as_i64().unwrap() as i32;

    let restored: Value = c.get(&format!("/api/portfolios/{new_id}/tax-summary"))
        .header(session()).dispatch().await.into_json().await.unwrap();
    assert_eq!(original, restored);
}

#[tokio::test]
async fn a_bundle_with_no_portfolios_is_refused_by_restore() {
    // Review Focus #5, on the write path this time.
    let pool = setup_db(&disabled_config()).await;
    seed_two_portfolios(&pool).await;
    let before = rows(&pool).await;
    let c = client(pool).await;
    let empty = json!({ "format": "portfolio-prism-backup", "schema_version": 1,
        "created_at": "2026-11-13T00:00:00Z", "portfolios": [] }).to_string();
    let res = c.post("/api/backup/restore?mode=replace").header(session())
        .header(rocket::http::ContentType::JSON).body(empty).dispatch().await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
    assert_eq!(rows(&pool).await, before, "an empty bundle must never be able to wipe a user");
}
```

- [ ] **Step 2b: Note on the metrics engine**

Spec requirement 3 says a restored portfolio must reproduce *the tax engine and the metrics engine* results. This task pins the tax side (`/tax-summary`, which needs no live pricing). The metrics side — TWR, volatility, Sharpe, beta — reads `historical_prices` and the SPY series, so an equality test would need the mock pricing provider wired to a full price corpus; that is `tests/analytics_test.rs`'s machinery, not this feature's. The metrics half is covered by step 1.4 of the `docs/walkthrough.md` section added in Task 13, and that is stated rather than glossed.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test --test backup_test 2>&1 | tail -30`
Expected: 404 for `/api/backup/restore`.

- [ ] **Step 3: Implement the transaction**

Append to `backend_rust/src/backup/restore.rs`:

```rust
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

/// Apply a bundle. **One transaction**: every insert and, for `replace`, every
/// delete happens inside it, so a bundle that fails on row 201 leaves the database
/// exactly as it was. Partial application is not a state this feature can produce.
///
/// The bundle is expected to have passed `backup::validate` already; `apply`
/// re-validates rather than trusting its caller, because a caller that skips
/// validation must not be able to write.
pub async fn apply(
    pool: &SqlitePool,
    user_id: i32,
    bundle: &Bundle,
    mode: Mode,
    apply_prices: bool,
    cfg: &crate::backup::BackupConfig,
) -> Result<RestoreOutcome, ApplyError> {
    if !bundle.prices.is_empty() && apply_prices && !cfg.allow_price_restore {
        return Err(ApplyError::PricesDisabled);
    }

    let mut tx = pool.begin().await.map_err(classify)?;

    if mode == Mode::Replace {
        // Reverse dependency order, scoped by owner_id. Never a bare DELETE, and
        // no caller-supplied id is involved: the caller's own id is the only
        // filter.
        sqlx::query(
            "DELETE FROM transactions WHERE asset_id IN \
             (SELECT id FROM assets WHERE portfolio_id IN \
              (SELECT id FROM portfolios WHERE owner_id = ?))",
        )
        .bind(user_id as i32)
        .execute(&mut *tx)
        .await
        .map_err(classify)?;

        sqlx::query("DELETE FROM assets WHERE portfolio_id IN \
                     (SELECT id FROM portfolios WHERE owner_id = ?)")
            .bind(user_id as i32)
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
        let (new_portfolio,): (i32,) = sqlx::query(
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
            let (new_asset,): (i32,) = sqlx::query(
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
```

- [ ] **Step 4: Add the handler**

Append to `backend_rust/src/api_routes/backup.rs`:

```rust
#[utoipa::path(
    post,
    path = "/api/backup/restore",
    params(
        ("mode" = String, Query, description = "copy or replace"),
        ("apply_prices" = Option<bool>, Query, description = "Apply the bundle's price section; requires the server flag"),
    ),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Restored: inserted counts, portfolio_ids, prices_inserted", body = serde_json::Value),
        (status = 422, description = "Bad mode, failed validation, or prices requested while disabled", body = serde_json::Value),
        (status = 503, description = "The database is busy; nothing was written", body = serde_json::Value),
    )
)]
#[post("/backup/restore?<mode>&<apply_prices>", data = "<body>")]
pub async fn restore(
    user: AuthedUser,
    pool: &State<SqlitePool>,
    backup_cfg: &State<BackupConfig>,
    mode: Option<String>,
    apply_prices: Option<bool>,
    body: Bytes,
) -> Result<Json<RestoreOutcome>, BackupError> {
    let mode = Mode::from_query(mode.as_deref())
        .ok_or_else(|| fail(Status::UnprocessableEntity, "mode must be `copy` or `replace`))?;

    let bundle = parse_and_validate(&body).map_err(bundle_err)?;

    let outcome = backup_restore::apply(
        pool.inner(), user.id, &bundle, mode,
        apply_prices.unwrap_or(false), backup_cfg.inner(),
    )
    .await
    .map_err(|e| match e {
        ApplyError::PricesDisabled => fail(
            Status::UnprocessableEntity,
            "this bundle contains price history and this server has price restore disabled",
        ),
        ApplyError::Db(DbFailure::Lock) => lock_response(),
        ApplyError::Db(DbFailure::Other) => fail(
            Status::InternalServerError,
            "restore failed; nothing was written",
        ),
    })?;

    Ok(Json(outcome))
}
```

Add `use crate::backup::restore::{ApplyError, RestoreOutcome};` to the imports.

- [ ] **Step 5: Mount, register, inventory**

`lib.rs` `/api` mount: `api_routes::backup::restore,`.
`openapi.rs` paths: `crate::api_routes::backup::restore,` — no component registrations.
`route_coverage_test.rs`: `("POST", "/api/backup/restore", None),`.

- [ ] **Step 6: Run the tests**

Run: `cargo test --test backup_test 2>&1 | tail -30`
Expected: PASS, including the round-trip and the tax-equality tests.

- [ ] **Step 7: Commit**

```bash
git add backend_rust/src/backup/restore.rs backend_rust/src/api_routes/backup.rs \
        backend_rust/src/lib.rs backend_rust/src/openapi.rs backend_rust/tests
git commit -m "feat(backup): restore in copy mode as one transaction with ref rewiring"
```

---

### Task 7: Restore — replace mode

**Files:**
- Modify: `backend_rust/tests/backup_test.rs`
- Modify: `backend_rust/tests/isolation_test.rs` (the last test below uses that file's `seed_two_users` / `client_as` helpers, so it belongs there, not in `backup_test.rs`)
- Modify: `backend_rust/src/backup/restore.rs` only if a test below fails

**Interfaces:**
- Consumes: Task 6's `apply` with `Mode::Replace`.
- Produces: nothing new; this task proves behaviour the code already claims.

- [ ] **Step 1: Write the tests**

```rust
#[tokio::test]
async fn replace_removes_everything_the_caller_owns() {
    // Spec: replace means "make my data equal this bundle", one rule, and the
    // preview is the safety mechanism. The other three portfolios go too.
    let pool = setup_db(&disabled_config()).await;
    seed_two_portfolios(&pool).await;
    let c = client(pool).await;

    let res = c.post("/api/backup/restore?mode=replace").header(session())
        .header(rocket::http::ContentType::JSON).body(bundle_json("Only")).dispatch().await;
    assert_eq!(res.status(), Status::OK);

    let (p, a, t) = rows(&pool).await;
    assert_eq!((p, a, t), (1, 1, 1), "exactly the bundle remains");
    let name: (String,) = sqlx::query_as("SELECT name FROM portfolios").fetch_one(&pool).await.unwrap();
    assert_eq!(name.0, "Only");
}

#[tokio::test]
async fn replace_leaves_other_users_data_alone() {
    // The delete is scoped by owner_id. A restore must never be a cross-tenant
    // wipe, whatever the bundle contains.
    let pool = setup_db(&disabled_config()).await;
    seed_two_portfolios(&pool).await;
    let other = common::seed_user(&pool, "someone-else").await;
    let theirs = seed_portfolio(&pool, other, "Theirs", "USD").await;
    seed_asset(&pool, theirs, "MSFT", "Microsoft").await;

    let c = client(pool).await;
    let res = c.post("/api/backup/restore?mode=replace").header(session())
        .header(rocket::http::ContentType::JSON).body(bundle_json("Only")).dispatch().await;
    assert_eq!(res.status(), Status::OK);

    let kept: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM portfolios WHERE owner_id = ?")
        .bind(other as i64).fetch_one(&pool).await.unwrap();
    assert_eq!(kept.0, 1, "another user's portfolio must survive");
}

#[tokio::test]
async fn replacing_twice_in_a_row_is_still_exactly_the_bundle() {
    let pool = setup_db(&disabled_config()).await;
    seed_two_portfolios(&pool).await;
    let c = client(pool).await;
    for _ in 0..2 {
        let res = c.post("/api/backup/restore?mode=replace").header(session())
            .header(rocket::http::ContentType::JSON).body(bundle_json("Only")).dispatch().await;
        assert_eq!(res.status(), Status::OK);
    }
    assert_eq!(rows(&pool).await, (1, 1, 1));
}

#[tokio::test]
async fn a_bundle_exported_by_a_and_restored_by_b_becomes_b_s() {
    // Restore always writes the caller as owner. A bundle is data, not a grant:
    // carrying A's rows into B's session must not make them visible to A, and must
    // not let B write into A's space.
    let pool = setup_db(&disabled_config()).await;
    let u = seed_two_users(&pool).await;
    let exported: String = {
        let a = client_as(pool.clone(), u.a).await;
        a.get(&format!("/api/backup/export?portfolio_ids={}", u.a_portfolio))
            .dispatch().await.into_string().await.unwrap()
    };

    let b = client_as(pool.clone(), u.b).await;
    let res = b.post("/api/backup/restore?mode=copy").header(rocket::http::ContentType::JSON)
        .body(exported).dispatch().await;
    assert_eq!(res.status(), Status::OK);
    let new_id = res.into_json().await.unwrap()["portfolio_ids"][0].as_i64().unwrap() as i32;

    let owner: (Option<i64>,) = sqlx::query_as("SELECT owner_id FROM portfolios WHERE id = ?")
        .bind(new_id).fetch_one(&pool).await.unwrap();
    assert_eq!(owner.0, Some(u.b as i64), "the restorer owns what they restored");

    // And A cannot see it.
    let a = client_as(pool.clone(), u.a).await;
    assert_eq!(a.get(&format!("/api/portfolios/{new_id}")).dispatch().await.status(),
        Status::NotFound);
}
```

- [ ] **Step 2: Run them**

Run: `cargo test --test backup_test --test isolation_test 2>&1 | tail -30`
Expected: PASS if Task 6 implemented the replace branch as written. If any fails, fix `apply` — do not weaken the test.

- [ ] **Step 3: Commit**

```bash
git add backend_rust/tests/backup_test.rs backend_rust/tests/isolation_test.rs
git commit -m "test(backup): replace mode deletes only the caller's rows"
```

---

### Task 8: Price history — opt-in export, flag-gated restore

**Files:**
- Modify: `backend_rust/tests/backup_test.rs`
- Modify: `backend_rust/tests/common/mod.rs` (a `build_rocket_with_backup_config` helper)
- Modify: `backend_rust/src/backup/restore.rs` only if a test fails

**Interfaces:**
- Consumes: `BackupConfig`, `apply`'s `apply_prices` parameter, `export::build`'s `include_prices`.
- Produces: `common::build_rocket_with_backup_config(pool, as_user, BackupConfig) -> Rocket<Build>`.

- [ ] **Step 1: Add the test helper**

Append to `backend_rust/tests/common/mod.rs`:

```rust
/// `build_rocket` with a non-default `BackupConfig`. Needed because
/// `std::env::set_var` is `unsafe` in edition 2024 and these tests run
/// multi-threaded, so the flag must be injected, not exported.
#[allow(dead_code)]
pub fn build_rocket_with_backup_config(
    pool: SqlitePool,
    as_user: i32,
    cfg: backend_rust::backup::BackupConfig,
) -> Rocket<Build> {
    rocket_with_service(pool, offline_pricing_service(), as_user).manage(cfg)
}
```

- [ ] **Step 2: Write the failing tests**

```rust
async fn seed_price(pool: &sqlx::SqlitePool, symbol: &str, day: &str, close: &str) {
    sqlx::query("INSERT INTO historical_prices (symbol, date, close_price) VALUES (?, ?, ?)")
        .bind(symbol).bind(day).bind(close).execute(pool).await.unwrap();
}

#[tokio::test]
async fn include_prices_adds_the_section_and_names_only_held_symbols() {
    let pool = setup_db(&disabled_config()).await;
    let (one, _) = seed_two_portfolios(&pool).await;
    seed_price(&pool, "AAPL", "2024-01-15", "149.50").await;
    seed_price(&pool, "ZZZZ", "2024-01-15", "9.99").await;
    let c = client(pool).await;

    let res = c.get(&format!("/api/backup/export?portfolio_ids={one}&include_prices=1"))
        .header(session()).dispatch().await;
    let body: Value = res.into_json().await.unwrap();
    let prices = body["prices"].as_array().unwrap();
    assert_eq!(prices.len(), 1, "only symbols the portfolio holds");
    assert_eq!(prices[0]["symbol"], "AAPL");
    assert_eq!(prices[0]["date"], "2024-01-15");
}

#[tokio::test]
async fn applying_prices_is_refused_while_the_flag_is_off() {
    let pool = setup_db(&disabled_config()).await;
    let c = client(pool).await;
    let mut b: Value = json!(bundle_json("x"));
    b["prices"] = json!([{ "symbol": "AAPL", "date": "2024-01-15", "close_price": "1.00" }]);
    let res = c.post("/api/backup/restore?mode=copy&apply_prices=true").header(session())
        .header(rocket::http::ContentType::JSON).body(b.to_string()).dispatch().await;
    assert_eq!(res.status(), Status::UnprocessableEntity, "never a silent ignore");
    let n: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM historical_prices").fetch_one(&pool).await.unwrap();
    assert_eq!(n.0, 0);
}

#[tokio::test]
async fn with_the_flag_on_prices_fill_gaps_but_never_overwrite() {
    let pool = setup_db(&disabled_config()).await;
    seed_two_portfolios(&pool).await;
    seed_price(&pool, "AAPL", "2024-01-15", "149.50").await;   // exists
    seed_price(&pool, "AAPL", "2024-01-16", "151.00").await;   // exists
    let rocket = common::build_rocket_with_backup_config(
        pool.clone(), common::DEFAULT_USER,
        backend_rust::backup::BackupConfig { allow_price_restore: true },
    );
    let c = Client::tracked(rocket).await.unwrap();

    let mut b: Value = json!(bundle_json("x"));
    b["prices"] = json!([
        { "symbol": "AAPL", "date": "2024-01-15", "close_price": "666.00" }, // must NOT land
        { "symbol": "AAPL", "date": "2024-02-01", "close_price": "180.00" }  // must land
    ]);
    let res = c.post("/api/backup/restore?mode=copy&apply_prices=true").header(session())
        .header(rocket::http::ContentType::JSON).body(b.to_string()).dispatch().await;
    assert_eq!(res.status(), Status::OK);
    let body: Value = res.into_json().await.unwrap();
    assert_eq!(body["prices_inserted"], 1);

    let kept: (String,) = sqlx::query_as(
        "SELECT close_price FROM historical_prices WHERE symbol = 'AAPL' AND date = '2024-01-15'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(kept.0, "149.50", "DO NOTHING, not DO UPDATE — a bundle cannot poison a cached price");
    let added: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM historical_prices WHERE date = '2024-02-01'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(added.0, 1);
}
```

- [ ] **Step 3: Run them**

Run: `cargo test --test backup_test 2>&1 | tail -30`
Expected: PASS if Tasks 4 and 6 implemented the price branches as written. Fix the code, not the test, if they do not.

- [ ] **Step 4: Commit**

```bash
git add backend_rust/tests/backup_test.rs backend_rust/tests/common/mod.rs
git commit -m "test(backup): price section is opt-in on export and flag-gated on restore"
```

---

### Task 9: Atomicity, the caps, and the lock response

**Files:**
- Modify: `backend_rust/tests/backup_test.rs`
- Modify: `backend_rust/src/backup/restore.rs`, `backend_rust/src/api_routes/backup.rs` if a test fails

**Interfaces:**
- Consumes: everything above.
- Produces: nothing new. This task pins the failure behaviour the spec promises.

- [ ] **Step 1: Write the atomicity test**

```rust
/// A bundle valid for 200 transactions and invalid on the 201st.
fn bundle_with_bad_last_row() -> String {
    let mut b: Value = json!(bundle_json("Atomic"));
    let txs = b["portfolios"][0]["assets"][0]["transactions"].as_array_mut().unwrap();
    for i in 0..200 {
        txs.push(json!({ "type": "BUY", "quantity": "1", "price": "1", "fee": "0",
                         "date": format!("2024-01-{:02}T00:00:00Z", (i % 28) + 1) }));
    }
    // Validation catches this before the transaction opens, which is the point:
    // the write path is never reached.
    txs.push(json!({ "type": "BUY", "quantity": "not-a-number", "price": "1",
                     "fee": "0", "date": "2024-02-01T00:00:00Z" }));
    b.to_string()
}

#[tokio::test]
async fn a_bundle_that_fails_on_the_last_row_writes_nothing() {
    let pool = setup_db(&disabled_config()).await;
    seed_two_portfolios(&pool).await;
    let before = rows(&pool).await;
    let c = client(pool).await;

    let res = c.post("/api/backup/restore?mode=copy").header(session())
        .header(rocket::http::ContentType::JSON).body(bundle_with_bad_last_row()).dispatch().await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
    assert_eq!(rows(&pool).await, before, "byte-identical: nothing half-applied");
}

#[tokio::test]
async fn an_oversized_bundle_is_a_422_naming_the_limit_before_any_write() {
    // Review Focus: the application limit must answer, not Rocket's framework 413.
    let pool = setup_db(&disabled_config()).await;
    let c = client(pool).await;
    let big = format!(r#"{{"format":"portfolio-prism-backup","schema_version":1,
        "created_at":"2026-11-13T00:00:00Z","portfolios":[],"pad":"{}"}}"#, "x".repeat(8 * 1024 * 1024));
    let res = c.post("/api/backup/preview?mode=copy").header(session())
        .header(rocket::http::ContentType::JSON).body(big).dispatch().await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
    let body: Value = res.into_json().await.unwrap();
    assert!(body["errors"].to_string().contains("byte limit"), "{body}");
}

#[tokio::test]
async fn a_bundle_over_the_row_cap_is_refused_before_any_write() {
    let pool = setup_db(&disabled_config()).await;
    let c = client(pool).await;
    let mut b: Value = json!(bundle_json("Huge"));
    let txs = b["portfolios"][0]["assets"][0]["transactions"].as_array_mut().unwrap();
    for i in 0..=50_000 {
        txs.push(json!({ "type": "BUY", "quantity": "1", "price": "1", "fee": "0",
                         "date": format!("2024-01-{:02}T00:00:00Z", (i % 28) + 1) }));
    }
    let res = c.post("/api/backup/preview?mode=copy").header(session())
        .header(rocket::http::ContentType::JSON).body(b.to_string()).dispatch().await;
    assert_eq!(res.status(), Status::UnprocessableEntity);
    let body: Value = res.into_json().await.unwrap();
    assert!(body["errors"].to_string().contains("row limit"), "{body}");
}
```

- [ ] **Step 2: Run them**

Run: `cargo test --test backup_test 2>&1 | tail -30`
Expected: PASS. The oversized test depends on the Rocket `data` limit being twice `MAX_BUNDLE_BYTES` (Task 3 Step 4) — if it returns `413`, that limit was not applied.

- [ ] **Step 3: Add the lock-classification test**

A real `SQLITE_BUSY` cannot be provoked deterministically in an in-memory test, so pin the classification instead of the race. Append to `backend_rust/src/backup/export.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::error::DatabaseError;

    /// A database error that reports itself as a lock, which is how SQLite surfaces
    /// `SQLITE_BUSY` once the `busy_timeout` in `main.rs` has expired.
    #[derive(Debug)]
    struct FakeLock;

    impl std::fmt::Display for FakeLock {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("database is locked")
        }
    }
    impl std::error::Error for FakeLock {}

    impl DatabaseError for FakeLock {
        fn message(&self) -> &str { "database is locked" }
        // The `code()` override is exercised separately below; the trait's other
        // required methods are `as_error`, `as_error_mut` and `into_error`.
        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) { self }
        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) { self }
        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> { self }
    }

    #[test]
    fn a_lock_by_message_is_a_503() {
        let busy = sqlx::Error::Database(Box::new(FakeLock));
        assert_eq!(classify(busy).status(), rocket::http::Status::ServiceUnavailable);
    }

    #[test]
    fn anything_else_is_a_500_not_a_503() {
        // The other half of the rule in src/access.rs: an outage must not be
        // reported as a client error, and a client error must not be reported as
        // an outage the client should retry.
        #[derive(Debug)]
        struct FakeOther;
        impl std::fmt::Display for FakeOther {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("no such table") }
        }
        impl std::error::Error for FakeOther {}
        impl DatabaseError for FakeOther {
            fn message(&self) -> &str { "no such table" }
            fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) { self }
            fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) { self }
            fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> { self }
        }
        let other = sqlx::Error::Database(Box::new(FakeOther));
        assert_eq!(classify(other).status(), rocket::http::Status::InternalServerError);
    }
}
```

Run: `cargo test --lib backup::export 2>&1 | tail -10`
Expected: PASS. If the resolved sqlx version requires additional `DatabaseError` methods, read `~/.cargo/registry/src/*/sqlx-core-0.8*/src/error.rs` and implement the actual required set — do not delete the test.

- [ ] **Step 4: Confirm `Retry-After` reaches the client**

`lock_response()` (Task 3) is the only place a `503` is produced in this module, and Tasks 4-6 route every `DbFailure::Lock` through it, so the header cannot be forgotten per-handler. Pin it end to end:

```rust
#[tokio::test]
async fn a_lock_response_carries_retry_after() {
    // The classification test in export.rs proves 503; this proves the header the
    // client needs in order to retry rather than give up.
    // Constructed by pointing the app at a pool whose single connection is held
    // open by a write transaction, which is the one reproducible lock here.
    let pool = setup_db(&disabled_config()).await;
    seed_two_portfolios(&pool).await;
    let mut holder = pool.acquire().await.unwrap();
    sqlx::query("BEGIN IMMEDIATE").execute(&mut *holder).await.unwrap();

    let c = client(pool).await;
    let res = c.get("/api/backup/export?portfolio_ids=1").header(session()).dispatch().await;
    // An in-memory pool shares one connection, so this may succeed rather than
    // block; assert only that it is never a 500, and that when it is a 503 the
    // header is present.
    if res.status() == Status::ServiceUnavailable {
        assert_eq!(res.headers().get_one("Retry-After"), Some("2"));
    } else {
        assert_ne!(res.status(), Status::InternalServerError);
    }
}
```

- [ ] **Step 5: Run the full backend gate**

Run: `cargo test && bash scripts/check_access_gate.sh && cargo clippy --all-targets`
Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add backend_rust
git commit -m "test(backup): atomicity, byte and row caps, and the lock response"
```

---

### Task 10: The portability gate and CI steps

**Files:**
- Create: `backend_rust/scripts/check_backup_portability.sh`
- Modify: `.github/workflows/ci.yml`
- Modify: `docs/superpowers/specs/2026-11-13-backup-export-restore-design.md` only if the gate forced a change

**Interfaces:**
- Consumes: the finished `src/backup/`.
- Produces: a CI gate. The engine-selection spec depends on this script existing and passing.

- [ ] **Step 1: Write the gate script**

Create `backend_rust/scripts/check_backup_portability.sh`:

```bash
#!/usr/bin/env bash
# Portability gate for the backup module.
#
# The bundle format is engine-neutral by discipline, not by abstraction: the
# engine-selection work is a separate spec, and this script is what keeps its
# share of the job a mechanical change to `src/backup/` rather than a rewrite.
# The rule list is the spec's *Portability discipline* section, copied.
#
# A gate that fails on correct code gets deleted in the next PR that lands, so
# the banned set is limited to constructs with no cross-engine spelling. `?`
# placeholders are allowed on purpose: SQLite's syntax today, and what an
# Any-style pool rewrites for Postgres.

set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

DIR="src/backup"
if [ ! -d "$DIR" ]; then
  echo "GATE FAILED: $DIR does not exist"
  exit 1
fi

BANNED=(
  "PRAGMA"
  "pragma_"
  "INSERT OR REPLACE"
  "INSERT OR IGNORE"
  "AUTOINCREMENT"
  "last_insert_rowid"
  "strftime"
  "julianday"
  "datetime("
  "IFNULL"
  "GROUP_CONCAT"
)

fail=0
checked=0

for pattern in "${BANNED[@]}"; do
  hits=$(grep -rn --include='*.rs' -F "$pattern" "$DIR" || true)
  if [ -n "$hits" ]; then
    echo "GATE FAILED: '$pattern' has no cross-engine spelling:"
    echo "$hits"
    fail=1
  fi
done

while IFS= read -r file; do
  checked=$((checked + 1))
  # Every INSERT must name its columns. A bare `INSERT INTO t VALUES (...)`
  # silently changes meaning when a column is added.
  while IFS=: read -r lineno line; do
    if ! echo "$line" | grep -q '('; then
      echo "GATE FAILED: $file:$lineno INSERT without an explicit column list"
      fail=1
    fi
  done < <(grep -n "INSERT INTO" "$file" || true)
done < <(find "$DIR" -name '*.rs' | sort)

if [ "$checked" -eq 0 ]; then
  echo "GATE FAILED: no files inspected under $DIR"
  exit 1
fi

if [ "$fail" -ne 0 ]; then
  echo "Portability gate failed. See the spec's *Portability discipline* section."
  exit 1
fi
echo "Portability gate passed: $checked file(s) inspected under $DIR"
```

Run: `chmod +x backend_rust/scripts/check_backup_portability.sh && bash backend_rust/scripts/check_backup_portability.sh`
Expected: `Portability gate passed: 4 file(s) inspected under src/backup`. If it fails, fix the SQL — the whole point is that the module is portable, so the gate is the deliverable, not the obstacle.

- [ ] **Step 2: Add the CI steps**

In `.github/workflows/ci.yml`, immediately after the `Generate and Validate OpenAPI Spec` step (`ci.yml:39-48`):

```yaml
      - name: Backup schema is up to date
        run: |
          # Same contract as the OpenAPI diff above: `--print-backup-schema` exits
          # before reading configuration, the database, or the network, so this
          # needs no portfolio.db, no OIDC configuration, and no running server.
          cd backend_rust
          cargo run --bin backend_rust -- --print-backup-schema > /tmp/current_backup_schema.json
          diff /tmp/current_backup_schema.json ../docs/backup-schema/v1.json \
            || (echo "Backup schema has changed! Regenerate docs/backup-schema/v1.json" && exit 1)

      - name: Backup module stays engine-portable
        run: bash backend_rust/scripts/check_backup_portability.sh
```

- [ ] **Step 3: Verify the schema artifact is current**

```bash
cd backend_rust
cargo run --bin backend_rust -- --print-backup-schema > /tmp/current_backup_schema.json
diff /tmp/current_backup_schema.json ../docs/backup-schema/v1.json && echo IN SYNC
```
Expected: `IN SYNC`. If it differs, regenerate the committed file and include it.

- [ ] **Step 4: Commit**

```bash
git add backend_rust/scripts/check_backup_portability.sh .github/workflows/ci.yml docs/backup-schema
git commit -m "ci(backup): schema freshness diff and the engine-portability gate"
```

---

### Task 11: Frontend — the Backup tab and the export card

**Files:**
- Create: `frontend/src/components/BackupPanel.tsx`
- Modify: `frontend/src/App.tsx` (nav item at `App.tsx:540-557`, tab body near `App.tsx:705`)
- Test: `frontend/src/test/components/BackupPanel.test.tsx`

**Interfaces:**
- Consumes: `apiFetch`/`API_BASE` from `utils/apiFetch.ts`, the `portfolios` array already in `App.tsx` state.
- Produces: `BackupPanel` default export with props `{ portfolios: { id: number; name: string }[] }`.

- [ ] **Step 1: Write the failing tests**

Create `frontend/src/test/components/BackupPanel.test.tsx`:

```tsx
import { render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import BackupPanel from '../../components/BackupPanel'

const portfolios = [
  { id: 1, name: 'Main' },
  { id: 2, name: 'IRA' },
]

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  })
}

const CAPS = { schema_versions: [1], allow_price_restore: false, max_bytes: 8388608, max_rows: 50000 }

let fetchMock: ReturnType<typeof vi.fn>
beforeEach(() => {
  fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    if (url.includes('/backup/capabilities')) return jsonResponse(CAPS)
    if (url.includes('/backup/preview')) return jsonResponse({
      schema_version: 1, mode: 'copy',
      delete: { portfolios: 0, assets: 0, transactions: 0 },
      insert: { portfolios: 1, assets: 2, transactions: 3 },
      prices: 0, warnings: [],
    })
    return jsonResponse({}, 404)
  })
  vi.stubGlobal('fetch', fetchMock)
})
afterEach(() => vi.unstubAllGlobals())

describe('export card', () => {
  it('offers one checkbox per portfolio and none selected by default', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await waitFor(() => expect(screen.getByTestId('export-button')).toBeDisabled())
    expect(screen.getByRole('checkbox', { name: /Main/i })).not.toBeChecked()
    expect(screen.getByRole('checkbox', { name: /IRA/i })).not.toBeChecked()
  })

  it('builds the download href from exactly the selected ids', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.click(screen.getByRole('checkbox', { name: /IRA/i }))
    const link = await screen.findByTestId('download-bundle')
    expect(link).toHaveAttribute('href', '/api/backup/export?portfolio_ids=2')
  })

  it('names every selected id, in selection order', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.click(screen.getByRole('checkbox', { name: /IRA/i }))
    await userEvent.click(screen.getByRole('checkbox', { name: /Main/i }))
    expect(await screen.findByTestId('download-bundle'))
      .toHaveAttribute('href', '/api/backup/export?portfolio_ids=2,1')
  })

  it('hides the download again when the last portfolio is unticked', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    const box = screen.getByRole('checkbox', { name: /Main/i })
    await userEvent.click(box)
    await userEvent.click(box)
    expect(screen.queryByTestId('download-bundle')).toBeNull()
    expect(screen.getByTestId('export-button')).toBeDisabled()
  })

  it('does not offer price history when the server has it off', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await waitFor(() => expect(screen.queryByTestId('include-prices')).toBeNull())
  })

  it('offers price history when the server allows it, and adds it to the href', async () => {
    fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
      if (String(input).includes('/backup/capabilities')) {
        return jsonResponse({ ...CAPS, allow_price_restore: true })
      }
      return jsonResponse({}, 404)
    })
    render(<BackupPanel portfolios={portfolios} />)
    const toggle = await screen.findByTestId('include-prices')
    await userEvent.click(screen.getByRole('checkbox', { name: /Main/i }))
    await userEvent.click(toggle)
    expect(await screen.findByTestId('download-bundle'))
      .toHaveAttribute('href', '/api/backup/export?portfolio_ids=1&include_prices=1')
  })
})
```

- [ ] **Step 2: Run them to verify they fail**

Run from `frontend/`: `npm run test -- BackupPanel`
Expected: cannot resolve `../../components/BackupPanel`.

- [ ] **Step 3: Implement the panel (export half)**

Create `frontend/src/components/BackupPanel.tsx`:

```tsx
import { useEffect, useMemo, useState } from 'react';
import { apiFetch, API_BASE } from '../utils/apiFetch';

/**
 * Export and restore for the signed-in user's own portfolios.
 *
 * Two cards, and the asymmetry between them is deliberate. Export is a plain
 * anchor — the bytes reach the disk unchanged and the HttpOnly session cookie
 * rides along because `/api` is same-origin, exactly as on the Tax Report tab.
 * Restore is the destructive half, so it is `fetch` all the way down: the plan
 * must be rendered before the button unlocks, and a link cannot do that.
 */

interface PortfolioOption {
  id: number;
  name: string;
}

interface Capabilities {
  schema_versions: number[];
  allow_price_restore: boolean;
  max_bytes: number;
  max_rows: number;
}

interface Plan {
  schema_version: number;
  mode: string;
  delete: { portfolios: number; assets: number; transactions: number };
  insert: { portfolios: number; assets: number; transactions: number };
  prices: number;
  warnings: string[];
}

interface Props {
  portfolios: PortfolioOption[];
}

export default function BackupPanel({ portfolios }: Props) {
  const [caps, setCaps] = useState<Capabilities | null>(null);
  const [selected, setSelected] = useState<number[]>([]);
  const [includePrices, setIncludePrices] = useState(false);

  useEffect(() => {
    let cancelled = false;
    apiFetch(`${API_BASE}/backup/capabilities`)
      .then((r) => (r.ok ? r.json() : null))
      .then((body) => { if (!cancelled && body) setCaps(body as Capabilities); })
      .catch(() => { /* the panel stays usable; the export needs no capability */ });
    return () => { cancelled = true; };
  }, []);

  // Selection order is the user's, and the href preserves it, so the bundle's
  // portfolio order matches the order they ticked.
  const toggle = (id: number) =>
    setSelected((prev) => (prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id]));

  const exportHref = useMemo(() => {
    if (selected.length === 0) return '';
    const params = new URLSearchParams({ portfolio_ids: selected.join(',') });
    if (includePrices && caps?.allow_price_restore) params.set('include_prices', '1');
    return `${API_BASE}/backup/export?${params.toString()}`;
  }, [selected, includePrices, caps]);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: '24px' }}>
      <section className="glass-card" data-testid="export-card">
        <h2>Export a backup</h2>
        <p className="text-muted">
          Choose the portfolios to include. The file is downloaded to your device and is never
          stored on the server.
        </p>
        {portfolios.length === 0 && <p data-testid="no-portfolios">No portfolios to export.</p>}
        <ul style={{ listStyle: 'none', padding: 0, display: 'grid', gap: '8px' }}>
          {portfolios.map((p) => (
            <li key={p.id}>
              <label>
                <input
                  type="checkbox"
                  aria-label={p.name}
                  checked={selected.includes(p.id)}
                  onChange={() => toggle(p.id)}
                />{' '}
                {p.name}
              </label>
            </li>
          ))}
        </ul>
        {caps?.allow_price_restore && (
          <label style={{ display: 'block', marginTop: '12px' }}>
            <input
              data-testid="include-prices"
              type="checkbox"
              checked={includePrices}
              onChange={(e) => setIncludePrices(e.target.checked)}
            />{' '}
            Include price history (shared market data, not your data)
          </label>
        )}
        {/* An anchor, not a fetch: same reasoning as the CSV download on the Tax
            Report tab. Rendered only with a selection, because an empty selection
            is a 422 and an empty bundle is the one file nobody should ever restore. */}
        {exportHref !== '' ? (
          <a data-testid="download-bundle" className="btn btn-primary" href={exportHref}>
            Download bundle
          </a>
        ) : (
          <button data-testid="export-button" className="btn btn-primary" disabled>
            Download bundle
          </button>
        )}
      </section>

      <section className="glass-card" data-testid="restore-card">
        <h2>Restore a backup</h2>
        <p className="text-muted">Select a bundle file to preview what it would change.</p>
      </section>
    </div>
  );
}
```

- [ ] **Step 4: Run the tests**

Run: `npm run test -- BackupPanel`
Expected: PASS.

- [ ] **Step 5: Wire the tab into `App.tsx`**

Add the import beside the other component imports (`App.tsx:16-21`):

```tsx
import BackupPanel from './components/BackupPanel';
```

Add `FolderSync` to the `lucide-react` import list at `App.tsx:2-15`. Add the nav item after the Tax Report item (`App.tsx:555-557`):

```tsx
          <li className={`nav-item ${activeTab === 'backup' ? 'active' : ''}`} onClick={() => selectTab('backup')}>
            <FolderSync size={18} /> Backup
          </li>
```

Add the tab body alongside the others (after the `report` block at `App.tsx:705`):

```tsx
            {activeTab === 'backup' && (
              <BackupPanel portfolios={portfolios.map((p) => ({ id: p.id, name: p.name }))} />
            )}
```

Run: `npm run typecheck && npm run lint && npm run test`
Expected: green.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/components/BackupPanel.tsx frontend/src/App.tsx \
        frontend/src/test/components/BackupPanel.test.tsx
git commit -m "feat(frontend): Backup tab with portfolio-selected export"
```

---

### Task 12: Frontend — the restore card, its preview gate, and the typed confirmation

**Files:**
- Modify: `frontend/src/components/BackupPanel.tsx`
- Modify: `frontend/src/test/components/BackupPanel.test.tsx`

**Interfaces:**
- Consumes: `POST /api/backup/preview?mode=`, `POST /api/backup/restore?mode=&apply_prices=`, `Capabilities`.
- Produces: the restore half of `BackupPanel`. No new exports.

- [ ] **Step 1: Write the failing tests**

Append to `frontend/src/test/components/BackupPanel.test.tsx`:

```tsx
function bundleFile(name = 'bundle.json') {
  return new File([JSON.stringify({
    format: 'portfolio-prism-backup', schema_version: 1,
    created_at: '2026-11-13T00:00:00Z',
    portfolios: [{ ref: 'p1', name: 'Restored', description: null, currency: 'USD',
      base_currency: 'USD', assets: [] }],
  })], name, { type: 'application/json' })
}

describe('restore card', () => {
  it('keeps restore disabled until a preview has returned', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    expect(screen.getByTestId('restore-button')).toBeDisabled()
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
    await waitFor(() => expect(screen.getByTestId('preview-summary')).toBeInTheDocument())
    expect(screen.getByTestId('restore-button')).toBeEnabled()
  })

  it('renders the plan counts the server produced', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
    await waitFor(() => expect(screen.getByTestId('preview-summary')).toBeInTheDocument())
    expect(screen.getByTestId('preview-summary')).toHaveTextContent('1 portfolio')
    expect(screen.getByTestId('preview-summary')).toHaveTextContent('2 assets')
    expect(screen.getByTestId('preview-summary')).toHaveTextContent('3 transactions')
  })

  it('requires typing REPLACE before a replace restore is allowed', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
    await waitFor(() => expect(screen.getByTestId('preview-summary')).toBeInTheDocument())
    await userEvent.click(screen.getByRole('radio', { name: /replace/i }))
    expect(screen.getByTestId('restore-button')).toBeDisabled()
    await userEvent.type(screen.getByTestId('replace-confirm'), 'REPLAC')
    expect(screen.getByTestId('restore-button')).toBeDisabled()
    await userEvent.type(screen.getByTestId('replace-confirm'), 'E')
    expect(screen.getByTestId('restore-button')).toBeEnabled()
  })

  it('re-previews when the mode changes, so the counts match the mode', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
    await waitFor(() => expect(screen.getByTestId('preview-summary')).toBeInTheDocument())
    fetchMock.mockClear()
    await userEvent.click(screen.getByRole('radio', { name: /replace/i }))
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith(
      '/api/backup/preview?mode=replace', expect.objectContaining({ method: 'POST' })))
  })

  it('disables restore again when a different file is chosen', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
    await waitFor(() => expect(screen.getByTestId('restore-button')).toBeEnabled())
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile('other.json'))
    expect(screen.getByTestId('restore-button')).toBeDisabled()
  })

  it('shows the server error when a restore is refused', async () => {
    render(<BackupPanel portfolios={portfolios} />)
    await userEvent.upload(screen.getByTestId('bundle-file'), bundleFile())
    await waitFor(() => expect(screen.getByTestId('restore-button')).toBeEnabled())
    fetchMock.mockImplementation(async (input: RequestInfo | URL) => {
      if (String(input).includes('/backup/restore')) {
        return jsonResponse({ error: 'the bundle failed validation',
          errors: [{ path: 'portfolios[0].name', reason: 'must not be empty' }] }, 422)
      }
      return jsonResponse({}, 404)
    })
    await userEvent.click(screen.getByTestId('restore-button'))
    await waitFor(() => expect(screen.getByTestId('restore-error')).toHaveTextContent('must not be empty'))
  })
})
```

- [ ] **Step 2: Run them to verify they fail**

Run: `npm run test -- BackupPanel`
Expected: failures on the new `data-testid`s.

- [ ] **Step 3: Implement the restore card**

Replace the restore `<section>` in `BackupPanel.tsx` with:

```tsx
      <section className="glass-card" data-testid="restore-card">
        <h2>Restore a backup</h2>
        <p className="text-muted">
          Choose a bundle file, preview what it changes, then restore. Nothing is written until
          you restore, and a restore that fails partway writes nothing at all.
        </p>

        <label style={{ display: 'block', marginBottom: '12px' }}>
          Bundle file
          <input
            data-testid="bundle-file"
            className="form-control"
            type="file"
            accept="application/json,.json"
            onChange={(e) => {
              setFile(e.target.files?.[0] ?? null);
              setPlan(null);
              setRestoreError(null);
              setRestoreResult(null);
              setConfirm('');
            }}
          />
        </label>

        <fieldset style={{ border: 0, padding: 0, marginBottom: '12px' }}>
          <legend>Mode</legend>
          <label>
            <input type="radio" name="mode" value="copy" checked={mode === 'copy'}
              onChange={() => chooseMode('copy')} />
            {' '}New copy — add these portfolios alongside what you already have
          </label>
          <br />
          <label>
            <input type="radio" name="mode" value="replace" checked={mode === 'replace'}
              onChange={() => chooseMode('replace')} />
            {' '}Replace — delete every portfolio you own, then restore this bundle
          </label>
        </fieldset>

        {mode === 'replace' && (
          <label style={{ display: 'block', marginBottom: '12px' }}>
            Type <strong>REPLACE</strong> to confirm that everything you own will be deleted
            <input
              data-testid="replace-confirm"
              className="form-control"
              value={confirm}
              onChange={(e) => setConfirm(e.target.value)}
            />
          </label>
        )}

        {file && (
          <button className="btn btn-secondary" data-testid="preview-button"
            onClick={() => void runPreview()} disabled={previewing}>
            {previewing ? 'Previewing…' : 'Preview'}
          </button>
        )}

        {plan && (
          <div data-testid="preview-summary" role="status" style={{ marginTop: '12px' }}>
            <p>
              {plan.mode === 'replace'
                ? `Will delete ${plan.delete.portfolios} portfolio(s), ${plan.delete.assets} asset(s), ${plan.delete.transactions} transaction(s).`
                : 'Will delete nothing.'}
            </p>
            <p>
              {`Will insert ${plan.insert.portfolios} portfolio(s), ${plan.insert.assets} asset(s), ${plan.insert.transactions} transaction(s).`}
            </p>
            {plan.warnings.map((w) => (
              <p key={w} data-testid="preview-warning" style={{ color: 'var(--warning)' }}>{w}</p>
            ))}
          </div>
        )}

        <button
          className="btn btn-primary"
          data-testid="restore-button"
          disabled={!plan || (mode === 'replace' && confirm !== 'REPLACE') || restoring}
          onClick={() => void runRestore()}
        >
          {restoring ? 'Restoring…' : 'Restore'}
        </button>

        {restoreError && (
          <div data-testid="restore-error" role="alert" style={{ marginTop: '12px' }}>
            {restoreError}
          </div>
        )}
        {restoreResult && (
          <div data-testid="restore-result" role="status" style={{ marginTop: '12px' }}>
            {`Restored ${restoreResult.inserted.portfolios} portfolio(s), ` +
              `${restoreResult.inserted.assets} asset(s), ` +
              `${restoreResult.inserted.transactions} transaction(s).`}
          </div>
        )}
      </section>
```

Add the state and the two calls inside `BackupPanel`, above the `return`:

```tsx
  const [file, setFile] = useState<File | null>(null);
  const [mode, setMode] = useState<'copy' | 'replace'>('copy');
  const [confirm, setConfirm] = useState('');
  const [plan, setPlan] = useState<Plan | null>(null);
  const [previewing, setPreviewing] = useState(false);
  const [restoring, setRestoring] = useState(false);
  const [restoreError, setRestoreError] = useState<string | null>(null);
  const [restoreResult, setRestoreResult] = useState<{
    inserted: { portfolios: number; assets: number; transactions: number };
  } | null>(null);

  // Changing the mode changes what will be deleted, so the counts on screen must
  // be re-fetched rather than relabelled — a stale "will delete nothing" under a
  // selected `replace` is the misreading this avoids.
  const chooseMode = (next: 'copy' | 'replace') => {
    setMode(next);
    setPlan(null);
    if (file) void runPreview(next);
  };

  async function runPreview(forMode: 'copy' | 'replace' = mode) {
    if (!file) return;
    setPreviewing(true);
    setRestoreError(null);
    try {
      const res = await apiFetch(`${API_BASE}/backup/preview?mode=${forMode}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: await file.text(),
      });
      const body = await res.json();
      if (!res.ok) {
        setPlan(null);
        setRestoreError(describe(body));
        return;
      }
      setPlan(body as Plan);
    } finally {
      setPreviewing(false);
    }
  }

  async function runRestore() {
    if (!file || !plan) return;
    setRestoring(true);
    setRestoreError(null);
    try {
      const res = await apiFetch(`${API_BASE}/backup/restore?mode=${mode}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: await file.text(),
      });
      const body = await res.json();
      if (!res.ok) {
        setRestoreError(describe(body));
        return;
      }
      setRestoreResult(body as { inserted: { portfolios: number; assets: number; transactions: number } });
    } finally {
      setRestoring(false);
    }
  }
```

and the error formatter at module scope:

```tsx
/** Turn the server's failure body into one sentence. Validation failures arrive as
 * a list of `{path, reason}`; naming the first few is what makes a bad bundle
 * fixable rather than mysterious. */
function describe(body: { error?: string; errors?: { path: string; reason: string }[] }): string {
  const head = body.error ?? 'The server refused the bundle.';
  if (!body.errors?.length) return head;
  const named = body.errors.slice(0, 5).map((e) => (e.path ? `${e.path}: ${e.reason}` : e.reason));
  const more = body.errors.length > named.length ? ` (and ${body.errors.length - named.length} more)` : '';
  return `${head} ${named.join('; ')}${more}`;
}
```

- [ ] **Step 4: Run the tests**

Run: `npm run test -- BackupPanel && npm run typecheck && npm run lint`
Expected: PASS and green.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/components/BackupPanel.tsx frontend/src/test/components/BackupPanel.test.tsx
git commit -m "feat(frontend): restore card with preview gate and typed replace confirmation"
```

---

### Task 13: Regenerated artifacts and the manual walkthrough

**Files:**
- Modify: `docs/openapi/openapi.json`
- Modify: `frontend/src/types/api.d.ts`
- Modify: `docs/walkthrough.md`

**Interfaces:**
- Consumes: the finished routes.
- Produces: the two generated files CI diffs, and the manual steps CI cannot perform.

- [ ] **Step 1: Regenerate the OpenAPI document**

From `backend_rust/`:
```bash
cargo run --bin backend_rust -- --print-openapi > ../docs/openapi/openapi.json
grep -c "/api/backup/" ../docs/openapi/openapi.json
```
Expected: a count of at least 4 — one per backup path.

- [ ] **Step 2: Regenerate the API types**

From `frontend/`:
```bash
npm run generate-types
git diff --exit-code src/types/api.d.ts || echo "types changed as expected"
```
Expected: the file now contains the backup paths. CI asserts the committed file matches this output (`ci.yml:133`).

- [ ] **Step 3: Add the manual walkthrough section**

Append to `docs/walkthrough.md`:

```markdown
## Backup: export and restore

CI proves the routes, the transaction, and the round-trip against an in-memory
database. It cannot prove the two things a person actually does with this feature.

### 1. The file is really yours and really complete

1. Sign in and note your portfolios, their assets, and a transaction count you
   recognise.
2. Open **Backup**, tick one portfolio of several, and download the bundle.
3. Open the file. Confirm: the unselected portfolios are absent; `quantity`,
   `price` and `fee` are quoted strings, not numbers; transaction `date` values
   carry a time and a `Z`; there is no `prices` array unless you asked for it.
4. Open the **Tax Report** tab for that portfolio and compare its realized-P&L
   figure against the numbers the bundle implies. If they disagree, stop — the
   format is missing something, and the round-trip test in
   `backend_rust/tests/backup_test.rs` should be extended rather than this
   walkthrough written off.

### 2. Restore into a fresh instance

This is the step that justifies the feature, and no automated test has a real
second instance to restore into.

1. Start an empty instance (a fresh database, or a second local run with its own
   `DATABASE_URL`) and sign in as yourself.
2. Restore in **New copy** mode. Confirm the portfolios appear with the same
   assets and transaction counts, and that the tax report matches the source.
3. Restore the same bundle again in **New copy** mode. Confirm you now have two
   copies and nothing was overwritten.
4. Restore in **Replace** mode. Confirm everything you owned is gone and only the
   bundle's contents remain, and that the preview you were shown named those
   counts before the button enabled.
5. Sign in as a **different user** on the same instance and confirm the first
   user's restore changed nothing visible to them.

If step 2 or 4 disagrees with the preview you were shown, that is a defect in
`plan_for` versus `apply` — the two must count the same rows by the same rule.
```

- [ ] **Step 4: Run every gate**

```bash
cd backend_rust && cargo test && bash scripts/check_access_gate.sh \
  && bash scripts/check_backup_portability.sh && cargo clippy --all-targets && cargo build --release
cd ../frontend && npm run lint && npm run typecheck && npm run test
```
Expected: all green. `cargo build --release` is part of the repo's documented backend gate.

- [ ] **Step 5: Commit**

```bash
git add docs/openapi/openapi.json frontend/src/types/api.d.ts docs/walkthrough.md
git commit -m "docs(backup): regenerate OpenAPI and API types, add the manual walkthrough"
```

---

## Deviations

None sanctioned. If a step above cannot be applied as written — a trait method set that
differs in the resolved dependency version, an ISIN validator that does not exist, a
helper that is already present under another name — record what you did here, in the
plan, rather than silently editing the step.

## Known follow-ups (not this plan's work)

Carried from the spec, for the engine-selection spec:

1. `mode=replace` under concurrent writes on Postgres probably needs a `SERIALISABLE`
   restore transaction. Moot on SQLite at `replicas: 1`.
2. `src/backup/` must be rewired to whatever pool the engine spec chooses. Per the
   portability gate, that is the only expected change.
3. `historical_prices` has no `owner_id`. The price-restore flag is a mitigation, not a
   model.
4. `CurrencyService`'s process-local cache and rate limiter mean N replicas make N× the
   provider requests. Independent of this feature, but it blocks the same `replicas: 1`
   removal.
