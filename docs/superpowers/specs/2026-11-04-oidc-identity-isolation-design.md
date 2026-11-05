# OIDC Identity & Per-User Data Isolation — Design Document

| Field | Value |
|---|---|
| Date | 2026-11-04; revised 2026-11-05 after review |
| Status | Proposed |
| Scope | Spec 1 of 4 (see [Deferred Work](#deferred-work)) |
| Review thread | `reviews/2026-11-04-oidc/` (9 reviewer passes, consolidated in `consolidated_review_report.md`) |
| Code claims | Re-verified against the working tree on 2026-11-05; see [Current State](#current-state) |

Contents: [Overview](#overview) · [Requirements](#requirements) · [Non-Goals](#non-goals) ·
[Current State](#current-state) · [Architecture](#architecture) ·
[Decisions and Alternatives Considered](#decisions-and-alternatives-considered) ·
[Risks and Mitigations](#risks-and-mitigations) · [Backend Changes](#backend-changes) ·
[Configuration & Deployment](#configuration--deployment) ·
[OpenAPI, CI & Frontend Impact](#openapi-ci--frontend-impact) · [Testing Strategy](#testing-strategy) ·
[Rollout & Rollback](#rollout--rollback) · [Security Considerations](#security-considerations) ·
[Deferred Work](#deferred-work) · [Known Limitations](#known-limitations) ·
[Open Questions](#open-questions) · [Revision History](#revision-history)

## Overview

Portfolio Prism currently has no authentication of any kind: every route under `/api` is reachable
by anyone who can reach the host, and `portfolios`, `assets` and `transactions` have no owner, so
every caller sees the same data. This design adds OpenID Connect (OIDC) authentication against an
**existing Authentik instance that is already operated**, stores a local identity record per
authenticated person, and enforces **per-user data isolation**: no caller can read or mutate a
portfolio, asset, or transaction outside their own granted access.

The OpenAPI document is public at `/api-docs/openapi.json` and **stays public by decision** — the API
contract is not a secret, and isolation must not depend on route secrecy
([Alternatives](#decisions-and-alternatives-considered)). What this spec changes is access to data,
not visibility of the contract. (No Swagger UI is mounted today — `utoipa_swagger_ui` is a dependency
but is never used in `build_rocket`.)

The isolation half is the substance of this spec. With untrusted users and sequential integer
primary keys, ownership checks written per query are a leak waiting to happen: twelve separate
statements today select, update, or delete a portfolio, asset, or transaction by caller-supplied
primary key (`analytics.rs:26,112`; `portfolios.rs:158,196,204,236,257`;
`transactions.rs:103,112,146,174,283`). The design therefore makes one module the only way to
resolve a portfolio, asset, or transaction by a caller-supplied id, and makes the cross-user test
matrix the main authorization gate.

Authentik owns user accounts, registration, passwords, MFA, and the admin user-management console.
This application stores one row per identity and nothing else. **There is no user-management UI in
this application.**

Terminology used consistently below:

| Term | Meaning in this document |
|---|---|
| `sub` | The OIDC subject claim in the ID token. The `users` column that stores it is named `subj` because `sub` is a SQL keyword in some dialects. |
| self-registration | The Authentik setting that lets a person create their own account without an administrator. The security argument about `email` claims depends on this one setting. |
| build mode | Debug or release, fixed at compile time (`cfg!(debug_assertions)`). Rocket's config profile is a separate, runtime-settable concept; the two are not the same axis. |

## Requirements

Requirement identifiers are stable; later documents and test names refer to them by these ids.

### Functional

| ID | Requirement |
|---|---|
| F1 | Every request under `/api` requires an authenticated identity; unauthenticated requests get `401`. Enforced at the request lifecycle (a fairing on that prefix), not per handler. |
| F2 | Authentication uses the OIDC authorization code flow with Proof Key for Code Exchange (PKCE) against the existing Authentik deployment. This application never handles a password. |
| F3 | A local `users` row is created just-in-time (JIT) on the first successful OIDC login, keyed on `(issuer, subj)`. |
| F4 | Every portfolio has exactly one owner. Pre-existing portfolios are assigned to a designated bootstrap owner during migration. The application refuses to start if unowned portfolios exist and no bootstrap owner can be determined. |
| F5 | A caller cannot read or mutate a portfolio, asset, or transaction outside their granted access level (owner, editor, or viewer). The response for any resource id outside the caller's access is `404`, never `403`. |
| F6 | `GET /api/portfolios/` returns the portfolios the caller owns plus those with a `portfolio_members` row for the caller. The members table is never written in this spec, so "member of" is the empty set here. The response shape does not change in this spec. |

Two items that were requirements in an earlier draft are not requirements here: `/api-docs/openapi.json`
stays public, and `/api/assets/lookup` stays unthrottled. See
[Decisions and Alternatives Considered](#decisions-and-alternatives-considered) and
[Revision History](#revision-history).

### Operational

| ID | Requirement |
|---|---|
| O1 | Kubernetes liveness and readiness probes continue to succeed without credentials. The `wait-for-backend` initContainer is removed by this spec: as written it cannot succeed (see [Current State](#current-state)). |
| O2 | `cargo test` and `npm test` pass with no OIDC provider reachable. |
| O3 | `./start.sh` continues to work for local development with no OIDC configuration and no secret key. |
| O4 | CI regenerates the OpenAPI document without starting a server, so the published API contract does not depend on auth mode, a database, or a secret key. |

### Forward compatibility

| ID | Requirement |
|---|---|
| C1 | The access layer is written against the final access model (owner / editor / viewer) even though sharing is not delivered until Spec 2. |
| C2 | The guard accepts a session cookie only. Bearer tokens are Spec 3; an `Authorization` header on an API request is ignored, not accepted. |

## Non-Goals

| Out of scope here | Where it belongs |
|---|---|
| Sharing portfolios (UI, share/unshare endpoints, "shared with me"). The `portfolio_members` table **is** created here and read by the access layer; nothing writes to it. | Spec 2 |
| Per-user API keys and the Model Context Protocol (MCP) server. | Spec 3 |
| MCP OAuth 2.1 with dynamic client registration for hosted connectors (ChatGPT, Claude.ai). | Spec 4 |
| Roles and admin panels (application-wide privilege levels, distinct from the per-portfolio membership role in `portfolio_members.role`). | No planned spec |
| Rate limiting `/api/assets/lookup` (the shared Yahoo Finance / EODHD quota). The design is written up and tracked separately. | [Issue #35](https://github.com/javierarrieta/portfolio-prism/issues/35) |
| Per-user quotas of any kind. | No planned spec |
| Syncing `is_active` from Authentik. The column exists so a future sync can populate it. | No planned spec |
| Replacing SQLite, or changing `replicas: 1`. | No planned spec; see [Known Limitations](#known-limitations) |
| Cleaning up the transaction rows orphaned by asset deletion. | No planned spec; see [Backend Changes §6](#6-schema-migration) |

## Current State

Every row below was re-checked against the working tree on 2026-11-05.

| Fact | Location |
|---|---|
| No auth anywhere; the only `Authorization` string is in the CORS allowed headers, and `allow_credentials` is already `true` | `backend_rust/src/lib.rs:53-54` |
| No `securitySchemes` in the published spec; 10 paths / 13 operations | `docs/openapi/openapi.json` |
| The mounted surface is 14 API operations. `PATCH /api/portfolios/<pid>/assets/<aid>` **does** carry `#[utoipa::path]` (`transactions.rs:88-95`); it is missing from the `paths(...)` list in `openapi.rs:8-24`, which is why the published spec has 13 | `lib.rs` (`build_rocket`), `openapi.rs`, `api_routes/transactions.rs:88-95` |
| `openapi_json` is mounted twice: `/openapi.json` and `/api-docs/openapi.json`. Nothing in the repo or the frontend fetches the root alias | `lib.rs:252,271` |
| CI generates the spec by starting the server and curling it: `cargo run --bin backend_rust &` → `sleep 10` → `curl http://127.0.0.1:8000/api-docs/openapi.json` → `diff`. No environment variables are set | `.github/workflows/ci.yml:22-36` |
| `AGENTS.md` documents the same procedure with `cargo run --release` | `AGENTS.md:24-27` |
| Rocket 0.5 refuses to launch when no `secret_key` is configured **and the config profile is not `debug`** (`ErrorKind::InsecureSecretKey`); in profile `debug` it generates a temporary key. `k8s/` sets no `ROCKET_*` variable, so the release image resolves to profile `release` | Rocket 0.5 `Rocket::ignite()`; `k8s/configmap.yaml`, `k8s/app-deployment.yaml` |
| `portfolios` / `assets` / `transactions` have no owner column; `portfolios.id` is `INTEGER PRIMARY KEY AUTOINCREMENT` — enumerable | `lib.rs` (`init_db`) |
| Startup migrations already exist, including `column_exists` (a read-only `PRAGMA table_info` probe) and `migrate_one_column`, a private crash-recovery helper whose every branch emits `ADD COLUMN <col> TEXT NOT NULL DEFAULT '0'` and whose state machine keys on the literal string `"TEXT"`. It cannot emit an `INTEGER` column | `lib.rs:141-200`, pinned by `tests/decimal_migration_test.rs` |
| Foreign keys are never enabled (`PRAGMA foreign_keys` is off), so `REFERENCES` clauses are documentation, not constraints | `lib.rs` (`init_db`) |
| `delete_asset` deletes only the asset row. `transactions` declares `FOREIGN KEY (asset_id) REFERENCES assets(id)` with no `ON DELETE CASCADE`, so deleting an asset leaves transaction rows with a dangling `asset_id` | `api_routes/transactions.rs:141-155`, `lib.rs:108` |
| `PATCH /api/portfolios/:id` updates `currency` / `base_currency`. There is no portfolio-rename endpoint | `api_routes/portfolios.rs:172-208` |
| Both nested write routes are already portfolio-scoped in SQL: `update_asset` runs `UPDATE assets … WHERE id = ? AND portfolio_id = ?` plus a portfolio-scoped `SELECT`, and `create_transaction` gates on `SELECT * FROM assets WHERE id = ? AND portfolio_id = ?` | `transactions.rs:103-127,169-177` |
| Tests build the app with `build_rocket(pool, currency_service, cors)` and `init_db(&pool)`; `seed_portfolio` inserts without an owner | `tests/common/mod.rs:7,13,22` |
| `build_rocket` returns `Rocket<Build>` (no error path); `init_db` returns `Result<(), sqlx::Error>` and is called from `main.rs:26` | `lib.rs:247,59`, `main.rs:26` |
| The `wait-for-backend` initContainer curls `http://127.0.0.1:8000/`, but initContainers run before the backend container starts, so nothing is listening: the loop exhausts and the container exits 1. Backend liveness/readiness probes hit `GET /` on `:8000`; frontend probes hit `:80`; the Service exposes only port 80 | `k8s/app-deployment.yaml` |
| nginx `location /` is `try_files $uri $uri/ /index.html`, so `/auth/login` today returns the SPA shell. The Vite proxy lists only `/api` and `/api-docs` | `frontend/nginx.conf`, `frontend/vite.config.js` |
| 14 raw `fetch` calls in `App.tsx`, plus calls in `CsvImportModal.tsx` and `PortfolioDetail.tsx`, none with status-code handling; no fetch wrapper exists in `src/utils/` | `frontend/src/App.tsx`, `frontend/src/utils/` |
| `reqwest` is rustls-only (`default-features = false`, `rustls-tls`); `Cargo.lock` already contains both `rand 0.8.6` and `rand 0.9.4` | `backend_rust/Cargo.toml`, `Cargo.lock` |
| Build image is `rust:1.92-slim`; local toolchain 1.98.1 | `backend_rust/Dockerfile` |

Same-origin in both prod and dev is load-bearing for this design: same-origin is what makes a
cookie session practical, and a cookie session is what keeps tokens out of the browser — no token
storage, no refresh logic, no new frontend authentication dependency.

## Architecture

```text
                        TARGET STATE — the /auth/* branch does not exist today

Browser ──HTTPS──▶ nginx (frontend container, :80)
                    ├── /           static SPA (try_files → index.html)
                    ├── /api/*      proxy 127.0.0.1:8000 ─┐
                    ├── /api-docs   proxy 127.0.0.1:8000 ─┤ Rocket
                    └── /auth/*     proxy 127.0.0.1:8000 ─┘  (new)
                                  │
                 ┌────────────────┴──────────────────────────────────┐
                 │ AuthMode: Oidc | Disabled (debug builds only)     │
                 │                                                   │
                 │ /auth/login /callback /logout /logout-all /me     │
                 │                  │                                │
                 │                  ▼                                │
                 │ AuthFairing on the /api prefix:                   │
                 │ no valid session → 401, before any handler runs   │
                 │                  │                                │
                 │                  ▼                                │
                 │ FromRequest → AuthedUser                          │
                 │                  │                                │
                 │                  ▼                                │
                 │ access::require(user, portfolio_id, level)        │
                 │ the only path to a portfolio by a caller id       │
                 │                  │                                │
                 │                  ▼                                │
                 │ handlers: portfolios · transactions · analytics   │
                 │                  │                                │
                 │                  ▼                                │
                 │ SQLite: users · portfolios.owner_id ·             │
                 │         portfolio_members (read, never written)   │
                 └────────────────┬──────────────────────────────────┘
                                  │
                    Authentik — outside the cluster, already running
```

Three units, each independently understandable:

| Unit | Responsibility | Depends on |
|---|---|---|
| `src/auth/` | OIDC code flow, session cookie issuance and verification, the `AuthFairing`, the `AuthedUser` guard | `AuthMode`, Rocket cookies, `users` table |
| `src/access.rs` | Map `(user, portfolio_id)` → `Access`, or nothing | `users`, `portfolios`, `portfolio_members` |
| `src/api_routes/*` | Business logic; never resolves ownership themselves | `AuthedUser`, `access` |

## Decisions and Alternatives Considered

This is the complete alternatives register for this spec. Each row names the choice made and the
option that was rejected with its reason.

| Decision | Choice | Rejected alternative, and why |
|---|---|---|
| Where the OIDC flow terminates | Backend (Rocket) | `oauth2-proxy` sidecar — gives no in-app identity, so it cannot enforce isolation; dev diverges from prod |
| SPA token handling | None; backend session cookie | SPA PKCE with `Bearer` — tokens in browser storage, refresh logic in React, no benefit given same-origin |
| Identity provider | Existing Authentik | Self-hosting Keycloak or a second Authentik — a new stateful service for a problem this deployment already solves; Google-only login — incompatible with self-registration |
| Account ownership | Authentik owns accounts; this app does JIT provisioning | App-side registration and passwords — authentication is the component not to hand-roll here |
| Session representation | Rocket private cookie + `users.token_version` | Server-side session table — a write per request and a cleanup job for no gain. The guard reads the `users` row anyway, because `AuthedUser` carries `email` and `name` |
| Non-owned resource response | `404` | `403` — confirms the id exists, which on sequential keys is an existence oracle |
| Ownership enforcement | Single `access` module; `resolve` private; the nested path check lives inside it | Per-handler `WHERE owner_id = ?` — twelve places to forget |
| Authentication enforcement | One `AuthFairing` on the `/api` prefix | Per-handler `AuthedUser` alone — a route that does not take the guard would be public, so F1 would hold only by authoring discipline |
| `Disabled` auth mode | Compiled out of release builds with `cfg!(debug_assertions)` | Runtime Rocket profile allowlist — a runtime string comparison guarding a build-time fact. Rocket 0.5 has no `ROCKET_ENV`, and `!= production` is fail-open on an unset variable; even an allowlist stays fail-open to one ConfigMap edit (`ROCKET_PROFILE=dev` on the release image) |
| `owner_id` nullability | Nullable `INTEGER` column, no table rebuild | `NOT NULL` via a rebuild — `migrate_one_column` cannot emit an `INTEGER` column, so "reuse" would mean generalizing the repo's most fragile, most specifically-tested helper. The invariant is enforced by the startup check and by `access` treating `NULL` as no access |
| Unowned rows at startup | Refuse to start (OIDC mode) | Assigning to an arbitrary user — silently misattributes financial data |
| Legacy-portfolio handover | Keyed on `BOOTSTRAP_OWNER_SUBJ` only | Matching the ID token's `email`, gated by `email_verified` — with self-registration, `email` is a claimable takeover vector, and Authentik does not verify addresses unless separately configured |
| OpenAPI document visibility | Public, as today | Requiring a session on `/api-docs/openapi.json` — the document is a contract, not a secret; nothing in the isolation model depends on route secrecy; and keeping it public leaves Swagger-style tooling and the CI diff working unchanged |
| Lookup rate limit | Out of this spec | A per-user bucket, then a global one — both protect a third-party quota rather than user data, so they belong in their own change. Carried to [issue #35](https://github.com/javierarrieta/portfolio-prism/issues/35) as one global bucket |
| OpenAPI generation | `--print-openapi` early exit; no server | Exporting `AUTH_MODE=disabled` in CI — works around the coupling instead of removing it, and leaves the published contract dependent on runtime configuration and, for a release build, on a secret key |
| Root `/openapi.json` alias | Remove the second mount | Keeping it — it is the same handler mounted twice with no consumer anywhere in the repo or the frontend. This is duplicate-surface cleanup, not access control |
| Three-level access model | Kept (C1), but the public response shape does not change in this spec | Shipping a new `access` field on `GET /api/portfolios/` now — no consumer exists until Spec 2's share UI, and an unpinned public field would drift |
| Revocation (`token_version`, `is_active`) | Kept; the guard already reads the row | Dropping them — saves nothing, since the row is read for `AuthedUser`, and adding the columns later is another migration against live data |
| Bootstrap handover mechanism | One placeholder row (`issuer='local'`, `subj='bootstrap'`) updated in place on handover | A `bootstrap:<value>` row per configured identity — more strings to keep consistent, and the id would change when the identity is provisioned |

## Risks and Mitigations

The full register is [Security Considerations](#security-considerations) and
[Known Limitations](#known-limitations); this is the short list a reviewer needs first.

| Risk | Mitigation | Detail |
|---|---|---|
| Sequential ids plus `403` would let a caller enumerate other users' data | `404` for everything outside the caller's access | [Enumeration](#security-considerations) |
| A future handler that writes its own `WHERE id = ?` silently breaks isolation | `resolve` is private; the exported functions return `Status`; a CI grep gate flags the three table names in handler files | [Chokepoint enforcement](#7-module-srcaccessrs) |
| A forgotten environment variable exposes every portfolio | `AuthMode::Disabled` is not compiled into the release binary; `AUTH_MODE=disabled` there is a startup error | [Disabled mode](#2-auth-mode-oidc-or-disabled) |
| A startup refusal turns a typo into downtime on a single replica | The three exit conditions are listed with their exact triggers, and rollout orders them so each is exercised before the next | [Rollout & Rollback](#rollout--rollback) |
| A legacy portfolio is handed to the wrong person | Handover is keyed on `sub` only, and self-registration stays off until the handover completes | [Legacy ownership handover](#security-considerations) |
| The API contract is public, so every route and payload shape is known to any caller | Isolation never depends on route secrecy: every `/api` route needs a session, and ids outside the caller's access are `404` | [Public API contract](#security-considerations) |
| `/api/assets/lookup` is unthrottled against a shared third-party quota | Out of this spec; designed and tracked in issue #35 | [Non-Goals](#non-goals) |

## Backend Changes

### 1. Module layout and dependencies

New files: `src/auth/mod.rs`, `src/auth/oidc.rs`, `src/auth/session.rs`, `src/auth/guard.rs`,
`src/auth/fairing.rs`, `src/access.rs`.

**Dependency added** to `backend_rust/Cargo.toml`: `openidconnect = "4.1"`. Verified against the
crates.io index: 4.1.0 exists, its MSRV is 1.88 (the build image is `rust:1.92-slim`, local
toolchain 1.98.1), and its default features are `reqwest` + `rustls-tls`, so it uses rustls rather
than pulling `native-tls` into a deliberately rustls-only build. It depends on `oauth2` 5.0, which
requires `rand ^0.8`; `Cargo.lock` already resolves `rand 0.8.6` alongside `rand 0.9.4`, so no
`rand` version conflict is introduced and none needs to be avoided. **Implementation step:** run
`cargo tree -d` and `cargo tree -e features -p openidconnect` after adding it, and record the
result in the PR description; the TLS unification claim above is from the index metadata, not from
a resolved build.

**Rocket feature change required:** `rocket = { version = "0.5", features = ["json"] }` becomes
`features = ["json", "secrets"]`. Private cookies and `rocket::config::SecretKey` are
`#[cfg(feature = "secrets")]` in Rocket 0.5; without this the session mechanism in §4 does not
compile.

No `rand` dependency is added directly: `openidconnect` provides `CsrfToken::new_random`,
`Nonce::new_random`, and `PkceCodeChallenge::new_random_sha256`.

### 2. Auth mode: `Oidc` or `Disabled`

```rust
#[derive(Clone)]
pub enum AuthMode {
    Oidc(OidcConfig),
    /// Debug builds only. Not present in a release binary at all.
    #[cfg(debug_assertions)]
    Disabled { as_user: i32 },
}

pub struct AuthedUser { pub id: i32, pub email: Option<String>, pub name: Option<String>, pub sub: String }
```

`AuthMode::Disabled` is gated at compile time, not at runtime. In a release binary the variant does
not exist, so no forgotten `ROCKET_*` variable, no ConfigMap drift, and no `AUTH_MODE` typo can
enable it. `AUTH_MODE=disabled` in a release build is a **startup error** that names the variable
and the build mode; it is not silently ignored. Debug builds honour it and print a startup banner
naming the user id being assumed.

Rocket's own launch check is separate and is keyed on the config **profile name**, not the build
mode: with the `secrets` feature, Rocket refuses to launch when no `secret_key` is configured and
the profile is not `debug` (`ErrorKind::InsecureSecretKey`). A debug build with no key gets a
temporary one, which is why `./start.sh` needs none — and why dev sessions do not survive a restart.
That is acceptable in dev and is recorded under [Known Limitations](#known-limitations).

`AuthMode` is resolved in `main.rs` and passed to both `init_db` and `build_rocket`; the guard reads
it from managed state and never reads the environment itself.

### 3. Startup wiring and signatures

The two functions that need the new configuration have no place for it today, and one of them has no
error path. The signatures this spec introduces:

```rust
pub struct StartupConfig {
    pub auth: AuthMode,
    pub bootstrap_subj: Option<String>,
}

/// Returns Err on any startup-refusing condition. Never calls process::exit itself.
pub async fn init_db(pool: &SqlitePool, cfg: &StartupConfig) -> anyhow::Result<()>;

pub fn build_rocket(
    pool: SqlitePool,
    currency_service: CurrencyService,
    cors: Cors,
    auth: AuthMode,
) -> Rocket<Build>;
```

`AuthMode` is also `.manage()`d so the guard and the fairing can read `&State<AuthMode>`.
`build_rocket` keeps its infallible `Rocket<Build>` return type: Rocket's launcher surfaces launch
failures (including `InsecureSecretKey`) on its own.

**Where the process-exit decision lives:** `main.rs`. `init_db` returns `Result`; `main` logs the
error and calls `std::process::exit(1)`. Keeping `exit` out of `init_db` is what makes "startup
refuses" testable — integration tests call `init_db` and assert `Err`.

`main.rs` also stops being a `#[launch]` function so it can handle `--print-openapi` before
touching the database or Rocket (see [OpenAPI, CI & Frontend Impact](#openapi-ci--frontend-impact)):

```rust
fn main() {
    if std::env::args().any(|a| a == "--print-openapi") {
        print!("{}", ApiDoc::json());
        return;                       // no DB, no Rocket, no secret key, no network
    }
    rocket::custom(/* … */).launch(); // former #[launch] body
}
```

Order of operations in `main.rs`, which resolves the dev seeding question:

1. read env into `StartupConfig` (`AUTH_MODE`, `OIDC_*`, `BOOTSTRAP_OWNER_SUBJ`);
2. connect the pool;
3. `init_db(&pool, &cfg)` — creates tables, adds `owner_id`, runs the backfill; on `Err`, exit 1;
4. resolve `as_user` for `Disabled` mode by reading the placeholder row created in step 3;
5. `build_rocket(pool, currency_service, cors, auth)`.

`tests/common/mod.rs` gains the matching helpers, and the existing ones change signature:
`build_rocket(pool, as_user)` wraps `AuthMode::Disabled`, `setup_db(cfg)` passes a
`StartupConfig`, `seed_user(pool, subj) -> i32` creates a caller, and
`seed_portfolio(pool, owner_id, name, currency) -> i32` gains the owner it currently omits.

### 4. OIDC endpoints and session cookies

| Method | Path | Behaviour |
|---|---|---|
| `GET` | `/auth/login` | Generate `state`, `nonce`, PKCE verifier; store all three in one short-lived private cookie; `302` to the Authentik authorization URL with `scope=openid profile email`. Mounted in OIDC mode only |
| `GET` | `/auth/callback` | Verify `state` matches the cookie, verify `nonce` in the ID token, exchange `code` at the token endpoint, validate the ID token (JWKS signature, `iss`, `aud`, `exp`, `nonce`), resolve or provision the `users` row, clear the transient cookie, issue the session cookie, `302` to `/` |
| `GET` | `/auth/logout` | Clear the session cookie; `302` to Authentik's `end_session_endpoint` with `post_logout_redirect_uri=https://<host>/` — that URI must be registered on the Authentik provider, alongside `/auth/callback` |
| `POST` | `/auth/logout-all` | Bump `users.token_version`, invalidating every outstanding session cookie for this user, then clear this one. No UI calls it in this spec; the endpoint exists so revocation is exercisable and testable |
| `GET` | `/auth/me` | `200` with `{ id, email, name, sub }` for the authenticated caller; `401` otherwise. Mounted in **both** modes, so the SPA works unchanged under `Disabled` |

`/auth/*` is not under `/api`, so it is not part of the `/api` OpenAPI surface. `/auth/me` is the
endpoint the SPA uses to learn it is signed in; `/auth/login` is reached through the `401` redirect
in `apiFetch`. It reports `AuthedUser.sub`, which is what the operator reads to fill in
`BOOTSTRAP_OWNER_SUBJ`.

**Cookies.** Both are Rocket **private** cookies (encrypted and signed with `ROCKET_SECRET_KEY`;
needs the `secrets` feature from §1), `HttpOnly`, `Secure`, `SameSite=Lax`, path `/`:

| Name | Payload (JSON) | Max-Age |
|---|---|---|
| `pp_session` | `{"uid": <users.id>, "tv": <token_version>}` | 8 hours |
| `pp_oauth_state` | `{"state": …, "nonce": …, "verifier": …}` | 10 minutes |

Distinct names, distinct lifetimes, both private, so the transient cookie cannot clobber the session
cookie and no serialization integrity scheme is needed beyond Rocket's own encryption.

`Secure` is set unconditionally, and there is no separate "production" concept behind it. Keying the
flag on the request scheme would be wrong here: TLS terminates at the operator's ingress, nginx
receives plain HTTP, and `X-Forwarded-Proto $scheme` therefore reads `http` even for an HTTPS
request — the flag would be dropped in the one environment that requires it. The plain-HTTP case
that does exist, `./start.sh`, runs in `Disabled` mode, which issues no cookie at all; running the
OIDC flow over plain HTTP is unsupported.

`SameSite=Lax` prevents a cross-site form `POST` from carrying the cookie, which is the CSRF
exposure for a cookie-authenticated API. No separate CSRF token is introduced, because the API is
same-origin in both prod and dev. CORS is not a CSRF defence — it governs whether a cross-origin
script can read a response — and `get_cors_options` already sets `allow_credentials: true` against
an exact-origin list, so adding an origin to that list is itself a change that needs review. If
credentialed cross-origin access is ever enabled, a CSRF token becomes mandatory and this paragraph
must be revisited.

**Discovery timing.** Provider metadata (`OIDC_ISSUER` → `/.well-known/openid-configuration`, then
JWKS) is fetched **lazily on the first `/auth/login`** and cached for the process lifetime in a
`OnceLock`. Nothing about startup depends on Authentik being reachable, which is what makes O2 true
and keeps a mistyped `OIDC_ISSUER` from CrashLooping the pod: the failure surfaces at login, not at
launch. Rollout step 4 is therefore reversible by env alone.

**Browser versus non-browser failures.** The branch is keyed on the request's `Accept` header: a
request whose `Accept` includes `text/html` gets `302 /?auth_error=auth_failed`; anything else gets
the `401` JSON described in §9. `/` accepts the `auth_error` query parameter as a display hint
only; the only value this spec emits is `auth_failed`, and no IdP detail is ever reflected.

### 5. Session validation and revocation

On every request under `/api`, the fairing resolves the caller once:

1. decrypt the `pp_session` cookie;
2. load `users` by `uid`;
3. reject if the row is missing, `is_active = 0`, or `token_version` differs from the cookie;
4. stash the resulting `AuthedUser` in the request's local state, which is what
   `AuthedUser: FromRequest` reads, so the row is loaded once per request. (If `local_state`
   proves unusable in Rocket 0.5.x, the guard resolves independently and the extra read is
   accepted.)

`token_version` is the revocation switch: bumping it invalidates every outstanding cookie for that
user without a session table, and `POST /auth/logout-all` performs the bump. `is_active` is read by
the guard but never written in this spec — see [Security Considerations](#security-considerations).

In `Disabled` mode the fairing resolves to `as_user` without touching cookies, and no `/api` route
is public.

### 6. Schema migration

Added to `init_db` in `backend_rust/src/lib.rs`, following the existing startup-migration style.
`ALTER TABLE … ADD COLUMN` is not idempotent in SQLite, so the add is guarded by the existing
read-only `column_exists` helper:

```sql
CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    issuer TEXT NOT NULL,
    subj TEXT NOT NULL,
    email TEXT,
    name TEXT,
    is_active INTEGER NOT NULL DEFAULT 1,
    token_version INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    last_login_at TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS uq_users_issuer_subj ON users(issuer, subj);

-- guarded by column_exists(pool, "portfolios", "owner_id")
ALTER TABLE portfolios ADD COLUMN owner_id INTEGER;

CREATE TABLE IF NOT EXISTS portfolio_members (
    portfolio_id INTEGER NOT NULL REFERENCES portfolios(id),
    user_id INTEGER NOT NULL REFERENCES users(id),
    role TEXT NOT NULL CHECK (role IN ('editor','viewer')),
    added_at TEXT NOT NULL,
    PRIMARY KEY (portfolio_id, user_id)
);
CREATE INDEX IF NOT EXISTS idx_portfolio_members_user ON portfolio_members(user_id);
```

`portfolio_members` is created here and read by the access layer and by `GET /api/portfolios/`, but
nothing writes to it in this spec: it lets `access` be written once against the final access model
instead of being rewritten in Spec 2.

**`owner_id` stays nullable.** `ALTER TABLE portfolios ADD COLUMN owner_id INTEGER` is a plain add:
no rename, no copy, no drop. A `NOT NULL` column would need a table rebuild, and the existing
`migrate_one_column` helper cannot produce one — every branch emits
`ADD COLUMN <col> TEXT NOT NULL DEFAULT '0'` and its crash-recovery state machine keys on the
literal string `"TEXT"`. Generalizing it would mean editing the repo's most fragile, most
specifically-tested helper (`tests/decimal_migration_test.rs` pins its behaviour) to serve one new
caller. The invariant that matters is enforced in two other places: the startup check below refuses
to boot while orphans exist, and `access::resolve` treats a `NULL` owner as no access.

**Foreign keys are not enforced.** Nothing in this repository issues `PRAGMA foreign_keys = ON`
(SQLite's default is off), so the `REFERENCES` clauses above and in the existing `assets` /
`transactions` tables are documentation, not constraints: a dangling `owner_id` is possible, and
F4 is enforced by application code. This spec does not change that; enabling FK enforcement is a
separate migration, because it would fail on any pre-existing dangling row.

**Synthetic rows.** Every `users` row this design creates without an identity provider is the same
single placeholder row, so there is exactly one set of literal values to get right:

```sql
INSERT INTO users (issuer, subj, email, name, created_at)
VALUES ('local', 'bootstrap', NULL, 'Placeholder owner', '2026-11-05T00:00:00+00:00')
ON CONFLICT (issuer, subj) DO NOTHING;
```

`issuer = 'local'` is a sentinel that is not a URL, so it cannot collide with a real issuer.
`email` is nullable precisely so a synthetic row does not need a fabricated address. `created_at` is
`chrono::Utc::now().to_rfc3339()` in Rust, not a SQL literal; the literal above shows the format.
A real JIT row is inserted with the ID token's own values:

```sql
INSERT INTO users (issuer, subj, email, name, created_at, last_login_at)
VALUES (:iss, :sub, :email, :name, :now, :now);
```

**Backfill.** On startup, `SELECT COUNT(*) FROM portfolios WHERE owner_id IS NULL`:

| Mode | Condition | Action |
|---|---|---|
| `Disabled` | orphans present | Upsert the placeholder row and assign every orphan to it; log the count at `warn` |
| `Oidc` | orphans present, `BOOTSTRAP_OWNER_SUBJ` set | Upsert the placeholder row and assign every orphan to it; log the count at `warn` |
| `Oidc` | orphans present, `BOOTSTRAP_OWNER_SUBJ` unset | `Err` naming the count and the variable → `main` exits 1 |
| either | no orphans | nothing |

Refusing to start is deliberate: silently assigning financial records to the wrong person is worse
than downtime. In `Disabled` mode there is no refuse-to-start path, which is what keeps `./start.sh`
working against a database that already has portfolios (O3).

**Placeholder reconciliation.** A real identity takes over the placeholder row only when the ID
token proves the match:

1. resolve `(issuer, subj)` from the validated ID token. If a row matches, update its `email`,
   `name`, and `last_login_at`, and use it.
2. Otherwise, if `BOOTSTRAP_OWNER_SUBJ` equals the token's `sub` and the placeholder row exists,
   `UPDATE` that row in place to the real `(issuer, subj, email, name, last_login_at)`. The id is
   preserved, so every portfolio it owns moves with it.
3. Otherwise insert a new row, and log `provisioned user id=… issuer=… sub=…` at `info` — that log
   line is how the operator finds their own `sub` to put in `BOOTSTRAP_OWNER_SUBJ`.

If both a real row and the placeholder exist, step 1 wins and the placeholder is left alone with a
`warn`; reassigning those portfolios is then a manual SQL step
([Known Limitations](#known-limitations)).

`email` is never used to decide ownership. With self-registration enabled, an attacker who
registered the bootstrap address before its real owner would otherwise be handed every pre-existing
portfolio, and `email_verified` is only as trustworthy as Authentik's configuration — the provider
does not verify addresses unless separately configured.

### 7. Module `src/access.rs`

```rust
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Access { Viewer, Editor, Owner }   // Ord gives Viewer < Editor < Owner

/// Err(Status::NotFound) when the caller does not hold `min`.
pub async fn require(pool: &SqlitePool, user_id: i32, portfolio_id: i32, min: Access)
    -> Result<Access, rocket::http::Status>;

/// Resolve the asset → portfolio chain, then require().
pub async fn require_asset(pool: &SqlitePool, user_id: i32, asset_id: i32, min: Access)
    -> Result<Access, rocket::http::Status>;

/// As require_asset, and additionally reject when the asset's real portfolio is not the one named
/// in the request path. Use this for routes nested under /api/portfolios/<id>/….
pub async fn require_asset_in(pool: &SqlitePool, user_id: i32, path_portfolio_id: i32,
    asset_id: i32, min: Access) -> Result<Access, rocket::http::Status>;

/// Resolve transaction → asset → portfolio, then require().
pub async fn require_transaction(pool: &SqlitePool, user_id: i32, tx_id: i32, min: Access)
    -> Result<Access, rocket::http::Status>;

/// Private. None ⇒ no access, including a NULL owner_id.
async fn resolve(pool: &SqlitePool, user_id: i32, portfolio_id: i32)
    -> Result<Option<Access>, sqlx::Error>;
```

There is no `AuthError` type. Handlers in this repository already return `Result<T, Status>`, and
`Status::NotFound` and `Status::Unauthorized` express both outcomes, so an enum with an
`IntoResponse` impl would be a second error vocabulary for no gain.

Resolution queries, stated so the dangling-row cases are not left to the implementer:

```sql
-- resolve(): owner first, then membership
SELECT owner_id FROM portfolios WHERE id = ?;
SELECT role FROM portfolio_members WHERE portfolio_id = ? AND user_id = ?;

-- require_asset(): the asset's real portfolio
SELECT portfolio_id FROM assets WHERE id = ?;

-- require_asset_in(): the path id must equal the value returned above

-- require_transaction(): transaction → asset → portfolio
SELECT asset_id FROM transactions WHERE id = ?;
SELECT portfolio_id FROM assets WHERE id = ?;
```

A transaction whose `asset_id` matches no asset row — reachable today, because asset deletion
orphans transactions and foreign keys are off — resolves to `Status::NotFound`.

`resolve` is private and the exported functions return `Status`, so a handler cannot ignore a
"no access" answer and keep going. This is the **only** module permitted to look up a portfolio,
asset, or transaction by caller-supplied id; handlers must not contain `WHERE id = ?` against those
tables. The rule is checked, not assumed: CI runs a grep gate over `src/api_routes/` that fails on
`FROM (portfolios|assets|transactions) WHERE id` and on `UPDATE`/`DELETE` against those tables with
a bare `WHERE id`. The gate is a heuristic — a join or a subquery satisfies the pattern-free case —
so it supplements the isolation matrix rather than replacing it.

**Access level semantics.** Ordering is `Owner > Editor > Viewer`.

| Level | May |
|---|---|
| `Viewer` | read the portfolio, its assets and transactions, its tax summary and its performance |
| `Editor` | additionally add an asset, and add or delete transactions |
| `Owner` | additionally delete the portfolio, change its base currency, and delete an asset |

The rule being applied: `Editor` may add and amend, `Owner` alone may destroy structure. Deleting an
asset is owner-only as a policy choice. It is **not** a cascade safeguard: `delete_asset` deletes
only the asset row, `transactions` has no `ON DELETE CASCADE`, and foreign keys are off, so today
deleting an asset leaves its transaction rows with a dangling `asset_id`. This spec does not change
that behaviour. The orphans are unreachable through the API — every transaction is read through its
asset — and are not deleted, so no data is lost. Cleaning them up (a `ON DELETE CASCADE` rebuild, or
a delete-then-asset order) is a separate change with its own migration.

### 8. Route changes

| Route | Change |
|---|---|
| `POST /api/portfolios/` | `owner_id` bound to `user.id`; insert includes the column |
| `GET /api/portfolios/` | `WHERE owner_id = ? OR id IN (SELECT portfolio_id FROM portfolio_members WHERE user_id = ?)`. Response shape unchanged |
| `GET /api/portfolios/:id` | `require(…, Viewer)` → `404` if not held |
| `PATCH /api/portfolios/:id` | `require(…, Owner)` — this route sets `currency` / `base_currency`; there is no rename endpoint |
| `DELETE /api/portfolios/:id` | `require(…, Owner)` |
| `GET /api/portfolios/:id/tax-summary` | `require(…, Viewer)` |
| `GET /api/portfolios/:id/performance` | `require(…, Viewer)` |
| `POST /api/portfolios/:id/assets` | `require(…, Editor)` |
| `PATCH /api/portfolios/:id/assets/:aid` | `require_asset_in(…, Editor)` — the path-versus-asset check lives in `access` |
| `DELETE /api/assets/:id` | `require_asset(…, Owner)` — no portfolio id in the path, so nothing to cross-check |
| `POST /api/portfolios/:id/assets/:aid/transactions` | `require_asset_in(…, Editor)` |
| `GET /api/portfolios/:id/transactions` | `require(…, Viewer)` |
| `DELETE /api/transactions/:id` | `require_transaction(…, Editor)` |
| `GET /api/assets/lookup` | authenticated. Unthrottled in this spec — see [issue #35](https://github.com/javierarrieta/portfolio-prism/issues/35) |
| `GET /` | **unchanged, public** — probes depend on it |
| `GET /api-docs/openapi.json` | **unchanged, public** — the contract is not a secret; see [Decisions](#decisions-and-alternatives-considered) |
| `GET /openapi.json` | **removed** — same handler, second mount (`lib.rs:252,271`), no consumer |

The nested routes already scope their SQL by portfolio (`update_asset` and `create_transaction`
both filter on `portfolio_id`; see [Current State](#current-state)), so the cross-portfolio write
this table guards against is not possible today. Moving the check into `require_asset` keeps that
behaviour, removes it from the handler, and makes it automatic for nested routes added later. The
cross-portfolio case stays in the isolation matrix as a regression guard.

The table is derived from the `routes![]` lists in `build_rocket` (`lib.rs:247-271`), which is 14
API operations. The published spec documents 13 because `update_asset` is missing from the
`paths(...)` list in `openapi.rs`, not because it lacks an annotation — the annotation is already
there (`transactions.rs:88-95`). The fix is to register the handler in `ApiDoc::paths`.

`Access::Editor` is defined in this spec and exercised by tests, even though no member rows exist
until Spec 2. Owner-only operations are already fixed at `Owner`; Spec 2 adds new routes but does
not change the access level assigned to any route in this table.

### 9. Error handling

| Condition | Response |
|---|---|
| No or invalid session cookie on `/api` | `401` JSON `{ "error": "unauthenticated" }` |
| Valid session, resource outside the caller's access | `404` — never `403` |
| Session valid but `is_active = 0` or `token_version` mismatch | `401` |
| `AUTH_MODE=oidc` with `OIDC_ISSUER`, `OIDC_CLIENT_ID`, `OIDC_CLIENT_SECRET` or `OIDC_REDIRECT_URI` missing | process exits at startup, naming the missing variables — deny-by-default, not a degraded anonymous mode |
| OIDC failure (`state`/`nonce` mismatch, token validation failure, Authentik unreachable or discovery failed during the browser flow on `/auth/callback`) | no session; `Accept: text/html` → `302 /?auth_error=auth_failed`, otherwise `401` JSON `{ "error": "authentication_failed" }`. The specific reason is logged server-side only |
| `AUTH_MODE=disabled` in a release build | process exits at startup, naming the variable and the build mode |
| Orphan portfolios with no bootstrap owner (OIDC mode) | process exits at startup, naming the count and the variable |
| Release-profile launch with no `ROCKET_SECRET_KEY` | Rocket's own launch error, before any route is served |

IdP error text is never reflected into a page or a response body; the only value the client ever
sees is the fixed string `auth_failed`. The existing pattern of collapsing database errors into
`InternalServerError` is retained, but authorization denials are logged at `warn` with user id and
target id so a probing pattern is visible in logs and distinguishable from genuine 500s.

## Configuration & Deployment

| Variable | Purpose | Required |
|---|---|---|
| `OIDC_ISSUER` | Authentik issuer URL from `/.well-known/openid-configuration` | yes when `AUTH_MODE=oidc` |
| `OIDC_CLIENT_ID` | Confidential client id | yes when `AUTH_MODE=oidc` |
| `OIDC_CLIENT_SECRET` | Confidential client secret | yes when `AUTH_MODE=oidc` |
| `OIDC_REDIRECT_URI` | `https://<host>/auth/callback` | yes when `AUTH_MODE=oidc` |
| `ROCKET_SECRET_KEY` | 256-bit base64 key for Rocket private cookies — `openssl rand -base64 32` | yes whenever the Rocket profile is not `debug`; a `debug`-profile launch generates a temporary one |
| `BOOTSTRAP_OWNER_SUBJ` | Authentik `sub` of the owner for pre-existing portfolios; also the handover match | yes if orphans exist under `AUTH_MODE=oidc` |
| `AUTH_MODE` | `oidc` (default) or `disabled`; `disabled` is accepted only in a debug build | no |

`AUTH_MODE` defaults to `oidc`, and a launch with `AUTH_MODE=oidc` and any `OIDC_*` variable missing
refuses to start rather than serving unauthenticated (§9) — that is the state CI would otherwise be
in, and it is why CI must generate the document without a server
([OpenAPI, CI & Frontend Impact](#openapi-ci--frontend-impact)). `./start.sh` exports no variables at
all, so `start.sh` gains `AUTH_MODE=disabled`. It needs no `ROCKET_PROFILE` and no
`ROCKET_SECRET_KEY`: `cargo run` is a debug build in the `debug` profile, where Rocket generates a
temporary key. That key changes on every restart, so a dev session ends when the backend restarts.

**Authentik side (operator task, not code):** create an OAuth2/OIDC provider with client type
*Confidential*, redirect URIs `https://<host>/auth/callback` **and** `https://<host>/` (the
`post_logout_redirect_uri`), scopes `openid profile email`, and bind it to an Application.
Self-registration in Authentik is what makes new accounts work; this application adds no
registration path of its own. Because self-registration makes an `email` claim claimable by anyone,
either set `BOOTSTRAP_OWNER_SUBJ` or complete the bootstrap handover (rollout step 6) before
enabling it.

**Same-origin proxy paths — `/auth` must be routed to the backend.** Login and logout are backend
routes, and neither proxy in front of Rocket forwards them today:

- `frontend/nginx.conf` has `location /` with `try_files $uri $uri/ /index.html`, so a request to
  `/auth/login` today returns `index.html` instead of reaching Rocket. A `location /auth` block
  proxying to `127.0.0.1:8000`, with the same headers as the existing `/api` block, is required.
- `frontend/vite.config.js` proxies only `/api` and `/api-docs`. `/auth` must be added to the proxy
  map or `./start.sh` cannot complete a login in development.

If either is missing, the SPA redirects to `/auth/login` and receives the static app back — a
redirect loop. Both files sit outside both test suites, so CI gains a step that fails unless
`frontend/nginx.conf` contains `location /auth` and `frontend/vite.config.js` contains `'/auth'`.
Rollout step 3 still smokes the path end to end.

**Kubernetes changes** in `k8s/`:

- new `Secret` holding `OIDC_CLIENT_SECRET` and `ROCKET_SECRET_KEY`; the backend container gains
  `envFrom: secretRef` alongside the existing `configMapRef`. The `Secret` must exist **before the
  first rollout step**: the backend image runs in Rocket's `release` profile, and Rocket refuses to
  launch with the `secrets` feature and no `secret_key` in any profile other than `debug`;
- `configmap.yaml` gains `OIDC_ISSUER`, `OIDC_CLIENT_ID`, `OIDC_REDIRECT_URI`,
  `BOOTSTRAP_OWNER_SUBJ`, and `AUTH_MODE`. No `ROCKET_PROFILE` is added: the release image should
  resolve to `release`, and Rocket's own secret-key check is what enforces that the key is present;
- the `wait-for-backend` initContainer is **removed**. It curls `http://127.0.0.1:8000/` before the
  backend container has started, so it can never succeed: the loop exhausts and the container exits
  1, which restarts the pod. The backend's own readiness probe already gates traffic, so the
  container is not needed for the ordering it was written to provide;
- `ingress.yaml` is unchanged — authentication is enforced by the application, not the proxy.

**Probe safety:** `GET /` on port 8000 stays unauthenticated. The backend's liveness and readiness
probes curl that path (the frontend container's probes hit `:80`); authenticating it would stop the
pod from becoming ready. This is asserted by an automated test, not left to review.

**Startup refusals and downtime.** With `replicas: 1` on one PVC, each of the four startup refusals —
no `ROCKET_SECRET_KEY` outside the `debug` profile, orphans with no bootstrap owner,
`AUTH_MODE=disabled` in a release build, `AUTH_MODE=oidc` with an `OIDC_*` variable missing — turns a
configuration typo into an outage. Three of the four are caught by rollout steps 1 and 4 before the
remaining one is attempted, and none of them can be triggered by a code change.

## OpenAPI, CI & Frontend Impact

- **Generation is decoupled from the server.** `main.rs` gains the `--print-openapi` early exit from
  §3. `ApiDoc` is a plain `#[derive(OpenApi)]` struct (`openapi.rs:6`), so the document is a pure
  function of the code and needs no running server. The CI step becomes
  `cargo run -- --print-openapi > current_openapi.json`, which deletes the dummy `portfolio.db`, the
  `sleep 10`, the background process and its `kill`, and any dependence on auth mode or secret key.
  `AGENTS.md`'s documented regeneration procedure is updated to the same command; its
  `cargo run --release` variant would otherwise fail Rocket's secret-key check. This is still
  required after `/api-docs` stayed public: the server CI starts would itself refuse to launch, in
  the default `AUTH_MODE=oidc` with no `OIDC_*` variables set.
- **The published surface becomes complete.** `crate::api_routes::transactions::update_asset` is
  added to `ApiDoc::paths`, so the document covers all 14 API operations instead of 13. The
  annotation already exists on the handler; only the registration is missing.
- utoipa annotations gain a cookie `securityScheme` (`apiKey`, `in: cookie`, name `pp_session`) and
  a `401` response on every `/api` path, plus `404` where it is now the isolation answer. The
  scheme documents what `/api` requires; it says nothing about how the document itself is served,
  because the document stays public. No response shape changes in this spec. `bearerAuth` is
  deliberately **not** declared: this spec authenticates with a session cookie, and a declared bearer
  scheme would make generated clients send an `Authorization` header that the API ignores. Spec 3
  adds `bearerAuth` when bearer tokens exist.
- `docs/openapi/openapi.json` is regenerated with the new command; CI diffs it and fails on drift.
- `npm run generate-types` refreshes `frontend/src/types/api.d.ts`.
- Frontend gains `frontend/src/utils/apiFetch.ts`: wraps `fetch`, and on `401` redirects the
  browser to `/auth/login`. The 14 raw `fetch` calls in `App.tsx` and the calls in
  `CsvImportModal.tsx` and `PortfolioDetail.tsx` are routed through it. JavaScript never reads,
  stores, or sends a token — that is an invariant of the design, not a preference.
- `App.tsx` shows a fixed "sign-in failed" message when the URL carries `?auth_error=auth_failed`,
  and strips the parameter.
- MSW gains a `401` handler so the redirect is covered by a test.
- CORS is unchanged: same-origin in prod and dev. `Authorization` remains in the allowed headers so
  Spec 3's bearer tokens need no later change.

## Testing Strategy

**Backend unit tests**

- `access::resolve` matrix: owner / editor / viewer / none, against each required level, including
  a `NULL` owner_id resolving to no access.
- `Access` ordering.
- `require_asset_in` rejects an asset whose `portfolio_id` differs from the path id; `require_asset`
  accepts the same asset when no path id is given.
- `require_transaction` returns `NotFound` for a transaction whose `asset_id` matches no asset.
- session cookie round-trip; tampered cookie rejected; a cookie carrying a `token_version` that
  differs from the row is rejected.
- `token_version` bump invalidates a live cookie; `logout-all` bumps it.
- `is_active = 0` rejects.
- `AUTH_MODE=disabled` is accepted in a debug build. The release case is not unit-testable under
  debug: it is covered by the `cargo build --release` local gate (the variant is not compiled in)
  and by rollout step 1.
- bootstrap handover: a login whose `sub` equals `BOOTSTRAP_OWNER_SUBJ` takes over the placeholder
  row and keeps its id; a login that does not match gets a new row; a login matching an existing real
  row does not touch the placeholder.

**Backend integration tests — the isolation matrix.** Two seeded users, A and B. For every API route
mounted in `build_rocket`, B attempts the operation against A's portfolio / asset / transaction id
and must receive `404`. Each case is paired with A performing the same operation successfully, so a
guard that merely breaks everything cannot pass. Tests use `AuthMode::Disabled { as_user }` with the
`seed_user` / `as_user` helpers from `tests/common/mod.rs`. The nested routes
(`PATCH …/assets/:aid`, `POST …/assets/:aid/transactions`) get a cross-portfolio case: B owns
portfolio 1, A owns portfolio 2 and asset 99 in it, and B attempts the write through portfolio 1's
path with asset 99's id.

The matrix runs under `Disabled` mode, so it proves **authorization** — who may touch which row —
not **authentication** — cookie decryption and session validation under OIDC. Authentication is
covered by the cookie and guard tests above and by the two probe tests below; rollout step 5 smokes
the real OIDC path.

**Backend integration tests — startup, migration and probes**

- orphan portfolios under `Oidc` with `BOOTSTRAP_OWNER_SUBJ` set → all owned after `init_db`, and
  `init_db` returns `Ok`.
- orphan portfolios under `Oidc` with it unset → `init_db` returns `Err` (asserted on the returned
  `Result`, not on a process exit).
- orphan portfolios under `Disabled` → assigned to the placeholder row, `Ok`.
- a database with no portfolios and no orphans → `Ok`, and `owner_id` exists as a column.
- `GET /` returns `200` with authentication enabled and no cookie.
- `GET /api-docs/openapi.json` returns `200` without a cookie — this pins the decision that the
  contract stays public, so a later change cannot quietly narrow it.
- **route coverage:** ignite the app and iterate `Rocket::routes()`, then assert that every mounted
  `/api` route appears in `ApiDoc::json()`'s paths and methods, and that every route under `/api` is
  behind the fairing. This is the drift CI cannot otherwise catch: `routes![]` and
  `ApiDoc::paths` are maintained by hand in two files, and CI diffs a document generated from one of
  them.

**Frontend tests** — `apiFetch` redirects on `401`; existing component tests still pass with the
wrapper in place; the `?auth_error` banner renders.

**CI gates** (`.github/workflows/ci.yml`): `cargo test`, `--print-openapi` + diff against the
committed spec, the `/auth` proxy-entry check, `npm run generate-types`, `npm test`.
**Local gates** (not run by CI): `npm run lint`, and `cargo build --release` to confirm the release
binary compiles without the `Disabled` variant.

## Rollout & Rollback

1. Create the `Secret` first (it needs `ROCKET_SECRET_KEY`; a non-`debug` Rocket profile refuses to
   launch without one) and remove the `wait-for-backend` initContainer. Deploy with
   `AUTH_MODE=disabled`. The backfill runs and pre-existing portfolios move to the placeholder row.
   Behaviour is unchanged for existing clients and data.
2. Add the `location /auth` block to `frontend/nginx.conf` and `'/auth'` to the Vite proxy.
3. Smoke the proxy paths before enabling auth: `curl -i https://<host>/auth/me` must return the
   backend's `401` JSON, not `index.html`. In dev, the same URL through the Vite port must behave
   identically.
4. Create the Authentik provider and application (self-registration still off); set `AUTH_MODE=oidc`
   and the OIDC variables; restart. If anything is wrong, this step is reversible by env alone:
   discovery is lazy, so a bad `OIDC_ISSUER` fails at login rather than at launch.
5. Verify login, logout, `logout-all`, and that an anonymous `curl` to any `/api` path returns `401`.
6. Complete the bootstrap handover: set `BOOTSTRAP_OWNER_SUBJ` to the operator's `sub` — read from
   the Authentik admin console, or from the `provisioned user … sub=…` log line at their first login
   — restart, and have that identity sign in once. The placeholder row becomes their row and the
   legacy portfolios come with it. Enable Authentik self-registration only after this step: until
   then only one identity can exist, which is the state the backfill was designed for.

**Rollback**: set `AUTH_MODE=disabled` (debug image) or roll back the image. The added columns and
tables are additive and unused by the previous code, so no schema rollback is needed.
`ROCKET_SECRET_KEY` must be stable across restarts; rotating it invalidates all sessions, which is
a forced re-login, not data loss.

## Security Considerations

- **Enumeration.** Sequential integer ids plus `403` would let a caller test which ids exist. `404`
  for everything outside the caller's access removes that oracle.
- **Chokepoint enforcement.** Isolation depends on `access` being the only path to a portfolio by
  caller-supplied id. A handler that writes its own `WHERE id = ?` breaks it silently. Three things
  make that harder: `resolve` is private and the exported functions return `Status`, so a denial
  cannot be ignored; the isolation matrix iterates every mounted route; and CI greps handler files
  for the pattern. A reviewer should still treat any new `WHERE id = ?` against `portfolios`,
  `assets`, or `transactions` as a defect.
- **`Disabled` mode.** Not compiled into a release binary. `AUTH_MODE=disabled` there is a startup
  error. In a debug build it prints a banner naming the assumed user id.
- **Deactivated users.** A user deactivated in Authentik keeps access to this app until their cookie
  expires (≤ 8 hours) or `token_version` is bumped. `users.is_active` exists so a future Authentik
  sync can close this window; nothing writes it in this spec.
- **Cookie.** `HttpOnly` + `Secure` + `SameSite=Lax`, encrypted with a 256-bit key
  (`openssl rand -base64 32`) from a k8s `Secret`, never a ConfigMap. `Secure` is unconditional
  because the ingress, not nginx, terminates TLS; the app must not be exposed over plain HTTP with
  OIDC enabled.
- **Legacy ownership handover.** Keyed on `sub` via `BOOTSTRAP_OWNER_SUBJ`. `email` never decides
  ownership, because with self-registration an `email` claim is claimable by anyone and
  `email_verified` is only as trustworthy as Authentik's configuration.
- **Public API contract.** `/api-docs/openapi.json` stays reachable without a session, so every route
  and payload shape is public. That is a deliberate choice, not an oversight: nothing in the isolation
  model depends on the route list being secret, and every `/api` route still requires a session while
  ids outside the caller's access are `404`. Anyone proposing to authenticate the document should
  re-read [Decisions](#decisions-and-alternatives-considered) first — CI must not go back to generating
  it from a running server.
- **Unthrottled lookup.** Any authenticated user can reach `/api/assets/lookup`, which proxies Yahoo
  Finance and EODHD against one shared upstream quota. Not addressed here; designed in
  [issue #35](https://github.com/javierarrieta/portfolio-prism/issues/35).
- **Transport.** TLS termination is assumed to be the operator's existing ingress.

## Deferred Work

| Spec | Contents | Depends on |
|---|---|---|
| 2 — Sharing | `portfolio_members` write paths, share/unshare endpoints, "shared with me", share UI, and the `access` field on the portfolio response shape that Spec 2's UI is the first consumer of | this spec |
| 3 — API keys + MCP | `api_keys` table, mint/revoke UI, `read`/`write` scopes, an `rmcp` (the Rust MCP SDK) streamable-HTTP server at `/mcp` | this spec's guard |
| 4 — MCP OAuth 2.1 | Dynamic client registration, protected-resource metadata, RFC 8707 resource parameter, for hosted connectors (ChatGPT, Claude.ai) | spec 3 |

Not a spec, but split out of this one: [issue #35](https://github.com/javierarrieta/portfolio-prism/issues/35),
rate limiting `/api/assets/lookup`. It can land before or after this spec; if it lands after, it
inherits the authenticated `/api` surface and only has to add the bucket.

Spec 3's keys must be scoped. An agent holding an unscoped write key can delete transactions and
rewrite prices, and the tax-lot engine (`backend_rust/src/engines/tax_engine.rs`, `TaxLotEngine`,
which computes FIFO, LIFO, and hybrid cost-basis) turns silent historical edits into a
tax-reporting integrity problem. Read-only is the default scope.

## Known Limitations

- SQLite on one PVC with `replicas: 1` is the configuration this spec ships against; it is the first
  thing to replace under real load. The `access` chokepoint is what makes a later Postgres move a
  query-shape change rather than an authorization change.
- `/api/assets/lookup` is unthrottled: one caller can consume the shared Yahoo / EODHD quota for
everyone. Designed in [issue #35](https://github.com/javierarrieta/portfolio-prism/issues/35), not
fixed here.
- `users.email` is copied from the ID token at login and is not re-verified if it changes in
  Authentik until the next login.
- If the bootstrap identity never signs in, pre-existing portfolios stay owned by the placeholder
  row; reassigning them is a manual SQL step. The same applies if that person signs in with a
  different `sub`, or if a real row for them already existed when the handover was attempted.
- Ownership of a legacy portfolio is decided by `sub` and by nothing else. That guarantee is a policy
  of this application, not something Authentik enforces on its behalf.
- Foreign keys are not enabled (`PRAGMA foreign_keys` is off throughout this repository), so
  `owner_id` and `portfolio_members` referential integrity is application-enforced only.
- Deleting an asset leaves its transactions with a dangling `asset_id`. They are unreachable through
  the API and are not cleaned up by this spec.
- `cargo test --release` is not supported: the release binary has no `AuthMode::Disabled`, and the
  integration tests need it. Run `cargo build --release` as the release check and `cargo test` in
  debug.
- Dev sessions do not survive a backend restart: Rocket generates a temporary `secret_key` in the
  `debug` profile, which invalidates `pp_session`. Set `ROCKET_SECRET_KEY` in `start.sh` if that
  becomes annoying.
- The isolation matrix proves authorization, not authentication.

## Open Questions

None.

## Revision History

**2026-11-05 (second revision) — scope reduced by the owner.** Two requirements left the spec:

- `/api-docs/openapi.json` stays **public**. The earlier draft authenticated it; the contract is not a
  secret and the isolation model does not depend on route secrecy. The fairing therefore guards the
  `/api` prefix only, and a test now pins the `200`.
- The `/api/assets/lookup` rate limit moved to
  [issue #35](https://github.com/javierarrieta/portfolio-prism/issues/35), which carries the design
  (one global bucket, capacity 10, refill 1/s, injectable clock). It protects a third-party quota,
  not user data.
- `--print-openapi` is kept even though it no longer protects CI from a `401`: a server started with
  the default `AUTH_MODE=oidc` and no `OIDC_*` variables now refuses to launch, so generating the
  document from a running server is still not workable. That refusal is now stated explicitly in §9.

**2026-11-05 — revised against `reviews/2026-11-04-oidc/`.** The changes below are the substantive
ones; the review thread (7 blockers, 24 majors, 16 nit groups) is the record of what prompted each.

- OpenAPI generation is decoupled from the running server (`--print-openapi`), and the unused root
  `/openapi.json` alias is removed.
- The `owner_id NOT NULL` rebuild is dropped; the column stays nullable and the invariant is enforced
  by the startup check and by `access`. The claim that `migrate_one_column` could be reused was
  false.
- The claim that `PATCH /api/portfolios/<pid>/assets/<aid>` lacks `#[utoipa::path]` was false; the
  real gap is `ApiDoc::paths`.
- F5 (requirement 5 in the old flat numbering) and the Overview now say "outside their granted access
  level" instead of "not owned", which no longer contradicts editor and viewer access.
- Asset deletion is described as it behaves: it orphans transactions. Owner-only deletion is a policy
  choice, not a cascade safeguard.
- Every synthetic `users` row has literal values, and `email` became nullable so it does not need a
  fabricated address.
- `build_rocket` and `init_db` signatures, the `StartupConfig` type, and the process-exit site are
  named; the dev seeding order is fixed, which removes the §1/§4 collision.
- One `Secure` rule, keyed on the fact that the ingress terminates TLS, replaces the two
  contradictory statements.
- `AuthMode::Disabled` moved from a runtime profile allowlist to a compile-time gate.
- A mount-level fairing makes F1 true by construction; the route-inventory test was replaced by a
  `Rocket::routes()` ↔ `ApiDoc::paths` coverage test.
- The chokepoint is enforced rather than described: `resolve` is private, the nested path check moved
  into `access`, and CI greps handler files.
- OIDC discovery is lazy; the browser-versus-non-browser branch is keyed on `Accept`.
- Cookie names, payloads, and the logout redirect registration are specified; `AuthError` was dropped
  in favour of `Status`.
- The rate limit became one global bucket with a clock the tests can control (superseded by the
  second revision: it left this spec entirely).
- `BOOTSTRAP_OWNER_EMAIL` and the `email_verified` handover path were removed.
- The `wait-for-backend` initContainer premise was false; the container is removed by this spec.
- `PATCH /api/portfolios/:id` is described as the currency-update route it is.
- Added `Decisions and Alternatives Considered`, `Risks and Mitigations`, requirement ids, a
  terminology table, and this Revision History.
