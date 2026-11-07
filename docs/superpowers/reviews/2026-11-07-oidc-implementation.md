Status: revised after EGM round 1 (3 Goldfish auditors)
Target: `feat/oidc-auth-and-isolation` @ post-review fixes
Standard: see per-reviewer mapping
Reviewer instruction: the artifact under review is **the implementation in this
worktree**. This document is the claim surface. Every claim below is checkable
against the source; report it as a Blocker where the code does not do what this
document says, and report the code itself where it is wrong, missing, or
insecure regardless of what this document claims.

# Implementation Record — OIDC Authentication and Per-User Isolation

## 1. What changed, in one paragraph

Every `/api` request is now authenticated by a session cookie issued by an OIDC
login, and every data route resolves the requested row and checks the caller
against its owner through one chokepoint. Enforcement lives in the application,
not the proxy. `AUTH_MODE=disabled` exists for local development only and is
compiled out of the release binary.

## 2. Backend modules

| File | Responsibility |
|---|---|
| `src/auth/mod.rs` | `AuthRequest` (parsed `AUTH_MODE`), `OidcConfig`, `StartupConfig::from_env_with`, `AuthMode`, `finalise_auth`, `is_local_issuer`. All configuration refusals. |
| `src/auth/session.rs` | `pp_session` / `pp_oauth_state` cookie shapes, `issue` / `pair` / `attach` / `read` / `clear`, expiry. |
| `src/auth/guard.rs` | `AuthedUser` request guard (carries `token_version`), `session_pair` / `issue_session`, shared `resolve_caller`, `Unauthenticated` responder, `pub(crate) type UserRow`. |
| `src/auth/oidc.rs` | lazy provider discovery, PKCE login URL, code exchange, ID-token validation, `provision_or_update`, logout URL, `bump_token_version`, TTL-bounded provider/JWKS caches. |
| `src/auth/fairing.rs` | `AuthFairing` — rewrites the URI of an unauthenticated `/api/` request to a denial sentinel, or to an outage sentinel when the lookup itself failed. |
| `src/auth/routes.rs` | `/auth/login`, `/auth/callback`, `/auth/logout`, `/auth/logout-all`, `/auth/me`, the seven-method `/_auth/denied` and `/_auth/unavailable` sentinels, the `401` catcher, `WantsHtml`. |
| `src/access.rs` | the chokepoint: `Access`, `require`, `require_asset`, `require_asset_in`, `require_transaction`, private `resolve`. |
| `src/lib.rs` | `init_db(pool, &StartupConfig)` (users table, `owner_id` column, placeholder row, ownership backfill, refusals), `build_rocket` wiring. |
| `src/main.rs` | `#[rocket::main]`, `--print-openapi` as the first branch, startup refusal path. |
| `src/openapi.rs` | `document()` adds the `pp_session` cookie scheme, global security, and a `401` on every `/api` operation; `document_json()` for the committed file. |

### 2.1 Enforcement chain for one `/api` request

1. `rocket_cors`' fairing runs **first** (`.attach(cors).attach(AuthFairing)`). On an
   `Origin` that is not in `ALLOWED_ORIGINS` it rewrites the request to its own
   `/cors/<status>` route, which is outside `/api/`, so the auth layer never sees
   that request and it answers `403`. No handler runs, so nothing leaks; the
   consequence is operational — see §2.6.
2. `AuthFairing::on_request` (`src/auth/fairing.rs`) — if the path starts with
   `/api/` and `resolve_caller` yields no user, the URI is rewritten to
   `/_auth/denied` before routing. Rocket 0.5.1's `on_request` returns `()` and
   cannot halt a request, so rewriting is the only in-fairing mechanism (deviation
   D-1). CORS preflight requests are exempted so the browser's `OPTIONS` probe does
   not consume a session lookup; the preflight carries no data.
