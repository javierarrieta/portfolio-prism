## Findings

### 1. Architecture after the three prior passes — which findings are *design* changes, and what the design becomes

- **Severity**: Major
- **Affected Scope**: whole document; specifically Requirements R7/R11/R12 (28–50), §3 (217–228), §4 (229–305), §5 (306–348), §6 (349–385), §7 (386–393)
- **Simplicity Rationale**: The spine survives: OIDC code flow at the backend, encrypted private-cookie session, JIT-provisioned `users` row, `portfolios.owner_id`, one `access` chokepoint, `404`-not-`403`, public `GET /`, deny-by-default dev escape hatch. Every prior finding that implies a *design* change (not a wording change) is a subtraction, and each one simplifies:

| Prior finding | Design becomes | Effect |
|---|---|---|
| zoom-out 3 (access lattice unobservable) | `access::require_owner(user, portfolio_id) -> bool`; no `Access` enum, no `meets()`, no `portfolio_members` DDL/index/CHECK, no `access` field on `PortfolioOut` | **simplifies** — deletes a table, an enum, a public contract change, and Spec-1 test cases |
| zoom-out 4 (revocation with no trigger) | guard = decrypt cookie → return `AuthedUser`; no `token_version`, no `is_active` read, no `logout-all` | **simplifies** — zero DB reads per request, which is the design's own stated reason for rejecting a session table |
| zoom-out 5 + Blocker 2 below | plain nullable `ADD COLUMN owner_id INTEGER`; no rebuild | **simplifies** — see finding 3 |
| zoom-out 6 (bootstrap over-built) | `BOOTSTRAP_OWNER_SUBJ` only; delete `BOOTSTRAP_OWNER_EMAIL`, the `bootstrap:` prefix, the `email_verified` gate and its reconciliation rules | **simplifies** — removes the only ownership decision that depends on an IdP claim the design cannot trust |
| zoom-out 7 (rate limiter) | cut R12 from Spec 1 | **simplifies** — deletes a stateful subsystem with a background eviction task |
| zoom-out 8 + comprehension 3 (route-table mirror) | delete the table-inventory test; the isolation matrix already iterates the router | **simplifies** — see finding 6 for the better replacement |
| comprehension 2 (`Disabled` backfill collision) | state the resolution order once: `AuthMode` resolved after `init_db`; in `Disabled` mode the refuse-to-start rule is satisfied by the seeded `bootstrap:dev` row | neutral — one sentence, but it is currently a contradiction between §1 and §4 |

Nothing in the three passes requires adding a subsystem. That is a good sign, and it means the correct Stage-9 edit is deletion, not elaboration.

### 2. Blocker adjudication — CI OpenAPI regeneration: fix by decoupling, not by configuring

