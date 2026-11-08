# Consolidated Review Report

**Document:** `docs/superpowers/specs/2026-11-04-oidc-identity-isolation-design.md` (614 lines)
**Worktree:** `/home/coder/wt-portfolio-prism-oidc`, branch `feat/oidc-identity-isolation`, HEAD `4df46b3 docs: revise OIDC spec after review`
**Ranges reviewed:** FeatureRange 1–137, DetailedRange 138–614, full file for the two Zoom Out passes
**Review mode:** Consolidated (all five stages, no interactive checkpoints)
**Date:** 2026-11-05

## Reviewer shortcodes used in the traceability matrix

| Code | Stage | Persona | Report file |
|---|---|---|---|
| `Z5-n` | 5 Initial Zoom Out | Skeptical Architect | `zoom_out_report.md` |
| `C-n` | 6 Comprehension | Naive Reader | `comprehension_report.md` |
| `D-n` | 7 Critic | Hostile Design Critic | `stage7_design.md` |
| `S-n` | 7 Critic | Structural Validator | `stage7_structural.md` |
| `ST7-n` | 7 Critic | Copy Editor (FeatureRange) | `stage7_style.md` |
| `Z8-n` | 8 Intermediate Zoom Out | Skeptical Architect | `intermediate_zoom_out_report.md` |
| `R-n` | 9 Readiness | Pragmatic Tech Lead | `stage9_readiness.md` |
| `CB-n` | 9 Readiness | Codebase Accuracy Auditor | `stage9_codebase.md` |
| `ST9-n` | 9 Readiness | Copy Editor (DetailedRange) | `stage9_style.md` |

Raw finding count across the nine reviewer reports: **154**. After deduplication: **7 Blockers, 24 Majors, 21 Nits**, plus 5 open decisions for the owner and 4 reviewer-vs-reviewer conflicts to settle.

---

## 1. Verdict

The document's core architecture is sound and was not challenged by any reviewer: backend-terminated OIDC authorization code flow with PKCE, an encrypted Rocket private cookie as the session, a JIT-provisioned local `users` row keyed on `(issuer, subj)`, `portfolios.owner_id`, one `access` module as the only path to a portfolio by caller-supplied id, `404` rather than `403` for anything outside a caller's access, and a public `GET /` so Kubernetes probes keep passing. Both Zoom Out passes produced only subtractions, which is the signal that the shape is right.

The document is not ready to implement as written. Seven items block it, and they fall into three groups:

1. **Two factual errors about the existing code**, each of which sends an implementer in a wrong direction: the `migrate_one_column` reuse claim, and the claim that `PATCH /api/portfolios/<pid>/assets/<aid>` carries no `#[utoipa::path]`.
2. **Two internal contradictions**: Requirement 5 forbids what Requirement 6 and the access table grant, and §5 says asset deletion cascades when the schema has no cascade and foreign keys are off.
3. **Three specification gaps that leave the implementer making design decisions**: how `AuthMode` and the bootstrap config reach `build_rocket`/`init_db` and where the process-exit decision lives, what values the synthetic `users` rows get for `NOT NULL` columns, and when the session cookie sets `Secure`.

Separately, one finding is worth more than any defect on this list: three of the twelve requirements (authenticate `/api-docs`, build the three-role access model now, per-user lookup rate limit) do not serve the stated root problem, and they are the origin of both architectural Blockers and four Majors. Cutting them shrinks the spec by roughly a third and removes the Blockers with it. That is a decision for you, not a defect to fix, so it is in section 4 rather than section 2.

---

## 2. Blockers

Each item is stated as: what the document says, what is actually true, and what to write instead.

### B1 — CI's OpenAPI regeneration breaks the moment this spec lands

**Where:** Requirement 7; "OpenAPI, CI & Frontend Impact" (~466–485); Testing Strategy "Gates" (~520). Evidence: `.github/workflows/ci.yml:22-33`.
**Raised by:** `C-1`, `Z5-1`, `D-1` (corollary), `Z8-2`. Four independent reviewers found this.

The spec makes `/api-docs/openapi.json` require authentication and makes `AUTH_MODE` default to `oidc`. The regeneration procedure it points at is an unauthenticated `curl http://127.0.0.1:8000/api-docs/openapi.json` against a locally started server, and the CI "Generate and Validate OpenAPI Spec" step does exactly that with a bare `cargo run` and no environment variables. After this lands, that curl receives the `401` JSON body, the diff against the committed spec fails, and every future API change is blocked. The spec's own gate "OpenAPI diff green" becomes unsatisfiable. The `AGENTS.md` variant of the procedure uses `cargo run --release`, which this spec says hard-errors without `ROCKET_SECRET_KEY`.

**Simplest fix (preferred, `Z8-2`):** `ApiDoc` is a plain `#[derive(OpenApi)]` struct (`backend_rust/src/openapi.rs:6`), so the document is a pure function of the code and needs no running server. Add an early exit in `backend_rust/src/main.rs`: if argv contains `--print-openapi`, print `ApiDoc::json()` and exit. CI becomes `cargo run -- --print-openapi > current_openapi.json`. This removes the server, the dummy `portfolio.db`, the `sleep`, the orphan-process kill, the auth-mode dependency, and the secret-key dependency in one change, and it makes Requirement 7 free: the runtime route can require a session while the generator never touches it.

**Minimum acceptable fix if you reject the generator change:** state in the spec that the CI job exports `AUTH_MODE=disabled ROCKET_PROFILE=dev`, that it also supplies a `ROCKET_SECRET_KEY` for any release-build variant, and that the regeneration procedure in `AGENTS.md` is updated to match. Say in the document that this is the second-best option, because it leaves the content of the published API contract dependent on runtime configuration.

### B2 — The `migrate_one_column` reuse claim is false

**Where:** §4 "Making `owner_id` NOT NULL" (~229–305); Decisions Summary "Unowned rows at startup" row.
**Raised by:** `Z5-2`, `CB-4`, `Z8-3`. Independently verified against source by two reviewers.