3. If the **lookup itself** returns `Err` (database unreachable, `SQLITE_BUSY`), the
   URI is rewritten to `/_auth/unavailable` → `503 {"error":"service_unavailable"}`.
   Not to the `401` sentinel: `apiFetch` redirects on `401`, so during a transient
   lock a signed-in user would be bounced to the identity provider, return, and be
   bounced again. `guard.rs` and `access.rs` already state that a database failure
   must never be presented as a denial; the fairing now obeys the same rule.
4. Routing then matches `/_auth/{GET,POST,PUT,DELETE,PATCH,HEAD,OPTIONS}` →
   `Unauthenticated` → `401 {"error":"unauthenticated"}`.
5. If the request survives, the handler's `AuthedUser` guard re-resolves the
   caller (`src/auth/guard.rs`) and rejects with the same body.
6. The handler calls a `require*` function from `src/access.rs`, which resolves the
   row's owner and compares. Denials are `404`, never `403` — a `403` tells an
   attacker the id exists.
7. `#[catch(401)]` (`src/auth/routes.rs`) makes every `401` in the process answer
   with the same JSON body. Rocket prefers a registered catcher over a guard's
   error responder, so without it a guard rejection rendered Rocket's HTML page.

Two `users` reads per request (fairing + guard) are accepted: Rocket 0.5.1 has no
request-local state to hand the result over, and passing the identity through a
request header would be forgeable (deviation D-2).

**There are three distinct machine-readable failure bodies, not one.**
`401 {"error":"unauthenticated"}` for "no usable session";
`503 {"error":"service_unavailable"}` for "the server could not decide"; and
`401 {"error":"authentication_failed"}` from `/auth/callback` when the provider
exchange itself failed. The last is a different fact — the user *had* an
interaction with the IdP and it did not work — and is deliberately not merged with
the first.

### 2.2 Session

- `SessionPayload { uid, tv, exp }`, stored via Rocket's **private** cookie jar
  (`add_private` / `get_private` / `remove_private`), so the value is
  authenticated and encrypted with `ROCKET_SECRET_KEY` (deviation D-3).
- `exp` is checked **server-side** on every read (deviation D-11). Without it the
  spec's "access until the cookie expires (≤ 8 hours)" claim was false, because
  `Max-Age` is client-advertised only.
- `tv` is the user's `token_version`. `POST /auth/logout-all` bumps it, which
  invalidates every previously issued cookie for that user.
- The version a new session carries is chosen in exactly one place:
  `AuthedUser::session_pair`, which `issue_session` also uses. The callback calls
  `user.issue_session(cookies)` and cannot name a version. It previously passed a
  literal `0`, which locked a user out permanently after their first `logout-all`
  (§8.7); `logging_out_everywhere_does_not_lock_the_user_out` fails if that returns.
- Attributes: `HttpOnly`, `SameSite=Lax`, `Path=/`, `Max-Age=8h`, and **`Secure`
  unconditionally** — not conditional on TLS. The deployment terminates TLS at the
  ingress and there is no supported http production path, so a conditional flag
  would only weaken the cookie where it is already weakest. `SameSite=Lax` is what
  prevents a cross-site `POST` from carrying the cookie; there is no separate CSRF
  token.
- `Authorization` headers are **ignored**, never accepted (spec constraint C2).

### 2.3 Login and provisioning

`/auth/login` builds a PKCE `authorize_url` with a CSRF state and a nonce, stores
the state in `pp_oauth_state` (10-minute expiry), and `303`s to the provider. If
the provider cannot be reached, the response follows the same `Accept` branch as
the callback: an HTML client is `303`ed to `/?auth_error=auth_failed`, a client that
asked for JSON gets a bare `401 {"error":"authentication_failed"}`.
`/auth/callback` verifies the state, exchanges the code, validates the ID token —
issuer, audience, expiry, and nonce, signed either by a JWKS key or, because
`CoreIdTokenVerifier::new_confidential_client` is used, by an HMAC keyed on
`OIDC_CLIENT_SECRET` (see §9.21) — and calls
`provision_or_update(pool, issuer, sub, email, name, bootstrap_subj)`
(deviation D-14 — primitives, so it is testable without an IdP). No provider detail
is reflected either way.

