## Findings

**1. Severity: Blocker**
**Affected Scope:** `## Current State (evidence)` — row 3 ("The mounted surface is 14 API operations…"), ~lines 74; the claim is load-bearing for the Decisions table and the OpenAPI plan.
**Rationale:** The stated cause is factually wrong. `PATCH /api/portfolios/<pid>/assets/<aid>` **does** carry `#[utoipa::path]` — `backend_rust/src/api_routes/transactions.rs:88-95` annotates it with `path = "/api/portfolios/<portfolio_id>/assets/<asset_id>"`. The real reason it is absent from `docs/openapi/openapi.json` is that `backend_rust/src/openapi.rs` lists only 13 handlers in `#[openapi(paths(...))]` and omits `crate::api_routes::transactions::update_asset`. I confirmed the published spec has exactly 10 paths / 13 operations and no `.../assets/<asset_id>` path. An implementer following this document literally will "add the missing annotation", ship, and still publish 13 operations. Fix: change the evidence row to name `openapi.rs` `ApiDoc::paths`, and change the remedy to "register `update_asset` in `ApiDoc::paths`".
Corollary: the inference "the OpenAPI diff in CI cannot catch a route omitted from this table" reaches a correct conclusion by the wrong mechanism. CI (`ci.yml`) diffs the *served* spec against the committed file; both are generated from the same incomplete `ApiDoc`, so they agree. The uncaught drift is `routes![]` ↔ `ApiDoc::paths`, and the proposed route-table test does not cover it either. Add a test asserting every `routes![]` handler appears in `ApiDoc::paths`.

**2. Severity: Major**
**Affected Scope:** `## Requirements` req 7 (~line 32) vs req 1 (~line 26).
**Rationale:** Req 7 says "`/api-docs` requires authentication", but `build_rocket` also mounts `openapi_json` at the **root**: `.mount("/", routes![index, openapi_json])` (`lib.rs:252`). Satisfying reqs 1 and 7 verbatim leaves the complete OpenAPI document publicly reachable at `GET /openapi.json`, which is precisely the exposure the Overview calls out. Req 7 must name both `/api-docs/openapi.json` and `/openapi.json`.

**3. Severity: Major**
**Affected Scope:** `## Requirements` req 8 (~line 34); `## Current State` probe row (~line 82).
**Rationale:** The claim that "the `wait-for-backend` initContainer continues to succeed without credentials" asserts a property that is already false. In `k8s/app-deployment.yaml` the initContainer curls `http://127.0.0.1:8000/`, but initContainers run **before** the backend container starts — nothing is listening, the 30×2s loop exhausts, and it exits 1. The design's probe-safety argument (and its dedicated test) rests on a broken premise, and rollout step 1's "behaviour unchanged" is unproven. Counter-argument: the operator's live manifests may differ from the committed ones — if so, the spec must say which, because it cites this file as evidence. Separately, the design adds three new process-exiting startup conditions (missing `ROCKET_SECRET_KEY` in a release build, orphans without a bootstrap variable, `Disabled` on a disallowed profile). With `replicas: 1`, one PVC, and a non-functional initContainer, each converts a config typo into total downtime with no automated rollback.

**4. Severity: Major**
**Affected Scope:** `## Architecture` unit table + diagram, ~lines 118-133 ("`access::require` … the only path to a portfolio by id").
**Rationale:** The chokepoint is a convention, not an enforced invariant. The module exposes `resolve(...) -> Result<Option<Access>, sqlx::Error>` publicly; a handler can call `resolve`, receive `Ok(None)`, and proceed. The promised test only checks that routes appear in the §6 table — it cannot detect a handler that queries `portfolios`/`assets`/`transactions` directly. Alternative: keep `resolve` private to the module, export only `require`/`require_asset`/`require_transaction` (which return `Err(NotFound)`), and add a CI grep/lint for raw `WHERE id = ?` against those tables. Otherwise "the design's safety rests on X" is an assertion, not a mechanism.

**5. Severity: Major**
**Affected Scope:** `## Decisions Summary` — "`Disabled` auth mode" row (~line 98) and the profile table in §1.
**Rationale:** The allowlist is deny-by-default only for the *unset* case. `dev` and `test` are permitted profile **strings**, so setting `ROCKET_PROFILE=dev` in `configmap.yaml` re-enables `Disabled` on the release image — one ConfigMap edit, no rebuild, only a startup banner. Verified: `k8s/configmap.yaml` carries only `DATABASE_URL` and the deployment only `RUST_LOG`, so the ConfigMap is the normal place operators add variables. Recommendation: gate `Disabled` on build mode as well (`cfg!(debug_assertions)`) so a release binary refuses `Disabled` regardless of profile string.

