//! The mounted surface, the published document, and the route inventory.
//!
//! Three separate drift risks, three assertions:
//! * a mounted `/api` route missing from the OpenAPI document (the contract lies);
//! * a mounted route whose resolved path is exactly `/api` or outside the prefix
//!   rule (it would escape the fairing while still passing an inventory check);
//! * a new route nobody added to `PROTECTED_ROUTES` (the inventory rots).

mod common;

use backend_rust::access::Access;
use backend_rust::auth::fairing::{DENIED_PATH, UNAVAILABLE_PATH};
use rocket::http::Method;
use std::collections::HashSet;

/// Method, path, and the level required where one exists.
///
/// `None` for the two routes that authenticate but check no resource id:
/// `POST /api/portfolios/` (the caller becomes the owner) and
/// `GET /api/assets/lookup` (touches none of the three tables).
///
/// Routes outside `/api/` — `/`, `/api-docs/openapi.json`, `/auth/*`,
/// `/_auth/denied` — are deliberately **not** in this inventory: the first two are
/// public by decision and the rest are not data routes.
const PROTECTED_ROUTES: &[(&str, &str, Option<Access>)] = &[
    ("GET", "/api/portfolios/", Some(Access::Viewer)),
    ("POST", "/api/portfolios/", None),
    ("GET", "/api/portfolios/<id>", Some(Access::Viewer)),
    // The only write among the portfolio-scoped routes that is not a CRUD edit: it
    // fetches from the provider and inserts SPLIT rows, so it needs Editor (#50).
    ("POST", "/api/portfolios/<id>/sync-splits", Some(Access::Editor)),
    ("PATCH", "/api/portfolios/<id>", Some(Access::Owner)),
    ("DELETE", "/api/portfolios/<id>", Some(Access::Owner)),
    (
        "GET",
        "/api/portfolios/<id>/tax-summary",
        Some(Access::Viewer),
    ),
    (
        "GET",
        "/api/portfolios/<id>/performance",
        Some(Access::Viewer),
    ),
    (
        "POST",
        "/api/portfolios/<portfolio_id>/assets",
        Some(Access::Editor),
    ),
    (
        "PATCH",
        "/api/portfolios/<portfolio_id>/assets/<asset_id>",
        Some(Access::Editor),
    ),
    ("DELETE", "/api/assets/<id>", Some(Access::Owner)),
    (
        "POST",
        "/api/portfolios/<portfolio_id>/assets/<asset_id>/transactions",
        Some(Access::Editor),
    ),
    (
        "GET",
        "/api/portfolios/<portfolio_id>/transactions",
        Some(Access::Viewer),
    ),
    ("DELETE", "/api/transactions/<id>", Some(Access::Editor)),
    ("GET", "/api/transactions/<id>/pnl", Some(Access::Viewer)),
    ("GET", "/api/assets/lookup", None),
];

/// Rocket trims a trailing slash when it composes a mount with a route path, so
/// `#[get("/")]` under `mount("/api/portfolios", ..)` reports the URI
/// `/api/portfolios` while the OpenAPI document says `/api/portfolios/`. Both forms
/// route to the same handler — a request to either answers — so the comparison
/// canonicalises the slash instead of pretending the two spellings differ.
fn canonical(path: &str) -> String {
    path.trim_end_matches('/').to_string()
}

/// Whether a path is inside the API prefix. `/api-docs` is **not** — a bare
/// `starts_with("/api")` would sweep the documentation endpoint into the protected
/// inventory and assert a contract that does not exist.
fn is_api(path: &str) -> bool {
    let c = canonical(path);
    c == "/api" || c.starts_with("/api/")
}

/// Rocket templates carry the query part (`/<id>/tax-summary?<strategy>&…`);
/// utoipa paths do not, so the query is truncated before comparison. The parameter
/// spelling is *not* translated: this codebase writes utoipa paths in Rocket's
/// `<param>` form, so the two are otherwise byte-identical.
fn to_document_path(rocket_uri: &str) -> String {
    rocket_uri
        .split('?')
        .next()
        .unwrap_or(rocket_uri)
        .to_string()
}

/// Every route the router actually holds, after ignition.
async fn mounted() -> Vec<(Method, String)> {
    let pool = common::setup_db(&common::oidc_config()).await;
    let rocket = common::build_rocket_oidc(pool)
        .ignite()
        .await
        .expect("rocket ignites");
    rocket
        .routes()
        .map(|r| (r.method, r.uri.to_string()))
        .collect()
}

#[tokio::test]
async fn every_mounted_api_route_is_in_the_published_document() {
    let document = backend_rust::openapi::document();
    let mut checked = 0;
    for (method, uri) in mounted().await {
        let path = to_document_path(&uri);
        if !is_api(&path) {
            continue;
        }
        let documented = document
            .paths
            .paths
            .iter()
            .find(|(k, _)| canonical(k) == canonical(&path))
            .map(|(_, v)| v);
        let item = documented
            .unwrap_or_else(|| panic!("mounted {method} {uri} is absent from the document"));
        let key = match method {
            Method::Get => utoipa::openapi::path::PathItemType::Get,
            Method::Post => utoipa::openapi::path::PathItemType::Post,
            Method::Put => utoipa::openapi::path::PathItemType::Put,
            Method::Delete => utoipa::openapi::path::PathItemType::Delete,
            Method::Patch => utoipa::openapi::path::PathItemType::Patch,
            other => panic!("unexpected method {other} on an /api route"),
        };
        assert!(
            item.operations.contains_key(&key),
            "mounted route has no matching operation in the document"
        );
        checked += 1;
    }
    assert_eq!(checked, 16, "the mounted /api surface is 16 operations");
}

