//! The backup routes: export, capabilities, preview, restore.
//!
//! No SQL here — that lives in `crate::backup`. Handlers here exist to authenticate,
//! authorise, translate a `BundleError` into a status, and hand off.
//!
//! Every handler takes `AuthedUser` because `scripts/check_access_gate.sh` Rule C
//! demands it of every route handler in this directory, including the ones that
//! resolve no caller-supplied id.
//!
//! A handler that resolves no caller-supplied id carries the marker
//! `// access-gate: global-config (no caller-supplied id)` immediately above the
//! route attribute — below `#[utoipa::path(...)]`, since the gate reads the line
//! directly above `#[get]`/`#[post]`. That waives Rule A's `require*` half for that
//! one handler and no other; the gate also refuses the marker if the route itself
//! takes a path parameter — any one, whatever it is called — Rules B and C still
//! apply, and every waiver is printed.
//!
//! An uploaded bundle is read through a `Data` guard and opened under
//! `MAX_BUNDLE_BYTES` **per route** — no global `Limits` entry is raised to serve
//! one endpoint — and a read that comes back `is_complete() == false` is this
//! module's `413`, in the same `{"error": ...}` shape as every other bundle
//! failure. Rocket does not make that conversion; the handler does. The probes in
//! `tests/backup_test.rs` pin the read pattern.
//!
//! Route paths carry the full `/backup/...` prefix because they are mounted on the
//! plain `/api` mount in `lib.rs`, alongside the transaction routes.

use rocket::http::{Header, HeaderMap, Status};
use rocket::request::Request;
use rocket::response::{self, Responder};
use rocket::serde::json::Json;
use rocket::State;
use serde::Serialize;
use chrono::Utc;
use sqlx::SqlitePool;

use crate::access::{require, Access};
use crate::auth::guard::AuthedUser;
use crate::backup::export::{self as backup_export, DbFailure};
use crate::backup::{BackupConfig, Bundle, ACCEPTED_VERSIONS, MAX_BUNDLE_BYTES, MAX_BUNDLE_ROWS};

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
/// gets forgotten. `HeaderMap` is Rocket 0.5's name for the header collection.
///
/// A struct with private fields rather than the `(Status, HeaderMap, Json)` tuple
/// this started life as: Rocket 0.5.1 implements `Responder` for `(Status, R)` and
/// `(ContentType, R)` only, so a three-element tuple is not a response, and the
/// orphan rule forbids adding the impl here. Private fields are the other half —
/// `fail` and `lock_response` are then the only ways to build one, so no handler
/// can hand-roll a `503` that forgot `Retry-After`.
pub struct BackupError {
    status: Status,
    headers: HeaderMap<'static>,
    body: Json<serde_json::Value>,
}

impl<'r> Responder<'r, 'static> for BackupError {
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'static> {
        let mut response = self.body.respond_to(req)?;
        response.set_status(self.status);
        // `HeaderMap` does not implement `IntoIterator` (rocket's own TODO), so
        // this is the explicit call rather than a `for` over the map.
        for header in self.headers.into_iter() {
            response.adjoin_header(header);
        }
        Ok(response)
    }
}

/// A failure with no extra headers.
pub fn fail(status: Status, message: impl Into<String>) -> BackupError {
    BackupError {
        status,
        headers: HeaderMap::new(),
        body: Json(serde_json::json!({ "error": message.into() })),
    }
}

/// A lock is a `503` the client may retry, and `Retry-After` says so out loud
/// rather than leaving the client to guess whether the failure was permanent.
pub fn lock_response() -> BackupError {
    let mut headers = HeaderMap::new();
    headers.add(Header::new("Retry-After", "2"));
    BackupError {
        status: Status::ServiceUnavailable,
        headers,
        body: Json(serde_json::json!({ "error": "the database is busy; try again in a moment" })),
    }
}

