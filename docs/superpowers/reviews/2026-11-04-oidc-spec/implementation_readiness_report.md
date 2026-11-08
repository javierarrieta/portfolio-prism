# Stage 9 — Implementation Readiness Test (DetailedRange: lines 138-614)

Target: `docs/superpowers/specs/2026-11-04-oidc-identity-isolation-design.md`
Reviewers: `egm-implementation-readiness` (pragmatic tech lead), `egm-codebase-details` (codebase accuracy auditor), `egm-style` (copy editor).
Standard: `implementation-readiness-standard.md` for readiness and codebase-details; style persona applied its own standard.

Raw per-reviewer reports: `stage9_readiness.md`, `stage9_codebase.md`, `stage9_style.md`.

---

# Part 1 — egm-implementation-readiness

## Findings

**1. Severity: Blocker**
**Affected Scope:** §1 New module `src/auth/` (L138–182) + §4 Schema migration (L253–330) — configuration plumbing for `AuthMode` and bootstrap config is unspecified.
**Rationale:** The plan never states how `AuthMode` reaches the guard or `build_rocket`. `build_rocket(pool, currency_service, cors)` (`backend_rust/src/lib.rs:252`) and `init_db(&pool)` (`lib.rs:57`, called from `main.rs:25`) have no place for it, and `tests/common/mod.rs:7` wraps the old signature. The implementer must decide: is `AuthMode` a new `build_rocket` parameter, `.manage()`d state read via `&State<AuthMode>` in `FromRequest`, or read from env inside the guard? Same for the profile-allowlist check ("checked in `build_rocket` before any route is served") — `build_rocket` returns `Rocket<Build>` and has no error path; the plan must say whether it panics, returns `Result`, or whether the check lives in `main.rs`. And the backfill's "exit non-zero" (L310) is inside `init_db`, which returns `Result<(), sqlx::Error>` and is called by every test's `setup_db()`. The plan must name the new signatures (e.g. `build_rocket(pool, currency_service, cors, auth: AuthMode)`, `init_db(&pool, cfg: &StartupConfig) -> anyhow::Result<()>`) and where the process-exit decision is made, or requirement "startup fails" is untestable as written.

**2. Severity: Blocker**
**Affected Scope:** §4 Schema migration — `users` DDL (L255–268) and Backfill/Placeholder reconciliation (L296–330).
**Rationale:** `users.issuer` and `users.created_at` are `NOT NULL` with no default, but the plan never states what values the bootstrap/placeholder rows get. `subj = 'bootstrap:<subj>'` is specified; the `issuer` for a synthetic row is not (no IdP was involved), nor is the `email` for a `BOOTSTRAP_OWNER_SUBJ`-only row (`email` is also `NOT NULL`), nor the `subj`/`issuer` for the `Disabled`-mode `bootstrap:dev` row (L166), nor `created_at` format/timezone. Each is an independent invention by the implementer, and the `uq_users_issuer_subj` index plus the reconciliation rule ("`subj` begins with `bootstrap:`") depend on those exact strings. The plan must give a literal INSERT for each synthetic row.

**3. Severity: Major**
**Affected Scope:** §6 Route changes (L369–375) and "OpenAPI, CI & Frontend Impact" (L505–508); also asserted in Current State.
**Rationale:** The plan's factual premise is wrong. `PATCH /api/portfolios/<pid>/assets/<aid>` **does** carry `#[utoipa::path]` (`api_routes/transactions.rs:88–95`); it is missing from the `paths()` list in `src/openapi.rs:8–24`, which is why the published spec has 13 operations. Following the instruction "the implementation adds the missing annotation" produces a duplicate attribute / no effect. The plan must state the actual change: add `crate::api_routes::transactions::update_asset` to `ApiDoc`'s `paths()`.

**4. Severity: Major**
**Affected Scope:** §6 Route changes, final paragraph (L377–386).
**Rationale:** "a test that fails when a mounted API route is absent from the table" leaves the mechanism entirely to the implementer. Rocket exposes no runtime route registry usable from an integration test, and the "table" is prose in a Markdown file. The plan must specify the artefact: e.g. a `const ROUTE_TABLE: &[(&str, &str)]` in a test module listing (method, path) pairs, asserted against a hand-maintained list derived from `routes![]` — or a test that iterates a `#[utoipa]`-free inventory. Without naming the mechanism and its location, this gate cannot be written on first attempt, and it is the plan's own stated defence against route drift.

**5. Severity: Major**
**Affected Scope:** §2 OIDC endpoints (L184–215) and §3 (L217–227).
**Rationale:** Several handler-level decisions are left open: (a) the session cookie **name** is never given (only the OpenAPI cookie name `sessionCookie`, L500, which is a documentation string, not necessarily the wire name); (b) the cookie payload encoding is unspecified — "carrying `{ user_id, token_version }`" does not say whether it is JSON, a signed `id:version` string, or an `openidconnect`-provided type, and the tamper test depends on that format; (c) the transient state cookie holds three values in one cookie with no stated serialization or integrity scheme beyond `HttpOnly`; (d) `/auth/login?error=auth_failed` is emitted by §8 but §2's `/auth/login` row never says it must accept and echo an `error` parameter, nor what it does with it; (e) `/auth/logout` requires `end_session_endpoint` and `post_logout_redirect_uri` — the plan does not say the redirect URI must be registered in Authentik (only `/auth/callback` is listed in the operator note), so logout may fail against a real provider.

**6. Severity: Major**
**Affected Scope:** §5 `src/access.rs` (L332–367) and §6 nested-route check (L371–375).
**Rationale:** `AuthError` is used in the signatures but never defined — no variant list, no `IntoResponse` impl, no mapping to the §8 status codes. The plan must define `enum AuthError { Unauthenticated, NotFound, … }` and its `Responder`. Also `require_asset`/`require_transaction` are named but their resolution queries are not given (asset→`portfolio_id`; transaction→`asset_id`→`portfolio_id`), and the plan does not say whether a transaction whose asset row is missing (FKs are off, L300–306) returns `None`/404. Finally, the §6 nested check `asset.portfolio_id == id` duplicates logic that `require_asset` already performs; the plan should say which one owns it to avoid two divergent code paths.

**7. Severity: Major**
**Affected Scope:** §7 Per-user rate limit (L388–393).
**Rationale:** Under-specified for direct implementation: no crate or hand-rolled bucket is chosen; no eviction interval or "idle" threshold; no `Retry-After` integer value; and no decision on how the `Mutex<HashMap<i32,Bucket>>` is shared (`.manage()`d state vs. a `LazyLock` static), which determines whether integration tests can reset it. The test "allows 10, blocks the 11th, refills" is not writable without an injectable clock — the plan must state that the bucket takes a time source, or the refill test is nondeterministic.

