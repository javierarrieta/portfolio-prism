# OIDC Authentication & Per-User Data Isolation — Implementation Plan

> **For agentic workers:** Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add OIDC (authorization code + PKCE) authentication against the existing Authentik deployment, and enforce per-user data isolation so no caller can read or mutate a portfolio, asset, or transaction outside their own access.

**Architecture:** Three units. `src/auth/` owns the OIDC flow, the session cookie, the request-lifecycle enforcement and the `/auth/*` routes. `src/access.rs` (the **chokepoint**) is the only module permitted to resolve a portfolio, asset, or transaction by a caller-supplied id. `src/api_routes/*` holds business logic and never resolves ownership. Enforcement is a session cookie issued by the backend; the browser never sees a token.

**Tech Stack:** Rust (Rocket 0.5.1), SQLx 0.8 + SQLite, `openidconnect` 4.1.0, utoipa 4.2, React + Vite + TypeScript, GitHub Actions, Kubernetes.

**Cost:** 24 tasks, roughly 2–3 focused engineer-days. Backend auth + chokepoint (Tasks 1–13) is the critical path; the frontend and deployment tasks can run in parallel once Task 9 lands. No new infrastructure is required: Authentik is already operated.

**Spec:** `docs/superpowers/specs/2026-11-04-oidc-identity-isolation-design.md`. Requirement ids (`F1`–`F6`, `O1`–`O4`, `C1`–`C2`) refer to that document. This plan is the implementation of that spec; **§Deviations lists where the spec does not match the tree or Rocket 0.5.1, and what this plan does instead.** Read that section first. The design alternatives behind each choice are argued in spec §Decisions and §Alternatives — this plan does not restate them.

**Review history:** round 1 (`egm-implementation-readiness`, `egm-codebase-details`, `egm-design`) and round 2 (`egm-structural`, `egm-comprehension`, `egm-style`) of the Goldfish loop are consolidated in `docs/superpowers/reviews/2026-11-07-oidc-plan.md`. Deviations that came out of review are D-11 … D-17.

**References**

| Short name | What it is |
|---|---|
| PR #34 | The design spec this plan implements (`feat/oidc-identity-isolation`) |
| PR #44 | Merged: per-sell P&L breakdown, which added the 15th API operation (see D-4) |
| Issue #35 | Rate limiting `/api/assets/lookup` — deliberately not in this spec |
| Spec 2 / 3 / 4 | Deferred follow-ons: sharing, API keys + MCP, MCP OAuth 2.1. Listed under Out of scope |

**Where the crate sources live.** Citations like `rocket-0.5.1/src/fairing/mod.rs:504` point into the local Cargo registry at `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`. The author verified each claim there and against the working tree. Most deviations are compile failures or leaks; the rest are testability and hygiene constraints the spec could not see.

---

## Global Constraints

- **Prefix rule (stated once):** enforcement applies to a request whose path starts with `/api/` — with the trailing slash. `/api-docs` is therefore not covered, by design. Task 19 asserts no mounted route resolves to exactly `/api`, which would escape the rule.
- Every `/api/` request requires an authenticated identity; unauthenticated gets `401` with body `{"error":"unauthenticated"}` (F1).
- A resource id outside the caller's access is `404`, never `403` (F5).
- `GET /` and `GET /api-docs/openapi.json` stay public. The duplicate `GET /openapi.json` mount is removed.
- `AuthMode::Disabled` is compiled out of a release binary (`#[cfg(debug_assertions)]`). In a release build `AUTH_MODE=disabled` is a startup error, not a silent ignore.
- `cargo test` and `npm test` pass with no OIDC provider reachable and no secret key set (O2). `./start.sh` works with no OIDC configuration (O3).
- OpenAPI generation does not start a server (O4): `cargo run --bin backend_rust -- --print-openapi`.
- No `portfolio_members` table in this change. Every `access::require*` call names the minimum level it needs as a positional `Access` argument — the spec calls this argument `min` (C1).
- The guard accepts a session cookie only. An `Authorization` header on an API request is ignored, not accepted (C2).
- `email` never decides ownership. Legacy handover is keyed on `sub` via `BOOTSTRAP_OWNER_SUBJ` only.
- Foreign keys stay unenforced; `owner_id` stays a nullable `INTEGER`; `migrate_one_column` is not touched.
- Do not change `tests/decimal_migration_test.rs` — it pins `migrate_one_column`, is untouched by this plan, and must keep passing.
- **Baseline:** run `cd backend_rust && cargo test` before any edit and record the number. The reference run was 160 passed / 0 failed. Any new red is this change's fault.
- **Green rule:** every task leaves `cargo test` green, with one declared exception — Task 8 changes `init_db`/`build_rocket` signatures, Task 9 repairs `main.rs`/`openapi.rs`, and Task 10 repairs every test call site. **Tasks 8, 9 and 10 are one commit**; green is restored at the end of Task 10.

---

## Deviations from the spec (verified against the tree and Rocket 0.5.1)

### D-1 — Rocket 0.5 `Fairing::on_request` cannot halt a request

Spec §Architecture and §5 make the `AuthFairing` the thing that answers `401` before any handler runs. In Rocket 0.5.1 the callback is `async fn on_request(&self, _req: &mut Request<'_>, _data: &mut Data<'_>) {}` (`rocket-0.5.1/src/fairing/mod.rs:504`) — it returns `()`, so a fairing cannot produce a response. The spec's mechanism does not exist.

**This plan's mechanism.** Fairings run in `Rocket::preprocess_request` (`rocket-0.5.1/src/server.rs:244`), before `route()` (`server.rs:288`), and `Request::set_uri` is public (`request/request.rs:183`). So the fairing rewrites the request URI to an internal denial route:

1. `AuthFairing::on_request` — if the path starts with `/api/` and the request is not a CORS preflight, resolve the caller. On failure, log method + original path at `warn` (the path is lost after the rewrite) and `req.set_uri(Origin::parse("/_auth/denied").unwrap())`. The router then dispatches the denial route, so the real handler never runs and no side effect happens.
2. The denial route is mounted at `/_auth/denied` for **all seven** methods Rocket routes — `get`, `post`, `put`, `delete`, `patch`, `head`, `options` — returning `401` + `{"error":"unauthenticated"}`. Mounting all seven is what stops an unauthenticated `PUT` from answering `405` where F1 demands `401`, and stops a bare `OPTIONS /api/…` (not a preflight, so it *is* rewritten) from landing on a missing route and answering `405`. Being outside `/api` keeps it out of the OpenAPI surface and out of the route-coverage test.
3. Preflight exemption is narrow: skip only when the method is `OPTIONS` **and** an `Origin` header **and** an `Access-Control-Request-Method` header are both present. A bare `OPTIONS /api/...` is not a preflight and is rewritten to the sentinel, where the seventh handler answers `401`. This matters because `rocket_cors::catch_all_options_routes()` — the documented rocket_cors 0.6 preflight fix — would otherwise mount an unauthenticated OPTIONS handler on every `/api` path.
4. `AuthedUser: FromRequest` is still the handler-facing guard and returns the D-16 body; it is what gives handlers a typed caller. The fairing is what makes F1 true for a handler that forgets the guard.
5. The fairing reads managed state through the request: `req.rocket().state::<SqlitePool>()` and `state::<AuthMode>()` (`rocket-0.5.1/src/rkt.rs:819`, `request/request.rs:643`). `AuthFairing` is a unit struct; it owns no pool and no mode, so there is exactly one copy of each.

### D-2 — Rocket 0.5.1 has no request-local state

Spec §5 item 4 stashes `AuthedUser` in "the request's local state". `grep -rn local_state rocket-0.5.1/src/` returns nothing — `set_local_state`/`local_state` are Rocket 0.4 API. The spec anticipates this ("the guard resolves independently and the extra read is accepted").

**Consequence:** the fairing resolves the session to decide about the rewrite, and the guard resolves again for the handler. Two `users` reads per authenticated `/api/` request. Accepted: a local SQLite primary-key lookup, and the spec sanctions it. Do not replace the second read with a request-header marker: request headers are client-controlled, so a marker would be an authentication bypass. Comment that at the call site.

### D-3 — `rocket::config::SecretKey` exposes no key material

Spec §Testing names `auth::session::encode(&SecretKey, user_id, token_version)`. `rocket::config::SecretKey` (`rocket-0.5.1/src/config/secret_key.rs:78`) has `from`, `derive_from`, `generate`, `is_zero`, `is_provided` and no accessor for the bytes, so `encode(&SecretKey, …)` cannot be written.

**This plan's mechanism.** Session cookies are minted and read through Rocket's own private-cookie jar, which holds the key internally:

- production code: `session::issue(&CookieJar, uid, tv)` via `jar.add_private(...)` (`cookies.rs:347`), `session::read(&CookieJar)` via `jar.get_private(...)` (`cookies.rs:238`), `session::clear(&CookieJar)` via `jar.remove_private(Cookie::named(SESSION_COOKIE))` (`cookies.rs:433`). There is no `remove_named` in 0.5 — that is 0.4.
- tests mint through `LocalRequest::private_cookie(("pp_session", session::value(uid, tv, exp)))` (`rocket-0.5.1/src/local/request.rs:193`), which encrypts with the same instance's key.
- the pure, unit-testable parts are `session::value(uid, tv, exp) -> String` and `session::parse(&str) -> Option<SessionPayload>`.