The document says the existing `migrate_one_column` helper already performs the rename → add → copy → drop rebuild, that the `owner_id` rebuild reuses it, and that therefore no new migration framework is introduced. It does not. `migrate_one_column` (`backend_rust/src/lib.rs:149`) is private, every branch issues `ALTER TABLE {table} ADD COLUMN {column} TEXT NOT NULL DEFAULT '0'`, and its crash-recovery state machine keys on the literal string `"TEXT"` (lines ~158–186). It cannot emit `owner_id INTEGER NOT NULL`. Reusing it means generalizing the repo's most fragile, most specifically-tested helper (`tests/decimal_migration_test.rs` pins its behaviour) to serve one new caller. That is a refactor of migration code, not reuse.

**Fix (preferred, `Z8-3`):** delete the `NOT NULL` rebuild. `ALTER TABLE portfolios ADD COLUMN owner_id INTEGER` is a plain SQLite add with no rename/copy/drop. The invariant is already enforced where it matters — the startup check refuses to boot with unowned portfolios — and `access::resolve` must treat `NULL` as no-access anyway, since the design itself notes foreign keys are never enabled. This deletes the helper generalization, the `PRAGMA table_info` trigger branch, the fresh-vs-upgraded schema-equality test, and the risk of rebuilding the table `assets` and `transactions` reference on every fresh install.
**Fix if you keep `NOT NULL`:** write a dedicated single-purpose rebuild function with its own crash test, do not touch `migrate_one_column`, and correct the Decisions Summary row so it no longer claims reuse.

### B3 — Requirement 5 forbids exactly what Requirement 6 grants

**Where:** Requirements 5 and 6 (38–42); Overview line 14; §5 access table.
**Raised by:** `ST7-1`.

Requirement 5: "A user cannot read or mutate a portfolio, asset, or transaction they do not own. The response for any non-owned resource id is `404`." Requirement 6 and the §5 access table grant `Editor` and `Viewer` read access to a portfolio the caller does not own. "Non-owned" is the operative term in both, so an implementer working from Requirement 5 alone builds a rule that denies editors and viewers, and the isolation matrix will not catch it because the members table is empty in this spec.

**Fix:** Requirement 5 → "A user cannot read or mutate a portfolio, asset, or transaction outside their granted access level (owner, editor, or viewer). The response for any resource id outside the caller's access is `404`, never `403`." Overview → "so that no user can read or mutate another user's data without an explicit grant."

### B4 — "Deleting an asset cascades to its transactions" describes a mechanism the schema does not have

**Where:** §5 (~346–347); contradicted by §4 (~275–279).
**Raised by:** `CB-2` (Blocker), `ST9-3` (Major).

`transactions` declares `FOREIGN KEY (asset_id) REFERENCES assets(id)` with no `ON DELETE CASCADE`, and the document itself states that `PRAGMA foreign_keys` is never enabled. `delete_asset` (`backend_rust/src/api_routes/transactions.rs:141-155`) deletes only the asset row, leaving transaction rows with a dangling `asset_id`. So the stated reason for making asset deletion owner-only is false, and an implementer who believes "cascades" will rely on the database to clean up child rows and leave orphans.

**Fix:** state the real behaviour — asset deletion currently orphans its transactions — say that owner-only deletion is a policy choice rather than a cascade safeguard, and decide what happens to the orphaned rows. That last point is a decision the spec currently leaves unmade.

### B5 — The synthetic `users` rows have no specified values for `NOT NULL` columns

**Where:** §4 `users` DDL (~255–268); Backfill and Placeholder reconciliation (~296–330); §1 `bootstrap:dev`.
**Raised by:** `R-2`.

`users.issuer` and `users.created_at` are `NOT NULL` with no default, and `email` is also `NOT NULL`. The spec specifies `subj = 'bootstrap:<subj>'` but never says what `issuer` a synthetic row gets (no identity provider was involved), what `email` a `BOOTSTRAP_OWNER_SUBJ`-only row gets, what `subj`/`issuer` the `Disabled`-mode `bootstrap:dev` row gets, or the `created_at` format and timezone. Each is an independent invention by the implementer, and both the `uq_users_issuer_subj` unique index and the reconciliation rule ("`subj` begins with `bootstrap:`") depend on those exact strings.

**Fix:** give a literal `INSERT` for every synthetic row: bootstrap-by-sub, bootstrap-by-email, and `bootstrap:dev`.

### B6 — How `AuthMode` reaches the running app is never specified, and the process-exit site is undefined

**Where:** §1 (138–182); §4 (229–330).
**Raised by:** `R-1` (Blocker), `CB-6` (Nit), `Z5-2` (adjacent).

`build_rocket(pool, currency_service, cors)` (`backend_rust/src/lib.rs:252`) and `init_db(&pool)` (`lib.rs:57`, called from `main.rs:25`) have no place for `AuthMode` or the bootstrap config, and `tests/common/mod.rs:7` wraps the existing signature. The spec does not say whether `AuthMode` becomes a `build_rocket` parameter, `.manage()`d state read via `&State<AuthMode>` in the guard, or an env read inside the guard. It says the profile-allowlist check happens "in `build_rocket` before any route is served" with a process-exiting error, but `build_rocket` returns `Rocket<Build>` and has no error path — so the spec must say whether it panics, returns `Result`, or whether the check moves to `main.rs`; a signature change also breaks the shared test helper. Same problem for the backfill's "exit non-zero", which sits inside `init_db`, which returns `Result<(), sqlx::Error>` and is called by every test's `setup_db()`.

**Fix:** name the new signatures (for example `build_rocket(pool, currency_service, cors, auth: AuthMode)` and `init_db(&pool, cfg: &StartupConfig) -> anyhow::Result<()>`) and state where the process-exit decision is made. Until this is written, "startup fails" is not testable.

### B7 — The `Secure` cookie rule is stated two ways and neither resolves the development case

**Where:** §2 session cookie (~207–209); Security Considerations "Cookie" (~565, 573–574).
**Raised by:** `ST9-1` (Blocker), `C-8` (Nit).

