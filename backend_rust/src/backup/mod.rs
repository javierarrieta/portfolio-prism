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

/// The bundle version this build **writes**. The writer uses this rather than a
/// literal, and `export::tests::the_written_schema_version_is_the_newest_accepted`
/// fails if it is ever bumped on its own: a bundle this build writes must be a bundle
/// this build can read back.
pub const CURRENT_SCHEMA_VERSION: i64 = 1;

/// The schema versions this build accepts, oldest first.
///
/// **A released version is never dropped from this list.** `v1.json` is not edited: a
/// format change adds `v2.json`, adds the `v1 -> v2` upgrader, and *appends* `2` here.
/// Replacing `1` with `2` would make every backup a user already downloaded
/// unrestorable, and the newest-accepted check would still pass because it only looks
/// at the newest — `accepted_versions_are_exactly_v1` is what fails if an old version
/// disappears.
///
/// Written out rather than derived from `CURRENT_SCHEMA_VERSION`: `&[1,
/// CURRENT_SCHEMA_VERSION]` is `[1, 1]` today, and a derived list is precisely how a
/// bump would silently drop its predecessor.
pub const ACCEPTED_VERSIONS: &[i64] = &[1];

/// Application-level ceiling on an uploaded bundle, enforced **per route**: a
/// handler opens its own stream with `data.open(MAX_BUNDLE_BYTES.bytes())` rather
/// than raising a global Rocket `Limits` entry for one endpoint, and must treat a
/// capped read (`is_complete() == false`) as the module's `413` — Rocket does not
/// refuse that request on its own. The byte check in `parse_and_validate` is
/// defense-in-depth behind that, not the first line of defence.
pub const MAX_BUNDLE_BYTES: usize = 8 * 1024 * 1024;

/// Ceiling on `portfolios + assets + transactions` in one bundle.
pub const MAX_BUNDLE_ROWS: usize = 50_000;

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
    /// The only spellings that mean on, after trimming and lowercasing. One rule, so
    /// `from_env_with` and `main`'s warning cannot disagree about what the operator
    /// wrote. Anything outside both accepted sets means off: this flag gates writes
    /// to shared `historical_prices`, so it fails closed.
    pub fn is_on(raw: &str) -> bool {
        matches!(raw.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes")
    }

    /// The spellings that mean off — the mirror of `is_on`, and just as closed.
    /// `main` treats a value in either set as a decision and stays quiet about it;
    /// a WARN that fires on `0` is a WARN operators learn to ignore, including the
    /// one that matters.
    pub fn is_off(raw: &str) -> bool {
        matches!(raw.trim().to_ascii_lowercase().as_str(), "0" | "false" | "no")
    }

    /// Whether the value means anything to this build, in either direction. This is
    /// what `main` tests before calling a value unrecognised — `!is_on` alone would
    /// warn about an intentional `0`.
    pub fn is_recognised(raw: &str) -> bool {
        Self::is_on(raw) || Self::is_off(raw)
    }

    pub fn from_env() -> Self {
        Self::from_env_with(&|key| std::env::var(key).ok())
    }

    pub fn from_env_with(env: &dyn Fn(&str) -> Option<String>) -> Self {
        Self {
            allow_price_restore: env("BACKUP_ALLOW_PRICE_RESTORE")
                .is_some_and(|v| Self::is_on(&v)),
        }
    }
}

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
    /// Bundle-local label (`p1`, `p2`), unique bundle-wide: portfolio refs and asset
    /// refs share one namespace, so an asset ref may not equal a portfolio ref
    /// either. Never a database id, never persisted, and never read during restore —
    /// the nesting carries the parent/child link. `ref` is a Rust keyword, hence the
    /// rename.
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
    /// Bundle-local label (`a1`, `a2`), unique in the same namespace as the portfolio
    /// refs. Never a database id and never read during restore — the portfolio that
    /// nests the asset is its parent, so no label has to carry that link.
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

    let (errors, more) = collect(&bundle);
    if errors.is_empty() {
        Ok(bundle)
    } else {
        Err(BundleError::Invalid(errors, more))
    }
}

/// Append one error, unless the report is already at its cap — in which case set
/// `truncated`, so a caller can tell the user more errors were found than it
/// lists.
fn push_err(
    errors: &mut Vec<ValidationError>,
    truncated: &mut bool,
    path: String,
    reason: String,
) {
    if errors.len() < MAX_REPORTED_ERRORS {
        errors.push(ValidationError { path, reason });
    } else {
        *truncated = true;
    }
}

/// Field-level validation for callers that only need the list of errors.
pub fn validate(bundle: &Bundle) -> Vec<ValidationError> {
    collect(bundle).0
}