### D-4 — The tree has 15 mounted operations; the published document has 14 today

The spec was written on a branch that predates PR #44.

| | spec said | tree today | after this change |
|---|---|---|---|
| Mounted `/api` handlers | 14 | **15** | 15 |
| Operations in `docs/openapi/openapi.json` | 13 | **14** | **15** |

The extra handler is `GET /api/transactions/<id>/pnl` (`analytics::get_transaction_sell_pnl`, `api_routes/analytics.rs:150-286`). It is already mounted and already registered in `ApiDoc::paths` (`openapi.rs:21`); the missing registration is `update_asset`, which takes the published count from 14 to 15.

The sell-pnl handler resolves transaction → asset → portfolio with three caller-id lookups (`analytics.rs:173,186,193`) — the pattern the chokepoint forbids. It gets `require_transaction(pool, user.id, id, Access::Viewer)`, a row in the route table, and an isolation-matrix case.

**The three lookups stay.** They are the response's data source, not authorization: `tx` supplies `type/price/fee/date`, `asset` supplies `symbol/asset_type/currency/portfolio_id`, `portfolio` supplies `base_currency`, and `SellPnlResponse` serializes all of it. **This is a general rule for every handler below:** `require*` answers *may this caller touch this id* and never returns the row, so handlers keep their own data reads.

### D-5 — `list_portfolios` leaks every other user's assets even after the portfolio filter

`GET /api/portfolios/` runs `SELECT * FROM portfolios` (`portfolios.rs:108`) **and** `SELECT * FROM assets` (`portfolios.rs:113`) with no filter, then groups assets by `portfolio_id`. Filtering only the portfolios query still returns other users' asset rows. Spec §8 says only "`WHERE owner_id = ?`".

**This plan:** both queries are scoped. Portfolios: `WHERE owner_id = ?`. Assets: `WHERE portfolio_id IN (SELECT id FROM portfolios WHERE owner_id = ?)`. `fetch_assets_for_portfolio` is already portfolio-scoped and needs no change.

### D-6 — `create_asset` inserts an unvalidated `portfolio_id` today

`POST /api/portfolios/<pid>/assets` (`transactions.rs:18-84`) never checks that `pid` exists; it inserts a row pointing at a nonexistent portfolio. `require(pool, user.id, portfolio_id, Access::Editor)` before the insert enforces isolation and turns that into a `404`. This **changes observable behavior** for a nonexistent portfolio id: previously `201` with a dangling asset, now `404`. Pin it with a test.

### D-7 — The CI grep gate as the spec drafts it is unsatisfiable

Spec §7 promises a CI grep failing on `FROM (portfolios|assets|transactions) WHERE id`. That pattern matches ~12 queries this plan must keep, because of D-4's rule: `portfolios.rs:158,196,204,236,257`, `transactions.rs:103,146,283`, `analytics.rs:27,113,173,186,193`. A gate that fails on the correct end state gets weakened or deleted in the PR that lands it — which is why the gate below is written to pass on the end state.

**This plan's gate** is a committed script, `backend_rust/scripts/check_access_gate.sh`, run by CI and runnable locally, with three satisfiable rules:

- **Rule A — every handler file goes through the chokepoint.** Every `.rs` file in `src/api_routes/` must contain at least one `access::require` call. Catches a whole file that skipped the chokepoint. `lookup.rs` is the one exception — it touches none of the three tables — and is listed in the script's explicit exemption array with its reason.
- **Rule B — every handler that resolves by caller id must have checked.** For each `#[get|post|put|patch|delete|head("…")]` attribute, take the function body up to the first line matching `^}`; if that body matches `(FROM|UPDATE|DELETE FROM)[[:space:]]+(portfolios|assets|transactions)[[:space:]].*WHERE[[:space:]]+id`, the body must also contain `access::require`. Sketch:

  ```sh
  awk '
    /^#\[(get|post|put|delete|patch|head)\(/ { infn=1; req=0; id=0 }
    infn {
      if ($0 ~ /access::require/) req=1
      if ($0 ~ /(FROM|UPDATE|DELETE FROM)[ \t]+(portfolios|assets|transactions)[ \t].*WHERE[ \t]+id/) id=1
      if ($0 ~ /^\}/) { if (id && !req) { print FILENAME ": unguarded id lookup"; f=1 } infn=0 }
    }
    END { exit f }' src/api_routes/*.rs
  ```

  Worked example: `delete_asset` contains `DELETE FROM assets WHERE id = ?` (`id=1`) and `access::require_asset` (`req=1`) → passes. The same query in a handler with no `require*` call → fails. Known limits, stated in the script: a `}` at column 0 inside a block comment or a string closes the block early, and a query assembled with `format!` is invisible. It is a heuristic and stays one — `analytics.rs:133`'s `WHERE asset_id IN (SELECT id FROM assets WHERE portfolio_id = ?)` already slips past it, which is why that handler runs `require(…, Viewer)` first and why the isolation matrix, not the grep, is the real gate.
- **Rule C — every route handler takes the caller.** Every function following a route attribute in `src/api_routes/` must name `AuthedUser` in its signature. This is the gate for the authenticated-but-unguarded case: an authenticated caller reaching a handler that omits the guard and queries by id is a cross-tenant read that the fairing cannot catch.

The script prints the number of handlers inspected and the number cleared, so a silently-empty gate is visible.

### D-8 — utoipa security scheme: build the document programmatically

Add the scheme and the `401` responses in one place rather than repeating derive attributes across 15 handlers: `crate::openapi::document()` post-processes `ApiDoc::openapi()` using `utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme}` (verified at `utoipa-4.2.3/src/openapi/security.rs:197-219`) and walks `doc.paths.paths` to insert a `401` response on every operation. One code path, so the handler and `--print-openapi` cannot disagree. **This function is the only name for the document** — `ApiDoc::document()` does not exist; `ApiDoc` carries only the derive-generated `openapi()`.

### D-9 — `k8s/secret.yaml` is not committed with values

Committing a `Secret` manifest invites a real credential into git. This plan adds `k8s/secret.example.yaml` (placeholders only) plus the `envFrom: secretRef` wiring, and the operator creates the live `Secret` with `kubectl create secret generic`. The deployment references a Secret named `backend-secrets` that must exist before rollout step 1.

### D-10 — `#[rocket::main]` and `--print-openapi`: no change required

`rocket_codegen::main` exists (`rocket_codegen-0.5.1/src/lib.rs:419`); the attributed function must be `async` and named `main`. Spec §3 stands as written.

### D-11 — The session cookie needs a server-checked expiry

Spec §Security says "a user deactivated in Authentik keeps access to this app until their cookie expires (≤ 8 hours)". A Rocket private cookie has no server-checked expiry: `Max-Age` is client-advertised only, so a stolen `pp_session` is valid indefinitely, and since nothing writes `is_active` in this spec there is no revocation path short of rotating `ROCKET_SECRET_KEY`. The spec's stated bound is false as written.

**This plan:** `SessionPayload` carries `exp: i64` (unix seconds, `now + 8 h` at issue) and `resolve_caller` rejects a past `exp`. `Max-Age` stays for the browser's benefit. It is the only server-side expiry check in this spec.

### D-12 — The placeholder row is created unconditionally

Spec §6 upserts the placeholder only when orphans exist, but `finalise_auth` needs `Disabled { as_user }` to name a real row. On a fresh dev database with zero portfolios that is a contradiction: no orphans → no placeholder → `finalise_auth` errors → `./start.sh` dies, violating O3.

**This plan:** `init_db` **always** upserts the placeholder row, orphan or no orphan. Consequences, all intended:

- `finalise_auth` always finds the row, so O3 holds on a fresh database.
- **Handover rule:** handover fires only when `BOOTSTRAP_OWNER_SUBJ == sub` **and** the placeholder row exists. It never fires on row existence alone.
- After a handover the placeholder row no longer exists (it is `UPDATE`d in place), so that `BOOTSTRAP_OWNER_SUBJ` value can never hand over again. A restart re-creates an empty placeholder that owns nothing; only rows that became orphans *after* the handover can be handed to whatever `BOOTSTRAP_OWNER_SUBJ` is then set to — which is what the variable means.

Pin placeholder existence in the no-orphans test case.

### D-13 — Configuration is read through an injectable source

`std::env::set_var` is `unsafe` in edition 2024 (`backend_rust/Cargo.toml:3`) and integration tests run multi-threaded, so `StartupConfig::from_env()` cannot be tested by mutating the environment. **This plan:** `StartupConfig::from_env_with(&dyn Fn(&str) -> Option<String>)` holds all the logic; `from_env()` is a thin wrapper that reads `std::env::var`. Tests exercise the injectable form. No `#[serial]` dependency, no process-global state.