§2 lists `Secure` as an unconditional attribute. Security Considerations says the app sets `Secure` cookies "unconditionally in production". "Production" is undefined — the document uses three other terms for the same axis (`release` build, allowlisted profile, `AUTH_MODE=oidc`). A `Secure` cookie over plain `http://localhost` is dropped by the browser, so login silently fails in development; omitting the flag in production is a security defect. An implementer cannot tell which the spec wants.

**Fix:** state one rule keyed on an observable property, for example: "The session cookie sets `Secure` when the request arrived over https. There is no separate production concept in the code; the decision keys on the request scheme, not on the profile." Then state what `./start.sh` produces.

---

## 3. Majors

Grouped by theme. Several groups are single defects reported by several reviewers; a few are clusters of related wording problems.

### Structural and design

**MA1 — The three-level access lattice is dead code in this spec.** With `portfolio_members` guaranteed empty, every authorized caller is `Owner`. `Editor`, `Viewer`, `meets()`, the members fallback query, the role `CHECK`, `idx_portfolio_members_user`, and the new `access` field on the public `PortfolioOut` shape are all unreachable until Spec 2. Requirement 11 instructs the project to build for a speculative future requirement, which is the textbook over-build. The durable commitment is the chokepoint, and that survives unchanged with a single `require_owner(user, portfolio_id)`. → `Z5-3`, `D-8`, `Z8-7`. **Owner decision — see D-1 in section 4.**

**MA2 — Revocation machinery with no trigger.** Nothing in this spec ever sets `is_active = 0` (identity-provider sync is an explicit non-goal) and there is no user-management UI, so `POST /auth/logout-all` has no caller. That is a column, a guard read, and an endpoint that cannot fire. Dropping them makes the guard a pure cookie decrypt with zero database reads per request, which is stronger than the design's own stated reason for rejecting a session table. → `Z5-4`. **Owner decision — D-2.**

**MA3 — The per-user rate limit is scope creep and is not implementable as written.** A token bucket with capacity and refill constants, `Mutex<HashMap<i32, Bucket>>` and "periodic eviction of idle entries" is a new stateful subsystem with a background task, inside an identity spec, for a deployment the document sizes at "tens of users". Separately it is underspecified: no crate or hand-rolled choice, no eviction interval or "idle" threshold, no `Retry-After` value or units, no decision on how the map is shared (which determines whether tests can reset it), and the "allows 10, blocks the 11th, refills" test is not writable without an injectable clock. And per-user limiting bounds one account, not the aggregate: N users × 1 req/s is an unbounded ceiling on a shared third-party quota, and the document contains no quota economics at all. → `Z5-7`, `R-7`, `ST9-16`, `D-7`. **Owner decision — D-3.**

**MA4 — The chokepoint is a convention, not an enforced invariant, and §6 contradicts it.** The design's safety rests on "`access` is the only path to a portfolio by id", yet: `resolve()` is public and returns `Ok(None)`, which a handler can ignore; the testable form of the rule is a literal string pattern (`WHERE id = ?`) that `WHERE portfolios.id = ?`, a join, or a subquery all satisfy while violating the rule; and §6 additionally requires handlers on nested routes to hand-write `asset.portfolio_id == id`, which is a per-site ownership predicate living in two places with two mechanisms. The fix is one change: keep `resolve` private, export only `require`/`require_asset`/`require_transaction` returning `Err(NotFound)`, move the path-versus-asset consistency check into `access` (resolve from the asset, reject if the resolved portfolio does not match the path id), and add a CI grep for the three table names in handler files. → `D-4`, `Z8-6`, `ST9-6`.

**MA5 — The route-drift test is a third source of truth with no stated mechanism, and it guards the wrong drift.** The spec promises "a test that fails when a mounted API route is absent from the table", but a test cannot read a markdown table and the mechanism is never named. Two reviewers independently concluded the test is redundant or misaimed: the isolation matrix already iterates every route mounted in `build_rocket` and pairs every denial with a success, so an omitted guard fails the matrix automatically; and the drift CI genuinely cannot catch is `routes![]` versus `ApiDoc::paths`, which the proposed test does not cover either. Consolidated fix: delete the markdown-table test, add a test asserting every `routes![]` handler appears in `ApiDoc::paths`, and let the matrix be the authorization-drift guard. → `C-3`, `Z5-8`, `D-1`, `R-4`.

**MA6 — `AuthMode::Disabled` is gated on a runtime string when it can be gated at compile time.** The design's central safety claim is "there is no other way to obtain an `AuthedUser`", and §1 immediately defines a second way, selected by an environment variable plus a config-profile string. The mitigation the document spends the most prose on is a runtime string comparison guarding a build-time fact. Gating on `cfg!(debug_assertions)` means the variant is not compiled into the release binary at all, so no forgotten `ROCKET_*`, no ConfigMap drift and no `AUTH_MODE` typo can enable it. This is strictly stronger than the allowlist and deletes the allowlist table, the `ROCKET_PROFILE` configuration row, the `ROCKET_PROFILE=dev` export in `start.sh`, the "unset in a release build" reasoning, and three test bullets. Related and independently found: the current allowlist is deny-by-default only for the *unset* case — `dev` and `test` are permitted profile strings, so adding `ROCKET_PROFILE=dev` to `configmap.yaml` re-enables `Disabled` on the release image with one ConfigMap edit and no rebuild. Listing `dev` and `test` also widens the allowlist for no benefit, since Rocket's debug-build default profile is already `debug`. → `Z8-4`, `D-5`, `Z5-10`.

**MA7 — Requirement 1 is enforced per-handler; a mount-level fairing makes it true by construction.** Enforcement is opt-in per handler ("any handler that takes `user: AuthedUser` is protected"), so a route that simply does not take `AuthedUser` is public, and the claim "the type system makes protection impossible to forget" is stronger than the mechanism. One Rocket `Fairing` on the `/api` and `/api-docs` prefixes returning `401` without a valid session makes Requirement 1 literally true, and removes the main reason the route-inventory test in MA5 exists. → `Z8-5`, `C-9`, `ST7-26`.

