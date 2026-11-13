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
//! apply, and every waiver is printed. `restore` resolves no caller-supplied id
//! either and deliberately carries **no** marker: the marker's cross-check fails
//! closed on any `<…>` in the route attribute, which includes a query parameter, so
//! marking it would fail the gate rather than pass it. It clears Rule A the way the
//! rest of this file does — through the `require` call `export` makes — and Rule C
//! by taking `AuthedUser`.
//!
//! An uploaded bundle is read through a `Data` guard and opened under
//! `MAX_BUNDLE_BYTES` **per route** — no global `Limits` entry is raised to serve
//! one endpoint — and a read that comes back `is_complete() == false` is this
//! module's `413`, in the same `{"error": ...}` shape as every other bundle
//! failure. Rocket does not make that conversion; the handler does. `preview` is
//! the first route to make it, and the probes in `tests/backup_test.rs` pin both
//! halves of the read pattern it uses.
//!
//! Route paths carry the full `/backup/...` prefix because they are mounted on the
//! plain `/api` mount in `lib.rs`, alongside the transaction routes.

use rocket::data::{Data, ToByteUnit};
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
use crate::backup::restore::{
    self as backup_restore, ApplyError, Mode, Plan, RestoreOutcome,
};
use crate::backup::{
    BackupConfig, Bundle, BundleError as BundleErrorKind, ACCEPTED_VERSIONS, MAX_BUNDLE_BYTES,
    MAX_BUNDLE_ROWS, parse_and_validate,
};

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

/// A query parameter that may appear **at most once**, or the refusal to answer.
///
/// Rocket cannot be trusted to see a duplicate: on an `Option` field two pushes
/// finalise to `ErrorKind::Duplicate` and `Option::finalize` swallows that into `None`
/// — so a repeated parameter arrives looking exactly like one that was never sent, or
/// like its first value. Reading the field as a sequence is what makes the duplicate
/// visible here; refusing it is what stops the caller being answered a question they did
/// not ask. Both query parameters on this route use this, because either one being
/// half-read changes what the caller gets back.
fn only_once<'a>(values: Option<&'a [String]>, refusal: &str) -> Result<Option<&'a str>, String> {
    match values {
        None | Some([]) => Ok(None),
        Some([only]) => Ok(Some(only)),
        Some(_) => Err(refusal.to_string()),
    }
}