Provider metadata and the JWKS are cached per issuer with a **15-minute TTL**.
Cached without a bound, an Authentik signing-key rotation failed every login until
someone restarted the pod — a scheduled outage with no alarm.

`provision_or_update` order:
1. known `(issuer, sub)` → refresh `email`/`name`, return the row **including its
   current `token_version`**;
2. else if `sub == BOOTSTRAP_OWNER_SUBJ` **and** the placeholder row exists →
   `UPDATE ... RETURNING token_version` that row in place, so the id and everything
   it owns transfers with it (the bootstrap handover);
3. else insert, with `ON CONFLICT DO NOTHING` + re-select so racing first logins
   both resolve to one row.

`OidcConfig` is the authoritative copy of `BOOTSTRAP_OWNER_SUBJ`;
`StartupConfig::bootstrap_subj` is a copy filled by the same function. A blank or
whitespace-only value counts as unset in **both** the handover rule and the
backfill rule.

### 2.4 Schema and startup

`users(id, issuer, subj, email, name, is_active, token_version, created_at,
last_login_at)` with `UNIQUE(issuer, subj)`; `portfolios` gains a nullable
`owner_id`. `init_db`:
- creates the `users` table and the `owner_id` column (the column is added through
  a `legacy_alter_table` connection, because SQLx caches schema);
- **unconditionally** upserts the placeholder row `('local','bootstrap')`
  (deviation D-12) — this is what keeps `./start.sh` alive on an empty database;
- backfills `owner_id` for orphan portfolios: in `disabled` mode to the
  placeholder; in `oidc` mode only when `BOOTSTRAP_OWNER_SUBJ` is set, otherwise
  it **refuses to start** and names the orphan count;
- returns `anyhow::Result<()>` rather than calling `process::exit`, so the refusals
  are testable.

### 2.5 Access levels

| Level | Meaning | Call sites |
|---|---|---|
| `Viewer` | row exists and is owned | reads: portfolio, tax-summary, performance, sell-pnl, transaction list |
| `Editor` | ownership plus write intent | create asset, create transaction, delete transaction, update asset |
| `Owner` | destructive | delete portfolio, delete asset, update portfolio |

**The three levels are behaviourally identical today.** `resolve` returns
`Access::Owner` for any row whose `owner_id` matches the caller, and there is no
sharing table, so `grade` can never deny a level to a caller who reached the row at
all. The enum is kept because the call sites are the place a future sharing model
will hang from, and threading it now is cheaper than threading it later. The hazard
this creates is concrete and is recorded in §9.13: `delete_transaction` is graded
`Editor` while `delete_asset` is `Owner`, and the day `resolve` starts returning
`Access::Viewer` for anything, every `Editor` call site — including deletion of
financial history — becomes writable by a viewer, with no test to catch it.

`require_asset_in` is the cross-portfolio nested-write check: the asset must belong
to the portfolio named in the path **and** to the caller. `update_asset` keeps its
inline `AND portfolio_id = ?` predicate as defence in depth (deviation D-18, a
deliberate departure from spec §8).

### 2.6 CORS ordering, and why `ALLOWED_ORIGINS` is a deployment requirement

`rocket_cors` runs before the auth fairing and rewrites a request whose `Origin` is
not allowed to `/cors/403`. Because the rewritten path is not under `/api/`, the
auth layer is skipped for that request. No data is exposed — the handler never
runs — but the default allowlist is the two localhost dev origins, and browsers
send `Origin` on **same-origin** `POST`/`PATCH`/`DELETE`. A deployment that does not
set `ALLOWED_ORIGINS` therefore answers `403` to every write from the public site,
and because `apiFetch` redirects only on `401`, the user sees a silent failure with
no explanation.