**MA8 — §1 and §4 collide in local development.** §4 refuses to start when any `portfolios.owner_id IS NULL` and no bootstrap variable is set. §1 says that in `Disabled` mode with no bootstrap variable the app instead creates a `subj = 'bootstrap:dev'` row so `./start.sh` needs no configuration. A developer with an existing local `portfolio.db` containing portfolios therefore hits refuse-to-start, contradicting Requirement 10. The spec never says whether the refuse-to-start rule is waived in `Disabled` mode, whether `bootstrap:dev` is implicitly the backfill owner, or in what order `AuthMode` resolves relative to `init_db` seeding. Related: `owner_id` becomes `NOT NULL` but `tests/common/mod.rs:22` `seed_portfolio` inserts without it and the matrix needs two users' portfolios, and the required helper signature changes are not listed. → `C-2`, `R-9`.

**MA9 — OIDC discovery timing is unspecified, and it determines whether rollout step 3 is safe.** The document never says whether issuer and JWKS discovery happens eagerly at process start or lazily on the first `/auth/login`. If eager, a mistyped `OIDC_ISSUER` or an unreachable Authentik CrashLoops the pod, and the probe-safety paragraph — which only guarantees `GET /` stays public — does not protect against it. It also bears on Requirement 9 (tests pass without a reachable provider). Same root cause, two more places: "browser flow" versus "direct (non-browser) call" in §8 is not defined by any request property, so the `302`-versus-`401` branch cannot be implemented consistently, and mapping "Authentik unreachable" to a redirect to `/auth/login` risks a loop when `/auth/login` itself needs discovery metadata. → `C-4`, `ST9-11`, `Z5-1`.

**MA10 — The session and OAuth-state cookies are under-specified.** No session cookie name is given (the OpenAPI `sessionCookie` string is documentation, not necessarily the wire name); the payload encoding is unspecified, and the tamper test depends on it; the transient state cookie holds three values with no stated serialization, no name, and omits `Secure` while the session cookie sets it, with identical `HttpOnly`/`SameSite` attributes and different lifetimes, so an implementer can reuse one name and clobber the session; `/auth/logout` names `post_logout_redirect_uri` without giving its value or saying it must be registered in Authentik, so logout can fail at cutover; and §8 emits `/auth/login?error=auth_failed` while §2 never says `/auth/login` accepts or echoes an `error` parameter. → `R-5`, `ST9-14`, `ST9-15`.

**MA11 — `AuthError` is used in signatures but never defined.** No variant list, no `IntoResponse` impl, no mapping to the §8 status codes. `require_asset` and `require_transaction` are named but their resolution queries are not given (asset → `portfolio_id`; transaction → `asset_id` → `portfolio_id`), and the spec does not say what a transaction whose asset row is missing returns. Related: introducing an `AuthError` enum at all breaks the pattern every handler in the repo uses (`Result<T, Status>`), when `Status::NotFound` and `Status::Unauthorized` already express both outcomes. → `R-6`, `Z5-9`.

**MA12 — The `GET /api/portfolios/` response change is unspecified.** "Each row carries the caller's `Access`" changes a public response shape, but no field name, JSON representation, or `Serialize`/`ToSchema` decision is given, the OpenAPI section lists only `401`/`404` additions so the generated TypeScript types would not gain the field, and the frontend section never mentions it. Spec 2's share UI will consume this field, so pinning the representation now is cheap. → `R-8`, `ST9-8`, `C-10`.

**MA13 — `/openapi.json` appears without provenance, and Requirement 7 names a prefix that is not a route.** The Current State table documents only `/api-docs/openapi.json`; a bare `/openapi.json` appears in the §6 route table with no citation. Both are the same handler mounted twice (`lib.rs:245,270`), and nginx's `location /` catch-all means the bare path is only reachable by talking to port 8000 directly, so it is not part of the proxied surface in the architecture diagram. Requirement 7 says "the `/api-docs` prefix requires authentication", which reads as already satisfied by a path that does not exist. → `D-2`, `ST7-9`, `ST9-10`, `R-11`, `S-10`.

**MA14 — The nested-route hole the spec claims to close is already closed today.** §6 says the `asset.portfolio_id == id` check "closes a hole". `update_asset` already runs `UPDATE assets SET currency = ? WHERE id = ? AND portfolio_id = ?` with a portfolio-scoped `SELECT` returning 404 (`transactions.rs:103-127`), and `create_transaction` already gates on `SELECT * FROM assets WHERE id = ? AND portfolio_id = ?` (`transactions.rs:169-177`). The check should be described as already present and to be preserved. Separately, §6 and the Testing Strategy describe the same attack with reversed ownership and a sentence with no grammatical subject, so an implementer can write a test that passes without exercising the case. → `CB-3`, `ST9-7`.

**MA15 — The initContainer premise is already false, and three new process-exit conditions turn config typos into downtime.** The spec asserts the `wait-for-backend` initContainer continues to succeed without credentials. In `k8s/app-deployment.yaml` the initContainer curls `http://127.0.0.1:8000/`, but initContainers run before the backend container starts, so nothing is listening, the retry loop exhausts and it exits 1. The probe-safety argument and its dedicated test rest on a broken premise, and rollout step 1's "behaviour unchanged" is unproven. Compounding it: the design adds three process-exiting startup conditions (missing `ROCKET_SECRET_KEY` in a release build, orphans without a bootstrap variable, `Disabled` on a disallowed profile) with `replicas: 1`, one PVC, and a non-functional initContainer, so each converts a typo into total downtime with no automated rollback. → `D-3`.

**MA16 — The bootstrap handover is over-built, and its email half rests on an unbacked assumption about Authentik.** Two environment variables, a `bootstrap:` subject namespace, an `email_verified` gate, reconciliation rules and dedicated tests exist to move one tenant's rows once, during a window the rollout declares single-identity. The `email_verified` gate is only as strong as Authentik's configuration, and Authentik does not verify addresses unless explicitly configured to; the document asserts "provably the same person" without establishing that the identity provider sets the claim honestly. Keeping only `BOOTSTRAP_OWNER_SUBJ` — operator signs in once, reads their `sub` from `/auth/me`, sets it, restarts — deletes the email variable, the verified-email gate, the `bootstrap:` prefix and the reconciliation logic while staying fail-safe. → `Z5-6`, `D-6`.