**8. Severity: Major**
**Affected Scope:** §6 route table row `GET /api/portfolios/` (L361) and OpenAPI section (L497–501).
**Rationale:** "each row carries the caller's `Access`" changes the `PortfolioOut` response schema, but the plan does not state the field name, its JSON representation, or whether `Access` is `Serialize`/`ToSchema`. This propagates into `docs/openapi/openapi.json`, the CI diff, and `frontend/src/types/api.d.ts`. The frontend section says only that calls are routed through `apiFetch.ts`, so no component is told to consume the new field — an unreconciled schema change.

**9. Severity: Major**
**Affected Scope:** §4 Backfill / §1 Disabled mode interaction with `seed_portfolio` (L296–312).
**Rationale:** `owner_id` becomes `NOT NULL`, but `tests/common/mod.rs:22` `seed_portfolio` inserts without `owner_id`, and the isolation matrix needs two users' portfolios. The plan never lists the required test-helper changes (`seed_user`, `as_user` are named at L544 but `seed_portfolio`'s new signature is not). It must state `seed_portfolio(pool, owner_id, name, currency)` and how `Disabled { as_user }` is injected per test given `build_rocket` is shared.

**10. Severity: Nit**
**Affected Scope:** §6 table rows `PATCH /api/portfolios/:id` (L363) and prose at L373–375.
**Rationale:** Described as "renaming/deleting is owner-only", but `update_portfolio` mutates `currency`/`base_currency` only (`api_routes/portfolios.rs:176–186`); there is no rename endpoint. Wording should match the real semantics so a reviewer doesn't reject the diff.

**11. Severity: Nit**
**Affected Scope:** §6 table, `GET /api-docs/openapi.json`, `GET /openapi.json` (L376).
**Rationale:** Both are the same handler `openapi_json` mounted twice (`lib.rs:245,270`). The plan does not say whether the root `/openapi.json` stays public (it is not under `/api`) or is also authenticated; the test list only covers `/api-docs/openapi.json`.

**12. Severity: Nit**
**Affected Scope:** Configuration & Deployment (L440–460); Rollout step 1 (L553–558).
**Rationale:** `AUTH_MODE=disabled` + `ROCKET_PROFILE=dev` in `start.sh` is stated, but no `ROCKET_SECRET_KEY` is provided for the dev path; the plan relies on "a debug build auto-generates one" without noting `cargo run` in `start.sh` is a debug build, so sessions break across restarts silently. One sentence would close it.