### D-14 — `provision_or_update` takes primitives, not claims

A `Claims` type cannot be constructed in a test without a signed ID token from a live IdP, so the handover rules would be untestable. **This plan:** the network half and the database half are separate functions, and the database half takes primitives (full signature in Task 5). Every handover rule is then testable without an IdP.

### D-15 — `require*` distinguishes denial from database failure

Collapsing `sqlx::Error` into `NotFound` (or `resolve_caller` into `None` → `401`) turns a `SQLITE_BUSY` into a `404`/`401`, which defeats the spec's own goal of making denials distinguishable from genuine 500s. **This plan:** `resolve` propagates `sqlx::Error`; `require*` maps `Ok(None)` → `NotFound` and `Err(db)` → `InternalServerError`, logging the database error separately from the denial.

### D-16 — The unauthenticated response body is defined in one place

`/auth/me` sits outside `/api`, so the denial route never runs for it and a bare `Status::Unauthorized` would return Rocket's default body. **This plan:** the guard's rejection returns a typed responder carrying `{"error":"unauthenticated"}` — not a bare status — so the guard and the denial route cannot diverge.

### D-17 — `openidconnect` 4.1 does not work the way the spec's §1 sketch implies

Verified by compiling a probe program against `openidconnect` 4.1.0 / `oauth2` 5.0.0. Four things differ from the 3.x shape most examples show:

1. There is **no** `async_http_client` function. `oauth2` implements `AsyncHttpClient` for `reqwest::Client` (`oauth2-5.0.0/src/reqwest_client.rs:6`), so you pass a `reqwest::Client` reference as the HTTP client.
2. `Client::new` takes three arguments `(client_id, issuer, jwks)`; endpoints are set with `set_auth_uri` / `set_token_uri` / `set_client_secret` / `set_redirect_uri` (`openidconnect-4.1.0/src/client.rs:189`, `UPGRADE.md` §"Upgrading from 3.x to 4.x").
3. JWKS must be fetched explicitly: `CoreJsonWebKeySet::fetch_async(md.jwks_uri(), &http)`. `md.jwks_uri()` is infallible (returns `&JsonWebKeySetUrl`); `md.token_endpoint()` is an `Option`.
4. `authorize_url` takes three arguments `(AuthenticationFlow<CoreResponseType>::AuthorizationCode, CsrfToken::new_random, Nonce::new_random)` and `.url()` returns `(url, csrf, nonce)`. `PkceCodeChallenge::new_random_sha256()` returns a `(challenge, verifier)` tuple and the challenge is attached with `.set_pkce_challenge(...)`. `exchange_code(...)` returns the request directly, not a `Result`. ID token validation is `id_token.claims(&CoreIdTokenVerifier::new_confidential_client(client_id, client_secret, issuer, jwks), &nonce)`.

The compiling reference snippet is in Task 5. The spec's other §1 claims held up: `oauth2` 5.0 pulls `rand ^0.8`, and `Cargo.lock` already resolves `rand 0.8.6` alongside `rand 0.9.4`, so no `rand` conflict is introduced; and the default features are `reqwest` + `rustls-tls`, so no `native-tls` enters this deliberately rustls-only build (Task 1 checks both against the resolved tree, not the index).

### D-18 — `update_asset` keeps its inline `portfolio_id` predicate

Spec §8 says moving the cross-portfolio check into `require_asset_in` "removes it from the handler". **This plan keeps both.** The `UPDATE ... WHERE id = ?1 AND portfolio_id = ?2` predicate and the scoped re-select stay, because the guard and the write are two statements and the predicate is what makes the write itself incapable of crossing portfolios if the guard is ever refactored away. The gate script (D-7) clears the handler because it also calls `require*`. The cost is one redundant predicate; the benefit is that the invariant does not live in exactly one place.

---

## File Structure

| File | Action | Responsibility |
|---|---|---|
| `backend_rust/Cargo.toml` | Modify | `rocket` gains the `secrets` feature; add `openidconnect = "4.1"` |
| `backend_rust/src/auth/mod.rs` | Create | `AuthMode`, `AuthRequest`, `StartupConfig::from_env_with` (D-13), `OidcConfig`, `finalise_auth`, env refusals |
| `backend_rust/src/auth/session.rs` | Create | `pp_session` / `pp_oauth_state` shapes incl. `exp` (D-11), `value`/`parse`/`issue`/`read`/`clear` (D-3), `issue_state`/`read_state`/`clear_state` |
| `backend_rust/src/auth/guard.rs` | Create | `AuthedUser`, `FromRequest`, shared `resolve_caller` (D-11, D-15, D-16) |
| `backend_rust/src/auth/oidc.rs` | Create | lazy discovery, login URL, `exchange_code`/`validate_id_token`, `provision_or_update` (D-14), logout URLs |
| `backend_rust/src/auth/fairing.rs` | Create | `AuthFairing` — the URI-rewrite enforcement from D-1, state via `req.rocket().state()` |
| `backend_rust/src/auth/routes.rs` | Create | `/auth/login`, `/auth/callback`, `/auth/logout`, `/auth/logout-all`, `/auth/me`, and the `/_auth/denied` route (seven methods) |
| `backend_rust/src/access.rs` | Create | the chokepoint: `Access`, `require`, `require_asset`, `require_asset_in`, `require_transaction`, private `resolve`, plus `#[cfg(test)] mod tests` for the private cases |
| `backend_rust/src/lib.rs` | Modify | `init_db(pool, &StartupConfig) -> anyhow::Result<()>`, `users` table, `owner_id` column, unconditional placeholder + backfill (D-12), refusals; `build_rocket(+auth)`; mount `/auth` + fairing; drop the `/openapi.json` mount |
| `backend_rust/src/main.rs` | Modify | `#[rocket::main]`, `--print-openapi` as the first branch, `init_db` → `exit(1)`, `finalise_auth` |
| `backend_rust/src/openapi.rs` | Modify | register `update_asset`, add `document()` post-processor (D-8) |
| `backend_rust/src/api_routes/portfolios.rs` | Modify | guard all five routes; scope both queries in `list_portfolios` (D-5) |
| `backend_rust/src/api_routes/transactions.rs` | Modify | guard all six routes; nested path check moves into the chokepoint and stays in the handler too (D-6, D-18) |
| `backend_rust/src/api_routes/analytics.rs` | Modify | guard three routes including sell-pnl (D-4) |
| `backend_rust/src/api_routes/lookup.rs` | Modify | guard `lookup_isin` (auth only, no id) |
| `backend_rust/scripts/check_access_gate.sh` | Create | the three gate rules from D-7, run by CI and locally |
| `backend_rust/tests/common/mod.rs` | Modify | `build_rocket(pool, as_user)`, `setup_db(cfg)`, `seed_user`, `seed_portfolio(+owner)`, `oidc_cfg()`, `session_cookie(uid, tv)` |
| `backend_rust/tests/auth_test.rs` | Create | session round-trip, tamper, `exp`, `token_version`, `is_active`, `/auth/me`, F1 `401` under `Oidc`, public `/` and `/api-docs`, the D-1 side-effect test |
| `backend_rust/tests/isolation_test.rs` | Create | the cross-user matrix over every mounted route (D-4 included) |
| `backend_rust/tests/startup_test.rs` | Create | `init_db` refusals, backfill in both modes, placeholder reconciliation, `from_env_with` refusals |
| `backend_rust/tests/route_coverage_test.rs` | Create | `Rocket::routes()` ↔ `document()` paths, the `PROTECTED_ROUTES` route inventory, denial-route and prefix assertions |
| `backend_rust/tests/access_test.rs` | Create | public `require*` behavior and `Access` ordering (the private `resolve` cases live in `src/access.rs`) |
| `backend_rust/tests/{portfolios,transactions,analytics,sell_pnl,cors}_test.rs` | Modify | new `setup_db`/`build_rocket` signatures; owner on seeded portfolios. `tests/decimal_migration_test.rs` needs **no** change — it calls `migrate_one_column` directly and touches no changed helper |
| `frontend/src/utils/apiFetch.ts` | Create | `fetch` wrapper; `401` → redirect to `/auth/login`; forwards `signal` |
| `frontend/src/App.tsx` | Modify | 14 `fetch` calls → `apiFetch`; `?auth_error=auth_failed` banner + strip |
| `frontend/src/components/CsvImportModal.tsx` | Modify | 3 `fetch` calls → `apiFetch` |
| `frontend/src/components/PortfolioDetail.tsx` | Modify | 1 `fetch` call → `apiFetch` |
| `frontend/src/components/SellPnlModal.tsx` | Modify | 1 `fetch` call (`SellPnlModal.tsx:93`, passes an `AbortController` `signal`) → `apiFetch` |
| `frontend/src/test/mocks/handlers.ts` | Modify | add a `401` handler |
| `frontend/src/test/utils/apiFetch.test.ts` | Create | redirect on `401`; `signal` is forwarded |
| `frontend/src/test/components/AppAuthError.test.tsx` | Create | banner renders and the parameter is stripped |
| `frontend/nginx.conf` | Modify | `location /auth` proxying to `127.0.0.1:8000` with the same headers as `/api` |
| `frontend/vite.config.js` | Modify | add `'/auth'` to the proxy map |
| `k8s/secret.example.yaml` | Create | template with placeholders for `OIDC_CLIENT_SECRET`, `ROCKET_SECRET_KEY` (D-9) |
| `k8s/configmap.yaml` | Modify | `AUTH_MODE`, `OIDC_ISSUER`, `OIDC_CLIENT_ID`, `OIDC_REDIRECT_URI`, `BOOTSTRAP_OWNER_SUBJ` |
| `k8s/app-deployment.yaml` | Modify | `envFrom: secretRef`; delete the `wait-for-backend` initContainer (O1) |
| `.github/workflows/ci.yml` | Modify | `--print-openapi`; the `/auth` proxy check; the gate script; a `cargo build --release` job |
| `AGENTS.md` | Modify | OpenAPI regeneration command, in the same commit as the generation change |
| `start.sh` | Modify | export `AUTH_MODE=disabled` |
| `docs/openapi/openapi.json` | Regenerate | in Task 9, with `serde_json::to_string(&document())` |
| `frontend/src/types/api.d.ts` | Regenerate | in Task 20, via `npm run generate-types` |