**MA17 — The mandated Alternatives and Risks sections are not discoverable, and decisions precede vocabulary.** No `## Alternatives Considered` heading exists anywhere, although the substance is present as a rejected-alternative column in the Decisions Summary table — a discoverability failure, not a content gap. No section in the FeatureRange is named for risks; the risk register lives under two non-standard headings far outside it, so a reviewer briefed on the FeatureRange never learns it exists. The Decisions table also forward-references `users.token_version`, `portfolios.owner_id`, `portfolio_members`, the `access` module and `BOOTSTRAP_OWNER_SUBJ` before Architecture defines them. → `S-1`, `S-2`, `S-3`.

**MA18 — Three of twelve Non-Goals have no owner.** Three bullets carry a destination pointer ("— Spec 2/3/4"); three do not (roles and admin panels and quotas, `is_active` sync, replacing SQLite). The reader cannot tell "assigned to a later spec" from "permanently unowned", and the Deferred Work table lists only Specs 2–4, leaving the latter three with no home. The SQLite item is also duplicated verbatim in Known Limitations. → `S-7`.

**MA19 — The architecture diagram has verified defects.** Line 118 carries 56 characters between its box borders where every other row carries 55, so its right border is one column out of alignment. The annotation "the only path to a portfolio by id" is split across three lines interleaved with two `▼` flow arrows belonging to the main path, so the sentence is broken by flow glyphs and the columns collide. The diagram shows `/auth/*` proxied by nginx, which Current State says does not exist today, and is not labelled as target state. The fence has no info string while the Rust and SQL fences do. → `S-4`, `S-5`, `S-13`, `ST7-17`.

**MA20 — Terminology drift on load-bearing terms.** `subject` / `subj` / `sub` for one concept across requirement text, column name and ID-token claim. Self-registration / open signup / open registration for one Authentik setting that the security argument depends on being one specific configuration. `Secure`-cookie "production" versus `release` build versus allowlisted profile versus `AUTH_MODE=oidc` for one axis. "Silent" and "silently" carry four distinct meanings across five lines (no error message, no audit trail, no compile-time signal, undetected). "Roles" means both the out-of-scope admin roles and `portfolio_members.role`, which is in scope and enforced here. → `ST7-8`, `ST9-18`, `ST9-46`, `ST7-11`.

**MA21 — Two sections contain review history rather than specification.** "Open Questions" contains no questions but a list of already-applied corrections, each duplicating its own section, phrased against a draft the reader has not seen ("replaced", "no longer trusts", "corrected to the real verbs"). "the draft wrote `configMap.yaml`" references an artifact outside the document. "the real asset-update route" makes the same correction three times in one sentence and a fourth time in Open Questions. → `ST9-57`, `ST9-17`, `ST9-38`, `C-7`.

**MA22 — The Gates list includes a check the same sentence then excludes.** "Gates: `cargo test`, `npm test`, `npm run lint`, OpenAPI diff green" followed by a parenthetical saying CI does not run lint. An implementer cannot tell whether a lint failure blocks this spec. → `ST9-51`, `Z5-1`.

**MA23 — Unmeasurable claims stand in for reasons.** "Appropriate for tens of users" with no measurement, workload or bottleneck named. "This is the highest-risk line in the design" with no risk ranking anywhere to check it against. "Makes the cross-user test matrix the primary acceptance gate" implying a secondary gate that is never named, while the matrix proves authorization, not authentication. The startup banner requirement is stated twice in two unmeasurable forms ("prominent", "logged loudly") and the Testing Strategy tests the refusal case but not the banner. "The authentication half is comparatively small … the isolation half is the substance" asserts a 50/50 split the next sentence denies. → `ST9-22`, `ST9-23`, `ST9-24`, `ST7-3`, `ST7-4`, `Z8-9`.

**MA24 — `PATCH /api/portfolios/:id` is described as a rename.** Four reviewers found that `update_portfolio` (`api_routes/portfolios.rs:172-208`) writes only `currency`/`base_currency`; `PortfolioUpdate` carries only `currency`; there is no rename endpoint. The spec calls it "renaming/deleting is owner-only" three times, which both misdescribes the route and weakens the stated Owner-versus-Editor rule for a level that has no non-owner holders in this spec. → `CB-5`, `R-10`, `Z5-13`, `ST9-39`.

---

## 4. Decisions for the owner

These are judgment calls, not defects. Each has a defensible answer on both sides, and the spec cannot be finalized until each is settled.

**D-1 — Keep or cut the three-level access model.** Keep it and Spec 1 ships unreachable code plus a public response-shape change for a consumer that does not exist yet. Cut it and Spec 2 introduces the lattice in the same change as its first write path, which is where the design effort belongs. The chokepoint commitment survives either way. *If you keep it, MA12 stops being optional: the wire representation must be pinned.*

**D-2 — Keep or defer revocation (`token_version`, `is_active`, `/auth/logout-all`).** The counter-argument to cutting is that adding a revocation column later means another migration and another guard change. The counter-argument to keeping is that nothing can set it, and the guard currently pays a database read per request for a check that cannot fail.

**D-3 — Keep, shrink, or cut the per-user lookup rate limit.** It is the only requirement in the spec that protects against a threat that did not exist before the spec (the endpoint is public today). If it stays, MA3's missing constants and the missing injectable clock must be written. If it goes, Requirement 12 and §7 and their tests leave with it.

**D-4 — Authenticate `/api-docs` at all?** Requirement 7 is what creates Blocker B1. If disclosure of the API surface is a real concern, keep it and take B1's preferred fix (generate the document without a server). If it is not, cutting Requirement 7 removes the Blocker, the `/openapi.json` alias question (MA13) and the cookie `securityScheme` work in one move.

