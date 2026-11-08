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