/// A query parameter that means on or off — `?include_prices=`, `?apply_prices=` —
/// read as a boolean, or `None` when the value means neither on nor off. `Some(false)`
/// when the parameter is absent: both flags are opt-in.
///
/// Parsed as a string against the same accepted sets as the server flag rather than
/// left to rocket's `bool` form field, because rocket accepts neither `1` nor `0`
/// there — the spellings the spec and the UI use — and a value it cannot parse on an
/// `Option` field becomes `None` rather than an error
/// (`Option<T>::finalize` is `T::finalize(this).ok()`). With `Option<bool>` the
/// caller would get a 200 with the thing they asked for missing, which is the one
/// failure mode a flag of this kind must never have: never a silent ignore. One
/// function for both parameters so the two cannot drift apart on what `yes` means.
fn parse_query_flag(raw: Option<&str>) -> Option<bool> {
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
        ("include_prices" = Vec<String>, Query, style = Form, explode = false, description = "Include the price-history section: 1/true/yes to include, 0/false/no to omit; the parameter may appear only once, with one value"),
    ),
    responses(
        (status = 200, description = "The bundle; see docs/backup-schema/v1.json for its shape", body = serde_json::Value),
        (status = 404, description = "A portfolio id is not yours", body = serde_json::Value),
        (status = 422, description = "No portfolio selected; `portfolio_ids` or `include_prices` given more than once; an id that did not parse; an `include_prices` value that means neither on nor off; or a selection over the row or byte ceiling", body = serde_json::Value),
        (status = 503, description = "The database is busy", body = serde_json::Value),
    )
)]
#[get("/backup/export?<portfolio_ids>&<include_prices>")]
pub async fn export(
    user: AuthedUser,
    pool: &State<SqlitePool>,
    portfolio_ids: Option<Vec<String>>,
    include_prices: Option<Vec<String>>,
) -> Result<BundleDownload, BackupError> {
    // Both parameters are read as sequences and both must appear at most once; see
    // `only_once`. For `portfolio_ids` the alternative is answering "name at least one
    // portfolio" to someone who named two; for `include_prices` it is handing back a
    // bundle with no prices to someone who asked for them.
    let requested = only_once(
        portfolio_ids.as_deref(),
        "portfolio_ids may appear only once, as a comma-separated list",
    )
    .map_err(|reason| fail(Status::UnprocessableEntity, reason))?;
    let prices_requested = only_once(
        include_prices.as_deref(),
        "include_prices may appear only once",
    )
    .map_err(|reason| fail(Status::UnprocessableEntity, reason))?;

    let ids = match parse_ids(requested) {
        Some(ids) => ids,
        None => return Err(fail(Status::UnprocessableEntity,
            "portfolio_ids must name at least one portfolio, e.g. ?portfolio_ids=1,2")),
    };

    let include_prices = match parse_query_flag(prices_requested) {
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

    // A bundle over either ceiling is one restore will refuse, so it is refused here
    // too — see `backup_export::cap_error`.
    if let Some(reason) = backup_export::cap_error(&bundle) {
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

/// The one error body shape for a bundle failure, so a client parses validation
/// failures the same way whichever route produced them.
///
/// A struct literal, and a helper in this module, for the same reason as `fail`:
/// `BackupError`'s fields are private so that nothing outside this module can build
/// one, and they are visible here. `BundleError::status()` is `422` for every variant
/// — a bundle names no server-side resource, so nothing in it is a `404`.
///
/// The errors inside an `Invalid` stay structured (`path` and `reason` as separate
/// fields) rather than being joined into one string here: two of them — the byte cap
/// and the row cap — describe the whole bundle and so carry an empty path, and a
/// `format!("{path}: {reason}")` would render those two as a message starting with
/// `": "`. A client that wants one line can join them knowing which half is empty.
fn bundle_err(e: BundleErrorKind) -> BackupError {
    BackupError {
        status: e.status(),
        headers: HeaderMap::new(),
        body: Json(e.body()),
    }
}

/// An upload over `MAX_BUNDLE_BYTES`.
///
/// Rocket does not refuse a `Data` upload of its own accord: the handler opened the
/// stream under the cap and is handed the truncated body with `is_complete() ==
/// false`, so converting that into a refusal is entirely this module's business. It
/// is a `413` rather than a `422` because the file is not too wrong to read, it is too
/// big to hold — and the caller has to know which of the two to fix.
fn too_large() -> BackupError {
    fail(
        Status::PayloadTooLarge,
        format!("the upload is over the {MAX_BUNDLE_BYTES} byte limit"),
    )
}

#[utoipa::path(
    post,
    path = "/api/backup/preview",
    params(
        ("mode" = Vec<String>, Query, style = Form, explode = false, description = "copy or replace; the parameter may appear only once"),
    ),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "The plan: mode, delete counts, insert counts, prices, warnings", body = serde_json::Value),
        (status = 400, description = "The upload could not be read", body = serde_json::Value),
        (status = 413, description = "The upload is over `max_bytes`", body = serde_json::Value),
        (status = 422, description = "`mode` absent, repeated, or neither `copy` nor `replace`; an upload that is not JSON, not a bundle, from an unsupported `schema_version`, or that failed validation", body = serde_json::Value),
        (status = 503, description = "The database is busy", body = serde_json::Value),
    )
)]
#[post("/backup/preview?<mode>", data = "<body>")]
pub async fn preview(
    user: AuthedUser,
    pool: &State<SqlitePool>,
    mode: Option<Vec<String>>,
    body: Data<'_>,
) -> Result<Json<Plan>, BackupError> {
    // `mode` is read as a sequence and refused when repeated, for the reason
    // `only_once` documents: a repeated `Option` field reaches the handler as `None`,
    // which here is indistinguishable from a caller who named no mode at all — and the
    // two modes differ by "delete everything you own". An absent mode is a 422 too,
    // because there is no default: neither answer is the safe one to guess.
    let raw_mode = only_once(
        mode.as_deref(),
        "mode may appear only once, as either `copy` or `replace`",
    )
    .map_err(|reason| fail(Status::UnprocessableEntity, reason))?;
    let mode = Mode::from_query(raw_mode)
        .ok_or_else(|| fail(Status::UnprocessableEntity, "mode must be `copy` or `replace`"))?;

    // The cap is opened here, per route, rather than as a global `Limits` entry: see
    // the module comment and `no_global_body_limit_is_raised_for_the_bundle`.
    // A read failure is the body never arriving — rocket's own `Vec<u8>` guard answers
    // the same io failure with a `400`, and so does this.
    let uploaded = body
        .open(MAX_BUNDLE_BYTES.bytes())
        .into_bytes()
        .await
        .map_err(|_| fail(Status::BadRequest, "the upload could not be read"))?;
    if !uploaded.is_complete() {
        return Err(too_large());
    }

    // Everything that can reject a bundle without touching the database happens here,
    // before the first query: a preview is the user's last look before a write, so it
    // answers either the plan or the list of what is wrong, and never a partial of
    // either.
    let bundle = parse_and_validate(&uploaded.value).map_err(bundle_err)?;

    // Reads only. `plan_for` takes the caller's id, never an id from the bundle, so
    // what it counts is the caller's own state however the file is addressed.
    let plan = backup_restore::plan_for(pool.inner(), user.id, &bundle, mode)
        .await
        .map_err(|e| match e {
            DbFailure::Lock => lock_response(),
            DbFailure::Other => fail(
                DbFailure::Other.status(),
                "could not read the current data",
            ),
        })?;
    Ok(Json(plan))
}