**D-5 — Compile-time or runtime gate for `AuthMode::Disabled`.** MA6's `cfg!(debug_assertions)` proposal is simpler and strictly stronger, but it makes `cargo test --release` unsupported and it removes the ability to run a release binary locally without an identity provider. That trade-off is yours, not the implementer's.

---

## 5. Conflicts between reviewers

Four places where reviewers disagree. Each needs a ruling before the findings are actioned.

**X1 — Is the route-inventory test redundant, or does its mechanism need specifying?** `C-3` says the mechanism is load-bearing and must be written down. `Z5-8` says the test is redundant because the isolation matrix already iterates the router. `D-1` says the drift CI actually misses is `routes![]` versus `ApiDoc::paths`, which neither proposal covers. **Consolidated recommendation:** delete the markdown-table test, add the `routes![]` → `ApiDoc::paths` test, rely on the matrix for authorization drift. This satisfies all three by changing what is tested.

**X2 — Is the `Secure`-cookie ambiguity a Blocker or a Nit?** `ST9-1` rated it Blocker, `C-8` rated it Nit. It is a specification ambiguity with a one-sentence fix, but either wrong resolution ships a real defect (broken dev login, or cookies without `Secure` in production). **Consolidated recommendation:** Blocker, as B7.

**X3 — Does the nested-route hole exist?** §6 and `ST9-7` treat it as a hole being closed; `CB-3` verified the guard is already present in SQL today. **Consolidated recommendation:** `CB-3` is correct on the code. Reword §6 to "already present, preserved by `require_asset`", and keep the cross-portfolio matrix case as a regression guard.

**X4 — Is the access lattice justified as "write against the final model once"?** The spec argues yes in Requirement 11; `Z5-3`, `D-8` and `Z8-7` argue no. This is D-1 above, and it is a conflict of judgment rather than of fact.

---

## 6. Nits

Grouped; each group is one editing pass.

**N1 — Unverified counts.** "Ten hand-written SQL statements" (Overview line 16, Decisions line 96) is not evidenced in the Current State table and collides with the 13/14-operation counts two paragraphs later; the actual count of id-based selects is 12 (`analytics.rs:26,112`; `portfolios.rs:158,196,204,236,257`; `transactions.rs:103,112,146,174,283`). "~14 raw `fetch` calls" is approximate and cannot be an acceptance criterion. → `S-6`, `ST7-2`, `ST9-48`.

**N2 — Undefined acronyms at first use.** `PKCE`, `JIT`, and `MCP` are never expanded; `MCP` is scope-defining because two of the three deferred specs are about it. `rmcp` is named as a bare crate, the "FIFO/LIFO engine" is invoked as a reason to scope API keys without being defined (it is `TaxLotEngine` in `backend_rust/src/engines/tax_engine.rs`), and EODHD is named without a link. → `ST7-18`, `C-5`.

**N3 — Requirement list has no structure or traceability.** A flat 1–12 list mixes functional (1–7), operational (8–10) and forward-compatibility (11–12) concerns, has no stable IDs despite later references to "requirement 4" and "requirement 10", and has no requirement-to-test table. Requirement 8 is a constraint on Requirement 1, not a functional requirement. → `S-12`, `ST7-18`.

**N4 — Heading and front-matter consistency.** `## Current State (evidence)` embeds a parenthetical unlike every other heading; conjunction style is inconsistent (`&` versus `and`); front matter uses loose `**Label**: value` lines while every other structure in the document is a table; there is no table of contents despite 20 H2 sections and four intra-document anchors. → `S-8`, `S-9`, `S-58`.

**N5 — Overstated or misattributed mechanism claims.** "The existing exact-origin CORS policy remains as a second layer" overstates what CORS contributes against CSRF — CORS governs whether a cross-origin script can read a response, not whether a cross-site form POST carries cookies. `get_cors_options` already sets `allow_credentials: true`, so the guard is the exact-origin list, and adding an origin later silently becomes a cookie-reading path. "Same-origin is load-bearing … it means the browser never handles a token" compresses a two-step argument into a false one: same-origin permits a cookie design; the cookie choice is what keeps tokens out of the browser. → `C-12`, `D-9`, `ST7-13`.

**N6 — `portfolio_members` "created but unused" is contradicted by §5 and §6.** The table is read in two places; what is true is that nothing writes to it. → `ST9-34`.

**N7 — Wit and bare assertions where reasons belong.** "The thing not to hand-roll", "needless new stateful service", "for no gain", "free enumeration oracle", "a shared failure", "a leak waiting to happen", "the forgotten variable the check exists to catch is the check's own default" (a garden-path sentence). Each has a plain-English replacement in `ST7-14`, `ST7-15`.

**N8 — Draft-relative voice outside Open Questions.** "The draft wrote `configMap.yaml`", "the real asset-update route", "corrected to the real verbs", "no longer trusts". Covered by MA21 but listed again because it appears in five places. → `ST9-17`, `ST9-38`.

**N9 — Small factual or scoping slips.** "The release image resolves to profile `release` and is refused by default" — the image is not refused, `AuthMode::Disabled` is refused on that profile. "`/auth/me` is the only endpoint the SPA needs to know it is signed in" — the SPA also needs `/auth/login` via the 401 redirect. "`prod`" in the deny column duplicates "any other value". "Existing users" in rollout step 1 — there are no `users` rows pre-migration; the meaning is existing clients and data. "Single-tenant" is not a property of this deployment (it is single-identity) and "cannot be raced" is jargon for an attack the spec spells out correctly elsewhere. "The operator note keeps the window closed" — a document section cannot complete a handover. → `ST9-25`, `ST9-32`, `ST9-31`, `ST9-19`, `ST9-20`, `ST9-37`.

**N10 — Buzzword headings and wrong property names.** "Chokepoint discipline" names a human practice, not the mechanism, and "the design's safety rests on" describes a security failure as safety. "The revocation switch" is a metaphor for a monotonically incremented integer; "sign out everywhere action" implies a UI this spec does not deliver. "Mechanical" as a claim about future Postgres-migration effort. → `ST9-21`, `ST9-33`, `ST9-55`.