`k8s/configmap.yaml` now sets `ALLOWED_ORIGINS` and says why.
`a_disallowed_origin_is_answered_before_authentication_gets_a_turn` pins both halves
of the ordering: disallowed origin → `403` (CORS wins), allowed origin → `401`
(auth gets its turn).

## 3. Frontend

`src/utils/apiFetch.ts` wraps `fetch`; on `401` it calls
`window.location.assign('/auth/login')` and returns the response. All 19 call
sites use it (`App.tsx` 14, `CsvImportModal.tsx` 3, `PortfolioDetail.tsx` 1,
`SellPnlModal.tsx` 1). `init` is forwarded verbatim — `SellPnlModal` passes an
`AbortController` signal. JavaScript never reads, stores, or sends a token.
`App.tsx` reads `?auth_error=auth_failed` once at initialisation, renders a fixed
message, and strips the parameter with `history.replaceState`.

`apiFetch` deliberately does **not** act on `403` (a cross-tenant `404`/a CORS
`403` are both "you may not", not "sign in") and not on `503` (redirecting to the
identity provider cannot fix a database).

## 4. Proxies, CI, k8s

- `frontend/nginx.conf` and `frontend/vite.config.js` both proxy `/auth`; without
  them the SPA fallback serves `index.html` for `/auth/login` and login loops.
- `.github/workflows/ci.yml`: the OpenAPI step uses `--print-openapi` instead of
  starting a server; new steps run `check_access_gate.sh`, grep both proxy files,
  run `npm run lint`, and a `cargo build --release` job that **also runs the release
  binary** to prove it refuses `AUTH_MODE=disabled` and refuses a missing OIDC
  configuration. The refusal is behind `!cfg!(debug_assertions)`, so no `cargo test`
  can reach it; before this, the claim was asserted by a comment and checked by
  nothing.
- `k8s/configmap.yaml` gains `AUTH_MODE`, the `OIDC_*` keys, `ALLOWED_ORIGINS`, and
  `BOOTSTRAP_OWNER_SUBJ` set to **empty** rather than a placeholder string — a
  sentinel like `REPLACE_ME` counts as *set*, which would quietly hand every
  pre-existing portfolio to the placeholder row instead of tripping the
  refuse-to-start guard.
- `k8s/app-deployment.yaml`: the backend image is pinned by digest instead of
  `:latest` (see §9.16), and a `startupProbe` was added because `init_db` now runs
  ALTERs plus a full-table backfill before the port is bound.
- `k8s/secret.example.yaml` (placeholders only) plus
  `envFrom: secretRef: backend-secrets`; the `wait-for-backend` initContainer is
  deleted — it curled `:8000` from an init phase, which runs before the backend
  container exists.

## 5. The gate script

`backend_rust/scripts/check_access_gate.sh`, three rules: **A** every handler file
imports `crate::access` and calls a `require*` function (`lookup.rs` exempt, with
its reason); **B** a handler whose body resolves a row by `WHERE id` must call
`require*` **before** that query — a guard that runs after the read has already
leaked the row to the handler's own logic; **C** every route handler must name
`AuthedUser` **in its signature**, not merely somewhere inside itself. It prints
`handlers inspected: 15, cleared: 15`.

It is still a heuristic: a `}` in column 0 inside a comment ends a block early, a
`format!`-built query is invisible, and `analytics.rs`'s
`WHERE asset_id IN (SELECT id FROM assets WHERE portfolio_id = ?)` slips past rule
B — that handler calls `require(..., Viewer)` first, and the isolation matrix is
the real gate.

`tests/access_gate_test.rs` tests the tester: the real tree must pass, and a
throwaway tree containing a late guard, a comment naming `AuthedUser`, and an
`OPTIONS` handler must fail with the right line numbers. All three of those passed
silently in an earlier version of the script.

## 6. Tests and gates