/// A bundle on its way out: the JSON body plus the two headers that make it a
/// download rather than a page. A struct for the same reason as `BackupError` —
/// Rocket has no `Responder` for a `(HeaderMap, Json)` tuple — and the derive turns
/// every field after the first into a response header.
#[derive(rocket::Responder)]
pub struct BundleDownload {
    bundle: Json<Bundle>,
    cache_control: Header<'static>,
    content_disposition: Header<'static>,
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
// access-gate: global-config (no caller-supplied id)
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

/// `?include_prices=` as a boolean, or `None` when the value means neither on nor
/// off — and `Some(false)` when the parameter is absent, because prices are opt-in.
///
/// Parsed as a string against the same accepted sets as the server flag rather than
/// left to rocket's `bool` form field, because rocket accepts neither `1` nor `0`
/// there — the spellings the spec and the UI use — and a value it cannot parse on an
/// `Option` field becomes `None` rather than an error
/// (`Option<T>::finalize` is `T::finalize(this).ok()`). With `Option<bool>` the
/// caller would get a 200 with the section they asked for missing, which is the one
/// failure mode the spec forbids for a flag of this kind: never a silent ignore.
fn parse_include_prices(raw: Option<&str>) -> Option<bool> {
    match raw {
        None => Some(false),
        Some(raw) if BackupConfig::is_on(raw) => Some(true),
        Some(raw) if BackupConfig::is_off(raw) => Some(false),
        Some(_) => None,
    }
}

#[utoipa::path(
    get,
    path = "/api/backup/export",
    params(
        ("portfolio_ids" = Vec<String>, Query, style = Form, explode = false, description = "Comma-separated portfolio ids to include; the parameter may appear only once"),
        ("include_prices" = Option<String>, Query, description = "Include the price-history section: 1/true/yes to include, 0/false/no to omit"),
    ),
    responses(
        (status = 200, description = "The bundle; see docs/backup-schema/v1.json for its shape", body = serde_json::Value),
        (status = 404, description = "A portfolio id is not yours", body = serde_json::Value),
        (status = 422, description = "No portfolio selected, `portfolio_ids` given more than once, an id that did not parse, or an `include_prices` value that means neither on nor off", body = serde_json::Value),
        (status = 503, description = "The database is busy", body = serde_json::Value),
    )
)]
#[get("/backup/export?<portfolio_ids>&<include_prices>")]
pub async fn export(
    user: AuthedUser,
    pool: &State<SqlitePool>,
    portfolio_ids: Option<Vec<String>>,
    include_prices: Option<String>,
) -> Result<BundleDownload, BackupError> {
    // Read as a sequence so that a repeated `?portfolio_ids=` is something this
    // handler can see and refuse: rocket collapses a duplicate `Option` field to
    // `None`, which would have answered "name at least one portfolio" to someone who
    // named two, and its lenient path would otherwise keep the first value and drop
    // the rest. One spelling, the comma-separated list the spec documents, is refused
    // over a half-read selection.
    let requested = match portfolio_ids.as_deref() {
        None | Some([]) => None,
        Some([only]) => Some(only.as_str()),
        Some(_) => {
            return Err(fail(
                Status::UnprocessableEntity,
                "portfolio_ids may appear only once, as a comma-separated list",
            ));
        }
    };

    let ids = match parse_ids(requested) {
        Some(ids) => ids,
        None => return Err(fail(Status::UnprocessableEntity,
            "portfolio_ids must name at least one portfolio, e.g. ?portfolio_ids=1,2")),
    };

    let include_prices = match parse_include_prices(include_prices.as_deref()) {
        Some(v) => v,
        None => return Err(fail(Status::UnprocessableEntity,
            "include_prices must be one of 1/true/yes or 0/false/no")),
    };

    // Every id, before any data query. The gate script's Rule B does not see
    // `WHERE portfolio_id = ?`, so this loop and tests/isolation_test.rs are what
    // make the scoping real.
    for id in &ids {
        require(pool.inner(), user.id, *id, Access::Viewer)
            .await
            .map_err(|status| fail(status, "portfolio not found"))?;
    }

    let bundle = backup_export::build(pool.inner(), &ids, include_prices)
        .await
        .map_err(|e| match e {
            // Neither arm can be driven from a local request: any database failure a
            // test could provoke would first deny `require`, which answers 500 from
            // `access.rs` before this call is reached. Both halves of the `Lock` path
            // are pinned where they can be — `build()` producing `DbFailure::Lock`
            // for an exhausted pool
            // (`tests/backup_test.rs::an_exhausted_pool_is_a_lock_not_an_outage`) and
            // the response a `Lock` becomes
            // (`a_lock_is_a_503_that_says_when_to_retry`).
            DbFailure::Lock => lock_response(),
            DbFailure::Other => fail(DbFailure::Other.status(),
                "could not read the portfolio data"),
        })?;

    // A bundle over the row ceiling is one restore will refuse, so it is refused here
    // too — see `backup_export::row_cap_error`.
    if let Some(reason) = backup_export::row_cap_error(&bundle) {
        return Err(fail(Status::UnprocessableEntity, reason));
    }

    Ok(BundleDownload {
        bundle: Json(bundle),
        cache_control: Header::new("Cache-Control", "no-store"),
        content_disposition: Header::new(
            "Content-Disposition",
            format!("attachment; filename=\"portfolio-prism-backup-{}.json\"", Utc::now().format("%Y-%m-%d")),
        ),
    })
}
