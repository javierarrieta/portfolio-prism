# OIDC Identity & Per-User Data Isolation — Design Document

**Date**: 2026-11-04
**Status**: Proposed
**Scope**: Spec 1 of 4 (see [Deferred Work](#deferred-work))

## Overview

Portfolio Prism currently has no authentication of any kind: every route under `/api` is
reachable by anyone who can reach the host, and the complete OpenAPI document is publicly
reachable at `/api-docs/openapi.json`. (No Swagger UI is mounted today — `utoipa_swagger_ui` is a
dependency but is never used in `build_rocket` — but the document alone enumerates every route
and payload.) This design adds OpenID Connect authentication against an **existing, already
operated Authentik instance**, introduces a local identity record per authenticated person, and
enforces **hard per-user data isolation** so that no user can read or mutate another user's
portfolios, assets, or transactions.

The authentication half is comparatively small. The isolation half is the substance of this
spec: with untrusted users and sequential integer primary keys, ownership checks scattered
through ten hand-written SQL statements are a leak waiting to happen. The design therefore makes
one module the only way to reach a portfolio by id, and makes the cross-user test matrix the
primary acceptance gate.

Authentik owns user accounts, registration, passwords, MFA, and the admin user-management
console. This application stores a shadow record per identity and nothing else. **There is no
user-management UI in this application.**

## Requirements

1. All `/api/*` routes require an authenticated identity. Unauthenticated calls return `401`.
2. Authentication uses the OIDC authorization code flow with PKCE against the existing
   Authentik deployment. No passwords are handled by this application.
3. A local `users` row is created just-in-time on first successful OIDC login, keyed on
   `(issuer, subject)`.
4. Every portfolio has exactly one owner. Existing portfolios are assigned to a designated
   bootstrap owner during migration; the application refuses to start if unowned portfolios
   exist and no bootstrap owner is configured.
5. A user cannot read or mutate a portfolio, asset, or transaction they do not own. The response
   for any non-owned resource id is `404`, never `403`.
6. `GET /api/portfolios/` returns only portfolios the caller owns or is a member of. The members
   table is empty until Spec 2, so in this spec "member of" is always the empty set.
7. `/api-docs` requires authentication.
8. Kubernetes liveness probes, readiness probes, and the `wait-for-backend` initContainer
   continue to succeed without credentials.
9. `cargo test` and `npm test` pass without an OIDC provider reachable.
10. `./start.sh` continues to work for local development without an OIDC provider.
11. The access-control layer is written against the final access model (owner / editor / viewer)
    even though sharing is not delivered until Spec 2.
12. `/api/assets/lookup` is rate limited per authenticated user, because it proxies a
    third-party API with a shared quota.

## Non-Goals

Explicitly out of scope for this spec:

- Sharing portfolios between users (UI, endpoints, "shared with me" view) — Spec 2. The
  `portfolio_members` table **is** created here; nothing writes to it yet.
- Per-user API keys and the MCP server — Spec 3. The auth guard **is** shaped to accept a bearer
  token here; no key is issued yet.
- MCP OAuth 2.1 with dynamic client registration for hosted connectors (ChatGPT, Claude.ai) —
  Spec 4.
- Roles, admin panels, per-user quotas beyond the lookup rate limit.
- Syncing `is_active` from Authentik (see [Security Considerations](#security-considerations)).
- Replacing SQLite, or changing `replicas: 1`.

## Current State (evidence)

| Fact | Location |
|---|---|
| No auth anywhere; only `Authorization` appears in CORS allowed headers | `backend_rust/src/lib.rs:53` |
| No `securitySchemes` in the published spec; 10 paths / 13 operations | `docs/openapi/openapi.json` |
| The mounted surface is 14 API operations — `PATCH /api/portfolios/<pid>/assets/<aid>` carries no `#[utoipa::path]`, so the published spec under-reports the surface | `backend_rust/src/lib.rs` (`build_rocket`), `api_routes/transactions.rs:96` |
| Rocket 0.5 selects its profile from `ROCKET_PROFILE` (else `debug`/`release` by build mode); `k8s/` sets no `ROCKET_*` variable, and the file is lowercase `configmap.yaml` | `k8s/configmap.yaml`, `k8s/app-deployment.yaml` |
| `portfolios` / `assets` / `transactions` have no owner column | `backend_rust/src/lib.rs` (`init_db`) |
| `portfolios.id` is `INTEGER PRIMARY KEY AUTOINCREMENT` — enumerable | `backend_rust/src/lib.rs` (`init_db`) |
| Startup migrations already exist, including a crash-safe column rebuild helper | `backend_rust/src/lib.rs` (`init_db`, `migrate_one_column`) |
| Tests build the app with `build_rocket(pool, currency_service, cors)` | `backend_rust/tests/common/mod.rs` |
| CI diffs the committed OpenAPI spec and regenerates frontend types from it | `.github/workflows/ci.yml` |
| Prod is same-origin: nginx proxies `/api` and `/api-docs` to `127.0.0.1:8000` | `frontend/nginx.conf` |
| Dev is same-origin via the Vite proxy | `frontend/vite.config.js` |
| Backend probes and the initContainer hit `GET /` on port 8000; the frontend container's probes hit `:80`; Service exposes only port 80 | `k8s/app-deployment.yaml`, `k8s/app-service.yaml` |
| ~14 raw `fetch` calls with no status-code handling | `frontend/src/App.tsx` |
| Yahoo/EODHD proxy endpoint already needed 429 handling | commit `2753049` |

Same-origin in both prod and dev is load-bearing for this design: it means the browser never
handles a token, so there is no token storage, no refresh logic, and no new frontend
authentication dependency.

## Decisions Summary

| Decision | Choice | Rejected alternative |
|---|---|---|
| Where the OIDC flow terminates | Backend (Rocket) | `oauth2-proxy` sidecar — gives no in-app identity, so it cannot enforce isolation; dev diverges from prod |
| SPA token handling | None; backend session cookie | SPA PKCE + `Bearer` — tokens in browser storage, refresh logic in React, no benefit given same-origin |
| Identity provider | Existing Authentik | Self-hosted Keycloak/Authentik-for-this-app — needless new stateful service; Google-only login — incompatible with open signup |
| Account ownership | Authentik owns accounts; app does JIT provisioning | App-side registration + passwords — the thing not to hand-roll |
| Session representation | Rocket private cookie + `users.token_version` | Server-side session table — write per request and a cleanup job for no gain |
| Non-owned resource response | `404` | `403` — confirms the id exists, free enumeration oracle on sequential keys |
| Ownership enforcement | Single `access` module, no handler bypass | Per-handler `WHERE owner_id = ?` — ten places to forget |
| Owner representation | `portfolios.owner_id` column | Owner as a `portfolio_members` row — two sources of truth, easy to delete the owner by accident |
| Unowned rows at startup | Refuse to start | Assign to an arbitrary user — silently misattributes financial data |
| `Disabled` auth mode | Allowed only on an explicit profile allowlist (`debug`, `dev`, `test`); refused on `release`, on any unknown profile, and when unset in a release build | Refusing only when the profile literally reads `production` — Rocket 0.5 has no `ROCKET_ENV`, so the forgotten variable the check exists to catch is the check's own default |
| Legacy-portfolio handover | Keyed on `BOOTSTRAP_OWNER_SUBJ`, or on an `email` match gated by `email_verified` | Matching the ID token's bare `email` — with open self-registration, email is a takeover vector |
| Lookup rate limit | Per authenticated user, included in this spec | Defer — open signup makes a shared third-party quota a shared failure |

## Architecture

```
Browser ──HTTPS──▶ nginx (frontend container, :80)
                    ├── /            → static SPA
                    ├── /api/*       → proxy 127.0.0.1:8000  ──┐
                    ├── /api-docs    → proxy 127.0.0.1:8000  ──┤ Rocket
                    └── /auth/*      → proxy 127.0.0.1:8000  ──┘
                                                              │
                     ┌────────────────────────────────────────┴──────────────┐
                     │ AuthMode (Oidc | Disabled)                            │
                     │   ├─ /auth/login /callback /logout /me   (OIDC)       │
                     │   └─ FromRequest guard → AuthedUser                    │
                     │        ▼                                              │
                     │ access::require(user, portfolio_id, level)  ◀── the   │
                     │        ▼                            only path to a    │
                     │ handlers (portfolios / transactions / analytics)      │
                     │        ▼                            portfolio by id   │
                     │ SQLite: users, portfolios(owner_id), portfolio_members│
                     └───────────────────────────────────────────────────────┘
                                                              │
                                             Authentik ───────┘  (out of cluster, already running)
```

Three units, each independently understandable:

| Unit | Responsibility | Depends on |
|---|---|---|
| `src/auth/` | OIDC code flow, session cookie issuance/verification, `AuthedUser` guard | `AuthMode`, Rocket cookies, `users` table |
| `src/access.rs` | Map `(user, portfolio_id)` → `Access`, or nothing | `users`, `portfolios`, `portfolio_members` |
| `src/api_routes/*` | Business logic; never resolves ownership themselves | `AuthedUser`, `access` |

## Backend Changes

### 1. New module `src/auth/`

`src/auth/mod.rs`, `src/auth/oidc.rs`, `src/auth/session.rs`, `src/auth/guard.rs`.

```rust
#[derive(Clone)]
pub enum AuthMode {
    Oidc(OidcConfig),
    Disabled { as_user: i32 },
}

pub struct AuthedUser { pub id: i32, pub email: String, pub name: Option<String> }
```

**`AuthMode::Disabled` is refused unless the Rocket profile is explicitly allowlisted.** Rocket
0.5 has no `ROCKET_ENV`. It reads `ROCKET_PROFILE`, and when that is unset the profile is `debug`
in a debug build and `release` in a release build. A check written as "`!= production`" is
fail-open: an unset variable is the normal state of this deployment, and it evaluates to "not
production". The rule is an allowlist instead, checked in `build_rocket` before any route is
served:

| Profile | `Disabled` allowed |
|---|---|
| `debug`, `dev`, `test` | yes — with a prominent startup banner |
| `release`, `prod`, any other value, unset in a release build | no — process-exiting error at launch |

This is the highest-risk line in the design: a forgotten environment variable must not silently
expose every portfolio, so the unset state has to be the refusing state. `k8s/configmap.yaml`
sets no `ROCKET_*` variable today, which means the release image resolves to profile `release`
and is refused by default.

In `Disabled` mode the guard resolves to `as_user`. In integration tests that id comes from
`seed_user`. In a non-production deployment it is the bootstrap placeholder user when
`BOOTSTRAP_OWNER_SUBJ` or `BOOTSTRAP_OWNER_EMAIL` is set, and otherwise a seeded row with
`subj = 'bootstrap:dev'` created at startup if absent, so `./start.sh` needs no configuration.
This is what makes rollout step 1 work before anyone has signed in.

`AuthedUser` implements `rocket::request::FromRequest`. Any handler that takes `user: AuthedUser`
is protected; the type system makes protection impossible to forget on a route that reads the
user, because there is no other way to obtain one.

**Dependency added** to `backend_rust/Cargo.toml`: `openidconnect = "4.1"`, whose default features
are `reqwest` + `rustls-tls`, so it reuses the existing rustls-based `reqwest` rather than pulling
`native-tls` into a deliberately rustls-only build. No `rand` dependency is needed:
`openidconnect` already provides `CsrfToken::new_random`, `Nonce::new_random`, and
`PkceCodeChallenge::new_random_sha256`, and adding `rand` directly risks two majors in the tree.

**Rocket feature change required:** `rocket = { version = "0.5", features = ["json"] }` becomes
`features = ["json", "secrets"]`. Private cookies and `rocket::config::SecretKey` are
`#[cfg(feature = "secrets")]` in Rocket 0.5, so without this the session mechanism in §3 does not
compile.

`openidconnect` 4.1.0 has MSRV 1.88; the build image is `rust:1.92-slim` and local toolchain is
1.98.1, so both clear it.

### 2. OIDC endpoints

| Method | Path | Behaviour |
|---|---|---|
| `GET` | `/auth/login` | Generate `state` + `nonce` + PKCE verifier; store all three in one short-lived `HttpOnly` `SameSite=Lax` cookie (10 min); `302` to the Authentik authorization URL with `scope=openid profile email` |
| `GET` | `/auth/callback` | Verify `state` matches the cookie, verify `nonce` in the ID token, exchange `code` at the token endpoint, validate the ID token (JWKS signature, `iss`, `aud`, `exp`, `nonce`), upsert the `users` row, clear the transient cookie, issue the session cookie, `302` to `/` |
| `GET` | `/auth/logout` | Clear the session cookie; `302` to Authentik's `end_session_endpoint` with `post_logout_redirect_uri` |
| `POST` | `/auth/logout-all` | Bump `users.token_version`, invalidating every outstanding session cookie for this user, then clear this one |
| `GET` | `/auth/me` | `200` with `{ id, email, name }` for the authenticated caller; `401` otherwise |

`GET /auth/me` is the only endpoint the SPA needs to know it is signed in. It is under `/auth`,
not `/api`, so it is not part of the `/api` OpenAPI surface.

Session cookie: Rocket **private** cookie (encrypted and signed with `ROCKET_SECRET_KEY`; needs
the `secrets` feature from §1), `HttpOnly`, `Secure`, `SameSite=Lax`, path `/`, max-age 8 hours,
carrying `{ user_id, token_version }`.

`SameSite=Lax` blocks cross-site `POST`, which is the CSRF exposure for a cookie-authenticated
API; the existing exact-origin CORS policy remains as a second layer. No separate CSRF token is
introduced, because the API is same-origin in both prod and dev. If credentialed cross-origin
access is ever enabled, a CSRF token becomes mandatory and this paragraph must be revisited.

### 3. Session validation and revocation

On every guarded request the guard:

1. decrypts the session cookie;
2. loads `users` by `user_id`;
3. rejects if the row is missing, `is_active = 0`, or `token_version` differs from the cookie.

`token_version` is the revocation switch: bumping it invalidates every outstanding cookie for
that user without a session table. `POST /auth/logout-all` performs that bump and clears the
cookie, giving the user a "sign out everywhere" action.

### 4. Schema migration

Added to `init_db` in `backend_rust/src/lib.rs`, following the existing startup-migration style:

```sql
CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    issuer TEXT NOT NULL,
    subj TEXT NOT NULL,
    email TEXT NOT NULL,
    name TEXT,
    is_active INTEGER NOT NULL DEFAULT 1,
    token_version INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    last_login_at TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS uq_users_issuer_subj ON users(issuer, subj);

ALTER TABLE portfolios ADD COLUMN owner_id INTEGER REFERENCES users(id);

CREATE TABLE IF NOT EXISTS portfolio_members (
    portfolio_id INTEGER NOT NULL REFERENCES portfolios(id),
    user_id INTEGER NOT NULL REFERENCES users(id),
    role TEXT NOT NULL CHECK (role IN ('editor','viewer')),
    added_at TEXT NOT NULL,
    PRIMARY KEY (portfolio_id, user_id)
);
CREATE INDEX IF NOT EXISTS idx_portfolio_members_user ON portfolio_members(user_id);
```

`portfolio_members` is created but unused in this spec, deliberately: it lets `access` be written
once against the final access model instead of being rewritten in Spec 2.

**Making `owner_id` NOT NULL.** SQLite cannot add a `NOT NULL` column without a default and
cannot relax a nullable column afterwards without a table rebuild. The existing
`migrate_one_column` helper already performs the rename → add → copy → drop rebuild safely on a
dedicated connection with `legacy_alter_table` enabled. The `owner_id` rebuild reuses that
mechanism, so no new migration framework is introduced.

The rebuild is triggered by **schema state, not row content**: run it whenever
`PRAGMA table_info(portfolios)` reports `notnull = 0` for `owner_id`, after the backfill below has
run. Gating it on "orphans exist" would mean a fresh database — no portfolios, so no orphans —
never rebuilds, and fresh and upgraded installs end up with different schemas.

**Foreign keys are not enforced.** Nothing in this repository issues `PRAGMA foreign_keys = ON`
(SQLite's default is off), so the `REFERENCES` clauses above and in the existing `assets` /
`transactions` tables are documentation, not constraints: a dangling `owner_id` is possible, and
requirement 4 is enforced by application code. This spec does not change that; enabling FK
enforcement is a separate migration, because it would fail on any pre-existing dangling row.

**Backfill.** On startup, if any `portfolios.owner_id IS NULL`:

- if `BOOTSTRAP_OWNER_SUBJ` is set, upsert a placeholder `users` row with
  `subj = 'bootstrap:<subj>'` and assign every orphan to it — the preferred form, since an
  Authentik subject cannot be claimed by whoever registers first;
- else if `BOOTSTRAP_OWNER_EMAIL` is set, upsert a placeholder row for that email with
  `subj = 'bootstrap:<email>'` and assign every orphan to it;
- if neither is set, log the count of orphaned portfolios and **exit non-zero**.

**Placeholder reconciliation.** A `bootstrap:` row is handed over to a real identity only when the
identity is provably the same person:

- if `BOOTSTRAP_OWNER_SUBJ` is set, hand over only when the ID token's `sub` equals it;
- otherwise hand over on an `email` match **only if** the ID token asserts
  `email_verified = true`, and only against a row whose `subj` begins with `bootstrap:`;
- in every other case insert a new `users` row.

A bare `email` match is not sufficient on its own. This design assumes open self-registration in
Authentik, so an attacker who registers the bootstrap address before its real owner would
otherwise be handed every pre-existing portfolio — this is the one place in the design where a
claim the IdP may not have verified decides ownership. The operator note in
[Configuration & Deployment](#configuration--deployment) keeps the window closed by completing the
handover before open registration is enabled.

Refusing to start is intentional: silently assigning financial records to the wrong person is
worse than downtime.

### 5. New module `src/access.rs`

```rust
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Access { Owner, Editor, Viewer }

impl Access {
    pub fn meets(self, min: Access) -> bool { /* Owner > Editor > Viewer */ }
}

/// None ⇒ caller has no access. Callers must translate None to 404.
pub async fn resolve(pool: &SqlitePool, user_id: i32, portfolio_id: i32)
    -> Result<Option<Access>, sqlx::Error>;

/// Ok(access) if held, Err(AuthError::NotFound) otherwise.
pub async fn require(pool: &SqlitePool, user: &AuthedUser, portfolio_id: i32, min: Access)
    -> Result<Access, AuthError>;

/// Resolve an asset's owning portfolio, then require().
pub async fn require_asset(pool: &SqlitePool, user: &AuthedUser, asset_id: i32, min: Access)
    -> Result<Access, AuthError>;

/// Resolve a transaction's asset → portfolio chain, then require().
pub async fn require_transaction(pool: &SqlitePool, user: &AuthedUser, tx_id: i32, min: Access)
    -> Result<Access, AuthError>;
```

`resolve` reads `portfolios.owner_id` first; on mismatch it falls back to `portfolio_members`;
on no row it returns `None`. This is the **only** module permitted to look up a portfolio, asset,
or transaction by caller-supplied id. Handlers must not contain `WHERE id = ?` against those
tables.

**Access level semantics.** Ordering is `Owner > Editor > Viewer`.

| Level | May |
|---|---|
| `Viewer` | read the portfolio, its assets and transactions, its tax summary and its performance |
| `Editor` | additionally add an asset, and add or delete transactions |
| `Owner` | additionally rename or delete the portfolio, and delete an asset |

The rule being applied: `Editor` may add and amend, `Owner` alone may destroy structure. Deleting
an asset cascades to its transactions, so it is owner-only even though adding one is not.

### 6. Route changes

| Route | Change |
|---|---|
| `POST /api/portfolios/` | `owner_id` bound to `user.id`; insert includes the column |
| `GET /api/portfolios/` | `WHERE owner_id = ? OR id IN (SELECT portfolio_id FROM portfolio_members WHERE user_id = ?)`; each row carries the caller's `Access` |
| `GET /api/portfolios/:id` | `require(…, Viewer)` → `404` if not held |
| `PATCH /api/portfolios/:id` | `require(…, Owner)` — renaming/deleting is owner-only |
| `DELETE /api/portfolios/:id` | `require(…, Owner)` |
| `GET /api/portfolios/:id/tax-summary` | `require(…, Viewer)` |
| `GET /api/portfolios/:id/performance` | `require(…, Viewer)` |
| `POST /api/portfolios/:id/assets` | `require(…, Editor)` |
| `PATCH /api/portfolios/:id/assets/:aid` | `require(…, Editor)` **and** verify `asset.portfolio_id == id` |
| `DELETE /api/assets/:id` | `require_asset(…, Owner)` |
| `POST /api/portfolios/:id/assets/:aid/transactions` | `require(…, Editor)` **and** verify `asset.portfolio_id == id` |
| `GET /api/portfolios/:id/transactions` | `require(…, Viewer)` |
| `DELETE /api/transactions/:id` | `require_transaction(…, Editor)` |
| `GET /api/assets/lookup` | authenticated only; per-user rate limit |
| `GET /` | **unchanged, public** — probes and initContainer depend on it |
| `GET /api-docs/openapi.json`, `GET /openapi.json` | authenticated |

The nested-route check `asset.portfolio_id == id` closes a hole where a caller who owns portfolio
1 and has access to asset 99 belonging to portfolio 2 could write through portfolio 1's path.
Both nested write routes need it: `POST …/assets/:aid/transactions` and
`PATCH /api/portfolios/:id/assets/:aid` (the real asset-update route — there is no
`PUT /api/assets/:id`; asset update is `PATCH` nested under the portfolio, and portfolio rename is
`PATCH /api/portfolios/:id`, not `PUT`).

The table is derived from the `routes![]` lists in `build_rocket` (`lib.rs:252-271`), which is 14
API operations — **not** from `docs/openapi/openapi.json`, which documents 13 of them. Because
`PATCH /api/portfolios/<pid>/assets/<aid>` carries no `#[utoipa::path]` today, the OpenAPI diff in
CI cannot catch a route omitted from this table, so the implementation adds the missing annotation
plus a test that fails when a mounted API route is absent from the table.

`Access::Editor` is defined in this spec and exercised by tests, even though no member rows exist
until Spec 2. Owner-only operations are already fixed at `Owner`; Spec 2 changes no route table.

### 7. Per-user rate limit on `/api/assets/lookup`

In-process token bucket keyed by `user_id`, capacity 10, refill 1/second, returning `429` with
`Retry-After` when exhausted. Managed state in a `Mutex<HashMap<i32, Bucket>>` with periodic
eviction of idle entries. In-process is sufficient because the deployment is `replicas: 1`; a
second replica makes this per-pod rather than global, which is recorded as an accepted
limitation rather than solved with new infrastructure.

### 8. Error handling

| Condition | Response |
|---|---|
| No or invalid session cookie on `/api/*` | `401` JSON `{ "error": "unauthenticated" }` |
| Valid session, resource not owned | `404` — never `403` |
| Session valid but `is_active = 0` or `token_version` mismatch | `401` |
| Rate limit exceeded | `429` with `Retry-After` |
| OIDC `state`/`nonce` mismatch, token validation failure, Authentik unreachable (browser flow on `/auth/callback`) | no session; `302` to `/auth/login?error=auth_failed`; specific reason logged server-side only |
| Same failures on a direct (non-browser) call to `/auth/*` | `401` JSON `{ "error": "authentication_failed" }` |

IdP error text is never reflected into a page or a response body — only a fixed string. The
existing pattern of collapsing database errors into `InternalServerError` is retained, but
authorization denials are logged at `warn` with user id and target id so a probing pattern is
visible in logs and distinguishable from genuine 500s.

## Configuration & Deployment

| Variable | Purpose | Required |
|---|---|---|
| `OIDC_ISSUER` | Authentik issuer URL from `/.well-known/openid-configuration` | yes when `AUTH_MODE=oidc` |
| `OIDC_CLIENT_ID` | Confidential client id | yes when `AUTH_MODE=oidc` |
| `OIDC_CLIENT_SECRET` | Confidential client secret | yes when `AUTH_MODE=oidc` |
| `OIDC_REDIRECT_URI` | `https://<host>/auth/callback` | yes when `AUTH_MODE=oidc` |
| `ROCKET_SECRET_KEY` | 256-bit base64 key for Rocket private cookies — `openssl rand -base64 32` | yes in a release build; a debug build auto-generates one |
| `BOOTSTRAP_OWNER_SUBJ` | Authentik `sub` of the owner for pre-existing portfolios — preferred | yes if orphans exist, unless `BOOTSTRAP_OWNER_EMAIL` is set |
| `BOOTSTRAP_OWNER_EMAIL` | Owner for pre-existing portfolios, matched only on a verified `email` claim | yes if orphans exist, unless `BOOTSTRAP_OWNER_SUBJ` is set |
| `ROCKET_PROFILE` | Rocket config profile; `Disabled` is allowed only on `debug` / `dev` / `test` | no — unset resolves to `release` in the release image, which is the refusing state |
| `AUTH_MODE` | `oidc` (default) or `disabled` | no |

`AuthMode::Disabled` is honoured only on an allowlisted profile (see
[Backend Changes §1](#1-new-module-srcauthmod)), regardless of `AUTH_MODE`. Because `AUTH_MODE`
defaults to `oidc` and `./start.sh` exports no variables at all, `start.sh` gains
`AUTH_MODE=disabled` and `ROCKET_PROFILE=dev` — otherwise requirement 10 breaks the moment this
lands, and local dev needs no OIDC configuration and no secret key.

**Authentik side (operator task, not code):** create an OAuth2/OIDC provider with client type
*Confidential*, redirect URI `https://<host>/auth/callback`, scopes `openid profile email`, and
bind it to an Application. Self-registration in Authentik is what makes open signup work; this
application adds no registration path of its own. Because that makes an `email` claim claimable by
anyone, either set `BOOTSTRAP_OWNER_SUBJ` or complete the bootstrap handover (rollout step 5)
before enabling self-registration.

**Same-origin proxy paths — `/auth` must be routed to the backend.** Login and logout are backend
routes, and neither proxy in front of Rocket currently forwards them:

- `frontend/nginx.conf` has `location /` with `try_files $uri $uri/ /index.html`, so a request to
  `/auth/login` today returns `index.html` instead of reaching Rocket. A `location /auth` block
  proxying to `127.0.0.1:8000`, with the same headers as the existing `/api` block, is required.
- `frontend/vite.config.js` proxies only `/api` and `/api-docs`. `/auth` must be added to the
  proxy map or `./start.sh` cannot complete a login in development.

Without both, the SPA redirects to `/auth/login` and receives the static app back — a silent
redirect loop. These are configuration files outside both test suites, so they are verified by the
manual smoke step in [Rollout & Rollback](#rollout--rollback) rather than by an automated test.

**Kubernetes changes** in `k8s/`:

- new `Secret` holding `OIDC_CLIENT_SECRET` and `ROCKET_SECRET_KEY`; the backend container gains
  `envFrom: secretRef` alongside the existing `configMapRef`. The `Secret` must exist **before the
  first rollout step**, not before the OIDC step: the backend image is a release build, and Rocket
  hard-errors at launch when a release build has the `secrets` feature and no `secret_key`;
- `configmap.yaml` (lowercase in this repo; the draft wrote `configMap.yaml`) gains `OIDC_ISSUER`,
  `OIDC_CLIENT_ID`, `OIDC_REDIRECT_URI`, `BOOTSTRAP_OWNER_SUBJ` or `BOOTSTRAP_OWNER_EMAIL`,
  `AUTH_MODE`, and `ROCKET_PROFILE`;
- `ingress.yaml` is unchanged — authentication is enforced by the application, not the proxy.

**Probe safety:** `GET /` on port 8000 stays unauthenticated. The backend's liveness and readiness
probes and the `wait-for-backend` initContainer curl that path (the frontend container's probes
hit `:80`); authenticating it would CrashLoop the pod. This is asserted by an automated test, not
left to review.

## OpenAPI, CI & Frontend Impact

- utoipa annotations gain a cookie `securityScheme` (`apiKey`, `in: cookie`, named
  `sessionCookie`) and a `401` response on every `/api` path, plus `404` where it is now the
  isolation answer. `bearerAuth` is deliberately **not** declared: this spec authenticates with a
  session cookie, and a declared bearer scheme teaches generated clients to send an `Authorization`
  header the API ignores. Spec 3 adds `bearerAuth` when bearer tokens actually exist.
- The missing `#[utoipa::path]` on `PATCH /api/portfolios/<pid>/assets/<aid>` is added, so the
  published spec covers all 14 API operations instead of 13.
- `docs/openapi/openapi.json` is regenerated with the documented `cargo run` + `curl` procedure;
  CI diffs it and fails on drift.
- `npm run generate-types` refreshes `frontend/src/types/api.d.ts`.
- Frontend gains `frontend/src/utils/apiFetch.ts`: wraps `fetch`, and on `401` redirects the
  browser to `/auth/login`. The ~14 raw `fetch` calls in `App.tsx` and the calls in
  `CsvImportModal.tsx` and `PortfolioDetail.tsx` are routed through it. No token is ever read,
  stored, or sent by JavaScript.
- MSW gains a `401` handler so the redirect is covered by a test.
- CORS is unchanged: same-origin in prod and dev. `Authorization` remains in the allowed headers
  so Spec 3's bearer tokens need no later change.

## Testing Strategy

**Backend unit tests**

- `access::resolve` matrix: owner / editor / viewer / none, against each required level.
- `Access::meets` ordering.
- session cookie round-trip; tampered cookie rejected.
- `token_version` bump invalidates a live cookie; `logout-all` bumps it.
- `is_active = 0` rejects.
- `AuthMode::Disabled` is refused on `release` and on any profile outside the allowlist, and
  allowed on `debug` / `dev` / `test`.
- bootstrap handover: `sub` match succeeds; `email` match succeeds only with
  `email_verified = true` and is refused without it.
- rate limiter allows 10, blocks the 11th, refills.

**Backend integration tests — the isolation matrix.** Two seeded users, A and B. For every
API route mounted in `build_rocket` (the §6 table is derived from the same list), B attempts the
operation against A's portfolio / asset / transaction id and must receive `404`. Each case is paired with A performing the same operation successfully, so a guard
that merely breaks everything cannot pass. Tests use `AuthMode::Disabled { as_user }` with
`seed_user` / `as_user` helpers added to `tests/common/mod.rs`. The nested routes
(`PATCH …/assets/:aid`, `POST …/assets/:aid/transactions`) get a cross-portfolio case: B owns
portfolio 1 and asset 99 in portfolio 2 is used through portfolio 1's path.

**Backend integration tests — migration and probes**

- orphan portfolios + `BOOTSTRAP_OWNER_SUBJ` (and, separately, `BOOTSTRAP_OWNER_EMAIL`) → all
  owned after startup.
- orphan portfolios, neither variable set → startup fails.
- a database with no orphans still ends up with `owner_id NOT NULL` (schema-state rebuild, so
  fresh installs match upgraded ones).
- every API route mounted in `build_rocket` appears in the §6 route table — the guard against the
  table drifting from the router, which the OpenAPI diff cannot catch.
- `GET /` returns `200` with authentication enabled and no cookie.
- `GET /api-docs/openapi.json` returns `401` without a cookie.

**Frontend tests** — `apiFetch` redirects on `401`; existing component tests still pass with the
wrapper in place.

**Gates**: `cargo test`, `npm test`, `npm run lint`, OpenAPI diff green. (`npm run lint` is a
local gate only — `.github/workflows/ci.yml` runs `cargo test`, the OpenAPI diff,
`npm run generate-types`, and `npm test`, but not lint.)

## Rollout & Rollback

1. Create the `Secret` first (it needs `ROCKET_SECRET_KEY`; a release build without `secret_key`
   does not launch), then deploy with `AUTH_MODE=disabled` and `BOOTSTRAP_OWNER_SUBJ` or
   `BOOTSTRAP_OWNER_EMAIL` set. The backfill runs, `owner_id` becomes `NOT NULL` — the rebuild is
   driven by schema state, so it also happens on a database that already has no orphans — and
   behaviour is unchanged for existing users.
2. Smoke the proxy paths before enabling auth: `curl -i https://<host>/auth/me` must return the
   backend's `401` JSON, not `index.html`. In dev, the same URL through the Vite port must behave
   identically. This is the step that catches a missing `location /auth` or Vite proxy entry.
3. Create the Authentik provider/application; set `AUTH_MODE=oidc` and the secrets; restart.
4. Verify login, logout, `logout-all`, and that an anonymous `curl` to any `/api` path returns
   `401`.
5. Complete the bootstrap handover: the bootstrap identity signs in once and the legacy portfolios
   move to it. Enable Authentik self-registration only after this step — until then the deployment
   is single-tenant, which is the state the backfill was designed for and the window in which an
   email-keyed handover cannot be raced.

**Rollback**: set `AUTH_MODE=disabled`. The added columns and tables are additive and unused by
the previous code, so no schema rollback is needed. `ROCKET_SECRET_KEY` must be stable across
restarts; rotating it invalidates all sessions, which is a forced re-login, not data loss.

## Security Considerations

- **Enumeration.** Sequential integer ids plus `403` would hand an attacker an existence oracle.
  `404` for everything non-owned is a requirement, not a style preference.
- **Chokepoint discipline.** The design's safety rests on `access` being the only path to a
  portfolio by id. A future handler that writes its own `WHERE id = ?` silently breaks isolation.
  The isolation matrix is the defence; a reviewer should treat any new `WHERE id = ?` against
  `portfolios`, `assets`, or `transactions` as a defect.
- **`Disabled` mode.** Allowed only on an explicit profile allowlist; refused on `release`, on
  unknown profiles, and when unset in a release build — the forgotten-variable case is the
  refusing case. Logged loudly when active.
- **Deactivated users.** A user deactivated in Authentik keeps access to this app until their
  cookie expires (≤ 8 hours) or `token_version` is bumped. `users.is_active` exists so a future
  Authentik sync can close this window; it is not populated from Authentik in this spec.
- **Cookie.** `HttpOnly` + `Secure` + `SameSite=Lax`; the secret is a 256-bit key
  (`openssl rand -base64 32`) from a k8s `Secret`, never a ConfigMap.
- **Legacy ownership handover.** The only place an ID-token claim that the IdP may not have
  verified decides ownership. Handover is keyed on `sub` when `BOOTSTRAP_OWNER_SUBJ` is set, and
  on `email` only when the token asserts `email_verified`; rollout step 5 keeps the window closed
  while registration is open.
- **Open signup + third-party quota.** Any authenticated user can reach `/api/assets/lookup`,
  which proxies Yahoo/EODHD. The per-user rate limit bounds one account's blast radius on a
  shared quota.
- **Transport.** TLS termination is assumed to be the operator's existing ingress; the app sets
  `Secure` cookies unconditionally in production and must not be exposed over plain HTTP.

## Deferred Work

| Spec | Contents | Depends on |
|---|---|---|
| 2 — Sharing | `portfolio_members` write paths, share/unshare endpoints, "shared with me", share UI | this spec |
| 3 — API keys + MCP | `api_keys` table, mint/revoke UI, `read`/`write` scopes, `rmcp` streamable-HTTP server at `/mcp` | this spec's guard |
| 4 — MCP OAuth 2.1 | Dynamic client registration, protected-resource metadata, RFC 8707 resource parameter, for hosted connectors (ChatGPT, Claude.ai) | spec 3 |

Spec 3's keys must be scoped. An agent holding an unscoped write key can delete transactions and
rewrite prices, and the FIFO/LIFO engine turns silent historical edits into a tax-reporting
integrity problem. Read-only is the default scope.

## Known Limitations

- SQLite on one PVC with `replicas: 1` is appropriate for tens of users and is the first thing to
  replace under real load. The `access` chokepoint is what makes a later Postgres move
  mechanical: query shapes change, authorization does not.
- The lookup rate limiter is per-process, therefore per-pod.
- `users.email` is copied from the ID token at login and is not re-verified if it changes in
  Authentik until the next login.
- If the bootstrap identity never signs in, pre-existing portfolios stay owned by its placeholder
  row; reassigning them is a manual SQL step. The same applies if that person later signs in with a
  different subject, or with a different email when only the email form is configured.
- Ownership of a legacy portfolio is decided by `sub`, or by a verified `email` when only
  `BOOTSTRAP_OWNER_EMAIL` is configured — never by an unverified email. That guarantee is a policy
  of this application, not something Authentik enforces on its behalf.
- Foreign keys are not enabled (`PRAGMA foreign_keys` is off throughout this repository), so
  `owner_id` and `portfolio_members` referential integrity is application-enforced only.

## Open Questions

None. All decisions in this document were reviewed and approved in design discussion on
2026-11-04, then revised after review: Rocket 0.5 profile semantics replace the non-existent
`ROCKET_ENV` check and are deny-by-default; the bootstrap handover no longer trusts an unverified
`email`; the route table is corrected to the real verbs, gains the nested asset-`PATCH` route, and
is derived from the router rather than the published spec; the Rocket `secrets` feature and the
release-build `secret_key` requirement are added to the dependencies and to rollout step 1; the
`NOT NULL` rebuild is driven by schema state; and the security scheme is a cookie, not
`bearerAuth`.