Backend **244** tests (baseline 160). New suites:

- `access_test.rs` — chokepoint levels, `404`-not-`403`, orphan-row handling, and
  that foreign keys are on (which contradicts the spec's assumption, §8.3).
- `auth_test.rs` — the unauthenticated sweep **and its mirror**: every mounted
  `/api` route answers `401` without a session, and every one of them (bar the one
  that would make an outbound HTTP call) reaches a handler *with* one. The mirror is
  what was missing when the lockout shipped: "401 without a cookie" is equally
  satisfied by a build that 401s with one. Both sweeps derive their route list from
  the ignited router, so a new route cannot be silently untested. Also: the
  `logout-all` lockout regression, a database failure answers `503` not `401`,
  malformed cookie payloads are denials not `500`s, expiry, tampering,
  `token_version` staleness, `is_active`, and the CORS preflight exemption (asserted
  as a real preflight: 2xx **and** an `Access-Control-Allow-Methods` header).
- `isolation_test.rs` — the cross-user matrix, every denial paired with the owner's
  success.
- `startup_test.rs` — backfill refusals, placeholder, handover.
- `route_coverage_test.rs` — mounted surface vs published document vs the route
  inventory; the denial and outage sentinels each cover all seven methods; and every
  mounted route is in a namespace that is public by decision, which is the inverted
  form of the prefix check and catches a data route mounted at `/data` or `/apifoo`.
- `access_gate_test.rs` — §5.
- `cors_test.rs` — the ordering of §2.6.

Frontend 150 tests, `tsc --noEmit` clean, `npm run lint` 0 errors (1 pre-existing
warning). `cargo build --release` compiles, and the release binary was run to
confirm both refusals. Live smoke in OIDC mode: anonymous `GET/POST/PATCH/DELETE/
OPTIONS` on `/api/…` all return `401 {"error":"unauthenticated"}`; `/` and
`/api-docs/openapi.json` return `200`.

## 7. Deviations from the spec

D-1 fairing URI rewrite (Rocket cannot halt a request from `on_request`);
D-2 two `users` reads per request; D-3 private cookie jar instead of manual
encoding; D-4 **15** API operations, not the spec's 14 — `get_transaction_sell_pnl`
was missing from the spec's inventory; D-5 the spec's `Secure`-when-TLS rule
replaced by unconditional `Secure` (§2.2); D-6 logout is local-only, the ID token is
not retained so no `id_token_hint` can be sent (§9.14); D-7 the spec's CI grep was
unsatisfiable, replaced by the three-rule script; D-8 the OIDC state cookie is
cleared by the response, not consumed server-side — there is no server-side state to
consume (§9.15); D-9 the Secret is an example file, not a committed manifest;
D-10 the session payload carries no issuer or environment binding (§9.17); D-11
server-checked `exp`; D-12 unconditional placeholder row; D-13 injectable env source
for testability; D-14 `provision_or_update` takes primitives; D-15 `sqlx::Error`
propagates as `500`, `Ok(None)` as `404`; D-16 typed `401` responder — **insufficient
on its own**, see §8; D-17 the `openidconnect` 4.1 API differs from the plan's
sketch (`ProviderMetadataWithLogout` for `end_session_endpoint`, `core_client!` for
the endpoint-set type parameters, `CoreIdTokenVerifier::new_confidential_client`),
verified by compiling a probe rather than by guessing; D-18 `update_asset` keeps its
inline predicate against spec §8.

## 8. Findings that contradicted the plan, and what was done

1. **`cargo build --release` did not compile.** `AuthMode::Disabled` is `cfg`'d out
   of release but `finalise_auth` still had to match `AuthRequest::Disabled`.
   Fixed by adding the release arm (unreachable — `from_env_with` refuses first).
   No pre-existing gate ran a release build.
2. **The gate script exited 0 while printing violations.** Caught by injecting a
   removed `require` call and a removed `AuthedUser`; both rules now set the exit
   code.
3. **sqlx enables `PRAGMA foreign_keys` by default.** The spec assumed foreign keys
   were off. Consequence: `DELETE FROM assets` on an asset that has transactions
   fails and the handler reports `500`. Pre-existing, pinned by
   `deleting_an_asset_that_has_transactions_fails_today`, not fixed.
4. **Rocket prefers a catcher over a guard's error responder.** D-16's typed
   responder alone rendered HTML; `#[catch(401)]` was added.
5. `create_asset` and `create_transaction` answer `200` while their own utoipa
   annotations document `201`. Pre-existing, pinned by tests, not fixed.
6. Rocket trims the trailing slash when composing a mount with a route path, so
   `/api/portfolios` (mounted) and `/api/portfolios/` (documented) are the same
   endpoint; `route_coverage_test.rs` canonicalises both. This codebase also writes
   utoipa paths in Rocket's `<param>` form, so no parameter translation is done.
7. **`logout-all` was an account lock, and every test passed.** The callback issued
   `session::issue(cookies, user.id, 0)` with a literal `0`. The first `logout-all`
   raises `users.token_version`, the guard rejects `tv != token_version`, and nothing
   lowers the column again — so the system's only revocation lever permanently
   locked the user out of all 15 routes. The suite missed it because every test
   fabricated its cookie at `tv = 0` instead of deriving it from the login path.
   Fixed by moving version selection into `AuthedUser::session_pair` (one place, not
   a call-site argument) and adding a regression test that **was verified to fail**
   with the literal reintroduced.
8. **A database failure was reported as a denial.** The fairing's `Err` branch
   rewrote to the `401` sentinel, which `apiFetch` obeys by redirecting to the
   provider — a redirect loop for the duration of any transient lock, and
   `SQLITE_BUSY` is reachable because the read endpoints write. Now a distinct
   `503` sentinel.
9. **`is_local_issuer` accepted `http://localhost.attacker.example`.** It compared a
   string prefix; it now parses the authority. That check is what allows plain http
   at all, so a prefix match was a way to take it off the loopback.
10. **The JWKS cache had no bound.** Key rotation would have failed every login
    until a restart. Now a 15-minute TTL.
11. **The gate had three silent holes** — order-insensitive rule B, rule C satisfied
    by a comment, `#[options(...)]` handlers invisible — and no test of its own. All
    three are now caught, with `tests/access_gate_test.rs` proving it.
12. **`rocket_cors` runs before authentication** (§2.6) and the shipped configmap
    never set `ALLOWED_ORIGINS`, so every same-origin write from the deployed site
    would have been a silent `403`.
13. **`BOOTSTRAP_OWNER_SUBJ` shipped as a placeholder string**, which the
    set/unset guard reads as *set* — applying the manifest unedited would have
    transferred every pre-existing portfolio to a row no identity can ever claim.
    Now empty, which the guard reads as unset and refuses on.

## 9. Known gaps and accepted risks

This section is the honest boundary of the work. Reviewers should attack it.

1. **No revocation path.** Nothing in this change ever sets `users.is_active = 0`.
   A user deactivated in Authentik keeps access until their cookie's `exp`, ≤ 8 h.
   The guard honours `is_active`, so the mechanism exists with no operator action
   wired to it.
2. **`token_version` is bumped only by `logout-all`.** It is not bumped on password
   change or deactivation, because nothing in this change observes either event.
   (Round 1 found that `logout-all` also locked the user out; that is fixed, §8.7.)
3. **Lazy discovery.** A mistyped `OIDC_ISSUER` fails at first login, not at
   launch. Deliberate — it prevents a crash-loop on a provider outage — but it
   means a bad value can ship, and it locks *everyone* out while surfacing only in
   the server log; the browser sees the generic `auth_failed`.
4. **No rate limiting** on `/auth/callback` or `/auth/login`. The expensive part
   (discovery, JWKS) is cached. The unbounded vector is
   `GET /api/assets/lookup`: two outbound calls and a shared OpenFIGI quota,
   available to any authenticated user, so one user can exhaust it for everyone.
5. **No CSRF token.** `SameSite=Lax` is the only defence against a cross-site
   state-changing request. Every state-changing route is non-`GET`, which is what
   makes `Lax` sufficient; `GET /auth/logout` is CSRF-able and only logs the victim
   out.
6. **Session fixation** is mitigated only in the sense that a session is minted
   from the ID token at callback; there is no separate pre-login session id.
7. **The gate is a heuristic** (§5). The isolation matrix is the real gate, and it
   covers only the routes enumerated in `isolation_test.rs`.
8. **`logout-all` has no UI.** It is reachable only by `curl -X POST`. There is no
   logout affordance in the SPA at all — the backend route exists, the frontend
   never calls it.
9. **Two `users` reads per request** (§2.1) — a measurable cost under load, not
   measured here.
10. **No end-to-end test against a real provider.** The OIDC path is tested with
    unit tests over `provision_or_update` and the guard, and by a live smoke
    against an unreachable issuer that exercises only the failure path. Every
    defect in §8.7 lives in code that has never run against a provider. A compose
    job with Keycloak or Authentik — login → `/auth/me` → `logout-all` → login — is
    the single highest-value missing test in this change.
11. **`ROCKET_SECRET_KEY` rotation** invalidates every session at once. Documented
    in `k8s/secret.example.yaml`, not automated. Note the tension: because items 1
    and 2 do nothing on their own, rotation is currently the only revocation
    mechanism that works, which sits badly next to "never rotate".
12. **`is_active` and `token_version` are not surfaced** to any admin surface; there
    is none.
13. **The access levels are inert** (§2.5). `Viewer`, `Editor` and `Owner` are one
    predicate. Nothing tests that a `Viewer` is denied at an `Editor` call site,
    because nothing can be.
14. **Logout ends the app session and not the provider session — measured, not
    guessed.** `/auth/logout` clears the local cookie and redirects to the provider's
    `end_session_endpoint` with only `post_logout_redirect_uri`. Per RP-initiated
    logout 1.0, without a hint the OP **MUST NOT perform post-logout redirection**
    unless it can confirm the target another way.

    **Observed against the live Authentik on 2026-11-07.** The flow works: the local
    cookie is cleared, Authentik answers with a logout-complete page reading
    *"You've logged out of portfolio-prism"*, and that page offers three actions —
    *Go back to overview*, *Log out of authentik*, *Log back into portfolio-prism*.
    Clicking the third returns to the app **without any credential prompt**. So:

    - the app session really ends — the cookie is gone, `/api` is `401` again;
    - the Authentik session does **not** end, and re-entry is one silent click;
    - the wording of that page is about the *application*, not the *identity provider*,
      which is why it reads as a completed logout while being only half of one.

    This is the specified behaviour of a hint-less logout, not a defect in the
    implementation: the ID token is deliberately not retained (§10), and without it
    the OP cannot be asked to end anything. The consequence is a **shared-browser
    exposure**: on a machine the next person sits down at, "log out" leaves the door
    unlocked one click away. On a single-user laptop it costs nothing.

    The fix, if wanted, is to retain the ID token server-side (Rocket's private cookie
    jar is encrypted, so browser-readable is not the concern — cookie size and
    lifetime are) and append it as `id_token_hint`. Whether Authentik then ends its own
    session rather than merely redirecting silently is **still unmeasured**; it should
    be probed before the work is committed to. Storing the ID token in *browser*
    storage was considered and rejected — see §10.
15. **The OAuth `state` cookie is cleared by the response, not consumed.** A second
    callback arriving before the first response lands still carries it. The real
    single-use guarantees are the authorization code at the provider and the nonce.
16. **Rollback depends on image pinning that the manifest did not have.** `AUTH_MODE`
    cannot undo a bad auth deploy — `disabled` is not compiled into the release
    image — so rollback is `kubectl rollout undo`, which is a no-op on a `:latest`
    tag because the pod template is byte-identical. The manifest now pins a digest;
    the operator still has to record the digest they deployed.
17. **The session is bound to nothing but `ROCKET_SECRET_KEY`.** The payload carries
    no issuer, audience, or environment tag, so if the same Secret is ever applied to
    two deployments, a cookie minted in staging authenticates in production as the
    same `uid`. **Closed by the operator on 2026-11-07: each environment has its own
    `ROCKET_SECRET_KEY`**, which is the property the whole scheme rests on. The
    payload binding is therefore not implemented; if a Secret is ever copied between
    environments, this becomes live again, and the fix is a constant environment
    string in the payload.
18. **The bootstrap handover is one-way and unverified.** `backfill_ownership`
    assigns every orphan portfolio to the placeholder as soon as
    `BOOTSTRAP_OWNER_SUBJ` is merely *set* — it is never checked against the IdP —
    and the first login whose `sub` matches takes that row and everything it owns.
    A mistyped subject plus an unrelated first login means all pre-existing
    financial data belongs to the wrong person, and with no admin surface (§9.12)
    the recovery is hand-editing SQLite with the pod scaled to zero. **Accepted by the
    operator on 2026-11-07**: the operator is the first identity to log in after
    setting `BOOTSTRAP_OWNER_SUBJ`, so the mistyping scenario is the residual risk
    rather than the expected path.
19. **`OPTIONS` preflights are exempted from authentication on three client-supplied
    headers**, with no check that the `Origin` is on the allowlist, so an
    unauthenticated `OPTIONS` reveals the allowed method and header set. No data
    leaks; the exemption exists because `rocket_cors` would otherwise answer
    preflights before authentication anyway.
21. **The client secret is also a token-signing key.** Using the confidential-client
    verifier means an ID token signed with HMAC under `OIDC_CLIENT_SECRET` is accepted,
    not only one signed by a JWKS key. Anyone who reads that Secret can mint tokens
    this app trusts without the provider. Spec-legal, and the credential already
    authenticates the app to the provider, but it is a second thing to protect.
    `at_hash` is never checked — there is nothing to check it against, because the
    access token is never used.
22. **URI-form attacks on the prefix rule are untested.** Protection is a raw-string
    prefix test on the unnormalised request target; nothing exercises `%2d`
    spellings, case variants, `//api/…`, or `/api/./portfolios`. Every handler also
    carries the guard, so this is defence in depth, but the layer `guard.rs` calls
    primary is the one with no URI-form coverage.

## 10. Rejected alternative — keeping the ID token in browser storage

Raised when deciding how to send an `id_token_hint` at logout, and rejected.

The `id_token_hint` is only ever needed by the **server**, at the moment it builds
the redirect to the provider's end-session endpoint. The browser never needs to
construct it, so browser storage buys nothing that a server-side store does not.

What it costs:

- `localStorage` and `sessionStorage` are readable by any cross-site scripting bug
  in the SPA, and they survive the session — a token parked there outlives both the
  tab and the cookie that authorises it. The design constraint this change is built
  on is that JavaScript never reads, stores, or sends a token; this would break it to
  buy a convenience.
- The backend could not treat a browser-supplied hint as proof of anything, so
  server-side validation would still be required. The result is strictly more
  exposure than keeping the token server-side, for the same behaviour.

If one-click silent logout turns out to matter, the token goes server-side: encrypted
with `ROCKET_SECRET_KEY`, scoped to the session, cleared on logout. The private
cookie jar is the obvious carrier, with the caveat that ID tokens run 1–2 KB against
a ~4 KB per-cookie budget. The decision is deferred behind a real test against
Authentik (§9.14), because the no-hint path may already be adequate at the cost of
one confirmation click.