**N11 — Duplication that will drift.** The 13-versus-14 operation count appears three times. The cookie key length and the `openssl rand -base64 32` command appear three times. The email-verification policy is stated in full three times. "There is no user-management UI" duplicates Non-Goals. → `ST9-40`, `ST9-54`, `ST9-56`, `ST7-18`.

**N12 — Wording that can be misparsed.** "Honoured only on an allowlisted profile … regardless of `AUTH_MODE`" reads as the inverse of the rule, and the intended AND-ing of the two checks is never stated. "The `Secret` must exist before the first rollout step, not before the OIDC step" is a double negative. "Without both" is ambiguous between "if either is missing" and "if both are missing". "The authentication half … the isolation half". "In-process is sufficient" with no threshold. "A guard that merely breaks everything cannot pass". → `ST9-12`, `ST9-13`, `ST9-44`, `ST9-41`, `ST9-50`, `ST7-7`.

**N13 — Test-plan gaps that are wording-level.** The isolation matrix runs entirely under `AuthMode::Disabled { as_user }`, so it proves authorization and not authentication; say so rather than calling it the primary gate. The proxy `/auth` entries are the only silent-failure path in the spec and are deliberately unautomated, when a one-line assertion that both config files contain an `/auth` entry is cheaper than the manual curl. `start.sh` gets no `ROCKET_SECRET_KEY`, relying on a debug build auto-generating one, which means dev sessions break silently across restarts. → `Z8-9`, `Z8-10`, `R-12`.

**N14 — Dependency claims stated as fact without verification.** `openidconnect = "4.1"`, its MSRV 1.88, and the "reuses the existing rustls-based reqwest" claim cannot be confirmed from the repo. The stated reason to avoid `rand` — risking two majors in the tree — is already moot: `Cargo.lock` carries `rand` 0.8.6 and 0.9.4 today. Confirm with `cargo tree -d` before pinning. → `Z5-11`.

**N15 — Spec-3 forward shaping after the correct `bearerAuth` rejection.** The document rightly refuses to declare a bearer scheme, but still says the guard "is shaped to accept a bearer token here", and does not say whether a bearer header is accepted, rejected or ignored in this spec. That line is where scope creep into Spec 3 starts. → `Z8-8`, `ST7-10`.

**N16 — Assorted single-line copy edits.** "Shadow record" (undefined shorthand, and "nothing else" is false), "existing, already operated" (synonym redundancy), "hard isolation" (intensifier with no meaning), "comparatively small", "created but unused", "safely" and doubled "already" in the migration rationale, "Refusing to start is intentional … worse than downtime" (value judgement already recorded in Decisions Summary), "teaches generated clients" (and the claim that the API would ignore an `Authorization` header — it would return 401), "No token is ever read" (passive; state it as an invariant), "following the existing startup-migration style" (the constraint is the mechanism, not the style), "the guard resolves to `as_user`" (it resolves the caller identity), "On every guarded request the guard" (repetition), "lists … which is" (number disagreement), "Enumeration is a requirement, not a style preference" (argues an objection nobody raised), "only a fixed string" (name the string), "genuine 500s" (non-sequitur), "the moment this lands", "not left to review" (meta-commentary), "Impact" in a heading for a list of required changes, British/American spelling mix (`behaviour`, `defence`). → `ST7-5`, `ST7-6`, `ST9-35`, `ST9-36`, `ST9-47`, `ST9-49`, `ST9-59`, `ST9-29`, `ST9-40`, `ST9-53`, `ST9-42`, `ST9-43`, `ST9-45`, `ST9-58`, `S-14`.

---

## 7. What holds up

Recorded so the next revision does not re-litigate settled ground. Both Zoom Out passes and the codebase auditor verified these against source.

- **Context sufficiency.** Objective, motivation and root problem (unauthenticated API plus enumerable ids) are stated up front and every proposal traces back to them.
- **Verified-correct evidence rows.** No auth anywhere (`lib.rs:53` is CORS only); no owner columns; `portfolios.id` is `INTEGER PRIMARY KEY AUTOINCREMENT`; probes and the initContainer hit `GET /` on 8000 and frontend probes hit `:80`; `configmap.yaml` is lowercase and carries only `DATABASE_URL` with no `ROCKET_*`; nginx `location /` swallows `/auth/login`; the Vite proxy has no `/auth`; `npm run lint` is absent from CI; committed spec is 10 paths / 13 operations with no `securitySchemes`; `rust:1.92-slim`; `reqwest` is rustls-only; no `rand` dependency; `utoipa-swagger-ui` is present and unused; `users` and `portfolio_members` DDL does not collide with existing tables; every §6 route path matches the router.
- **Reuse checks came back clean.** Rocket private cookies is the right platform standard — no bespoke JWT layer. `frontend/src/utils/apiFetch.ts` is genuinely new; the existing utils are `csvParser`, `csvValidator`, `decimal`, `formatters`, `isinValidator`, with no fetch wrapper to reuse. MSW is already wired. `seed_user` matches the `tests/common/mod.rs` seed pattern.
- **Decisions that survived hostile review unchanged.** `404` rather than `403` to avoid an existence oracle; refusing to start on unowned rows rather than misattributing financial data; delegating identity to the already-operated Authentik rather than hand-rolling passwords; terminating the flow in the application rather than behind a proxy; the same-origin cookie design that keeps tokens out of the browser.
- **Structural hygiene.** Heading hierarchy is strictly H1 → H2 with no skipped levels; all FeatureRange tables are well-formed with matching column counts; all code fences are closed; all four FeatureRange anchors resolve; all twelve Requirements are genuinely delivered somewhere in the DetailedRange — no orphaned promises.

---

## 8. Traceability matrix

