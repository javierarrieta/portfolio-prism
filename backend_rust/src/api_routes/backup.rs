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
//! `// access-gate: global-config (no caller-supplied id)` on the line directly
//! above its route attribute. That waives Rule A's `require*` half for that one
//! handler and no other; Rules B and C still apply to it, and the gate prints what
//! it waived.
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
/// gets forgotten. `HeaderMap` is Rocket 0.5's name for the header collection.
pub type BackupError = (Status, HeaderMap<'static>, Json<serde_json::Value>);

/// A failure with no extra headers.
pub fn fail(status: Status, message: impl Into<String>) -> BackupError {
    (
        status,
        HeaderMap::new(),
        Json(serde_json::json!({ "error": message.into() })),
    )
}

/// A lock is a `503` the client may retry, and `Retry-After` says so out loud
/// rather than leaving the client to guess whether the failure was permanent.
pub fn lock_response() -> BackupError {
    let mut headers = HeaderMap::new();
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