// The write. Everything that makes a restore safe lives in `backup_restore::apply`;
// this handler's job is to refuse a bad request before a transaction is opened and to
// translate what came back.
//
// The order is deliberate: both query parameters, then the upload, then validation,
// then the transaction. A request that names no mode, or names it twice, or uploads a
// file that is not a bundle, never reaches the database at all — which is what makes
// `422` the honest answer for every refusal this route gives. The prose is a `//`
// comment rather than a `///` doc for the same reason as its siblings here: utoipa
// turns a doc comment into the operation's summary and description, and every other
// backup operation in the published document carries prose only.
#[utoipa::path(
    post,
    path = "/api/backup/restore",
    params(
        ("mode" = Vec<String>, Query, style = Form, explode = false, description = "copy or replace; the parameter may appear only once"),
        ("apply_prices" = Vec<String>, Query, style = Form, explode = false, description = "apply the bundle's price section: 1/true/yes to apply, 0/false/no to leave it alone; needs the server flag, and may appear only once"),
    ),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Restored: inserted counts, portfolio_ids, prices_inserted", body = serde_json::Value),
        (status = 400, description = "The upload could not be read", body = serde_json::Value),
        (status = 413, description = "The upload is over `max_bytes`", body = serde_json::Value),
        (status = 422, description = "`mode` or `apply_prices` absent, repeated, or unrecognised; an upload that is not JSON, not a bundle, from an unsupported `schema_version`, or that failed validation; a bundle carrying price history while price restore is disabled", body = serde_json::Value),
        (status = 500, description = "The write failed; the transaction was rolled back, so nothing was written", body = serde_json::Value),
        (status = 503, description = "The database is busy; nothing was written", body = serde_json::Value),
    )
)]
#[post("/backup/restore?<mode>&<apply_prices>", data = "<body>")]
pub async fn restore(
    user: AuthedUser,
    pool: &State<SqlitePool>,
    backup_cfg: &State<BackupConfig>,
    mode: Option<Vec<String>>,
    apply_prices: Option<Vec<String>>,
    body: Data<'_>,
) -> Result<Json<RestoreOutcome>, BackupError> {
    // Both parameters go through `only_once` for the reason `only_once` documents: a
    // repeated `Option` field reaches the handler as `None`, and here `None` means
    // something specific for each of them — "no mode" and "do not touch prices". The
    // two modes differ by "delete everything you own", so neither collapse is safe
    // to answer.
    let raw_mode = only_once(
        mode.as_deref(),
        "mode may appear only once, as either `copy` or `replace`",
    )
    .map_err(|reason| fail(Status::UnprocessableEntity, reason))?;
    let mode = Mode::from_query(raw_mode)
        .ok_or_else(|| fail(Status::UnprocessableEntity, "mode must be `copy` or `replace`"))?;

    let raw_prices = only_once(
        apply_prices.as_deref(),
        "apply_prices may appear only once",
    )
    .map_err(|reason| fail(Status::UnprocessableEntity, reason))?;
    let apply_prices = match parse_query_flag(raw_prices) {
        Some(v) => v,
        None => return Err(fail(Status::UnprocessableEntity,
            "apply_prices must be one of 1/true/yes or 0/false/no")),
    };

    // The same per-route capped read as `preview`: opened under `MAX_BUNDLE_BYTES`
    // here rather than by raising a global limit, and a truncated read is this
    // module's `413` because Rocket hands it over silently and would otherwise be
    // handed to validation as if it were the whole file.
    let uploaded = body
        .open(MAX_BUNDLE_BYTES.bytes())
        .into_bytes()
        .await
        .map_err(|_| fail(Status::BadRequest, "the upload could not be read"))?;
    if !uploaded.is_complete() {
        return Err(too_large());
    }

    // Nothing that can reject a bundle runs after this point's failure: an invalid
    // file is refused before the transaction opens, which is what makes "nothing was
    // written" true for every refusal this route can give.
    let bundle = parse_and_validate(&uploaded.value).map_err(bundle_err)?;

    // `user.id` is the only owner this write can have: every row inserted is scoped
    // to the caller and nothing in the bundle can name a row to write into, because
    // nothing in the bundle is read as an id.
    let outcome = backup_restore::apply(
        pool.inner(),
        user.id,
        &bundle,
        mode,
        apply_prices,
        backup_cfg.inner(),
    )
    .await
    .map_err(|e| match e {
        ApplyError::PricesDisabled => fail(
            Status::UnprocessableEntity,
            "this bundle contains price history and this server has price restore disabled",
        ),
        ApplyError::Db(DbFailure::Lock) => lock_response(),
        ApplyError::Db(DbFailure::Other) => fail(
            DbFailure::Other.status(),
            "restore failed; nothing was written",
        ),
    })?;

    Ok(Json(outcome))
}