| ID | Consolidated action item | Raw findings mapped |
|---|---|---|
| B1 | Decouple OpenAPI generation from the running server; fix CI and `AGENTS.md` | `C-1`, `Z5-1`, `D-1`, `Z8-2` |
| B2 | Drop the `owner_id NOT NULL` rebuild, or write a dedicated rebuild; correct the Decisions row | `Z5-2`, `CB-4`, `Z8-3` |
| B3 | Reword Requirement 5 and the Overview to "outside their granted access level" | `ST7-1` |
| B4 | State that asset deletion orphans transactions; decide the disposition | `CB-2`, `ST9-3` |
| B5 | Give a literal INSERT for every synthetic `users` row | `R-2` |
| B6 | Name the `build_rocket`/`init_db` signatures and the process-exit site | `R-1`, `CB-6` |
| B7 | State one `Secure`-cookie rule keyed on the request scheme | `ST9-1`, `C-8` |
| MA1 | Cut or justify the three-level access lattice | `Z5-3`, `D-8`, `Z8-7` |
| MA2 | Cut or justify revocation | `Z5-4` |
| MA3 | Cut or fully specify the rate limiter | `Z5-7`, `R-7`, `ST9-16`, `D-7` |
| MA4 | Make the chokepoint enforced: private `resolve`, nested check into `access`, grep gate | `D-4`, `Z8-6`, `ST9-6` |
| MA5 | Replace the route-table test with a `routes![]` → `ApiDoc::paths` test | `C-3`, `Z5-8`, `D-1`, `R-4` |
| MA6 | Gate `AuthMode::Disabled` on `cfg!(debug_assertions)` | `Z8-4`, `D-5`, `Z5-10` |
| MA7 | Add a mount-level fairing for `/api` and `/api-docs` | `Z8-5`, `C-9`, `ST7-26` |
| MA8 | Resolve the §1 seeding versus §4 refuse-to-start collision; list test-helper changes | `C-2`, `R-9` |
| MA9 | Specify discovery timing and the browser-versus-direct branch | `C-4`, `ST9-11`, `Z5-1` |
| MA10 | Specify cookie names, payload encoding, state-cookie attributes, logout redirect URI | `R-5`, `ST9-14`, `ST9-15` |
| MA11 | Define `AuthError` or drop it for `Status`; give the resolution queries | `R-6`, `Z5-9` |
| MA12 | Pin the `access` field name and representation, or cut it with MA1 | `R-8`, `ST9-8`, `C-10` |
| MA13 | Cite `/openapi.json` provenance; reword Requirement 7 | `D-2`, `ST7-9`, `ST9-10`, `R-11`, `S-10` |
| MA14 | Reword §6: the nested guard already exists; fix the contradictory attack example | `CB-3`, `ST9-7` |
| MA15 | Correct the initContainer premise; state the downtime risk | `D-3` |
| MA16 | Reduce the handover to `BOOTSTRAP_OWNER_SUBJ`, or justify the email path | `Z5-6`, `D-6` |
| MA17 | Add `## Alternatives Considered` and `## Risks & Mitigations`; reorder Architecture | `S-1`, `S-2`, `S-3` |
| MA18 | Give every Non-Goal a spec pointer or an explicit "no planned spec" | `S-7` |
| MA19 | Fix the architecture diagram: line 118 width, split annotation, target-state label | `S-4`, `S-5`, `S-13`, `ST7-17` |
| MA20 | Standardize `sub`/`subj`, self-registration, the environment axis, "silent", "roles" | `ST7-8`, `ST7-11`, `ST9-18`, `ST9-46` |
| MA21 | Move the Open Questions changelog to a Revision History section | `ST9-57`, `ST9-17`, `ST9-38`, `C-7` |
| MA22 | Split CI gates from local-only gates | `ST9-51`, `Z5-1` |
| MA23 | Replace unmeasurable claims with the checkable property | `ST9-22`, `ST9-23`, `ST9-24`, `ST7-3`, `ST7-4`, `Z8-9` |
| MA24 | Call `PATCH /api/portfolios/:id` the currency-update route | `CB-5`, `R-10`, `Z5-13`, `ST9-39` |
| N1–N16 | Copy-editing and evidence-count pass | see section 6 for the per-group mapping |

---

## 9. Review coverage and caveats

Disclosed so you know what was and was not checked.

- **Seven of the nine reviewer passes were cut short by the runner's 30-minute deadline** and produced their report from work already done, under instruction to stop reading and write. The Comprehension pass and the Stage 8 Zoom Out finished naturally. Treat the `## Not assessed` / `## Not verified` sections in each raw report as real gaps, not as clean results.
- **`egm-codebase-details` did not open**: `frontend/nginx.conf`, `frontend/vite.config.js`, `.github/workflows/ci.yml`, `start.sh`, `frontend/src/App.tsx`, `CsvImportModal.tsx`, `PortfolioDetail.tsx`, the MSW setup, `package.json`, `frontend/src/types/api.d.ts`, `k8s/app-service.yaml`, `k8s/ingress.yaml`, or commit `2753049`. Its findings are confined to `backend_rust/src/lib.rs`, `api_routes/transactions.rs`, `api_routes/portfolios.rs`, `openapi.rs`, `Cargo.toml`, `Cargo.lock`, `Dockerfile` and `k8s/app-deployment.yaml`. The CI claims in B1 therefore come from the Comprehension and Zoom Out passes, not from the codebase auditor.
- **`egm-design` did not verify** the frontend `fetch` inventory, the MSW setup, or the `openidconnect` 4.1 API surface.
- **`egm-implementation-readiness` read only `k8s/app-deployment.yaml`** of the deployment and CI surface; it noted in passing that the deployment's PVC is named `postgresql-db-pvc` despite the app using SQLite, which the spec does not mention.
- **`egm-structural` did not open source files** — its pass is structural only, so its "ten hand-written SQL statements" finding is a citation gap, not a count. The count of 12 comes from `ST7-2`.
- **No reviewer verified** `openidconnect = "4.1"` existence, its MSRV, or the `cargo tree` outcome for TLS unification. `Z5-11` flags this as a claim to check with `cargo tree -d` before pinning.
- **Line numbers cited by reviewers** are from the document at HEAD `4df46b3`. Any edit shifts them; re-derive before applying fixes in bulk.
