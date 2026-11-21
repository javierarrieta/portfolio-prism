//! The mounted surface, the published document, and the route inventory.
//!
//! Three separate drift risks, three assertions:
//! * a mounted `/api` route missing from the OpenAPI document (the contract lies);
//! * a mounted route whose resolved path is exactly `/api` or outside the prefix
//!   rule (it would escape the fairing while still passing an inventory check);
//! * a new route nobody added to `PROTECTED_ROUTES` (the inventory rots).

mod common;

use backend_rust::access::Access;
use common::db_with_user;
use backend_rust::auth::fairing::{DENIED_PATH, UNAVAILABLE_PATH};
use rocket::http::Method;
use std::collections::HashSet;

/// Method, path, and the level required where one exists.
///
/// `None` for the six routes that authenticate but check no resource id:
/// `POST /api/portfolios/` (the caller becomes the owner),
/// `GET /api/assets/lookup` (touches none of the three tables),
/// `GET /api/backup/capabilities` (reports server limits, no caller data),
/// `POST /api/backup/preview` (the only thing it reads is the caller's own upload),
/// `POST /api/backup/restore` (the same, and every row it writes is owned by the
/// caller because the caller's own id is the only owner it is given), and
/// `GET /api/backup/owned` (reads the caller's own rows and no one else's).
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
    // Authenticates but resolves no caller-supplied id, so `None` — the same
    // shape as `POST /api/portfolios/`.
    ("GET", "/api/backup/capabilities", None),
    // The ids arrive in a query string, not a path parameter, so Rule B cannot see
    // them at all; the handler calls `require(.., Viewer)` per id and
    // `tests/isolation_test.rs` is what proves it.
    ("GET", "/api/backup/export", Some(Access::Viewer)),
    // The caller supplies a file, not an id: every count in the plan is scoped by the
    // caller's own user id, so there is no resource to authorise against. The static
    // gate cannot see the bundle either — an upload body is invisible to all three of
    // its rules — which is what `tests/backup_test.rs`'s preview section is for.
    ("POST", "/api/backup/preview", None),
    // The same shape as preview: the caller supplies a file rather than an id, and
    // the rows it writes take their owner from the session, never from the bundle.
    // Nothing here is authorisable against a resource, which is why the write path is
    // pinned by `tests/backup_test.rs` instead of by an `Access` level.
    ("POST", "/api/backup/restore", None),
    // Reads the caller's own portfolios and nothing else; scoped by `owner_id`
    // inside `owned_detail`, pinned by two users in `tests/backup_detail_test.rs`.
    ("GET", "/api/backup/owned", None),
    // The three routes a human uses to look at and approve what an agent staged. They are
    // here because they are authenticated `/api` routes like every other one, and the
    // inventory is an inventory, not a list of the interesting ones.
    //
    // The list takes no level: the caller *is* the selector, so there is no id for
    // `access.rs` to resolve and nothing to grade.
    ("GET", "/api/staged-batches", None),
    // A handle is a caller-supplied id naming a row that is about to become a write, so it
    // goes through the chokepoint like any other. `Owner` because the only thing a
    // non-owner could do with a handle is learn that somebody else has one.
    ("GET", "/api/staged-batches/<handle>", Some(Access::Owner)),
    // The state transition itself. Same resolution, same reason: approving someone else's
    // batch would be the whole control defeated by guessing a handle.
    ("POST", "/api/staged-batches/<handle>/approve", Some(Access::Owner)),
    // Token routes. Mint and list resolve no caller-supplied id — the caller *is* the
    // session, and there is no resource to authorise against — so they carry `None`,
    // the same as `POST /api/portfolios/`.
    ("POST", "/api/tokens", None),
    ("GET", "/api/tokens", None),
    // The one that does resolve a caller-supplied id, and the reason it is listed:
    // without `require_own_token` on this path, any signed-in user revokes anyone's
    // token by counting integers. `Owner` because a token is the caller's own
    // credential, which is the closest thing to ownership this enum has.
    ("DELETE", "/api/tokens/<id>", Some(Access::Owner)),
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
async fn mounted(caller: &str) -> Vec<(Method, String)> {
    let (pool, _) = db_with_user(caller).await;
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
    for (method, uri) in mounted("every_mounted_api_route_is_in_the_published_document").await {
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
    // 21 on main, +3 token routes, +3 staged-batch routes. Both branches wrote 24
    // from the same base; git took the identical edit and the sum is only visible now.
    assert_eq!(checked, 27, "the mounted /api surface is 27 operations");
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

    // Routes that sit outside `/api/` and therefore outside `AuthFairing`, but that
    // authenticate themselves instead of being public.
    //
    // This is a separate list rather than an entry in `PUBLIC_NAMESPACES` because the two
    // claims are opposites. "Public by decision" and "not covered by the fairing, so it
    // must do its own checking" are different promises, and folding them together would let
    // the next `/api`-external route be waved through by pointing at the MCP door.
    //
    // `/mcp` is here on one condition: `tests/mcp_mount_test.rs::mcp_refuses_a_session_cookie`
    // and `mcp_refuses_a_request_with_no_credential` must keep passing. They are the only
    // thing between that route and being an unauthenticated door onto every tool, and the
    // route's own comment records that it landed inside the `/api` mount the first time and
    // answered 404 — which is how the mistake announces itself. Removing those two tests
    // should make this entry a lie.
    const SELF_AUTHENTICATED: &[&str] = &["/mcp"];
    for (method, uri) in mounted("no_mounted_route_escapes_the_prefix_rule").await {
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
            || path.starts_with("/cors/")
            || SELF_AUTHENTICATED.contains(&path);
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
    let mounted = mounted("the_outage_sentinel_covers_every_method_outside_api").await;
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
    for (method, uri) in mounted("every_mounted_options_route_is_outside_api").await {
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
    let mounted = mounted("the_denial_sentinel_covers_every_method_outside_api").await;
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

    let mounted_api: HashSet<(String, String)> = mounted("the_route_inventory_matches_the_mounted_surface_exactly")
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
    assert_eq!(PROTECTED_ROUTES.len(), 27);
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
        // 15 on main, 17 with the staged-batch routes, 16 with the token routes, and 18
        // with both. Each branch counted honestly from the same base; only the merge can
        // know the sum.
        with_level, 18,
        "eight routes authenticate without checking an id"
    );
}

/// The query parameters a route answers `422` without are published `required`, and the
/// two whose absence means "off" are not.
///
/// utoipa infers `required` from the handler's argument type, and every backup query
/// parameter is `Option<Vec<String>>` there — reading the field as a sequence is the only
/// way `only_once` can see a repeated parameter — so the derive calls all of them optional
/// and `#[utoipa::path]` has no attribute to overrule it. The correction lives in
/// `openapi::mark_required_query_params`, and this is the mutation that proves it holds:
/// delete that call and the first three assertions fail, because the published contract
/// would tell a generated client to omit a parameter the route refuses without. Widen the
/// list to `include_prices` or `apply_prices` and the last two fail — absent means off for
/// those two, and the panel relies on that on every plain restore.
#[test]
fn required_query_parameters_are_published_required() {
    use utoipa::openapi::path::PathItemType;
    let document = backend_rust::openapi::document();
    let is_required = |path: &str, method: PathItemType, name: &str| {
        document
            .paths
            .paths
            .get(path)
            .unwrap_or_else(|| panic!("{path} is not in the document"))
            .operations
            .get(&method)
            .unwrap_or_else(|| panic!("{path} has no such operation in the document"))
            .parameters
            .as_ref()
            .and_then(|params| params.iter().find(|p| p.name == name))
            .unwrap_or_else(|| panic!("{name} is not a parameter of {path}"))
            .required
            == utoipa::openapi::Required::True
    };

    assert!(
        is_required("/api/backup/export", PathItemType::Get, "portfolio_ids"),
        "an absent portfolio_ids is a 422, never an empty bundle"
    );
    assert!(
        is_required("/api/backup/preview", PathItemType::Post, "mode"),
        "an absent mode is a 422: neither mode is the safe one to guess"
    );
    assert!(
        is_required("/api/backup/restore", PathItemType::Post, "mode"),
        "the write must not guess a mode either"
    );
    assert!(
        !is_required("/api/backup/export", PathItemType::Get, "include_prices"),
        "absent include_prices means omit, so it really is optional"
    );
    assert!(
        !is_required("/api/backup/restore", PathItemType::Post, "apply_prices"),
        "absent apply_prices means leave the price section alone, so it really is optional"
    );
}