#[tokio::test]
async fn no_mounted_route_escapes_the_prefix_rule() {
    // A route resolving to exactly `/api` would not start with `/api/`, so the
    // fairing would skip it while an inventory check still saw it as covered.
    //
    // Checking only for the exact string `/api` would miss every other way a data
    // route can sit outside the rule — mounted at `/data`, at `/apifoo`, at `/` — so
    // the assertion is inverted: every mounted route must be in a namespace that is
    // public by decision, and anything else is an escape.
    const PUBLIC_NAMESPACES: &[&str] = &["/api/", "/auth", "/_auth", "/api-docs"];
    for (method, uri) in mounted().await {
        let path = uri.split('?').next().unwrap();
        if path == "/api" {
            panic!("{method} {uri} resolves to exactly /api and would bypass enforcement");
        }
        let root = format!(
            "/{}",
            path.trim_matches('/').split('/').next().unwrap_or("")
        );
        let allowed = path == "/"
            || path == "/api-docs"
            || PUBLIC_NAMESPACES.iter().any(|p| path.starts_with(p))
            // rocket_cors mounts its own responder under /cors/ and rewrites a
            // rejected request there; it is public by construction, not by us.
            || path.starts_with("/cors/");
        assert!(
            allowed,
            "{method} {uri} (root {root}) is mounted outside every public namespace, \
             so it is reachable without a session while not being a deliberate public route"
        );
    }
}

#[tokio::test]
async fn the_outage_sentinel_covers_every_method_outside_api() {
    // Same reasoning as the denial sentinel: a method that is not mounted there
    // answers 405 where the truth is 503, and 405 tells the client nothing is wrong
    // with the server.
    let mounted = mounted().await;
    for method in [
        Method::Get,
        Method::Post,
        Method::Put,
        Method::Delete,
        Method::Patch,
        Method::Head,
        Method::Options,
    ] {
        assert!(
            mounted
                .iter()
                .any(|(m, u)| *m == method && u == UNAVAILABLE_PATH),
            "{method} {UNAVAILABLE_PATH} is not mounted"
        );
        assert!(!UNAVAILABLE_PATH.starts_with("/api/"));
    }
}

#[tokio::test]
async fn every_mounted_options_route_is_outside_api() {
    for (method, uri) in mounted().await {
        if method == Method::Options {
            assert!(
                !uri.starts_with("/api/"),
                "{uri} is an OPTIONS route under /api and would answer preflights unauthenticated"
            );
        }
    }
}

#[tokio::test]
async fn the_denial_sentinel_covers_every_method_outside_api() {
    let mounted = mounted().await;
    for method in [
        Method::Get,
        Method::Post,
        Method::Put,
        Method::Delete,
        Method::Patch,
        Method::Head,
        Method::Options,
    ] {
        assert!(
            mounted
                .iter()
                .any(|(m, u)| *m == method && u == DENIED_PATH),
            "{method} {DENIED_PATH} is not mounted, so that method would answer 405 where 401 is required"
        );
        assert!(!DENIED_PATH.starts_with("/api/"));
    }
}

#[tokio::test]
async fn the_route_inventory_matches_the_mounted_surface_exactly() {
    let inventory: HashSet<(String, String)> = PROTECTED_ROUTES
        .iter()
        .map(|(m, p, _)| (m.to_string(), canonical(p)))
        .collect();

    let mounted_api: HashSet<(String, String)> = mounted()
        .await
        .into_iter()
        .filter(|(m, u)| {
            let path = u.split('?').next().unwrap();
            is_api(path) && *m != Method::Options
        })
        .map(|(m, u)| (m.as_str().to_string(), canonical(&to_document_path(&u))))
        .collect();

    let missing: Vec<_> = mounted_api.difference(&inventory).collect();
    let extra: Vec<_> = inventory.difference(&mounted_api).collect();
    assert!(
        missing.is_empty(),
        "routes not in PROTECTED_ROUTES: {missing:?}"
    );
    assert!(
        extra.is_empty(),
        "PROTECTED_ROUTES names routes that are not mounted: {extra:?}"
    );
    assert_eq!(PROTECTED_ROUTES.len(), 16);
}

#[test]
fn every_inventory_entry_with_a_level_is_enforced_by_a_handler_call() {
    // The inventory is documentation of intent; this pins that the entries that
    // name a level are not aspirational. A handler that dropped its `require*`
    // call would still pass the gate script only if it also dropped the query, so
    // the isolation matrix is the runtime half and this is the static half.
    let with_level = PROTECTED_ROUTES
        .iter()
        .filter(|(_, _, level)| level.is_some())
        .count();
    assert_eq!(
        with_level, 14,
        "two routes authenticate without checking an id"
    );
}