---

## Tasks

### Task 1: Dependencies

**Files:** `backend_rust/Cargo.toml`

- [ ] Change `rocket = { version = "0.5", features = ["json"] }` to `features = ["json", "secrets"]`. Private cookies and `SecretKey` are `#[cfg(feature = "secrets")]`; without it nothing in `auth/session.rs` compiles.
- [ ] Add `openidconnect = "4.1"`.
- [ ] `cargo fetch` first, then `cargo build`. A fetch failure is not the compile-failure stop in the last bullet: `openidconnect` and `oauth2` are not in the local registry until this step, so an offline run fails at resolution, not at compilation. Distinguish the two before reporting.
- [ ] Run `cargo tree -d` and `cargo tree -e features -p openidconnect`; record both in the PR description. This is where the resolved tree is checked against the spec's `rand` and rustls claims (D-17).
- [ ] Confirm `reqwest` did not gain `native-tls`: `cargo tree -e features | grep -i native-tls` must be empty.
- [ ] Note for the Docker build: `backend_rust/Dockerfile` pre-builds dependencies in a cached layer keyed on `Cargo.toml`/`Cargo.lock`; both change, so that layer invalidates and the first build is slow. Not a failure.
- [ ] **Stop condition:** if `openidconnect` 4.1 resolves and then fails to compile, stop and report. Do not substitute a hand-rolled OIDC client; that is a spec change, not an implementation detail.

### Task 2: `src/auth/mod.rs` — configuration and mode

**Files:** Create `backend_rust/src/auth/mod.rs`

**Interfaces:** `AuthRequest` (the parsed `AUTH_MODE` value, known before a database exists), `StartupConfig { requested: AuthRequest, oidc: Option<OidcConfig>, bootstrap_subj: Option<String> }`, `StartupConfig::from_env_with(&dyn Fn(&str) -> Option<String>) -> anyhow::Result<StartupConfig>`, `StartupConfig::from_env()`, `OidcConfig { issuer, client_id, client_secret, redirect_uri, post_logout_redirect_uri, bootstrap_subj: Option<String> }`, `AuthMode { Oidc(OidcConfig), #[cfg(debug_assertions)] Disabled { as_user: i32 } }`, `finalise_auth(&SqlitePool, &StartupConfig) -> anyhow::Result<AuthMode>`.

`AuthRequest` exists because `AuthMode::Disabled { as_user }` needs a user id that only exists after `init_db`, so the parsed mode and the final mode are two different values (spec §3). `bootstrap_subj` is authoritative on `OidcConfig`; `StartupConfig::bootstrap_subj` is the same parsed value kept for `init_db`'s backfill, and `from_env_with` is the single place that fills both.

- [ ] Define the types per spec §3 plus the D-11/D-14 additions. `AuthMode` is `Clone`. `Disabled` carries `as_user: i32` and is `#[cfg(debug_assertions)]`.
- [ ] `post_logout_redirect_uri` is **derived**, not configured: the origin of `OIDC_REDIRECT_URI` with path `/` (spec §4 registers `https://<host>/` as a `post_logout_redirect_uri`; a sixth variable for a value already implied by the callback is a second thing to get wrong).
- [ ] `from_env_with` reads `AUTH_MODE` (default `oidc`), `OIDC_ISSUER`, `OIDC_CLIENT_ID`, `OIDC_CLIENT_SECRET`, `OIDC_REDIRECT_URI`, `BOOTSTRAP_OWNER_SUBJ`. `from_env()` is `from_env_with(&|k| std::env::var(k).ok())`. All logic lives in `from_env_with` (D-13).
- [ ] Trim every value. An empty-or-whitespace `BOOTSTRAP_OWNER_SUBJ` counts as **unset**: a set-but-empty value would skip the OIDC refuse-to-start branch and assign orphans to a placeholder no identity can ever claim — the misattribution F4 exists to prevent.
- [ ] `AUTH_MODE` values other than `oidc`/`disabled` → `Err` naming the variable and the accepted values.
- [ ] `AUTH_MODE=disabled` with `cfg!(debug_assertions) == false` → `Err` naming the variable **and** the build mode. This is the failure mode the compile-time gate exists for; the message is the only thing the operator sees.
- [ ] `AUTH_MODE=oidc` with any `OIDC_*` missing → `Err` listing **every** missing variable, not the first one. Deny-by-default: no degraded anonymous mode.
- [ ] `AUTH_MODE=oidc` with a non-`https` `OIDC_REDIRECT_URI` → `Err`, allowing `http://localhost` and `http://127.0.0.1` for dev. `Secure` is set unconditionally (spec §4), so an `http:` redirect URI would send the code in cleartext and then loop with the cookie dropped by the browser.
- [ ] `finalise_auth`: `Oidc` → `AuthMode::Oidc(cfg)`. `Disabled` → `SELECT id FROM users WHERE issuer = 'local' AND subj = 'bootstrap'` → `AuthMode::Disabled { as_user }`; `Err` if absent, which means the order of operations in `main` is wrong (Task 8 makes `init_db` create it unconditionally, D-12).
- [ ] **Gate the `Disabled` match arms** with `#[cfg(debug_assertions)]`, not just the variant: `AuthRequest::Disabled` exists in a release build while `AuthMode::Disabled` does not, so an un-gated arm fails `cargo build --release`. Task 21 adds that CI job and Task 24 runs it locally; do not let either be the first place the error appears.
- [ ] Print a startup banner in `Disabled` mode naming the assumed user id (spec §2).

### Task 3: `src/auth/session.rs`

**Files:** Create `backend_rust/src/auth/session.rs`

**Interfaces:** `SESSION_COOKIE`, `STATE_COOKIE`, `SESSION_MAX_AGE` (8 h), `STATE_MAX_AGE` (10 min), `SessionPayload { uid: i32, tv: i64, exp: i64 }`, `StatePayload { state, nonce, verifier }`, `value(uid, tv, exp) -> String`, `parse(&str) -> Option<SessionPayload>`, `issue(&CookieJar, uid, tv)`, `read(&CookieJar) -> Option<SessionPayload>`, `clear(&CookieJar)`, `issue_state`, `read_state`, `clear_state`.

- [ ] One private cookie builder: `HttpOnly`, `Secure` unconditional, `SameSite=Lax`, `path = "/"`, `Max-Age` per cookie. `Secure` is unconditional on purpose — TLS terminates at the ingress, nginx sees plain HTTP, so keying the flag on the request scheme would drop it in production (spec §4). Put that reason in a comment next to the flag.
- [ ] `value`/`parse` are pure JSON of `{"uid":…,"tv":…,"exp":…}` (D-11); unit-testable without Rocket. `issue` computes `exp = Utc::now() + SESSION_MAX_AGE`.
- [ ] `issue` → `jar.add_private((SESSION_COOKIE, value(uid, tv, exp)))`; `read` → `jar.get_private(SESSION_COOKIE)` then `parse`; `clear` → `jar.remove_private(Cookie::named(SESSION_COOKIE))`. There is no `remove_named` in Rocket 0.5 (D-3).
- [ ] Test the pure parts here: `value`/`parse` round-trips, and `parse` rejects malformed or tampered strings. The `exp` and `token_version` rejection tests belong to Task 15, because they go through `resolve_caller` (Task 4), not through this module.

### Task 4: `src/auth/guard.rs`

**Files:** Create `backend_rust/src/auth/guard.rs`

**Interfaces:** `AuthedUser { id: i32, email: Option<String>, name: Option<String>, sub: String }`, `impl FromRequest<'r> for AuthedUser`, and

```rust
pub(crate) async fn resolve_caller(
    req: &Request<'_>,
    mode: &AuthMode,
    pool: &SqlitePool,
) -> Result<Option<AuthedUser>, sqlx::Error>;
```

That one signature is what Tasks 4, 6 and 7 code against: `Ok(None)` means "no valid session" (deny), `Err(db)` means "the database failed" (500, per D-15).

