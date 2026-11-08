## Findings

**1. Severity: Blocker**
**Affected Scope:** "OpenAPI, CI & Frontend Impact" (lines ~472-484) + "Testing Strategy / Gates" (~556); evidence in `.github/workflows/ci.yml:22-33`
**Simplicity Rationale:** The CI OpenAPI job does `cargo run --bin backend_rust` with no env vars, then `curl http://127.0.0.1:8000/api-docs/openapi.json`. This spec makes `/api-docs` authenticated (req. 7) and defaults `AUTH_MODE=oidc`, so that curl returns `401` and the diff compares garbage — CI goes red on the first PR. The doc notices that CI omits `npm run lint` but never notices that CI's own spec-generation step is now an unauthenticated call under a default-on auth mode. It also never states whether OIDC issuer discovery happens at launch; if it does, `cargo run` and `cargo test` (req. 9) need network. Both are one-line decisions (CI job exports `AUTH_MODE=disabled ROCKET_PROFILE=dev`; discovery is lazy on first `/auth/login`) that are absent.

**2. Severity: Blocker**
**Affected Scope:** "§4 Schema migration — Making `owner_id` NOT NULL" (lines ~300-312)
**Simplicity Rationale:** The claim "the existing `migrate_one_column` helper already performs the rename → add → copy → drop rebuild … the `owner_id` rebuild reuses that mechanism, so no new migration framework is introduced" is false as written. `migrate_one_column` (`backend_rust/src/lib.rs:149`) is private and hardcodes the target type as `TEXT NOT NULL DEFAULT '0'` in every branch; it cannot emit `owner_id INTEGER NOT NULL`. Reusing it means generalizing a crash-recovery-tested six-branch helper that `tests/decimal_migration_test.rs` pins — a refactor of the repo's most fragile code path, not a reuse. The spec should say which it is.

**3. Severity: Major — YAGNI: the three-level access lattice is unobservable in this spec**
**Affected Scope:** Requirement 11 (lines ~40); Non-Goals (~52); §4 `portfolio_members` DDL (~276-290); §5 `Access` enum + `resolve` fallback (~340-380); §6 "each row carries the caller's `Access`" (~392)
**Simplicity Rationale:** With the members table guaranteed empty, every authorized caller is `Owner`. `Editor`, `Viewer`, `meets()`, the members fallback query, the role `CHECK`, `idx_portfolio_members_user`, and the `access` field added to the public `PortfolioOut` contract are all dead weight until Spec 2. Requirement 11 is the spec explicitly instructing itself to build for a speculative future requirement — the textbook Zoom-Out rejection. The durable architectural commitment is the *chokepoint* (`access::require` as the only path to a portfolio id), and that survives unchanged with a single `require_owner(user, portfolio_id)`. Spec 2 introduces the lattice in the same change as its first write path, which is where the design effort belongs.

**4. Severity: Major — YAGNI: revocation machinery with no trigger**
**Affected Scope:** §2 `POST /auth/logout-all` (~236); §3 Session validation and revocation (~250-262); `users.token_version` / `is_active` DDL (~266-275); Security Considerations "Deactivated users" (~596)
**Simplicity Rationale:** Nothing in this spec ever sets `is_active = 0` (Authentik sync is an explicit non-goal) and there is no user-management UI by design, so `logout-all` has no caller. That is three coupled mechanisms — a column, a guard read, an endpoint — none of which can fire. Dropping them makes the guard a pure cookie decrypt with **zero database reads per request**, which is strictly simpler and faster than the design's own stated reason for rejecting a session table. Add `token_version` in the spec that first needs to revoke something.

**5. Severity: Major — the `NOT NULL` rebuild buys nothing**
**Affected Scope:** §4 "Making `owner_id` NOT NULL" + its schema-state trigger (~300-312); test bullet "a database with no orphans still ends up with `owner_id NOT NULL`" (~546)
**Simplicity Rationale:** The startup check already refuses to boot with unowned portfolios, so the invariant is enforced at the only place it matters. The rebuild adds a rename/copy/drop of `portfolios` — the table `assets` references — on *every fresh install*, plus a `PRAGMA table_info` trigger branch, plus the helper generalization from finding 2. A nullable `owner_id` that `resolve` treats as no-access is the minimal design; "fresh and upgraded installs have identical schemas" is an aesthetic goal costing a migration.

**6. Severity: Major — bootstrap handover is over-built for a one-time single-tenant move**
**Affected Scope:** Decisions Summary "Legacy-portfolio handover" (~90); §4 Backfill + Placeholder reconciliation (~318-345); rollout step 5 (~570); Known Limitations (~604-610)
**Simplicity Rationale:** Two env vars, a `bootstrap:` subject namespace, an `email_verified` gate, reconciliation rules, and dedicated tests exist to move one tenant's rows once — during a window the rollout itself declares single-tenant. The email variant is the risky half and the doc spends its most elaborate security prose defending it. Keep only `BOOTSTRAP_OWNER_SUBJ`: the operator signs in once, reads their `sub` from `/auth/me`, sets it, restarts. That deletes `BOOTSTRAP_OWNER_EMAIL`, the verified-email gate, the `bootstrap:` prefix, and the reconciliation logic while staying fail-safe.