- **Severity**: Blocker
- **Affected Scope**: R7 (28–50), "OpenAPI, CI & Frontend Impact" (466–485), Testing "Gates" (~520); evidence `.github/workflows/ci.yml:23-33`, `backend_rust/src/openapi.rs:6-33`
- **Simplicity Rationale**: Both prior passes implied the same fix — export `AUTH_MODE=disabled ROCKET_PROFILE=dev` in the CI job. That is configuring *around* a coupling rather than removing it, and it leaves three live couplings: CI must keep a valid auth-mode env forever, must additionally supply `ROCKET_SECRET_KEY` for any release-build variant (AGENTS.md's procedure uses `cargo run --release`, which this spec says hard-errors without one), and the *content of the published API contract* now depends on runtime configuration. The spec-generation path also currently needs a dummy `portfolio.db`, a `sleep 10`, and a `kill $(jobs -p)` — a running server is being used as a serializer.

  **Simplest acceptable resolution**: `ApiDoc` is a plain `#[derive(OpenApi)]` struct (`backend_rust/src/openapi.rs:6`), so the document is a pure function of the code. Add an early exit in `backend_rust/src/main.rs` — if argv contains `--print-openapi`, print `ApiDoc::json()` and exit — and change the CI step to `cargo run -- --print-openapi > current_openapi.json`. No server, no DB, no auth mode, no secret key, no sleep, no orphan process. This deletes the Blocker instead of working around it, and it makes R7 (authenticate `/api-docs`) free: the runtime route can require a session while the generator never touches it. If the team rejects the generator change, the minimum acceptable text is the CI env export **plus** an explicit `ROCKET_SECRET_KEY` for the release-build variant **plus** a corrected regeneration procedure in AGENTS.md — but say in the doc that this is the second-best option.

### 3. Blocker adjudication — `migrate_one_column` reuse: resolve by deleting the requirement

- **Severity**: Blocker
- **Affected Scope**: §4 "Making `owner_id` NOT NULL" and its schema-state trigger (229–305); Decisions Summary "Unowned rows at startup" row (89–104); Testing bullet "a database with no orphans still ends up with `owner_id NOT NULL`" (~505)
- **Simplicity Rationale**: I verified the claim is false. `migrate_one_column` (`backend_rust/src/lib.rs:149`) is a private crash-recovery state machine whose every branch hardcodes `ADD COLUMN {column} TEXT NOT NULL DEFAULT '0'` and whose state is keyed on the literal string `"TEXT"` (lines 158–186). It cannot emit `owner_id INTEGER NOT NULL`; generalizing it means editing the repo's most fragile, most specifically-tested helper to serve one new caller. That is a refactor of migration code, not reuse, and the doc's "no new migration framework is introduced" is doing load-bearing work it has not earned.

  **Simplest acceptable resolution**: drop the `NOT NULL` rebuild entirely. `ALTER TABLE portfolios ADD COLUMN owner_id INTEGER` is a plain SQLite add — no rename/copy/drop. The invariant is already enforced at the only place it matters (refuse to start when orphans exist), and `access::resolve` must treat `NULL` as no-access anyway because foreign keys are unenforced by the design's own admission. This deletes: the helper generalization, the `PRAGMA table_info` trigger branch, the fresh-vs-upgraded schema test, and the risk of rebuilding the table that `assets` and `transactions` reference — on every fresh install. "Fresh and upgraded schemas are byte-identical" is an aesthetic goal being paid for with a migration. If the team insists on `NOT NULL`, then the honest text is: write a dedicated single-purpose rebuild function with its own crash test, do **not** touch `migrate_one_column`, and correct the Decisions Summary row. Prefer deletion.

### 4. Missed at altitude — `AuthMode::Disabled` is gated on a runtime string when it can be gated at compile time

- **Severity**: Major
- **Affected Scope**: §1 allowlist table and prose (140–194), Decisions Summary "`Disabled` auth mode" row (89–104), Configuration & Deployment `ROCKET_PROFILE` row (410–465), `start.sh` paragraph (~440), Testing bullets on the allowlist (~500), Security Considerations "`Disabled` mode" (550–575)
- **Simplicity Rationale**: The design's central safety claim is "there is no other way to obtain an `AuthedUser`" — but §1 immediately defines a second way, selected by an environment variable plus a config-profile string. The mitigation the document spends the most prose on (allowlist, unset-is-refusing, "highest-risk line in the design") is a *runtime* string comparison guarding a *build-time* fact. Gate `AuthMode::Disabled` on `cfg!(debug_assertions)` instead: the variant is not compiled into the release binary at all, so no forgotten `ROCKET_*`, no ConfigMap drift, no `AUTH_MODE` typo can enable it. I confirmed `start.sh:13` is a plain debug `cargo run`, so requirement 10 is unaffected, and `cargo test` runs debug so the isolation matrix is unaffected.

  This deletes the allowlist table, the `ROCKET_PROFILE` row from the configuration contract, the `ROCKET_PROFILE=dev` export in `start.sh`, the "unset in a release build" reasoning, and three test bullets — while being strictly stronger than the current design. The only residual is `cargo test --release`, which should be recorded as unsupported. This is the single highest-value simplification available and no prior pass raised it.

### 5. Missed at altitude — requirement 1 is enforced per-handler; a mount-level fairing makes it true by construction

- **Severity**: Major
- **Affected Scope**: R1 (28–50), §1 `FromRequest` claim (178–194), §6 closing paragraph (381–385), Testing bullets (486–527)
- **Simplicity Rationale**: Comprehension finding 9 correctly noticed that "the type system makes protection impossible to forget" is stronger than the mechanism, but filed it as a Nit and left the design unchanged. At altitude the fix is structural and cheap: one Rocket `Fairing` on the request lifecycle that returns `401` for any URI under the `/api` and `/api-docs` prefixes unless a valid session resolves. That is the platform-standard Rocket pattern, and it makes R1 literally true rather than conditionally true on handler authoring. It also collapses two other open items: the route-table inventory test (zoom-out 8, comprehension 3) exists mainly to catch an unauthenticated route, and the fairing removes that failure mode entirely — leaving the isolation matrix as the only authorization-drift guard, which is the single guard the design actually needs. Net: one small unit added, one unspecified-mechanism test and one class of silent hole removed.

### 6. Missed at altitude — the chokepoint claim is contradicted by the nested-route rule

- **Severity**: Major
- **Affected Scope**: §5 `require_asset` / `require_transaction` (306–348), §6 nested-route paragraph and table rows (349–385), Security Considerations "Chokepoint discipline" (550–575)
- **Simplicity Rationale**: The design's safety rests on "handlers must not contain `WHERE id = ?` against those tables" — yet §6 requires handlers on nested routes to additionally "verify `asset.portfolio_id == id`". That is a hand-written ownership predicate in the handler, i.e. exactly the forgettable, per-site rule the design set out to eliminate, and it now lives in two places with two different mechanisms. The stated invariant should be: *one entry point, keyed on whatever id the caller supplied*. `require_asset` and `require_transaction` already resolve the parent chain inside `access`; the path-vs-asset consistency check belongs in the same call (resolve from the asset, then reject if the resolved portfolio does not match the path id), not in a prose convention repeated at two call sites. As written, a future nested route can satisfy "I called `access`" and still be cross-portfolio writable — the precise hole the paragraph was added to close. This is a design change (move the check into `access`), and it simplifies: one rule, one place, no handler-side memory requirement.

### 7. Root-problem drift — three of twelve requirements do not serve the root problem, and they are the origin of both Blockers

- **Severity**: Major
- **Affected Scope**: R7, R11, R12 (28–50); §4 members DDL (229–305); §5 (306–348); §7 (386–393); OpenAPI/CI section (466–485)
- **Simplicity Rationale**: The root problem is stated crisply and correctly in the Overview: unauthenticated callers can read and mutate each other's financial data on enumerable ids. R7 (authenticate `/api-docs`), R11 (write access-control against the final three-role model now) and R12 (per-user lookup rate limit) do not serve it. Trace what those three requirements alone cost: the cookie `securityScheme` and `401` annotation on every path, the CI/spec-regeneration coupling that is Blocker 1, the `/openapi.json` alias question, the `portfolio_members` table plus `Access` enum plus `meets()` plus role `CHECK` plus index plus a new field on a public response shape (zoom-out 3), and a stateful token-bucket subsystem with eviction (zoom-out 7). Cut them and Spec 1 is: OIDC login → `users` row → `owner_id` → `require_owner` → `404` → bootstrap backfill by `sub` → dev mode compiled out of release. That is materially closer to the minimal shape and every Blocker disappears with it. If R7 is kept for disclosure reasons, finding 2's generator decoupling makes it cost nothing. R11 is the textbook Zoom-Out rejection the document explicitly volunteers for ("written against the final access model even though sharing is not delivered until Spec 2"). R12 protects a quota against users who, before this spec, were not able to reach the endpoint at all.

### 8. Missed — remaining Spec-3 forward-shaping after the `bearerAuth` rejection

- **Severity**: Nit
- **Affected Scope**: Non-Goals (52–64), OpenAPI section "bearerAuth is deliberately not declared" and "CORS unchanged … so Spec 3's bearer tokens need no later change" (466–485)
- **Simplicity Rationale**: The document correctly refuses to *declare* a bearer scheme, but still says the guard "is shaped to accept a bearer token here". Same class as finding 7 / zoom-out 3, not called out. The guard should be cookie-only; Spec 3 adds a second credential path in the change that first needs it.

### 9. Missed — the primary acceptance gate never exercises the production authentication path

- **Severity**: Nit
- **Affected Scope**: Testing Strategy, isolation matrix paragraph (486–527)
- **Simplicity Rationale**: The matrix runs entirely under `AuthMode::Disabled { as_user }`, so cookie decryption, session validation and guard attachment under OIDC are covered by two bullets (`GET /` public, `/api-docs` 401). That is fine as a stated scope boundary, but the document should say plainly that the matrix proves *authorization*, not *authentication* — otherwise "primary acceptance gate" overstates it. If finding 5's fairing is adopted, one matrix run with a real session cookie is nearly free.

### 10. Missed — the only silent-failure path in the design is deliberately unautomated

- **Severity**: Nit
- **Affected Scope**: Configuration & Deployment "Same-origin proxy paths" (410–465), Rollout step 2 (528–549)
- **Simplicity Rationale**: The document correctly identifies that nginx's `location /` and the Vite proxy both swallow `/auth`, that the failure is a silent redirect loop, and that both files sit outside both test suites — then resolves it with a manual curl. A one-line assertion in an existing test that the two config files contain an `/auth` proxy entry is cheaper than the manual step and removes the only unautomated silent failure in the spec. Accepting the manual step is also defensible; the document just should not claim the failure mode is guarded.

## Architecture verdict

The core shape — backend-terminated OIDC, private-cookie session, JIT `users` row, `portfolios.owner_id`, a single `access` chokepoint returning `404`, and a public `GET /` for probes — is the simplest shape that meets the root requirement, and the three prior passes produced only subtractions, which confirms it. It is not yet minimal: three of twelve requirements (R7, R11, R12) generate both Blockers and four of the Majors without touching the root problem, and two safety mechanisms (the profile-string allowlist, the per-handler auth claim) can be replaced by a compile-time gate and a mount-level fairing that are simultaneously simpler and stronger. Adopt findings 2, 3, 4, 5 and 7 and the spec shrinks by roughly a third while its fail-safe posture improves; leave them and it ships a working but over-scoped Spec 1 that has already taught CI and the migration helper to depend on it.