**6. Severity: Major**
**Affected Scope:** `## Requirements` req 4 (~line 29) and `## Decisions Summary` "Legacy-portfolio handover" row (~line 99).
**Rationale:** Req 4 treats `BOOTSTRAP_OWNER_EMAIL` as an equal alternative to `BOOTSTRAP_OWNER_SUBJ`. The `email_verified` gate is only as strong as Authentik's configuration, and Authentik does not verify addresses unless explicitly configured to. The document asserts `email_verified` is "provably the same person" without establishing that the IdP sets it honestly — an unbacked assertion about an external system. Alternative: make `sub` the only supported production form and mark the email form dev/bootstrap-only, or require an explicit opt-in flag plus an operator check that Authentik email verification is on.

**7. Severity: Major**
**Affected Scope:** `## Requirements` req 12 (~line 44); `## Overview` "open signup" framing (~line 20).
**Rationale:** Per-user rate limiting bounds one account, not the aggregate. With open signup, N users × 1 req/s is an unbounded ceiling on a shared third-party quota (Yahoo/EODHD), and the design explicitly accepts per-pod enforcement. The document contains no cost/quota economics at all: no assumed user count in scope, no upstream calls/day budget, no storage footprint. Add a global bucket (or a stated upstream quota and the user count at which the per-user limit alone saturates it) to satisfy the standard's Risks & Limits bar.

**8. Severity: Nit**
**Affected Scope:** `## Requirements` reqs 5 and 11 (~lines 30, 43).
**Rationale:** Req 5 defines isolation as "do not own" while req 11 mandates the four-level owner/editor/viewer model. "Own" is ambiguous across the two. Consequence: `Editor`/`Viewer` are dead code end-to-end in this spec (members table is empty per req 6), so the primary acceptance gate exercises only Owner-vs-none. State that explicitly and require Spec 2 to reuse the same matrix harness.

**9. Severity: Nit**
**Affected Scope:** `## Current State` CORS row (~line 72); Overview same-origin claim (~line 21).
**Rationale:** `get_cors_options` already sets `allow_credentials: true` with exact origins (`lib.rs:53-54`). The doc defers CSRF risk to a hypothetical future "if credentialed cross-origin access is ever enabled", but credentials are already on; the real guard is the exact-origin list, and adding an origin later silently becomes a cookie-reading path. Worth one sentence.

**10. Severity: Nit**
**Affected Scope:** `## Decisions Summary` as a whole (~lines 87-102) vs the Alternatives Rule.
**Rationale:** The table satisfies the letter of the standard (11 rejected alternatives). Two viable options are unlisted: (a) no local `users` table at all, deriving authorization from ID-token/session claims — rejected only implicitly; (b) `oauth2-proxy` + OIDC claim injection, dismissed under "where the flow terminates" without considering the hybrid (proxy for authn, claim-based authz) that the isolation requirement rules out for a different reason than stated.

**Standard compliance:** Goals are implicit in Overview (no `## Goals` heading); Risks/Alternatives live outside the assessed range. Evidence rows I checked (`lib.rs:53`, 10 paths/13 ops, 14 mounted operations, no `owner_id`, `AUTOINCREMENT`, `migrate_one_column` exists, `build_rocket(pool, currency_service, cors)`, CI diff, nginx/Vite same-origin, `routes![]` at `lib.rs:252-271`, `rust:1.92-slim`, `rocket = features = ["json"]`, no `rand`, `reqwest` rustls-only, no `/auth` in nginx or Vite, `npm run lint` absent from CI) are all accurate — finding 1 is the sole factual failure.

## Not assessed
- `~14 raw fetch calls` in `frontend/src/App.tsx`, and existence of `CsvImportModal.tsx` / `PortfolioDetail.tsx` / MSW setup.
- Commit `2753049` (429 handling) and `frontend/src/types/api.d.ts`.
- `openidconnect` 4.1 API surface beyond the version/MSRV claim (notably whether async token-exchange features are enabled by default features), and `cargo tree` outcome for `native-tls` unification.
- Everything after line 137, except where cited as a consequence of in-scope claims.