**7. Severity: Major — rate limiter is scope creep and an altitude drop**
**Affected Scope:** Requirement 12 (~44); §7 (~420-430); Security Considerations "Open signup + third-party quota" (~598); rate-limiter tests (~530)
**Simplicity Rationale:** A token bucket with capacity/refill constants, `Mutex<HashMap<i32, Bucket>>` and "periodic eviction of idle entries" is a new stateful subsystem with a background task, inside an identity-and-isolation spec, for a deployment the doc sizes at "tens of users". The endpoint is already strictly better protected than today (it is currently public to anyone who can reach the host). Cut it from Spec 1, or reduce it to a fixed-window counter; the bucket constants and eviction policy are code-level detail that does not belong in an architecture spec.

**8. Severity: Major — the route-table drift test is a third source of truth**
**Affected Scope:** §6 closing paragraph (~405-412); test bullet "every API route mounted in `build_rocket` appears in the §6 route table" (~548)
**Simplicity Rationale:** This adds a machine-readable mirror of the router next to the router. The isolation matrix already iterates "every API route mounted in `build_rocket`" and pairs B→`404` with A→success, so a route omitted from the guard fails the matrix automatically. The matrix *is* the drift test; the table-diff test is redundant.

**9. Severity: Nit**
**Affected Scope:** §5 signatures returning `AuthError` (~350-365); §8 (~432-445)
**Simplicity Rationale:** Every handler in `portfolios.rs`, `transactions.rs`, `analytics.rs`, `lookup.rs` uses `Result<T, Status>`. Introducing an `AuthError` enum for one call site breaks pattern consistency for no gain — `Status::NotFound` and `Status::Unauthorized` already express both outcomes.

**10. Severity: Nit**
**Affected Scope:** §1 `AuthMode::Disabled` allowlist table (~150-165); Configuration & Deployment `ROCKET_PROFILE` (~455)
**Simplicity Rationale:** Rocket's debug-build default profile is already `debug`, and `cargo test` runs under it. Listing `dev` and `test` widens the security allowlist and forces `start.sh` to export `ROCKET_PROFILE=dev` for no benefit — `{debug, test}` or just "the debug default" is the same guarantee with less surface.

**11. Severity: Nit — unverifiable / stale dependency claims**
**Affected Scope:** §1 "Dependency added" (~167-176)
**Simplicity Rationale:** `openidconnect = "4.1"`, its MSRV 1.88, and its "reuses the existing rustls-based reqwest" claim cannot be confirmed from the repo (Cargo.lock has reqwest 0.12.28, no openidconnect). The stated reason to avoid `rand` — "risks two majors in the tree" — is already moot: Cargo.lock carries `rand` 0.8.6 **and** 0.9.4 today. Confirm with `cargo tree -d` before pinning.

**12. Severity: Nit — evidence error changes the fix**
**Affected Scope:** Current State table row 3 (~108); OpenAPI section (~476)
**Simplicity Rationale:** The doc says `PATCH /api/portfolios/<pid>/assets/<aid>` "carries no `#[utoipa::path]`". It does carry one (`api_routes/transactions.rs:85`); the route is simply absent from the `paths(...)` list in `src/openapi.rs`. The prescribed fix ("adds the missing annotation") is a no-op and the published spec would still report 13 operations.

**13. Severity: Nit**
**Affected Scope:** §6 access-level semantics (~370-380) and route table `PATCH /api/portfolios/:id` (~395)
**Simplicity Rationale:** `PATCH /api/portfolios/:id` is described as "renaming"; `update_portfolio` (`portfolios.rs:178`) only writes `currency`/`base_currency`. That weakens the stated Owner-vs-Editor rule ("Owner alone may destroy structure") for a level that, per finding 3, has no non-owner holders anyway.

---

**What holds up well (no action):** Context sufficiency is strong — objective, motivation, and root problem (unauthenticated API + enumerable ids) are stated up front and every proposal traces back to them. Verified-correct evidence: no auth anywhere (`lib.rs:53` is CORS only), no owner columns, `portfolios.id` AUTOINCREMENT, probes and initContainer on `GET /:8000` (`k8s/app-deployment.yaml`), `configmap.yaml` lowercase with no `ROCKET_*`, nginx `location /` swallowing `/auth/login`, Vite proxy missing `/auth`, no lint gate in CI. Reuse checks came back clean: Rocket **private cookies** is the right platform standard (no bespoke JWT layer), `frontend/src/utils/apiFetch.ts` is genuinely new (existing utils are csvParser/csvValidator/decimal/formatters/isinValidator only — no fetch wrapper to reuse), MSW is already wired, and `seed_user` matches the `tests/common/mod.rs` seed pattern. The `404`-not-`403` decision, the deny-by-default profile allowlist, and refusing to start on unowned rows are all correctly reasoned and I would not simplify them.

## Not assessed

- Frontend test-plan specifics (MSW `401` handler shape, existing component-test churn from routing ~14 `fetch` calls through `apiFetch`) — noted as plausible, not verified case by case.
- Spec 2/3/4 boundary content beyond what is quoted in Non-Goals and Deferred Work.
- Whether `openidconnect` 4.1 exists and its reqwest major — flagged as finding 11 rather than verified.