- [ ] `resolve_caller`: `Disabled { as_user }` → load that row, no cookie touched. `Oidc` → `session::read`, `Ok(None)` if `exp` is past (D-11), load `users` by `uid`, `Ok(None)` if missing or `is_active = 0` or `token_version != tv`, `Err` if the query itself fails.
- [ ] `FromRequest` rejects with the typed responder of D-16 (`{"error":"unauthenticated"}`), never a bare status, so `/auth/me` and the denial route cannot diverge. Handlers keep the existing `Result<T, Status>` vocabulary; no `AuthError` enum (spec §7). A database error maps to `InternalServerError`.
- [ ] **`Authorization` headers are ignored, never accepted** (constraint C2). `FromRequest` reads only the private session cookie; add a comment saying an `Authorization` header on an `/api` request must not authenticate, and Task 15 asserts a forged header still gets `401`.
- [ ] The double read (fairing + guard) is D-2. Comment there: do not replace the second read with a request-header marker, which a client could forge.
- [ ] Gate the `Disabled` arm with `#[cfg(debug_assertions)]` (Task 2's note).

### Task 5: `src/auth/oidc.rs`

**Files:** Create `backend_rust/src/auth/oidc.rs`

**Interfaces:** `provider(&OidcConfig) -> anyhow::Result<CoreProviderMetadata>`, `login_url(&OidcConfig) -> anyhow::Result<(Redirect, StatePayload)>`, `exchange_code(&OidcConfig, &str, &StatePayload) -> anyhow::Result<CoreTokenResponse>`, `validate_id_token(&OidcConfig, &CoreTokenResponse, &Nonce) -> anyhow::Result<CoreIdTokenClaims>` (the JWKS/`iss`/`aud`/`exp`/`nonce` checks, kept separate from the network exchange so it can be reasoned about alone), `logout_url(&OidcConfig) -> anyhow::Result<Option<String>>`, and

```rust
pub async fn provision_or_update(
    pool: &SqlitePool,
    issuer: &str,
    sub: &str,
    email: Option<&str>,
    name: Option<&str>,
    bootstrap_subj: Option<&str>,
) -> anyhow::Result<AuthedUser>
```

- [ ] **Reference snippet** — compiled against `openidconnect` 4.1.0 / `oauth2` 5.0.0 / `reqwest` 0.12 (rustls-only), so the generic choices are already made (D-17):

  ```rust
  use openidconnect::core::{
      CoreClient, CoreIdTokenClaims, CoreIdTokenVerifier, CoreJsonWebKeySet,
      CoreProviderMetadata, CoreResponseType, CoreTokenResponse,
  };
  use openidconnect::{
      AuthenticationFlow, AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl,
      Nonce, PkceCodeChallenge, RedirectUrl, Scope, TokenResponse,
  };

  let http = reqwest::Client::new();                       // implements AsyncHttpClient
  let md = CoreProviderMetadata::discover_async(issuer.clone(), &http).await?;
  let jwks = CoreJsonWebKeySet::fetch_async(md.jwks_uri(), &http).await?;

  let client = CoreClient::new(client_id.clone(), issuer.clone(), jwks.clone())
      .set_client_secret(ClientSecret::new(secret.clone()))
      .set_auth_uri(md.authorization_endpoint().clone())
      .set_token_uri(md.token_endpoint().cloned().ok_or_else(|| anyhow!("no token endpoint"))?)
      .set_redirect_uri(RedirectUrl::new(cfg.redirect_uri.clone())?);

  let (pkce, verifier) = PkceCodeChallenge::new_random_sha256();
  let (auth_url, csrf, nonce) = client
      .authorize_url(
          AuthenticationFlow::<CoreResponseType>::AuthorizationCode,
          CsrfToken::new_random,
          Nonce::new_random,
      )
      .add_scope(Scope::new("openid profile email".to_string()))
      .set_pkce_challenge(pkce)
      .url();

  let response: CoreTokenResponse = client
      .exchange_code(AuthorizationCode::new(code.to_string()))?
      .request_async(&http).await?;

  let id_token = response.id_token().ok_or_else(|| anyhow!("no id_token"))?;
  let verifier = CoreIdTokenVerifier::new_confidential_client(
      client_id.clone(), ClientSecret::new(secret.clone()), issuer.clone(), jwks,
  );
  let claims: &CoreIdTokenClaims = id_token.claims(&verifier, &nonce)?;
  ```

  Note `Scope::new("openid profile email")` is one scope string; `use_openid_scope` is on by default, so do not add `openid` twice.
- [ ] Discovery is lazy and cached **only on success** in a `OnceLock` (spec §4). A failed discovery must not be cached, so one Authentik outage cannot disable logins until restart. Nothing at startup touches the network — that is what keeps O2 true and a mistyped `OIDC_ISSUER` from CrashLooping the pod.
- [ ] `exchange_code` verifies `state` against the cookie payload, exchanges the code, validates the ID token (JWKS signature, `iss`, `aud`, `exp`, `nonce`), and returns the three claims as primitives (D-14). No IdP detail leaves the process; see Task 6 for the two fixed strings the client is allowed to see.
- [ ] `provision_or_update` implements spec §6 reconciliation in this order: (1) match `(issuer, subj)` → update `email`, `name`, `last_login_at`, use it; (2) else if `bootstrap_subj == Some(sub)` and the placeholder row exists → `UPDATE` that row in place to the real identity, preserving `id`; (3) else insert and log `provisioned user id=… issuer=… sub=…` at `info`. If both a real row and the placeholder exist, step 1 wins and the placeholder is left alone with a `warn`.
- [ ] Step 3 inserts with `ON CONFLICT (issuer, subj) DO NOTHING` and re-selects, so two concurrent first logins for the same `sub` cannot fail one whole login on the unique index.
- [ ] `email` is stored and never compared. Comment why: with self-registration an `email` claim is claimable by anyone.
- [ ] `logout_url` uses the provider's `end_session_endpoint` plus `post_logout_redirect_uri` from `OidcConfig::post_logout_redirect_uri` (derived in Task 2). `Ok(None)` means the provider advertises no `end_session_endpoint`; `/auth/logout` then redirects to `/` instead (see Task 6).

### Task 6: `src/auth/fairing.rs` and `src/auth/routes.rs`

**Files:** Create `backend_rust/src/auth/fairing.rs`, `backend_rust/src/auth/routes.rs`

**Interfaces:** `AuthFairing` (unit struct, `Kind::Request`); denial handlers `denied_get`/`denied_post`/`denied_put`/`denied_delete`/`denied_patch`/`denied_head` at `/_auth/denied`; `/auth/login`, `/auth/callback`, `/auth/logout`, `/auth/logout-all`, `/auth/me`.

- [ ] `on_request`: return unless `req.uri().path().starts_with("/api/")`; return for a real preflight (`OPTIONS` **and** `Origin` **and** `Access-Control-Request-Method` present — D-1 item 3); otherwise read `req.rocket().state::<AuthMode>()` and `state::<SqlitePool>()` and call `resolve_caller`. On `Ok(None)`, log method + original path at `warn` and `req.set_uri(Origin::parse("/_auth/denied").unwrap())`. On `Err(db)`, log and also rewrite to the denial route — never let a database error fall through to an unauthenticated handler.
- [ ] Denial handlers return `401` with `Json(json!({"error":"unauthenticated"}))`. **Seven** methods (`get`/`post`/`put`/`delete`/`patch`/`head`/`options`), so no method can produce a `405` where F1 demands `401` (D-1 item 2).
- [ ] `/auth/login` and `/auth/callback` implement the **`Accept` branch of spec §9**: a request whose `Accept` includes `text/html` gets `302 /?auth_error=auth_failed` on OIDC failure; anything else gets `401` with `{"error":"authentication_failed"}`. Spec §9 words this for the callback; this plan applies the same branch to `/auth/login` too, because discovery can fail there and a browser navigating to a broken login should not see bare JSON. Two distinct fixed strings: `unauthenticated` means "no session on `/api/`", `authentication_failed` means "the OIDC exchange failed". No IdP detail is ever reflected.
- [ ] `/auth/logout` clears the cookie and `302`s to `logout_url`, or to `/` when `logout_url` is `Ok(None)` (the provider advertises no `end_session_endpoint`). `/auth/logout-all` bumps `users.token_version` then clears this cookie. `/auth/me` returns `{id, email, name, sub}` for the caller, `401` otherwise, and is mounted in **both** modes; under `Disabled` it reports the placeholder `sub` — the literal string `bootstrap`, which is a marker, not a real subject.
- [ ] The test that proves the rewrite prevents handler execution (a mounted `/api` route with a side effect must leave the database unchanged without a session) is written in **Task 15**, which owns `tests/auth_test.rs`. This task only needs the fairing and routes to compile and the existing suite to stay green.

### Task 7: `src/access.rs` — the chokepoint

**Files:** Create `backend_rust/src/access.rs`

**Interfaces — all four pinned here:**

```rust
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Access { Viewer, Editor, Owner }   // Ord gives Viewer < Editor < Owner

pub async fn require(pool: &SqlitePool, user_id: i32, portfolio_id: i32, min: Access)
    -> Result<Access, rocket::http::Status>;

pub async fn require_asset(pool: &SqlitePool, user_id: i32, asset_id: i32, min: Access)
    -> Result<Access, rocket::http::Status>;

/// Also rejects when the asset's real portfolio is not the one named in the path.
pub async fn require_asset_in(pool: &SqlitePool, user_id: i32, path_portfolio_id: i32,
    asset_id: i32, min: Access) -> Result<Access, rocket::http::Status>;

pub async fn require_transaction(pool: &SqlitePool, user_id: i32, tx_id: i32, min: Access)
    -> Result<Access, rocket::http::Status>;

/// Private. Ok(None) ⇒ no access, including a NULL owner_id.
async fn resolve(pool: &SqlitePool, user_id: i32, portfolio_id: i32)
    -> Result<Option<Access>, sqlx::Error>;
```

- [ ] `resolve` is private, `SELECT owner_id FROM portfolios WHERE id = ?`, `Ok(None)` for a `NULL` owner.
- [ ] Exported functions map `Ok(None)` → `NotFound` and `Err(db)` → `InternalServerError` with the database error logged separately (D-15). A handler cannot ignore a denial and keep going.
- [ ] `require_asset`: `SELECT portfolio_id FROM assets WHERE id = ?` then `require`. `require_asset_in`: same query, plus reject when the returned `portfolio_id != path_portfolio_id`, no extra query.
- [ ] `require_transaction`: `SELECT asset_id FROM transactions WHERE id = ?` then the asset lookup. A transaction whose `asset_id` matches no asset resolves to `NotFound` (reachable today: asset deletion orphans transactions and foreign keys are off).
- [ ] Log every denial at `warn` with user id and target id, so a probing pattern is visible and distinguishable from genuine 500s (spec §9).
- [ ] Doc-comment the module as the only permitted path to a portfolio/asset/transaction by caller id, name the gate script (D-7), and restate that `require*` answers *may this caller touch this id* and never returns the row.
- [ ] Put the `resolve` unit cases (owner / another user / `NULL` owner) in a `#[cfg(test)] mod tests` **inside this file** — an integration test under `tests/` cannot reach a private item.

### Task 8 + Task 9: `lib.rs`, then `openapi.rs` and `main.rs`

Tasks 8 and 9 are **one commit**: Task 8 changes two signatures whose call sites Task 9 repairs, so the green rule is suspended between them and restored at the end of Task 9.

#### Task 8: `lib.rs` — schema, backfill, refusals, wiring

**Files:** Modify `backend_rust/src/lib.rs`

**Interfaces:** `pub async fn init_db(pool: &SqlitePool, cfg: &StartupConfig) -> anyhow::Result<()>`, `pub fn build_rocket(pool, currency_service, cors, auth: AuthMode) -> Rocket<Build>`.

- [ ] Add `pub mod auth; pub mod access;`.
- [ ] `users` table and `uq_users_issuer_subj` exactly as spec §6, `CREATE TABLE IF NOT EXISTS`.
- [ ] `ALTER TABLE portfolios ADD COLUMN owner_id INTEGER` guarded by the existing `column_exists` helper. Run it on the **same dedicated connection with `PRAGMA legacy_alter_table = ON`** that `migrate_decimal_columns` already uses (`lib.rs:257-283`). That pragma was added because the bundled SQLite's stale per-connection schema makes `DROP COLUMN` followed by `RENAME` + `ADD` fail on file-backed databases; a fresh `ALTER` on a pooled connection is the same class of hazard. Do **not** touch `migrate_one_column`.
- [ ] Upsert the placeholder row **unconditionally** with the literal values from spec §6 (`issuer='local'`, `subj='bootstrap'`, `email` NULL), `ON CONFLICT (issuer, subj) DO NOTHING`. `created_at` from `chrono::Utc::now().to_rfc3339()` in Rust (D-12).
- [ ] Backfill per the spec §6 table: `Disabled` + orphans → assign to placeholder, `warn` the count, `Ok`. `Oidc` + orphans + `bootstrap_subj` set → same. `Oidc` + orphans + unset → `Err` naming the count and the variable. No orphans → nothing.
- [ ] `init_db` returns `Result` and never calls `process::exit` — that is what makes "startup refuses" testable.
- [ ] `build_rocket` gains `auth: AuthMode`, `.manage(auth)`, `.attach(AuthFairing)`, mounts `/auth` routes (the OIDC-only routes only in OIDC mode; `/auth/me` in both) and the seven `/_auth/denied` handlers.
- [ ] Remove `openapi_json` from the `.mount("/", routes![index, openapi_json])` list so only the `/api-docs` mount remains. **Leave the `#[get("/openapi.json")]` attribute on the handler** — the attribute *is* the route path and the same handler is mounted at `/api-docs`; deleting it removes both mounts and breaks the public document Task 16 pins.
- [ ] `GET /` stays unauthenticated and outside the fairing's prefix.

#### Task 9: `openapi.rs`, `main.rs`, and the regenerated document

**Files:** Modify `backend_rust/src/openapi.rs`, `backend_rust/src/main.rs`; regenerate `docs/openapi/openapi.json`

- [ ] In `openapi.rs`: add `crate::api_routes::transactions::update_asset` to `paths(...)`. The annotation already exists on the handler (`transactions.rs:88-95`); only the registration is missing. Published operations go from 14 to 15 (D-4).
- [ ] Add `pub fn document() -> utoipa::openapi::OpenApi` (D-8): insert the `pp_session` cookie `apiKey` scheme, set top-level `security`, and add a `401` response to every operation under a path starting `/api`. Do **not** declare `bearerAuth` — Spec 3 does, when bearer tokens exist.
- [ ] **Serializer, pinned once:** `serde_json::to_string(&document())`. Both `openapi_json` and `--print-openapi` use it, and the committed file is regenerated with it in this task, because the current file came from `Json(serde_json::to_value(...))` and key order differs between the two. The CI diff is byte-exact.
- [ ] In `main.rs`: replace `#[launch]` with `#[rocket::main] async fn main() -> Result<(), rocket::Error>` (D-10). `#[launch]` generates its own `main`, so the two cannot coexist.
- [ ] **`--print-openapi` is the first statement in `main`, before `StartupConfig::from_env`.** CI sets no `OIDC_*` variables; if the config refusal ran first, `--print-openapi` would exit 1 and O4 and Task 21 would both break. Print the document, then `return Ok(())` — no DB, no Rocket, no secret key, no network.
- [ ] Then: `StartupConfig::from_env` (on `Err`: log `startup refused: …`, `exit(1)`) → connect pool → `init_db(&pool, &cfg)` (same refusal) → `finalise_auth` → `build_rocket(...).launch().await`. `exit(1)` and not `return`: `main`'s error type is `rocket::Error`, which has no variant for "the operator configured this wrong", and routing a configuration refusal through it buries the message (spec §3).
- [ ] Regenerate now: `cd backend_rust && cargo run --bin backend_rust -- --print-openapi > ../docs/openapi/openapi.json`. This is the only task that owns that file.

### Task 10: test helpers

**Files:** Modify `backend_rust/tests/common/mod.rs` and every existing `tests/*.rs`

- [ ] `build_rocket(pool, as_user: i32)` wraps `AuthMode::Disabled { as_user }`; `setup_db(cfg: &StartupConfig)`; `seed_user(pool, subj) -> i32`; `seed_portfolio(pool, owner_id, name, currency) -> i32` gains the owner it currently omits; `oidc_cfg()` returns an `OidcConfig` built from parseable loopback URLs (discovery is lazy, so nothing is contacted); `session_cookie(uid, tv)` returns the `("pp_session", session::value(uid, tv, now + 8h))` pair tests pass to `LocalRequest::private_cookie`.
- [ ] Update every existing test file to the new signatures. `tests/decimal_migration_test.rs` needs no change (it calls `migrate_one_column` directly); leave it alone and confirm it still passes.
- [ ] `cargo test` green again at the end of this task.

### Task 11: `portfolios.rs`

**Files:** Modify `backend_rust/src/api_routes/portfolios.rs`

- [ ] `create_portfolio`: take `AuthedUser`, insert `owner_id = user.id`.
- [ ] `list_portfolios`: scope **both** queries (D-5).
- [ ] `get_portfolio` → `require(…, Viewer)`; `update_portfolio` → `require(…, Owner)`; `delete_portfolio` → `require(…, Owner)` **before** the asset-count check, so a non-owner cannot use the `409`-vs-`404` difference as an existence oracle.
- [ ] Keep the handlers' own `SELECT * FROM portfolios WHERE id = ?` reads — they are the response body (D-4). The gate script (D-7) is written so a `require*` call in the same handler clears them.

### Task 12: `transactions.rs`

**Files:** Modify `backend_rust/src/api_routes/transactions.rs`

- [ ] `create_asset` → `require(portfolio_id, Editor)` before any query (D-6).
- [ ] `update_asset` → `require_asset_in(portfolio_id, asset_id, Editor)`. **Keep** the `AND portfolio_id = ?` in the `UPDATE` and the scoped `SELECT` (D-18). Record the duplication in the commit message so a reviewer does not read it as an oversight.
- [ ] `delete_asset` → `require_asset(id, Owner)`.
- [ ] `create_transaction` → `require_asset_in(portfolio_id, asset_id, Editor)`; keep the existing asset lookup as the source of the row it validates.
- [ ] `list_portfolio_transactions` → `require(portfolio_id, Viewer)`.
- [ ] `delete_transaction` → `require_transaction(id, Editor)`.

### Task 13: `analytics.rs` and `lookup.rs`

**Files:** Modify `backend_rust/src/api_routes/analytics.rs`, `backend_rust/src/api_routes/lookup.rs`

- [ ] `get_portfolio_tax_summary` → `require(id, Viewer)`; `get_portfolio_performance` → `require(id, Viewer)` before the subquery at `analytics.rs:133` (D-7 blind spot).
- [ ] `get_transaction_sell_pnl` → prepend `require_transaction(id, Viewer)` (D-4). **Keep all three lookups** — they are the response's data source.
- [ ] `lookup_isin` → takes `AuthedUser` only. No id to check, no throttle in this spec (issue #35).

### Task 14: `access_test.rs`

**Files:** Create `backend_rust/tests/access_test.rs`; add `#[cfg(test)] mod tests` to `src/access.rs` (Task 7)

- [ ] Public API only: `require(…, Viewer)` and `require(…, Editor)` both admit the owner and both `404` a non-owner — pins that the level argument is threaded while membership is absent.
- [ ] `Access` ordering: `Viewer < Editor < Owner`.
- [ ] `require_asset_in` rejects an asset whose `portfolio_id` differs from the path id; `require_asset` accepts the same asset with no path id.
- [ ] `require_transaction` returns `NotFound` for a transaction whose `asset_id` matches no asset.
- [ ] A database failure surfaces as `InternalServerError`, not `NotFound` (D-15).
- [ ] The `resolve` owner / other-user / `NULL`-owner cases live in `src/access.rs`'s own `#[cfg(test)] mod tests` (Task 7), not here.

### Task 15: `auth_test.rs`

**Files:** Create `backend_rust/tests/auth_test.rs`

- [ ] Session round-trip via `LocalRequest::private_cookie(session_cookie(uid, tv))` (D-3); a tampered cookie is rejected; a cookie whose `tv` differs from the row is rejected; a cookie whose `exp` is past is rejected with a matching `token_version` (D-11).
- [ ] `token_version` bump invalidates a live cookie; `POST /auth/logout-all` performs the bump; `clear` leaves the cookie non-authenticating.
- [ ] `is_active = 0` rejects.
- [ ] `AUTH_MODE=disabled` is exercised by constructing `AuthMode::Disabled { as_user }` directly — not by setting an environment variable, which D-13 forbids in a multi-threaded test binary. The release case is covered by the `cargo build --release` job in Task 21, not by a unit test.
- [ ] A request carrying a forged `Authorization: Bearer …` header and no session cookie still gets `401` (C2: the guard reads the cookie only).
- [ ] **The F1 case:** with `AuthMode::Oidc(oidc_cfg())` and no cookie, every mounted `/api` route returns `401` with `{"error":"unauthenticated"}`. This is the only test that exercises unauthenticated `/api/` denial — the isolation matrix runs under `Disabled`, where nothing denies.
- [ ] A `PUT /api/...` without a session returns `401`, not `405` — the seven-method denial route.
- [ ] A bare `OPTIONS /api/portfolios/` (no `Origin`, no `Access-Control-Request-Method`) gets `401` from the seventh sentinel handler, not `405`; a real preflight is not rewritten.
- [ ] The D-1 side-effect test: a mounted `/api` route with a side effect (a `POST` that inserts) leaves the database unchanged when called without a session. This is the test that distinguishes D-1 from a guards-only design; without it, a future refactor back to guards-only would silently pass everything else.
- [ ] `GET /` is `200` with auth enabled and no cookie (O1). `GET /api-docs/openapi.json` is `200` without a cookie — pins the decision that the contract stays public. `GET /openapi.json` is `404`.
- [ ] `/auth/me` under `Disabled` reports the placeholder `sub` (`bootstrap`); under `Oidc` without a cookie it returns `401` **with the `{"error":"unauthenticated"}` body** (D-16).

### Task 16: `isolation_test.rs` — the matrix

**Files:** Create `backend_rust/tests/isolation_test.rs`

- [ ] Two seeded users A and B. For **every** route mounted in `build_rocket`, B attempts the operation against A's portfolio / asset / transaction id and must get `404`; each case is paired with A succeeding, so a guard that merely breaks everything cannot pass.
- [ ] Include the sell-pnl route (D-4) and the cross-portfolio nested writes: B owns portfolio 1, A owns portfolio 2 and asset 99 in it; B writes through portfolio 1's path with asset 99's id.
- [ ] `POST /api/portfolios/` and `GET /api/assets/lookup` have no id to attack: covered by Task 15's F1 case and A's success case, and say so in a comment rather than inventing a cross-user case.
- [ ] Two users whose `email` claim is identical cannot reach each other's rows: seed A and B with the same `email` and different `sub`, and assert B still gets `404` on A's portfolio. This is what makes the "`email` never decides ownership" constraint enforceable rather than a comment.
- [ ] `GET /api/portfolios/` returns only A's portfolios **and none of B's assets** (D-5).
- [ ] `delete_portfolio` against B's id is `404`, not `409` (Task 11).
- [ ] `POST /api/portfolios/<nonexistent>/assets` is `404`, not `201` (D-6).

### Task 17: `startup_test.rs`

**Files:** Create `backend_rust/tests/startup_test.rs`

- [ ] Orphans + `Oidc` + `bootstrap_subj` set → all owned after `init_db`, `Ok`.
- [ ] Orphans + `Oidc` + unset → `init_db` returns `Err` (asserted on the `Result`, never on a process exit).
- [ ] Orphans + `Oidc` + `BOOTSTRAP_OWNER_SUBJ` set to `""` or whitespace → treated as unset → `Err` (D-13's trim rule).
- [ ] Orphans + `Disabled` → assigned to the placeholder, `Ok`.
- [ ] No portfolios → `Ok`, `owner_id` exists as a column, **and the placeholder row exists** (D-12 — this is what keeps `./start.sh` alive on a fresh database).
- [ ] Handover via `provision_or_update` primitives (D-14): a `sub` equal to `bootstrap_subj` takes over the placeholder and keeps its `id`; a non-matching `sub` gets a new row; a `sub` matching an existing real row does not touch the placeholder; calling again after a handover creates no new row and re-handover is impossible because the placeholder is gone (D-12's handover rule).
- [ ] `from_env_with` refusals (D-13): unknown `AUTH_MODE` value; each missing `OIDC_*` (assert every missing name appears in one message); non-https `OIDC_REDIRECT_URI` rejected, `http://127.0.0.1:8000/auth/callback` accepted.

### Task 18: `route_coverage_test.rs`

**Files:** Create `backend_rust/tests/route_coverage_test.rs`

- [ ] Ignite the app; iterate `Rocket::routes()`; for each route `route.as_any().downcast_ref::<rocket::Route>()`, take `uri.to_string()`, truncate at `?` (Rocket templates carry the query part: `/<id>/tax-summary?<strategy>&<threshold_days>`), translate Rocket's `<param>` to utoipa's `{param}`, and apply one trailing-slash rule consistently. Assert every mounted `/api/` route's method + translated path appears in `crate::openapi::document()`'s paths.
- [ ] Assert every mounted `/api`-prefixed route's resolved path `starts_with("/api/")` — a route resolving to exactly `/api`, or any future mount outside the prefix, escapes the fairing while still passing an inventory check.
- [ ] Assert every mounted `OPTIONS` route is outside `/api` (D-1 item 3).
- [ ] Assert `/_auth/denied` is mounted for each of the seven methods and that none of them is under `/api`.
- [ ] Maintain the route inventory as `const PROTECTED_ROUTES: &[(&str, &str, Option<Access>)]` — method, path, and the level required where one exists (`None` for `POST /api/portfolios/` and `GET /api/assets/lookup`, which authenticate but check no id) — and assert it matches the mounted set whose resolved path starts with `/api/` exactly: same length, same members. Routes outside that prefix (`/`, `/api-docs/openapi.json`, `/auth/*`, `/_auth/denied`) are deliberately not in the inventory; say so in a comment. A new `/api/` route must be added here, which is the drift check the hand-maintained `routes![]` / `ApiDoc::paths` pair otherwise lacks.

### Task 19: Frontend

**Files:** `frontend/src/utils/apiFetch.ts` (new), `App.tsx`, `components/CsvImportModal.tsx`, `components/PortfolioDetail.tsx`, `components/SellPnlModal.tsx`, `test/mocks/handlers.ts`, `test/utils/apiFetch.test.ts` (new), `test/components/AppAuthError.test.tsx` (new); regenerate `frontend/src/types/api.d.ts`

- [ ] `apiFetch(input, init)`: wraps `fetch`, and on `401` calls `window.location.assign('/auth/login')` and returns the response so callers do not double-handle. **Forward `init` verbatim** — `SellPnlModal.tsx:93` passes an `AbortController` `signal`, and dropping it would break in-flight cancellation. Use the existing `API_BASE` from `src/utils/api.ts`; do not retype `'/api'`.
- [ ] Replace all 19 call sites: `App.tsx` 14, `CsvImportModal.tsx` 3, `PortfolioDetail.tsx` 1, `SellPnlModal.tsx` 1. JavaScript never reads, stores, or sends a token — an invariant, not a preference.
- [ ] `App.tsx`: when the URL carries `?auth_error=auth_failed`, render a fixed "sign-in failed" message and strip the parameter with `history.replaceState`. Only that one value is ever emitted; no IdP detail is reflected.
- [ ] MSW: add a `401` handler. Tests: `apiFetch` redirects on `401` and forwards `signal`; the banner renders and the parameter is stripped; existing component tests (including `SellPnlModal.test.tsx`) still pass with the wrapper in place.
- [ ] Regenerate the types: `cd frontend && npm run generate-types`. This is the only task that owns `src/types/api.d.ts`.

### Task 20: Proxies

**Files:** `frontend/nginx.conf`, `frontend/vite.config.js`

- [ ] Add `location /auth { proxy_pass http://127.0.0.1:8000; … }` with the same four `proxy_set_header` lines as `/api`. Without it `try_files` serves `index.html` for `/auth/login` and login is a silent redirect loop.
- [ ] Add `'/auth': { target: 'http://127.0.0.1:8000', changeOrigin: true }` to the Vite proxy map, or `./start.sh` cannot complete a login.
- [ ] Both files sit outside both test suites, so Task 21 gates them.

### Task 21: CI and the gate script

**Files:** `.github/workflows/ci.yml`, `backend_rust/scripts/check_access_gate.sh` (new)

- [ ] Write `check_access_gate.sh` implementing Rules A/B/C of D-7, printing the counts it saw. Run it locally before committing.
- [ ] Replace the server+curl OpenAPI step with `cd backend_rust && cargo run --bin backend_rust -- --print-openapi > /tmp/current_openapi.json && diff /tmp/current_openapi.json ../docs/openapi/openapi.json`. Delete the dummy `portfolio.db`, the `sleep 10`, the background process and the `kill`.
- [ ] Add a step failing unless `frontend/nginx.conf` contains `location /auth` and `frontend/vite.config.js` contains `'/auth'`.
- [ ] Add `bash backend_rust/scripts/check_access_gate.sh` to the backend job.
- [ ] Add a **`cargo build --release` job**. It proves the release binary compiles with `AuthMode::Disabled` compiled out — compilation only; it does not run the release binary, so it cannot prove the runtime refusal. `cargo build --release` only; `cargo test --release` is **not** supported (the integration tests need `AuthMode::Disabled`) and must not be added.
- [ ] Keep `cargo test`, `npm run generate-types`, `npm run typecheck`, `npm run lint`, `npm test` — the same set Task 24 runs locally, so CI and the local gate cannot diverge.

### Task 22: Kubernetes

**Files:** `k8s/secret.example.yaml` (new), `k8s/configmap.yaml`, `k8s/app-deployment.yaml`

- [ ] `secret.example.yaml` with `stringData` placeholders and a header comment: copy it, fill it, never commit it (D-9). Document `kubectl create secret generic backend-secrets -n investment-portfolio-manager --from-literal=OIDC_CLIENT_SECRET=… --from-literal=ROCKET_SECRET_KEY="$(openssl rand -base64 32)"`. That command yields 44 base64 characters decoding to 32 bytes, which is what Rocket 0.5 accepts (`config/secret_key.rs:218`: 44 or 88 base64 chars, or 64 hex; fewer than 32 bytes after decoding is rejected).
- [ ] `configmap.yaml` gains `AUTH_MODE: "oidc"`, `OIDC_ISSUER`, `OIDC_CLIENT_ID`, `OIDC_REDIRECT_URI`, `BOOTSTRAP_OWNER_SUBJ`. No `ROCKET_PROFILE`: the release image should resolve to the `release` profile, and Rocket's own secret-key check is what proves the key is present. `post_logout_redirect_uri` is derived in code (Task 2), so it is not a ConfigMap key.
- [ ] `app-deployment.yaml`: backend gains `envFrom: secretRef: backend-secrets` alongside the existing `configMapRef`; delete the `wait-for-backend` initContainer — it curls `:8000/` before the backend container exists, so it can only ever exit 1, and the readiness probe already gates traffic (O1).
- [ ] `ingress.yaml` unchanged: authentication is enforced by the application, not the proxy.

### Task 23: Docs and dev script

**Files:** `AGENTS.md`, `start.sh`

- [ ] `AGENTS.md`: replace the `cargo run --release` + curl regeneration procedure with `cd backend_rust && cargo run --bin backend_rust -- --print-openapi > ../docs/openapi/openapi.json`. **Land it in the same PR as Task 9** — it is the documented human path, and a stale copy tells the next person to run a command that cannot work.
- [ ] `start.sh`: export `AUTH_MODE=disabled` before `cargo run`. No `ROCKET_PROFILE`, no `ROCKET_SECRET_KEY`: `cargo run` is a debug build in the `debug` profile, where Rocket generates a temporary key. Note in the script that dev sessions therefore end on a backend restart.

### Task 24: Verification

**Files:** none — run the gates

- [ ] `cd backend_rust && cargo test` — all green, including the new files.
- [ ] `cd backend_rust && bash scripts/check_access_gate.sh` — passes, and reports a non-zero number of handlers inspected.
- [ ] `cd backend_rust && cargo build --release` — proves the release binary compiles without the `Disabled` variant.
- [ ] `cd backend_rust && cargo run --bin backend_rust -- --print-openapi > /tmp/openapi.json && diff /tmp/openapi.json ../docs/openapi/openapi.json` — empty diff, and the document has 15 operations, the cookie scheme, and a `401` on every `/api` operation.
- [ ] `cd frontend && npm run generate-types && npm run typecheck && npm run lint && npm test`.
- [ ] `./start.sh` smoke: `curl -i http://127.0.0.1:8000/` → `200`; `curl -i http://127.0.0.1:8000/api/portfolios/` under `Disabled` → `200`; `AUTH_MODE=oidc cargo run` with no `OIDC_*` → refuses with every missing variable named.

---

## Rollout (operator steps, not code)

1. Create the `Secret` (`ROCKET_SECRET_KEY`, `OIDC_CLIENT_SECRET`) before anything starts.
2. Create the Authentik provider with self-registration **off**, registering **both** `https://<host>/auth/callback` and `https://<host>/` as redirect URIs — the second is the `post_logout_redirect_uri`, and logout fails at the IdP with no local diagnostic if it is missing.
3. Read the operator's own `sub` from the Authentik admin console. It is needed in step 4, not later.
4. Deploy with `AUTH_MODE=oidc`, the `OIDC_*` variables, and `BOOTSTRAP_OWNER_SUBJ` set to that `sub`. The backfill runs; pre-existing portfolios move to the placeholder row. From here every anonymous `/api` call is `401`.
5. Land the proxy changes (Task 20) and smoke them: `curl -i https://<host>/auth/me` must return the backend's `401` JSON, not `index.html`. In dev, the same URL through the Vite port must behave identically.
6. Verify login, logout, and `logout-all` (by `curl -X POST`; no UI calls it), and that an anonymous `curl` to any `/api` path returns `401`. Discovery is lazy, so a bad `OIDC_ISSUER` fails at login rather than at launch and this step is reversible by env alone.
7. Have the identity named by `BOOTSTRAP_OWNER_SUBJ` sign in once. The placeholder row becomes their row and the legacy portfolios come with it. Enable Authentik self-registration only after this step.

**Rollback:** there is no env-only rollback. `AUTH_MODE=disabled` is not compiled into the release image, so a bad auth change is rolled back by image revision, not by flipping a variable. The added column and tables are additive and unused by the previous code, so no schema rollback is needed. `ROCKET_SECRET_KEY` must be stable across restarts; rotating it invalidates all sessions, which is a forced re-login, not data loss.

## Out of scope (do not add)

- `portfolio_members`, share endpoints, "shared with me", the `access` field on the portfolio response — **Spec 2**, which is the first spec with a write path for membership.
- API keys and the `/mcp` server — **Spec 3**.
- MCP OAuth 2.1 with dynamic client registration — **Spec 4**.
- Rate limiting `/api/assets/lookup` — **issue #35**; it protects a third-party quota, not user data.
- `PRAGMA foreign_keys = ON` — a separate migration, because it fails on any pre-existing dangling row.
- Cleaning up transactions orphaned by asset deletion — needs its own migration.
- Any user-management UI — Authentik owns accounts, registration, passwords, MFA and the admin console.