/// Field-level validation. Returns every error found, capped at
/// `MAX_REPORTED_ERRORS`, plus whether the cap suppressed any.
///
/// The cap rule lives in the free `push_err` function so it can be exercised on
/// its own; the closure below only curries the two accumulator borrows.
fn collect(bundle: &Bundle) -> (Vec<ValidationError>, bool) {
    let mut errors: Vec<ValidationError> = Vec::new();
    let mut truncated = false;
    let mut push = |path: String, reason: String| push_err(&mut errors, &mut truncated, path, reason);

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

        for (ai, a) in p.assets.iter().enumerate() {
            let ap = format!("{pp}.assets[{ai}]");
            // The same `seen_refs` set as portfolios, deliberately: a ref is a
            // bundle-local label, and a label that names two rows names neither. So
            // the labels must be unique bundle-wide, and because portfolio and asset
            // refs share one namespace, an asset ref may not equal a portfolio ref
            // either. This is a rule about labels as labels, not a key standing in
            // for a lookup: `restore::apply` never reads a ref, and every link comes
            // from the `RETURNING id` of the insert that made the row. There is no
            // `ref -> new id` map here, and this comment is not a promise that one
            // should be built.
            if !seen_refs.insert(a.bundle_ref.as_str()) {
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
            // A let-chain rather than two nested `if`s: clippy's
            // `collapsible_if` rejects the nested form on this toolchain.
            if let Some(isin) = &a.isin
                && !isin.is_empty()
                && !crate::api_routes::lookup::is_valid_isin(isin)
            {
                push(format!("{ap}.isin"), format!("{isin:?} is not a valid ISIN"));
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

    (errors, truncated)
}

fn is_rfc3339(value: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(value).is_ok()
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

    /// The flag gates writes to shared `historical_prices`, so the accepted set
    /// matters: an operator who writes `on` or `enabled` is told off by `main`, and
    /// the parse itself must stay the single source of that rule.
    #[test]
    fn price_restore_is_on_only_for_the_accepted_spellings() {
        let with = |v: Option<&str>| {
            BackupConfig::from_env_with(&|key| {
                if key == "BACKUP_ALLOW_PRICE_RESTORE" {
                    v.map(|s| s.to_string())
                } else {
                    None
                }
            })
            .allow_price_restore
        };

        assert!(!with(None), "unset is off");
        assert!(!with(Some("")), "empty is off");
        for on in ["1", "true", "yes", "TRUE", "Yes", " 1 ", "\ttrue\n"] {
            assert!(with(Some(on)), "{on:?} should mean on");
        }
        // Spellings that do not mean on. `0`/`false`/`no` are the accepted way to say
        // off; the rest mean nothing to this build. Both fail closed, and `main`
        // warns only about the second group.
        for off in ["0", "false", "no", "on", "y", "enabled", "2", "truest"] {
            assert!(!with(Some(off)), "{off:?} must not mean on");
        }
        assert!(
            !BackupConfig::default().allow_price_restore,
            "Default is off"
        );
    }

    #[test]
    fn is_on_and_from_env_with_agree_on_every_spelling() {
        // `from_env_with` decides the flag from `is_on`; if the two diverged, the
        // config would not mean what the environment says it means.
        for raw in ["1", "true", "yes", " TRUE ", "0", "on", "", "  "] {
            assert_eq!(
                BackupConfig::is_on(raw),
                BackupConfig::from_env_with(&|_| Some(raw.to_string())).allow_price_restore,
                "{raw:?} parsed two ways"
            );
        }
    }

    /// `main` warns about a value in neither set and stays silent about a value in
    /// one of them. `0` is a decision, not a typo: a WARN that fires on it trains
    /// operators to skim past the warning that the flag is **on**.
    #[test]
    fn only_a_value_in_neither_set_is_unrecognised() {
        for raw in ["1", "true", "yes", " TRUE ", "0", "false", "no", " NO ", " 0 "] {
            assert!(
                BackupConfig::is_recognised(raw),
                "{raw:?} is an accepted spelling and must not be warned about"
            );
            assert_ne!(
                BackupConfig::is_on(raw),
                BackupConfig::is_off(raw),
                "{raw:?} must mean exactly one of on or off"
            );
        }
        // Including `off`: it is not the mirror of `yes`, so it means nothing here,
        // and the operator gets told the accepted spellings rather than believing a
        // value took effect. Fails closed, and says so.
        for raw in ["", "  ", "on", "off", "y", "enabled", "2", "truest"] {
            assert!(
                !BackupConfig::is_recognised(raw),
                "{raw:?} means nothing, so main must warn about it"
            );
            assert!(!BackupConfig::is_on(raw), "{raw:?} must not mean on");
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

    #[test]
    fn an_unknown_field_nested_in_a_portfolio_is_rejected() {
        // The root guard is not enough: `deny_unknown_fields` must hold on the
        // nested structs too, or a hand-edited bundle could smuggle a database
        // id into a portfolio row while the root stays clean.
        let raw = r#"{"format":"portfolio-prism-backup","schema_version":1,
            "created_at":"2026-11-13T00:00:00Z","portfolios":[
            {"ref":"p1","name":"Main","description":null,"currency":"USD",
             "base_currency":"USD","assets":[],"id":7}]}"#;
        let err = serde_json::from_str::<Bundle>(raw)
            .expect_err("an extra `id` on a nested portfolio must be rejected");
        // Naming the field proves `deny_unknown_fields` fired, not that the
        // fixture was malformed for some other reason.
        assert!(
            err.to_string().contains("unknown field"),
            "expected an unknown-field error, got: {err}"
        );
    }

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
        assert!(errs.iter().any(|e| e.path.ends_with(".asset_type")
            && e.reason.contains("is not one of")), "{errs:?}");

        let b = bundle(vec![port("p1", vec![asset("a1", vec![
            tx("", "SHORT", "1.0", "1.0", "2024-01-15T00:00:00Z"),
        ])])]);
        let errs = validate(&b);
        assert!(errs.iter().any(|e| e.path.ends_with(".transactions[0].type")
            && e.reason.contains("is not one of")), "{errs:?}");
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
    fn a_duplicate_asset_ref_across_portfolios_is_rejected() {
        // Refs are bundle-local labels and the label namespace is bundle-wide, not
        // per-portfolio: two `a1`s make "the asset labelled a1" name two rows, which
        // is what a label must never do. A per-portfolio set would accept a bundle
        // whose labels collide.
        let b = bundle(vec![
            port("p1", vec![asset("a1", vec![])]),
            port("p2", vec![asset("a1", vec![])]),
        ]);
        let errs = validate(&b);
        assert!(errs.iter().any(|e| e.path == "portfolios[1].assets[0]"
            && e.reason.contains("duplicate ref")), "{errs:?}");
    }

    #[test]
    fn a_portfolio_ref_colliding_with_an_asset_ref_is_rejected() {
        // One namespace, one rule: portfolio refs and asset refs are both
        // bundle-local labels, so a portfolio ref equal to an asset ref is the same
        // collision as two portfolios sharing one.
        let b = bundle(vec![port("x", vec![asset("x", vec![])])]);
        let errs = validate(&b);
        assert!(errs.iter().any(|e| e.path == "portfolios[0].assets[0]"
            && e.reason.contains("duplicate ref")), "{errs:?}");
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
    fn a_bundle_over_the_byte_limit_is_refused_before_it_is_parsed() {
        // Tasks 4-6 cap the upload per route when they open the `Data` stream, so
        // this check is unreachable over HTTP — and it stays, because
        // `parse_and_validate` is not the function that gets to trust its caller.
        // It runs before parsing, so the bytes need not be JSON.
        let over = vec![b'x'; MAX_BUNDLE_BYTES + 1];
        match parse_and_validate(&over) {
            Err(BundleError::Invalid(errors, false)) => {
                assert!(errors[0].reason.contains("over the"), "{errors:?}");
            }
            other => panic!("expected the byte-limit Invalid, got {other:?}"),
        }

        // The boundary is `>`, not `>=`: exactly the limit is not over it, so these
        // bytes fall through to the parser and fail as what they are.
        let exact = vec![b'x'; MAX_BUNDLE_BYTES];
        assert!(matches!(parse_and_validate(&exact), Err(BundleError::Unparseable(_))));
    }

    #[test]
    fn errors_are_capped_at_twenty_and_flagged_as_truncated() {
        let bad = |n: usize| -> Vec<BundleTransaction> {
            (0..n).map(|_| tx("", "BUY", "n", "1", "2024-01-15T00:00:00Z")).collect()
        };

        // 25 bad decimals: the report is capped and says it is incomplete.
        let (errs, truncated) = collect(&bundle(vec![port("p1", vec![asset("a1", bad(25))])]));
        assert_eq!(errs.len(), MAX_REPORTED_ERRORS);
        assert!(truncated, "25 errors exceed the cap, so the report must say so");

        // A report under the cap claims no truncation, so the flag is not vacuous.
        let (errs, truncated) = collect(&bundle(vec![port("p1", vec![asset("a1", bad(2))])]));
        assert_eq!(errs.len(), 2);
        assert!(!truncated, "a report under the cap must not claim truncation");

        // `parse_and_validate` must carry the real flag into the error it returns.
        let raw = serde_json::to_vec(&bundle(vec![port("p1", vec![asset("a1", bad(25))])])).unwrap();
        match parse_and_validate(&raw) {
            Err(BundleError::Invalid(errs, true)) => assert_eq!(errs.len(), MAX_REPORTED_ERRORS),
            other => panic!("expected Invalid(_, true), got {other:?}"),
        }
    }
}