## Not assessed
- Lines 1–137 (orientation only; findings above are scoped to 138–614).
- Frontend `App.tsx` / `CsvImportModal.tsx` / `PortfolioDetail.tsx` call-site inventory (not opened; the "~14 fetch calls" count is unverified).
- `frontend/nginx.conf`, `frontend/vite.config.js`, `.github/workflows/ci.yml`, `k8s/configmap.yaml`/`ingress.yaml` contents (not read this run; only `k8s/app-deployment.yaml` was verified — probes on `:8000/` and `envFrom: configMapRef` confirmed, and the deployment's PVC is named `postgresql-db-pvc` despite SQLite, which the plan does not mention).
- `openidconnect = "4.1"` API surface and MSRV claims; Rocket `secrets` feature behaviour.
- Authentik-side operator steps beyond the redirect-URI note in finding 5(e).
---

# Part 2 — egm-codebase-details

## Findings

**1. Blocker — §6 route-table rationale (~lines 373–377) and "OpenAPI, CI & Frontend Impact" (~lines 477–479)**
Document asserts `PATCH /api/portfolios/<pid>/assets/<aid>` "carries no `#[utoipa::path]` today" and prescribes "adds the missing annotation." Reality: `update_asset` **does** carry `#[utoipa::path(patch, path = "/api/portfolios/<portfolio_id>/assets/<asset_id>", …)]` at `backend_rust/src/api_routes/transactions.rs:89–96`. It is absent from `docs/openapi/openapi.json` only because `update_asset` is omitted from the `paths(...)` list in `backend_rust/src/openapi.rs:8–23` (13 entries listed; `update_asset` is not among them). Correction: the fix is to add `crate::api_routes::transactions::update_asset` to the `paths(...)` list in `openapi.rs`; the annotation already exists. As written, an implementer will hunt for a nonexistent gap and the spec's "missing annotation" claim is false.

**2. Blocker — §5 Access level semantics (~lines 335–337)**
"Deleting an asset cascades to its transactions, so it is owner-only." No cascade exists: `transactions` is declared `FOREIGN KEY (asset_id) REFERENCES assets(id)` with **no `ON DELETE CASCADE`** (`backend_rust/src/lib.rs`, `init_db`), and the document itself states `PRAGMA foreign_keys` is never enabled. `delete_asset` (`backend_rust/src/api_routes/transactions.rs:141–155`) deletes only the asset row, leaving orphaned transaction rows. Correction: state that asset deletion currently **orphans** transactions (dangling `asset_id`), and that owner-only deletion is a policy choice, not a cascade safeguard; the design should also decide what happens to the orphaned rows (zero-decision rule).

**3. Major — §6 nested-route check (~lines 369–377)**
"The nested-route check `asset.portfolio_id == id` closes a hole where a caller who owns portfolio 1 … could write through portfolio 1's path." The hole does not exist today: `update_asset` already runs `UPDATE assets SET currency = ? WHERE id = ? AND portfolio_id = ?` and a portfolio-scoped `SELECT` returning 404 (`transactions.rs:103–127`), and `create_transaction` already gates on `SELECT * FROM assets WHERE id = ? AND portfolio_id = ?` (`transactions.rs:169–177`). Correction: describe the check as **already present and to be preserved** alongside `require_asset`-style checks, not as a new hole-closer; the isolation-matrix cross-portfolio test case remains valid as a regression guard.

**4. Major — §4 "Making owner_id NOT NULL" (~lines 258–263)**
"The existing `migrate_one_column` helper already performs the rename → add → copy → drop rebuild … The `owner_id` rebuild reuses that mechanism." The helper is hard-coded to convert columns to `TEXT NOT NULL DEFAULT '0'`: every branch issues `ALTER TABLE {table} ADD COLUMN {column} TEXT NOT NULL DEFAULT '0'` (`backend_rust/src/lib.rs`, `migrate_one_column`, ~lines 140–210), and it is only invoked for decimal→TEXT columns by `migrate_decimal_columns`. Reusing it verbatim would create `owner_id TEXT`, not `INTEGER NOT NULL`. Correction: specify generalizing `migrate_one_column` to take a target column type (and default), or add a dedicated INTEGER rebuild path; also note the helper's crash-state logic keys on `TEXT` type checks that would misfire for INTEGER.

**5. Nit — §6 table + parenthetical (~lines 348, 375–377)**
"`PATCH /api/portfolios/:id` — renaming/deleting is owner-only" and "portfolio rename is `PATCH /api/portfolios/:id`, not `PUT`." `update_portfolio` (`backend_rust/src/api_routes/portfolios.rs:172–208`) updates only `currency`/`base_currency`; `PortfolioUpdate` carries only `currency` (`docs/openapi/openapi.json`). There is no rename endpoint. Verb citation is correct; the "rename" characterization is not. Correction: call it the currency-update route.

**6. Nit — §1 profile check (~lines 152–165)**
The check is "checked in `build_rocket` before any route is served" with a "process-exiting error," but `build_rocket` returns `Rocket<Build>` (`backend_rust/src/lib.rs:250`) and is called by `#[launch]` (`backend_rust/src/main.rs:28`) and by `backend_rust/tests/common/mod.rs:7–11`. The plan doesn't say whether to panic, `std::process::exit`, or change the signature (a signature change breaks the test helper). Specify the mechanism.

**Verified accurate (no finding):** `lib.rs:252–271` routes! citation and 14 mounted API operations; `lib.rs:53` CORS `Authorization` header; committed spec = 10 paths / 13 operations, no `securitySchemes`; `rocket = { version = "0.5", features = ["json"] }` (`backend_rust/Cargo.toml:8`); `utoipa-swagger-ui` present but unused (`Cargo.toml:17`, no source use); `reqwest` rustls-only (`Cargo.toml:12`); no `rand` dependency; `rust:1.92-slim` build image (`backend_rust/Dockerfile:4`); `k8s/configmap.yaml` lowercase, only `DATABASE_URL`, no `ROCKET_*`; deployment probes and `wait-for-backend` initContainer hit `GET /` on 8000, frontend probes `:80`, `envFrom: configMapRef`, `replicas: 1` (`k8s/app-deployment.yaml`); `tests/common/mod.rs` wraps `build_rocket(pool, currency_service, cors)`, no `seed_user` yet (consistent with "added"); `users`/`portfolio_members` DDL does not collide with existing tables; route paths in the §6 table match the router and committed spec (incl. `/api/assets/lookup`, tax-summary, performance, root `/openapi.json` mounted at `/` and `/api-docs`).

## Not verified
Killed before reading: `frontend/nginx.conf` (`location /api`, `/api-docs`, `try_files` claims), `frontend/vite.config.js` proxy map, `.github/workflows/ci.yml` (cargo test / OpenAPI diff / generate-types / npm test / no lint), `start.sh`, `frontend/src/App.tsx` "~14 fetch calls", existence of `CsvImportModal.tsx`/`PortfolioDetail.tsx`, MSW setup, `package.json` scripts (`generate-types`, `lint`), `frontend/src/types/api.d.ts`, `k8s/app-service.yaml` (port 80 only), `k8s/ingress.yaml`, commit `2753049`, `openidconnect = "4.1"` existence and MSRV 1.88, local toolchain "1.98.1", Rocket 0.5 `secrets`-feature gating (consistent with upstream docs, not re-checked in-vendored-source), `api_routes/analytics.rs`/`lookup.rs` handler attributes (paths verified via committed spec only).
---

# Part 3 — egm-style

## Findings

### 1. `Secure` cookie rule is stated unconditionally and contradicts the dev path
**Severity**: Blocker
**Affected Scope**: §8 Error handling / Security Considerations — "**Cookie.**" (line 565) and "the app sets `Secure` cookies unconditionally in production" (lines 573-574); interacts with §2 "Session cookie: … `HttpOnly`, `Secure`, `SameSite=Lax`" (lines 207-209).
**Rationale**: "`Secure`" is listed in §2 as an unconditional attribute of the session cookie, and the Security bullet says "unconditionally in production". "Production" is undefined here — the document elsewhere uses three other terms for the same axis (`release` build, allowlisted profile, `AUTH_MODE=oidc`). An implementer cannot tell whether `Secure` is set in `Disabled`/dev mode, where `./start.sh` serves over plain `http://localhost`. A `Secure` cookie over http is dropped by the browser, so login silently fails in dev; omitting it in prod is a security defect.
**Rewrite**: Replace both statements with one per-mode rule: "The session cookie sets `Secure` when the request arrived over https; in `AuthMode::Disabled` the flag follows the request scheme. There is no separate 'production' concept in the code — the decision keys on the request scheme, not on the profile." Alternatively introduce and document a named variable, and state the value `./start.sh` sets.

### 2. "Spec 2 changes no route table" is contradicted by the Deferred Work table
**Severity**: Major
**Affected Scope**: §6 Route changes, closing paragraph (lines 384-385): "Owner-only operations are already fixed at `Owner`; Spec 2 changes no route table." Compare Deferred Work (line 578): "2 — Sharing | `portfolio_members` write paths, share/unshare endpoints, …".
**Rationale**: Spec 2 is defined to add share/unshare endpoints, i.e. new routes. The claim as written is false on its face and gives a future implementer licence to assume the §6 table is complete after Spec 2.
**Rewrite**: "`Owner`-only routes are final. Spec 2 adds new sharing routes and does not change the access level of any route listed above."

### 3. "Deleting an asset cascades to its transactions" uses a term the spec disclaims
**Severity**: Major
**Affected Scope**: §5 New module `src/access.rs`, after the access table (lines 346-347): "Deleting an asset cascades to its transactions, so it is owner-only even though adding one is not." Compare §4 (lines 275-279): "Foreign keys are not enforced … the `REFERENCES` clauses … are documentation, not constraints".
**Rationale**: "Cascades" is the term of art for `ON DELETE CASCADE`, which the spec explicitly says is not in effect. An implementer may rely on the database to delete child transactions and leave orphans.
**Rewrite**: "Deleting an asset also deletes its transactions. This is application code, not an FK cascade — `PRAGMA foreign_keys` is off (§4) — so the delete must be transactional in the handler."

### 4. The `Editor` row omits asset update, and the stated rule is contradicted by its own table
**Severity**: Major
**Affected Scope**: §5 access table (lines 341-344) and the rationale line (lines 345-346): "`Editor` may add and amend, `Owner` alone may destroy structure."
**Rationale**: Two problems. (a) The `Editor` row lists "add an asset, and add or delete transactions" but omits updating an asset, which §6 grants at `Editor` (`PATCH /api/portfolios/:id/assets/:aid`, line 361). "Amend" appears only in the prose rationale, never in the table. (b) "Owner alone may destroy structure" is contradicted by `Editor` deleting transactions, which is destroying data. An implementer reading only the table would deny asset updates to editors.
**Rewrite**: Add to the `Editor` row: "update an asset (`PATCH …/assets/:aid`)". Replace the rationale with: "The distinction is entries versus container: `Editor` may add or remove entries (assets, transactions) and correct an asset; `Owner` alone may alter the container (rename or delete the portfolio, delete an asset and with it its transactions)."

### 5. `resolve` lookup order is under-specified — "on mismatch" and "on no row" have no stated referents
**Severity**: Major
**Affected Scope**: §5 (lines 332-334): "`resolve` reads `portfolios.owner_id` first; on mismatch it falls back to `portfolio_members`; on no row it returns `None`."
**Rationale**: "Mismatch" could mean `owner_id != user.id` or "no portfolio row at all"; "on no row" could mean no portfolio row or no members row. The three-way decision the caller depends on (`None` for "portfolio does not exist" vs `None` for "not yours") is not decidable from this sentence.
**Rewrite**: "`resolve` loads the portfolio row by id. No portfolio row → `None`. `owner_id == user_id` → `Owner`. Otherwise it looks up the `portfolio_members` row for `(portfolio_id, user_id)` and maps `role` to `Editor` or `Viewer`; no members row → `None`. `resolve` does not distinguish 'does not exist' from 'not yours'; both are `None`, and callers map `None` to `404`."

### 6. The handler prohibition is narrower than the rule it implements
**Severity**: Major
**Affected Scope**: §5 (lines 334-336): "This is the **only** module permitted to look up a portfolio, asset, or transaction by caller-supplied id. Handlers must not contain `WHERE id = ?` against those tables."
**Rationale**: The first sentence states the rule; the second is the testable form, and it is a literal string pattern. `WHERE portfolios.id = ?`, `WHERE t.id = ?`, a join, or a subquery all satisfy the second sentence and violate the first. This is the exact failure mode the "chokepoint" bullet in Security Considerations says is the design's main risk.
**Rewrite**: "Handlers must not query `portfolios`, `assets`, or `transactions` by a caller-supplied id at all — in any predicate form (`WHERE id = ?`, table-qualified `WHERE portfolios.id = ?`, joins, or subqueries) — except through `access`. A reviewer checking a diff should search for the table names, not for the literal `WHERE id = ?`."

### 7. The nested-route hole is described with the wrong actor and contradicts the test description
**Severity**: Major
**Affected Scope**: §6 (lines 371-372): "closes a hole where a caller who owns portfolio 1 and has access to asset 99 belonging to portfolio 2 could write through portfolio 1's path." Compare Testing Strategy (lines 506-508): "B owns portfolio 1 and asset 99 in portfolio 2 is used through portfolio 1's path."
**Rationale**: In §6 the caller owns portfolio 1 and the asset belongs to portfolio 2, so the caller does **not** "have access to asset 99" — that is the point of the check. In Testing Strategy the ownership is reversed (B owns portfolio 1) and the sentence has no grammatical subject for "is used". Two incompatible readings of the same attack; an implementer can build a test that passes without exercising the hole.
**Rewrite**: Use one example in both places: "A owns portfolio 1; B owns portfolio 2 and asset 99. `POST /api/portfolios/1/assets/99/transactions` from B passes `require(…, portfolio 1, Editor)` only if the check is missing — with the check, `asset.portfolio_id (2) != id (1)` and the request returns `404`."

### 8. `GET /api/portfolios/` response change is unstated
**Severity**: Major
**Affected Scope**: §6 route table (line 354): "each row carries the caller's `Access`".
**Rationale**: This is a response-schema change, but no field name, representation, or placement is given, and the OpenAPI section (lines 466-478) lists only `401`/`404` additions — so the generated TypeScript types would not gain the field. An implementer must invent the wire format.
**Rewrite**: "Each portfolio object in the list response gains a string field `access` with values `owner` | `editor` | `viewer`, so the SPA can hide controls it cannot use. The field is added to the published schema and therefore to `frontend/src/types/api.d.ts`."

### 9. "authenticated only" for `/api/assets/lookup` is ambiguous
**Severity**: Major
**Affected Scope**: §6 route table (line 365): "`GET /api/assets/lookup` | authenticated only; per-user rate limit".
**Rationale**: The column is "Change" and every other row names an `Access` level. "Authenticated only" can be read as "requires authentication and no `Access`" or as "the only change is authentication". The first is presumably intended (lookup is not portfolio-scoped) but is not stated.
**Rewrite**: "requires an authenticated caller; no portfolio `Access` applies because the lookup is not portfolio-scoped. Adds the per-user rate limit from §7."

### 10. `/openapi.json` appears without provenance
**Severity**: Major
**Affected Scope**: §6 route table (line 368): "`GET /api-docs/openapi.json`, `GET /openapi.json` | authenticated".
**Rationale**: The Current State table (line 71) documents only `/api-docs/openapi.json`. A second bare path appears here with no citation and no statement of whether it exists today or is being added. An implementer will guard a route that may not exist, or miss one that does.
**Rewrite**: Cite the `routes![]` entry for each path, or drop `/openapi.json` if it is not mounted. If it is mounted, add it to the Current State evidence table.

### 11. Browser vs. non-browser failure routing is undecidable as written
**Severity**: Major
**Affected Scope**: §8 Error handling (lines 401-402): "OIDC `state`/`nonce` mismatch, token validation failure, Authentik unreachable (browser flow on `/auth/callback`) | no session; `302` to `/auth/login?error=auth_failed`" and "Same failures on a direct (non-browser) call to `/auth/*` | `401` JSON".
**Rationale**: "Browser flow" and "direct (non-browser) call" are not defined by any request property, so the branch cannot be implemented consistently. Separately, mapping "Authentik unreachable" to a redirect to `/auth/login` risks a loop when `/auth/login` itself needs discovery metadata from the unreachable IdP.
**Rewrite**: "The `302` path applies only to `GET /auth/callback`. All other `/auth/*` failures return `401` JSON. For IdP-unreachable specifically, `/auth/callback` returns `502` with no redirect, and `/auth/login` returns `500` if discovery metadata is unavailable, so a downed IdP cannot produce a redirect loop."

### 12. `AuthMode::Disabled` gating sentence reads as the inverse of the rule
**Severity**: Major
**Affected Scope**: Configuration & Deployment (lines 424-425): "`AuthMode::Disabled` is honoured only on an allowlisted profile …, regardless of `AUTH_MODE`."
**Rationale**: `AUTH_MODE` is the only input that selects `Disabled`, so "regardless of `AUTH_MODE`" is incoherent as written; a reader can take it to mean the profile allowlist applies even when `AUTH_MODE=oidc`. The intended rule — the two checks are ANDed, and the combination `AUTH_MODE=disabled` + non-allowlisted profile is fatal — is never stated.
**Rewrite**: "`AUTH_MODE` selects the mode; the profile allowlist gates it. `AUTH_MODE=disabled` on a profile outside `debug`/`dev`/`test` is a process-exiting error at launch, not a fallback to `oidc`."

### 13. "before the first rollout step, not before the OIDC step" is a double negative
**Severity**: Major
**Affected Scope**: Configuration & Deployment, Kubernetes changes (lines 449-452): "The `Secret` must exist **before the first rollout step**, not before the OIDC step".
**Rationale**: The corrective clause inverts on first reading and reads as though the Secret must *not* exist before the OIDC step. The actual requirement is a timing correction (step 1, not step 3).
**Rewrite**: "The `Secret` is required at rollout step 1, not at step 3: the backend image is a release build, and Rocket exits at launch when a release build has the `secrets` feature and no `secret_key`."

### 14. Transient OAuth state cookie is under-specified and confusable with the session cookie
**Severity**: Major
**Affected Scope**: §2 OIDC endpoints, `/auth/login` row (line 199): "store all three in one short-lived `HttpOnly` `SameSite=Lax` cookie (10 min)".
**Rationale**: No cookie name is given, `Secure` is omitted while the session cookie (line 208) sets it, and the two cookies have identical `HttpOnly`/`SameSite` attributes with different lifetimes. An implementer can reuse one name and clobber the session, or omit `Secure` on the state cookie.
**Rewrite**: "Store all three in a single cookie named `pp_oauth_state`, `HttpOnly`, `Secure`, `SameSite=Lax`, `Max-Age=600`, path `/`. It is distinct from and never reused as the session cookie defined below, and is cleared in `/auth/callback`."

### 15. `post_logout_redirect_uri` value is unspecified
**Severity**: Major
**Affected Scope**: §2, `/auth/logout` row (line 201): "`302` to Authentik's `end_session_endpoint` with `post_logout_redirect_uri`".
**Rationale**: The parameter is named but its value is not given, and Authentik validates it against a registered allowlist. An implementer picks a value; the operator does not know to register it; logout fails at cutover.
**Rewrite**: "`302` to `end_session_endpoint?post_logout_redirect_uri=https://<host>/`; `https://<host>/` must be added to the provider's post-logout redirect allowlist (operator task, see Configuration & Deployment)."

### 16. Rate limiter eviction and `Retry-After` are unimplementable as written
**Severity**: Major
**Affected Scope**: §7 (lines 388-391): "returning `429` with `Retry-After` when exhausted. Managed state in a `Mutex<HashMap<i32, Bucket>>` with periodic eviction of idle entries."
**Rationale**: "Periodic" and "idle" carry no interval, so the requirement cannot be verified or tested; `Retry-After` has no stated units or value. The rest of the section is precise (capacity 10, refill 1/second), which makes the gaps stand out.
**Rewrite**: "On `429`, send `Retry-After: 1` (seconds — the time to accrue one token). Evict buckets untouched for 15 minutes, swept every 5 minutes." If the intervals are not load-bearing, say so explicitly rather than leaving them open.

### 17. "The draft wrote `configMap.yaml`" references an artifact outside the document
**Severity**: Major
**Affected Scope**: Configuration & Deployment (line 453): "`configmap.yaml` (lowercase in this repo; the draft wrote `configMap.yaml`)".
**Rationale**: A design document has no "draft" its readers can consult. The parenthetical is review history masquerading as a specification note.
**Rewrite**: "`configmap.yaml` (lowercase in this repo) gains …". Move any revision note to a Revision History section.

### 18. Three terms for one concept: self-registration / open signup / open registration
**Severity**: Major
**Affected Scope**: §4 (lines 296-297) "open self-registration in Authentik"; Configuration & Deployment (line 433) "Self-registration in Authentik is what makes open signup work"; Security Considerations (line 569) "Open signup + third-party quota"; Decisions Summary (line 95) "open signup".
**Rationale**: The security argument depends on this being one specific Authentik setting. Three names invite a reader to treat them as three configurations, and "open signup" additionally overlaps with "sign out everywhere" style UI language elsewhere.
**Rewrite**: Standardise on "Authentik self-registration" everywhere; define it once at first use as "the Authentik setting that lets anyone create an account". Rewrite line 433 as: "Authentik self-registration is the only registration path; this application adds none. Because it makes the `email` claim claimable by anyone, …".

### 19. "existing users" in rollout step 1 — there are no users
**Severity**: Major
**Affected Scope**: Rollout & Rollback step 1 (line 534): "behaviour is unchanged for existing users."
**Rationale**: "User" is a defined term in this spec (a `users` row, created by OIDC login or the bootstrap placeholder). Pre-migration there are no `users` rows. The intended meaning is "existing clients/data".
**Rewrite**: "no observable change for existing clients or data."

### 20. "single-tenant" and "cannot be raced" are undefined shorthand
**Severity**: Major
**Affected Scope**: Rollout & Rollback step 5 (lines 545-547): "until then the deployment is single-tenant, which is the state the backfill was designed for and the window in which an email-keyed handover cannot be raced."
**Rationale**: "Single-tenant" is not used anywhere else and is not a property of this deployment (it is single-*identity*). "Raced" is jargon for a specific attack — an attacker registering the address first — which the spec spells out in full at line 297-299. The compressed version is weaker than the original.
**Rewrite**: "Until this step only the bootstrap identity can sign in, so no other identity can claim the placeholder row first. This is the state the backfill assumes."

### 21. "Safety" used for a security property; "Chokepoint discipline" is a buzzword heading
**Severity**: Major
**Affected Scope**: Security Considerations (lines 554-557): "**Chokepoint discipline.** The design's safety rests on `access` being the only path to a portfolio by id."
**Rationale**: "Safety" is the wrong property — the failure is a security failure, and the document has a section titled Security Considerations. "Discipline" names a human practice, not the mechanism, and the mechanism is what the bullet describes.
**Rewrite**: Heading "Chokepoint enforcement". Body: "The isolation guarantee depends on `access` being the only path to a portfolio by id. A handler that queries those tables directly breaks isolation without any compile-time or runtime signal."

### 22. The startup banner requirement is stated twice, in two unmeasurable forms
**Severity**: Major
**Affected Scope**: §1 profile table (line 163): "yes — with a prominent startup banner"; Security Considerations (line 560): "Logged loudly when active."
**Rationale**: "Prominent" and "loudly" are not verifiable and are not the same requirement, so neither can be tested. The Testing Strategy (lines 495-496) tests the refusal case but not the banner, so nothing pins the wording down.
**Rewrite**: State it once in §1: "On entering `Disabled` mode the process emits one `WARN`-level log line naming the mode, the active profile, and the `as_user` id." Replace the Security bullet with "Emits the §1 startup warning."

### 23. Unmeasurable capacity claim
**Severity**: Major
**Affected Scope**: Known Limitations (lines 588-590): "SQLite on one PVC with `replicas: 1` is appropriate for tens of users and is the first thing to replace under real load."
**Rationale**: "Appropriate for tens of users" is an unverifiable capacity claim with no measurement, workload, or bottleneck named; "the first thing to replace under real load" is conversational. Non-Goals already declares SQLite replacement out of scope, which is the load-bearing statement.
**Rewrite**: "Capacity has not been measured. SQLite on one PVC with `replicas: 1` is the current constraint and replacing it is out of scope (Non-Goals)."

### 24. "Highest-risk line in the design" is an unranked superlative
**Severity**: Nit
**Affected Scope**: §1 (line 166): "This is the highest-risk line in the design: a forgotten environment variable must not silently expose every portfolio".
**Rationale**: No risk ranking exists in the document, so the superlative cannot be checked. The technical content after the colon stands on its own.
**Rewrite**: Delete the first clause: "A forgotten environment variable must not expose every portfolio, so the unset state is the refusing state."

### 25. "the release image resolves to profile `release` and is refused by default" — wrong object
**Severity**: Nit
**Affected Scope**: §1 (lines 167-169): "the release image resolves to profile `release` and is refused by default".
**Rationale**: The image is not refused; `AuthMode::Disabled` is refused on that profile. As written a reader can conclude the release image cannot start at all.
**Rewrite**: "…resolves to profile `release`, a profile on which `AuthMode::Disabled` is refused."

### 26. "the type system makes protection impossible to forget … no other way to obtain one"
**Severity**: Nit
**Affected Scope**: §1 (lines 179-181): "the type system makes protection impossible to forget on a route that reads the user, because there is no other way to obtain one."
**Rationale**: "Impossible to forget" is an absolute; "one" has two candidate antecedents (`AuthedUser`, "a route"). The verifiable statement is the constructor-uniqueness claim.
**Rewrite**: "A route is protected exactly when its handler takes an `AuthedUser` parameter; `AuthedUser` has no other constructor."

### 27. Editorialising about the dependency tree
**Severity**: Nit
**Affected Scope**: §1 (lines 182-188): "rather than pulling `native-tls` into a deliberately rustls-only build … adding `rand` directly risks two majors in the tree."
**Rationale**: "Deliberately" attributes intent rather than stating the constraint; "two majors" is non-standard shorthand for two incompatible major versions.
**Rewrite**: "…rather than adding `native-tls`; the dependency tree contains rustls only. Adding `rand` directly risks resolving two incompatible major versions of `rand`."

### 28. Cross-reference points at the wrong section; casual MSRV phrasing
**Severity**: Nit
**Affected Scope**: §1 (lines 190-194): "without this the session mechanism in §3 does not compile" and "so both clear it".
**Rationale**: The private-cookie issuance is §2; §3 is validation. "Clear it" is casual for a hard compatibility check.
**Rewrite**: "…the session mechanism in §2 and §3 does not compile." / "`openidconnect` 4.1.0 requires Rust 1.88; the build image is `rust:1.92-slim` and the local toolchain is 1.98.1, both ≥ 1.88."

### 29. "the guard resolves to `as_user`" and "On every guarded request the guard"
**Severity**: Nit
**Affected Scope**: §1 (line 171) and §3 (line 219).
**Rationale**: The guard does not resolve to an integer; it resolves the caller identity. Line 219 repeats "guard" twice in six words.
**Rewrite**: "In `Disabled` mode the guard resolves the caller to `as_user`." / "On each request to a protected route, the request guard:".

### 30. `Disabled` mode `as_user` selection is a run-on with implicit priority
**Severity**: Nit
**Affected Scope**: §1 (lines 171-175): "In integration tests that id comes from `seed_user`. In a non-production deployment it is the bootstrap placeholder user when … and otherwise a seeded row with `subj = 'bootstrap:dev'`".
**Rationale**: Three sources are given in prose with no stated precedence, and "non-production deployment" is a fourth term for the profile axis.
**Rewrite**: "`as_user` is resolved in this order: (1) integration tests — the id from `seed_user`; (2) `BOOTSTRAP_OWNER_SUBJ` or `BOOTSTRAP_OWNER_EMAIL` set — the corresponding bootstrap placeholder row; (3) otherwise — a row with `subj = 'bootstrap:dev'`, created at startup if absent."

### 31. `prod` in the deny column duplicates "any other value"
**Severity**: Nit
**Affected Scope**: §1 profile table (line 164): "`release`, `prod`, any other value, unset in a release build | no".
**Rationale**: `prod` is already covered by "any other value"; listing it implies special handling that the prose does not describe.
**Rewrite**: "`release`, any value not in the row above, or unset in a release build | no — process-exiting error at launch."

### 32. "`/auth/me` is the only endpoint the SPA needs to know it is signed in"
**Severity**: Nit
**Affected Scope**: §2 (lines 205-206).
**Rationale**: The sentence parses as "the only endpoint the SPA needs in order to know", but the SPA also needs `/auth/login` (via the `apiFetch` 401 redirect, line 477). "Only" is scoped to the wrong set.
**Rewrite**: "`GET /auth/me` is the only endpoint the SPA calls to determine whether the caller is signed in. It is under `/auth`, not `/api`, so it is outside the `/api` OpenAPI surface."

### 33. "the revocation switch" and "sign out everywhere action"
**Severity**: Nit
**Affected Scope**: §3 (lines 225-227).
**Rationale**: "Switch" is metaphor for a monotonically incremented integer; "action" implies a UI affordance, and this spec delivers no UI.
**Rewrite**: "`token_version` is the revocation counter: incrementing it invalidates every outstanding cookie for that user without a session table. `POST /auth/logout-all` performs the increment and clears the current cookie, so a client can end all of a user's sessions with one request."

### 34. "created but unused" conflicts with the reads in §5 and §6
**Severity**: Major
**Affected Scope**: §4 (lines 259-260): "`portfolio_members` is created but unused in this spec, deliberately". Compare §5 (line 333) "falls back to `portfolio_members`" and §6 (line 354) `… IN (SELECT portfolio_id FROM portfolio_members …)`.
**Rationale**: The table is read in two places in this spec; what is true is that nothing writes to it (as Non-Goals line 55 says). "Unused" is the wrong word and directly contradicts two later sections.
**Rewrite**: "`portfolio_members` is created in this spec but never written to; it is read by `access::resolve` and by `GET /api/portfolios/`, and is empty in every deployment until Spec 2. Creating it now lets `access` be written once against the final access model."

### 35. "safely" and doubled "already" in the migration rationale
**Severity**: Nit
**Affected Scope**: §4 (lines 263-266): "The existing `migrate_one_column` helper already performs the rename → add → copy → drop rebuild safely on a dedicated connection".
**Rationale**: "Safely" is an unverified quality adjective; the specific property (crash-safety, cited in Current State line 74) is what matters. "Already" appears twice within three lines.
**Rewrite**: "The `migrate_one_column` helper performs a rename → add → copy → drop rebuild on a dedicated connection with `legacy_alter_table` enabled (Current State: crash-safe column rebuild). The `owner_id` rebuild reuses it, so no new migration framework is introduced."

### 36. "Refusing to start is intentional … worse than downtime"
**Severity**: Nit
**Affected Scope**: §4 (lines 303-304).
**Rationale**: "Worse than downtime" is a value judgement restating Requirement 4. The decision is already recorded in Decisions Summary line 101 with the same reasoning.
**Rewrite**: "Refusing to start is the behaviour required by Requirement 4: portfolios must not be assigned to an arbitrary user."

### 37. "the operator note … keeps the window closed" — wrong actor, mixed metaphor
**Severity**: Nit
**Affected Scope**: §4 (lines 300-301): "The operator note in [Configuration & Deployment] keeps the window closed by completing the handover before open registration is enabled."
**Rationale**: A document section cannot complete a handover; the operator does. "Window closed" is reused at lines 435 and 570 with slightly different referents.
**Rewrite**: "The operator procedure in [Configuration & Deployment] requires the handover to complete before self-registration is enabled."

### 38. "the real asset-update route" — draft-relative voice, stated three times in one sentence
**Severity**: Major
**Affected Scope**: §6 (lines 373-376): "(the real asset-update route — there is no `PUT /api/assets/:id`; asset update is `PATCH` nested under the portfolio, and portfolio rename is `PATCH /api/portfolios/:id`, not `PUT`)".
**Rationale**: "The real route" only means something relative to an earlier draft the reader has not seen. The same correction is made three times in one sentence, and a fourth time in Open Questions ("corrected to the real verbs").
**Rewrite**: "Asset update is `PATCH /api/portfolios/:id/assets/:aid`; portfolio rename is `PATCH /api/portfolios/:id`. Neither has a `PUT` variant." Delete the Open Questions restatement.

### 39. "renaming/deleting is owner-only" on a `PATCH` row
**Severity**: Nit
**Affected Scope**: §6 route table (line 355): "`PATCH /api/portfolios/:id` | `require(…, Owner)` — renaming/deleting is owner-only".
**Rationale**: `PATCH` does not delete; the deleting route has its own row. The gloss describes the neighbouring row.
**Rewrite**: "`require(…, Owner)` — rename is owner-only."

### 40. Grammar slip and third restatement of the 13-vs-14 count
**Severity**: Nit
**Affected Scope**: §6 (lines 378-383): "the `routes![]` lists in `build_rocket` (`lib.rs:252-271`), which is 14 API operations".
**Rationale**: "lists … which is" disagrees in number. The 13-vs-14 fact is stated in Current State (line 72), here, and again in OpenAPI Impact (lines 470-471).
**Rewrite**: "The table lists the 14 API operations in the `routes![]` lists of `build_rocket` (`lib.rs:252-271`), not the 13 in `docs/openapi/openapi.json`." Drop the restatement in OpenAPI Impact to "see §6".

### 41. "In-process is sufficient" and the self-referential limitation note
**Severity**: Nit
**Affected Scope**: §7 (lines 390-392): "In-process is sufficient because the deployment is `replicas: 1`; a second replica makes this per-pod rather than global, which is recorded as an accepted limitation rather than solved with new infrastructure."
**Rationale**: "Sufficient" states adequacy without a threshold; the final clause describes the document's own editorial choice instead of the consequence.
**Rewrite**: "In-process, because `k8s/app-deployment.yaml` sets `replicas: 1`. With N replicas the effective limit is capacity × N per second; see Known Limitations."

### 42. "only a fixed string" is a fragment naming no string; "genuine 500s" is a non-sequitur
**Severity**: Nit
**Affected Scope**: §8 (lines 405-408): "IdP error text is never reflected into a page or a response body — only a fixed string." and "…so a probing pattern is visible in logs and distinguishable from genuine 500s."
**Rationale**: The fixed string is `auth_failed` (line 401) — say so. Denials are `401`/`404`, so contrasting them with 500s needs the missing link (the existing pattern collapses database errors to `InternalServerError`). "A probing pattern" is vague.
**Rewrite**: "The only IdP error value returned to the client is the literal `auth_failed`; no IdP message text reaches a response body or a page." / "Authorization denials are logged at `warn` with user id and target id, so repeated denials by one caller are greppable and are not mixed into the `InternalServerError` log stream."

### 43. "regardless of `AUTH_MODE`" / "requirement 10 breaks the moment this lands"
**Severity**: Nit
**Affected Scope**: Configuration & Deployment (lines 428-431): "otherwise requirement 10 breaks the moment this lands".
**Rationale**: "The moment this lands" is conversational; the requirement number is cited instead of its content, forcing a lookup.
**Rewrite**: "otherwise Requirement 10 (`./start.sh` works with no OIDC provider) fails as soon as this spec is implemented."

### 44. "Without both" and "a silent redirect loop"
**Severity**: Nit
**Affected Scope**: Configuration & Deployment (lines 446-447): "Without both, the SPA redirects to `/auth/login` and receives the static app back — a silent redirect loop."
**Rationale**: "Without both" is ambiguous between "if either is missing" and "if both are missing". "Silent" here means "no error message", which is a fourth sense of the word in this document (see finding 46).
**Rewrite**: "If either proxy entry is missing, the SPA redirects to `/auth/login` and receives `index.html` back, producing a redirect loop with no error surfaced to the user."

### 45. "This is asserted by an automated test, not left to review"
**Severity**: Nit
**Affected Scope**: Configuration & Deployment, Probe safety (lines 460-461).
**Rationale**: "Not left to review" is meta-commentary about the review process, not a requirement.
**Rewrite**: "Covered by an automated test (Testing Strategy: `GET /` returns `200` with authentication enabled and no cookie)."

### 46. "Silent/silently" carries four different meanings
**Severity**: Nit
**Affected Scope**: Lines 166 ("must not silently expose"), 303 ("silently assigning financial records"), 446 ("a silent redirect loop"), 555 ("silently breaks isolation"), 585 ("silent historical edits").
**Rationale**: The word alternates between "without an error message", "without an audit trail", "without a compile-time signal", and "without detection". Each is a distinct property and each deserves its own word.
**Rewrite**: 166 → "must not expose every portfolio without failing to start"; 303 → "assigning financial records to the wrong person without an audit trail"; 446 → "a redirect loop that surfaces no error"; 555 → "breaks isolation with no compile-time or runtime signal"; 585 → "undetected historical edits".

### 47. "teaches generated clients"; "when bearer tokens actually exist"
**Severity**: Nit
**Affected Scope**: OpenAPI, CI & Frontend Impact (lines 471-472).
**Rationale**: "Teaches" is anthropomorphic, and "an `Authorization` header the API ignores" is wrong — the API would reject such a call with `401`, which is a worse outcome than ignoring it. "Actually" is filler.
**Rewrite**: "A declared bearer scheme causes generated clients to send an `Authorization` header, which this API does not accept — every such call returns `401`. Spec 3 adds `bearerAuth` when bearer tokens exist."

### 48. "~14 raw `fetch` calls" is not verifiable
**Severity**: Nit
**Affected Scope**: OpenAPI, CI & Frontend Impact (lines 476-478): "The ~14 raw `fetch` calls in `App.tsx`".
**Rationale**: An approximate count cannot be used as an acceptance criterion; an implementer cannot tell when the migration is complete.
**Rewrite**: "Every `fetch` call in `App.tsx`, `CsvImportModal.tsx`, and `PortfolioDetail.tsx` is routed through it."

### 49. "No token is ever read, stored, or sent by JavaScript"
**Severity**: Nit
**Affected Scope**: OpenAPI, CI & Frontend Impact (line 480).
**Rationale**: "Ever" reads as a historical claim rather than an invariant; passive voice obscures the constraint's subject.
**Rewrite**: "Invariant: JavaScript never reads, stores, or sends a token."

### 50. "a guard that merely breaks everything cannot pass"
**Severity**: Nit
**Affected Scope**: Testing Strategy (lines 503-504): "Each case is paired with A performing the same operation successfully, so a guard that merely breaks everything cannot pass."
**Rationale**: Conversational phrasing for a precise property (the suite must reject an over-broad guard, not just a missing one). The sentence is also mid-paragraph and hard to parse at line width.
**Rewrite**: "Each negative case is paired with a positive case in which A performs the same operation successfully. This rejects an over-broad guard as well as a missing one."

### 51. "Gates" lists `npm run lint` and then says CI does not run it
**Severity**: Major
**Affected Scope**: Testing Strategy, Gates (lines 522-525): "**Gates**: `cargo test`, `npm test`, `npm run lint`, OpenAPI diff green. (`npm run lint` is a local gate only — `.github/workflows/ci.yml` runs `cargo test`, the OpenAPI diff, `npm run generate-types`, and `npm test`, but not lint.)"
**Rationale**: As written the gate list includes a check the parenthetical then excludes, and the parenthetical re-lists the CI commands. An implementer cannot tell whether lint failure blocks this spec.
**Rewrite**: "CI gates: `cargo test`, OpenAPI diff, `npm run generate-types`, `npm test`. Local gate, not enforced in CI: `npm run lint`."

### 52. "the guard against the table drifting" — "guard" used for two things
**Severity**: Nit
**Affected Scope**: Testing Strategy (lines 516-517): "every API route mounted in `build_rocket` appears in the §6 route table — the guard against the table drifting from the router".
**Rationale**: "Guard" is the document's term for the Rocket request guard (`src/auth/guard.rs`). Using it for a test creates a false connection.
**Rewrite**: "…— the check that keeps the §6 table in step with the router, which the OpenAPI diff cannot catch."

### 53. "Enumeration … is a requirement, not a style preference"
**Severity**: Nit
**Affected Scope**: Security Considerations (lines 552-553).
**Rationale**: The second sentence is rhetorical — it argues with an objection nobody raised, and Requirement 5 already fixes the response code.
**Rewrite**: Delete the second sentence; keep "Sequential integer ids plus `403` would give an attacker an existence oracle. Requirement 5 fixes the response as `404`."

### 54. Cookie bullet duplicates the configuration table verbatim
**Severity**: Nit
**Affected Scope**: Security Considerations (lines 564-566): "the secret is a 256-bit key (`openssl rand -base64 32`) from a k8s `Secret`, never a ConfigMap."
**Rationale**: The key length and the `openssl` command already appear in the `ROCKET_SECRET_KEY` row (line 419) and the Secret requirement at line 448. Three copies will drift.
**Rewrite**: "The cookie signing key comes from the k8s `Secret`, never a ConfigMap (see Configuration & Deployment)."

### 55. "mechanical" is a subjective quality claim
**Severity**: Nit
**Affected Scope**: Known Limitations (lines 591-592): "The `access` chokepoint is what makes a later Postgres move mechanical: query shapes change, authorization does not."
**Rationale**: "Mechanical" is an opinion about future effort. The checkable content is the scoping claim about where authorization lives.
**Rewrite**: "Because authorization is confined to `access`, a later Postgres migration changes query shapes only, not authorization logic."

### 56. Third restatement of the email-verification policy
**Severity**: Nit
**Affected Scope**: Known Limitations (lines 600-602): "Ownership of a legacy portfolio is decided by `sub`, or by a verified `email` … never by an unverified email. That guarantee is a policy of this application, not something Authentik enforces on its behalf."
**Rationale**: This is the third full statement of the §4 handover rule (lines 289-299, then Security Considerations lines 567-570). "On its behalf" is muddled — Authentik is not acting for the application.
**Rewrite**: Reduce to: "The verified-`email` condition in §4 is enforced by this application; Authentik does not know about it."

### 57. Open Questions contains a review changelog
**Severity**: Major
**Affected Scope**: Open Questions (lines 605-614): "None. All decisions in this document were reviewed and approved in design discussion on 2026-11-04, then revised after review: Rocket 0.5 profile semantics replace the non-existent `ROCKET_ENV` check …".
**Rationale**: The section's purpose is unresolved questions. What follows is a list of already-applied corrections, each of which duplicates its own section (§1 profile semantics, §4 handover, §6 verbs, §1 secrets feature, §4 schema-state rebuild). It is relative to a prior draft ("replaced", "no longer trusts", "corrected to the real verbs") and will be meaningless to the next reader.
**Rewrite**: Replace the section body with "None." Move the correction list to a new `## Revision History` section with a date and one line per change, phrased against the current document rather than against the draft.

### 58. Section heading "OpenAPI, CI & Frontend Impact"
**Severity**: Nit
**Affected Scope**: Heading at line 466.
**Rationale**: "Impact" is filler; the section is a list of required changes, not an impact assessment.
**Rewrite**: "OpenAPI, CI, and Frontend Changes".

### 59. "following the existing startup-migration style"
**Severity**: Nit
**Affected Scope**: §4 (line 232): "Added to `init_db` in `backend_rust/src/lib.rs`, following the existing startup-migration style:".
**Rationale**: "Style" is vague — the constraint is the mechanism, not the style.
**Rewrite**: "Added to `init_db` in `backend_rust/src/lib.rs`, implemented with the existing `CREATE TABLE IF NOT EXISTS` and `migrate_one_column` patterns:".

## Not assessed

Coverage of lines 138-614 is complete. Code blocks were inspected for factual consistency only and not flagged for style, per instructions: the Rust `AuthMode`/`AuthedUser` block (§1), the SQL block (§4), the `access.rs` signatures block (§5), and the Architecture diagram (lines 106-136, outside the range).

Two items I could not verify without reading source and flag as unverified rather than as findings: whether `/openapi.json` (bare) is actually mounted (finding 10), and the exact `fetch` call count in the frontend (finding 48). Both are reported as specification gaps, not as confirmed factual errors.