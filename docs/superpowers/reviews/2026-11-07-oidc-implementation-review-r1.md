Status: awaiting operator decision (egm-resolve step 4)
Round: 1 of the implementation review
Reviewed: `docs/superpowers/reviews/2026-11-07-oidc-implementation.md` (full file), and the implementation in this worktree
Fixes landed in: `d97878d`

# Goldfish Annotated Review Report

**Target:** `docs/superpowers/reviews/2026-11-07-oidc-implementation.md` (`full file`)

Three Goldfish auditors reviewed independently:

* **[readiness]** — implementation-readiness
* **[details]** — codebase-details (document vs source)
* **[design]** — design critic (security, operations, standard compliance)

Findings are deduplicated across reviewers; a finding reported by two or three is
listed once with all attributions. **Every `PROPOSE FIX` below has already been
applied in the working tree** — the code fixes were made as they were confirmed, and
each was re-verified against the gates. The report is the record of what was accepted,
what was rejected, and why, so it can be contested item by item.

Totals: **9 Blockers, 19 Majors, 11 Nits** consolidated from 44 raw findings.
Applied: 26. Contested: 3. Recorded as accepted risk, not fixed: 8. Declined: 2.

---

## Blockers

### B1. `logout-all` permanently locks the user out — and every test passed `[readiness, details, design]` — Severity: High

* **Affected Scope:** `backend_rust/src/auth/routes.rs` (callback), `src/auth/oidc.rs`, `src/auth/guard.rs`
* **Discrepancy Description:** The callback issued `session::issue(cookies, user.id, 0)` with a literal `0`. `POST /auth/logout-all` raises `users.token_version`; the guard rejects `tv != token_version`; nothing lowers the column again. After one `logout-all`, every subsequent login is rejected forever.
* **Rationale:** The system's only revocation lever is an account lock. The suite missed it because every test fabricated its cookie at `tv = 0` rather than deriving it from the login path — the bug was baked into the fixture.
* ***AI Suggestion:*** PROPOSE FIX — **applied**
* ***Justification:*** Confirmed by reading the code, and confirmed behaviourally. Three independent reviewers reached the same conclusion from different directions.
* ***Proposed Edit:***
    * Location: `src/auth/guard.rs`, `src/auth/routes.rs`, `src/auth/oidc.rs`, `tests/auth_test.rs`
    * Change: `AuthedUser` now carries `token_version`; `provision_or_update` returns it on all three paths (including `UPDATE … RETURNING token_version` on the bootstrap handover); version selection lives in exactly one place, `AuthedUser::session_pair`, which `issue_session` derives from, so the callback cannot name a version at all. New test `logging_out_everywhere_does_not_lock_the_user_out` mints through the same seam production uses.
    * **Verification:** the test was run with the literal `0` reintroduced into the seam — it **failed** — and passes with the fix. A regression test that cannot fail is not a fix.

### B2. No test proves an authenticated session reaches a data route `[readiness, design]` — Severity: High

* **Affected Scope:** `backend_rust/tests/auth_test.rs`
* **Discrepancy Description:** Every `private_cookie` call in the suite targeted `/auth/me`. The 15-route sweep asserted only the *unauthenticated* half, so "every `/api` route 401s without a session" was equally satisfied by a build that 401s **with** one — which is precisely B1.
* **Rationale:** A one-sided test on the security property is a test of the failure path only.
* ***AI Suggestion:*** PROPOSE FIX — **applied**
* ***Proposed Edit:***
    * Location: `tests/auth_test.rs`
    * Change: added `every_api_route_reaches_a_handler_with_a_session`, the mirror sweep, asserting non-`401` for every mounted route with a valid cookie. `GET /api/assets/lookup` is excluded with a stated reason (it would make a real outbound call); the sweep asserts 14 of 15 so the exclusion cannot silently grow.

### B3. The route list was duplicated in two test files `[details]` — Severity: High

* **Affected Scope:** `tests/auth_test.rs` (`all_api_routes`), `tests/route_coverage_test.rs` (`PROTECTED_ROUTES`)
* **Discrepancy Description:** Two hand-maintained lists of the same 15 routes. Each could agree with itself while neither matched the mount.
* ***AI Suggestion:*** PROPOSE FIX — **applied**
* ***Proposed Edit:*** `all_api_routes()` now enumerates the ignited router and fills `<param>` slots, so a route added without touching the test is swept automatically. `PROTECTED_ROUTES` stays as the *level* inventory, which the mounted-surface test already reconciles against the router.

### B4. Record claimed `Secure` is set when the request arrived over TLS; the code sets it unconditionally `[details, design]` — Severity: High

* **Affected Scope:** record §2.2; `src/auth/session.rs`
* **Discrepancy Description:** The document stated a conditional that does not exist. The code is the better answer; the document was wrong and would be cited as the contract.
* ***AI Suggestion:*** PROPOSE FIX — **applied**
* ***Proposed Edit:*** record §2.2 now states unconditional `Secure` and the reason (TLS terminates at the ingress, no supported http production path, a conditional flag weakens the cookie exactly where it is weakest). Added as deviation D-5.

### B5. Record's `users` schema omitted `last_login_at` `[details]` — Severity: High

* **Affected Scope:** record §2.4
* ***AI Suggestion:*** PROPOSE FIX — **applied**
* ***Proposed Edit:*** column list corrected; `provision_or_update` writes it on all three paths.

### B6. Record claimed one shared 401 body; there are two, and now three `[details]` — Severity: High

* **Affected Scope:** record §2.1; `src/auth/routes.rs`
* **Discrepancy Description:** `/_auth/denied` and the `401` catcher return `{"error":"unauthenticated"}`; `/auth/callback` on a failed exchange returns `{"error":"authentication_failed"}`. The document claimed a single body for every 401.
* ***AI Suggestion:*** PROPOSE FIX — **applied**
* ***Proposed Edit:*** record §2.1 now names all three bodies and states why the callback's is deliberately distinct (a different fact: the user *did* interact with the provider). The new `503` body is listed too.

### B7. A database failure during authentication was reported as a denial `[details, design]` — Severity: High

* **Affected Scope:** `src/auth/fairing.rs`
* **Discrepancy Description:** The fairing's `Err` branch rewrote to the `401` sentinel. `guard.rs` and `access.rs` both state as a design rule that a database failure must never be presented as a denial; the fairing violated its own module's rule. Concretely: `SQLITE_BUSY` is reachable because the read endpoints write, and `apiFetch` turns a `401` into a redirect to the identity provider — a loop for the duration of the lock, discarding in-flight form state.
* ***AI Suggestion:*** PROPOSE FIX — **applied**
* ***Proposed Edit:*** new `/_auth/unavailable` sentinel (all seven methods) answering `503 {"error":"service_unavailable"}`; `a_database_failure_is_not_reported_as_a_denial` drops the `users` table and asserts `503`. `route_coverage_test` pins that the sentinel covers every method, so no method answers `405` where `503` is the truth.

### B8. The release-binary refusal was claimed and checked by nothing `[details, design]` — Severity: High

* **Affected Scope:** `.github/workflows/ci.yml`, record §6
* **Discrepancy Description:** The refusal is behind `!cfg!(debug_assertions)`, so no `cargo test` can reach it. The CI job only compiled, and its own comment said "that is checked by a test". No such test existed.
* ***AI Suggestion:*** PROPOSE FIX — **applied**
* ***Proposed Edit:*** two CI steps run the built release binary and assert a non-zero exit plus the named variable — one for `AUTH_MODE=disabled`, one for a missing OIDC configuration. Both were run locally and pass. The comment now says what actually happens.

### B9. `/api-docs` does not exist as a route `[details]` — Severity: Medium

* **Affected Scope:** record §6
* ***AI Suggestion:*** PROPOSE FIX — **applied**
* ***Proposed Edit:*** record now says `/api-docs/openapi.json`.

---

## Majors

### M1. `rocket_cors` runs before authentication, and the shipped configmap never set `ALLOWED_ORIGINS` `[design, details]` — Severity: High

* **Affected Scope:** `src/lib.rs` attach order, `k8s/configmap.yaml`, `frontend/src/utils/apiFetch.ts`
* **Discrepancy Description:** `rocket_cors` rewrites a request whose `Origin` is not allowed to its own `/cors/403` route. The rewritten path is not under `/api/`, so the auth layer never sees it. No data leaks — the handler does not run — but browsers send `Origin` on **same-origin** writes, so a deployment left at the localhost default answers `403` to every `POST`/`PATCH`/`DELETE` from the public site, and `apiFetch` redirects only on `401`, so the user sees a silent failure.
* ***AI Suggestion:*** PROPOSE FIX — **applied**
* ***Proposed Edit:*** `k8s/configmap.yaml` sets `ALLOWED_ORIGINS` with a comment explaining the mechanism; new test `a_disallowed_origin_is_answered_before_authentication_gets_a_turn` pins both halves (disallowed → `403`, allowed → `401`); record gains §2.6.

### M2. The JWKS cache had no expiry — key rotation is an outage `[design]` — Severity: High

* **Affected Scope:** `src/auth/oidc.rs`
* **Discrepancy Description:** Both caches were keyed on issuer with no TTL and no refetch on an unknown `kid`. When Authentik rotates its signing key, every login fails until the pod restarts, with no alarm.
* ***AI Suggestion:*** PROPOSE FIX — **applied**
* ***Proposed Edit:*** 15-minute TTL on the JWKS cache (`CACHE_TTL`), with the reasoning recorded at the cache declaration.

### M3. The three access levels are behaviourally identical `[readiness, design]` — Severity: Medium

* **Affected Scope:** `src/access.rs`, record §2.5
* **Discrepancy Description:** `resolve` returns `Access::Owner` for any owner, so `grade` can never deny a level. `delete_transaction` is graded `Editor` while `delete_asset` is `Owner` — two destructive operations, two grades, no difference in effect. The day `resolve` returns `Access::Viewer` for anything, every `Editor` call site becomes writable by a viewer, with no test to catch it.
* ***AI Suggestion:*** PROPOSE FIX (documentation, not code) — **applied**
* ***Justification:*** The enum is deliberately threaded now so a future sharing model has somewhere to hang; collapsing it to a boolean would be a larger change than this PR's scope and would have to be redone. The honest fix is to say plainly that the levels are inert and name the hazard.
* ***Proposed Edit:*** record §2.5 states the levels are inert today and why the enum is kept; §9.13 records the drift hazard.

### M4. `npm run lint` was in the plan's gate list and absent from CI `[readiness]` — Severity: Medium

* ***AI Suggestion:*** PROPOSE FIX — **applied**
* ***Proposed Edit:*** lint step added to the frontend job. Verified locally: 0 errors, 1 pre-existing warning.

### M5. `BOOTSTRAP_OWNER_SUBJ: "REPLACE_WITH_AUTHENTIK_SUB"` defeats the refuse-to-start guard `[readiness, design]` — Severity: High

* **Affected Scope:** `k8s/configmap.yaml`
* **Discrepancy Description:** The guard distinguishes set from unset. A sentinel string is *set*, so applying the manifest unedited hands every pre-existing portfolio to the placeholder row — which no identity can ever claim, because no provider issues that `sub` — instead of stopping.
* ***AI Suggestion:*** PROPOSE FIX — **applied**
* ***Proposed Edit:*** value set to `""` (blank counts as unset, exactly like an absent key), with the reasoning written into the manifest.

### M6. Rollback as documented was unreachable `[design]` — Severity: Medium

* **Affected Scope:** `k8s/app-deployment.yaml`
* **Discrepancy Description:** Both containers pinned to `:latest`. `AUTH_MODE` cannot undo a bad auth deploy (`disabled` is not compiled into the release image), so rollback is `kubectl rollout undo` — which on a `:latest` tag produces a byte-identical pod template and rolls out nothing.
* ***AI Suggestion:*** PROPOSE FIX — **applied**
* ***Proposed Edit:*** image pinned by digest with the reasoning inline; §9.16 records that the operator must still record the digest they deployed. A `startupProbe` was added alongside (design 5.5): `init_db` now runs ALTERs and a full-table backfill before the port binds, which can outlast the liveness `initialDelaySeconds`.

### M7. The gate script had three silent holes and no test of its own `[details, design]` — Severity: High

* **Affected Scope:** `backend_rust/scripts/check_access_gate.sh`
* **Discrepancy Description:** (a) rule B was order-insensitive — a handler that reads by id and calls `require*` *afterwards* passed, though the guard then guards nothing; (b) rule C was satisfied by the string `AuthedUser` anywhere in the block, including a comment; (c) `#[options(…)]` and `#[route(…)]` attributes were not matched, so such a handler was invisible to all three rules. Reported line numbers were also wrong after the first handler in a file.
* ***AI Suggestion:*** PROPOSE FIX — **applied**
* ***Proposed Edit:*** rule B compares the byte position of the first `require` against the first id query and reports `LATE-GUARD`; rule C inspects only the signature up to `->`; the attribute pattern includes `options` and `route`; line numbers come from `FNR`. New `tests/access_gate_test.rs` runs the real script (must pass, must report 15 handlers) and runs it against a throwaway tree containing all three defects (must fail, with the right line numbers). A lint that fails by reporting success is the worst failure mode it can have.

### M8. The prefix-escape assertion was narrower than the risk `[details, design]` — Severity: Medium

* **Affected Scope:** `tests/route_coverage_test.rs`
* **Discrepancy Description:** The test checked only for a route resolving to exactly `/api`. A data route mounted at `/data` or `/apifoo` escapes the fairing just as thoroughly and would have passed.
* ***AI Suggestion:*** PROPOSE FIX — **applied**
* ***Proposed Edit:*** the assertion is inverted — every mounted route must live in a namespace that is public by decision (`/api/`, `/auth`, `/_auth`, `/api-docs`, `/cors/`, `/`), anything else is an escape. It immediately caught rocket_cors' own `/cors/<status>` route, which is now allowed with a stated reason.

### M9. The "malformed payload" test did not test a malformed payload `[details]` — Severity: Low

* ***AI Suggestion:*** PROPOSE FIX — **applied**
* ***Proposed Edit:*** `a_malformed_session_cookie_is_a_denial_not_a_500` feeds six genuinely malformed values (non-JSON, empty, an array, wrong field types, a truncated object) and asserts each is a `401`.

### M10. The preflight assertion was satisfiable by anything non-401 `[details]` — Severity: Low

* ***AI Suggestion:*** PROPOSE FIX — **applied**
* ***Proposed Edit:*** the preflight case now asserts a 2xx **and** the presence of `Access-Control-Allow-Methods` — the thing the browser actually sent the request for.

### M11. `/auth/login` did not apply the `Accept` branch the plan required `[readiness]` — Severity: Medium

* ***AI Suggestion:*** PROPOSE FIX — **applied**
* ***Proposed Edit:*** `login` takes `WantsHtml`; a JSON client that fails to reach the provider gets `401 {"error":"authentication_failed"}` instead of a `303` it will silently follow to an HTML page.

### M12. Five plan deviations were missing from record §7 `[details]` — Severity: Medium

* ***AI Suggestion:*** PROPOSE FIX — **applied**
* ***Proposed Edit:*** D-5 (`Secure`), D-6 (local-only logout), D-8 (state cookie cleared, not consumed), D-10 (no environment binding in the payload), D-17 (the `openidconnect` 4.1 API differs from the plan's sketch) added.

### M13. Logout is local-only; the provider session survives `[design]` — Severity: Medium

* **Affected Scope:** `src/auth/oidc.rs`, `src/auth/session.rs`
* **Discrepancy Description:** `post_logout_redirect_uri` is sent with no `id_token_hint`, and the ID token is deliberately not retained so it *cannot* be sent. At Authentik, RP-initiated logout needs the hint. After "logging out" on a shared browser, `/auth/login` returns the user straight in with no credentials.
* ***AI Suggestion:*** PROPOSE FIX (documentation) — **applied as a record change, code unchanged**
* ***Justification:*** Fixing it properly means retaining the ID token for the session's lifetime, which contradicts the design decision that JavaScript and storage never hold a token. That trade is the operator's to make, not a reviewer's, so the record now states plainly what the logout button means (§9.14, D-6).
* ***Proposed Edit:*** record §9.14 and D-6.

### M14. The session payload is bound to nothing but `ROCKET_SECRET_KEY` `[design]` — Severity: Medium

* **Affected Scope:** `src/auth/session.rs`, `k8s/secret.example.yaml`
* **Discrepancy Description:** No issuer, audience, or environment tag in the payload. If the same Secret is applied to two deployments, a cookie minted in staging authenticates in production as the same `uid`.
* ***AI Suggestion:*** ESCALATE — **recorded, not fixed**
* **Justification:** Whether the same `ROCKET_SECRET_KEY` is ever shared across environments is a fact about the operator's deployment, not about this repository — no file a reviewer can read settles it. The fix (a constant environment string in the payload) invalidates every live session, so it should be scheduled rather than slipped into an auth PR. Recorded as §9.17 with the fix named. **Needs the operator's answer: are per-environment secrets actually distinct today?**

### M15. The bootstrap handover is one-way and unverified `[design]` — Severity: Medium

* **Affected Scope:** `src/lib.rs` backfill, `src/auth/oidc.rs` handover
* **Discrepancy Description:** `backfill_ownership` assigns every orphan portfolio to the placeholder as soon as `BOOTSTRAP_OWNER_SUBJ` is merely *set* — never checked against the IdP — and the first matching login takes that row and everything it owns. A mistyped subject plus an unrelated first login means all pre-existing financial data belongs to the wrong person, and with no admin surface the recovery is hand-editing SQLite with the pod scaled to zero.
* ***AI Suggestion:*** ESCALATE — **partly mitigated, not fixed**
* **Justification:** The empty-sentinel fix (M5) removes the most likely way to trip it. A real fix is a recorded, reversible handover (previous `issuer`/`subj`, timestamp) or an admin surface — both beyond this PR's scope and both decisions the operator should make consciously. Recorded as §9.18.

### M16. `access.rs` asserted foreign keys are not enforced; another suite proves they are `[details, design]` — Severity: Low

* ***AI Suggestion:*** PROPOSE FIX — **applied**
* ***Proposed Edit:*** the comment now says the branch is defensive rather than reachable, names the test that pins FK-on behaviour, and explains why the test schema makes the case constructible there and nowhere else. Left in place rather than deleted: it costs nothing and a hand-edited database still reaches it.

### M17. Test-infrastructure contracts were undocumented `[readiness]` — Severity: Low

* ***AI Suggestion:*** PROPOSE FIX — **applied**
* ***Proposed Edit:*** record §6 now states what each suite actually proves, including the `lookup` exclusion and that the route sweeps derive from the router.

### M18. `route_coverage_test` was a count assertion, not an enforcement `[details]` — Severity: Low

* ***AI Suggestion:*** CONTEST (partly)
* **Justification:** The inventory test already asserts set equality in both directions (`missing` and `extra` are both asserted empty) and then pins the count at 15; the count is a tripwire for someone collapsing the table, not the mechanism. The genuine gap was the prefix-escape form, fixed in M8.

### M19. Unpaired denials in the isolation matrix `[details]` — Severity: Medium

* ***AI Suggestion:*** CONTEST
* **Justification:** Checked mechanically. Every route-denial test in `tests/isolation_test.rs` builds a second client as user A and asserts the owner's success on the same path — `b_cannot_list_or_delete_a_s_transactions` asserts `Ok` then `NoContent` for A on the very paths B was denied. Three tests have no owner-success client and each has a reason: `delete_of_a_foreign_empty_portfolio_is_404_not_409` is about the emptiness oracle, `a_nonexistent_portfolio_is_404_not_201_when_creating_an_asset` has no owner, and `two_users_sharing_an_email_cannot_reach_each_other_s_rows` is symmetric by construction.

### M20. The guard's `Err` path answers 500 with Rocket's HTML page `[design]` — Severity: Low

* ***AI Suggestion:*** DECLINE (recorded)
* **Justification:** Reachable only when the same lookup the fairing already performed fails between the fairing and the guard — a window of one query. The fairing now answers the realistic case with `503` JSON. A `#[catch(500)]` would also rewrite the body of every unrelated internal error, which is a wider change than the gap warrants.

### M21. `at_hash` is never checked; the client secret doubles as a token-signing key `[design]` — Severity: Low

* ***AI Suggestion:*** PROPOSE FIX (documentation) — **applied**
* **Justification:** `new_confidential_client` accepts an HMAC-signed ID token keyed by the client secret — spec-legal, and it means anyone who reads `OIDC_CLIENT_SECRET` can mint acceptable tokens. The code is left as is: that credential already authenticates the app to the provider, so the marginal exposure is small, and the access token is never used by this application, which is why `at_hash` has nothing to check against. What was wrong was the record's flat claim of "signature against the JWKS", which hid the second path.
* ***Proposed Edit:*** record §2.3 names both verification paths; §9.21 records the Secret-as-signing-key consequence and why `at_hash` is absent.

---

## Nits

| # | Finding | Reviewer | Disposition |
|---|---|---|---|
| N1 | `is_local_issuer` accepted `http://localhost.attacker.example` | details | **Fixed** — authority parsing, plus a unit test enumerating both sides. |
| N2 | Gate reported wrong line numbers after the first handler | details | **Fixed** — `FNR`, asserted by `access_gate_test`. |
| N3 | Gate attribute pattern missed `#[options(` / `#[route(` | details | **Fixed** — see M7. |
| N4 | `openapi.rs` used `starts_with("/api")` where the canonical rule is `/api/` | details | **Fixed** — aligned with the fairing's rule. |
| N5 | Record §5 listed the wrong limits of the gate | design | **Fixed** — §5 now lists the three that were real. |
| N6 | Record §2.3 said "signature against the JWKS" | design | **Fixed** — §2.3 now says what is verified, and M21 records the HMAC caveat. |
| N7 | Record claimed the state cookie is "consumed whatever happens next" | design | **Fixed** — D-8 and §9.15 state the truth: it is cleared by the response; the single-use guarantees are the code and the nonce. |
| N8 | Preflight exemption does not check the `Origin` against the allowlist | design | **Recorded** as §9.19. No data leaks; the exemption exists because `rocket_cors` answers preflights before authentication regardless. |
| N9 | No URI-form tests for the prefix rule (`%2d`, `//api/…`, case variants) | design | **Recorded** as §9.22. Defence in depth today: every handler also carries the guard. |
| N10 | No `startupProbe` while `init_db` migrates before binding | design | **Fixed** — see M6. |
| N11 | §7 gave no rejected-alternative for six deviations | design | **Declined** — the parent spec is where that rule properly applies; this is an implementation record. |

---

## What is still open, and what needs the operator

Nothing in `PROPOSE FIX` is unapplied. The open items are the ones that were
deliberately not fixed:

1. **M14 — are `ROCKET_SECRET_KEY` values actually distinct per environment?** If
   they are not, a staging cookie authenticates in production. This is the one finding
   whose severity depends on a fact outside the repository.
2. **M13 — is local-only logout acceptable to you?** If the app is ever used on a
   shared browser, "log out" currently does not end the provider session.
3. **M15 — the bootstrap handover has no undo.** Acceptable if you are the first
   person to log in after setting `BOOTSTRAP_OWNER_SUBJ`; a data-recovery job if you
   mistype it.
4. **§9.10 — there is still no test against a real provider.** Every defect in B1
   lived in code that has never executed against an IdP. A compose job with Keycloak
   or Authentik running login → `/auth/me` → `logout-all` → login is the highest-value
   missing test in this change, and it is not written.

## Gate status after these fixes

| Gate | Result |
|---|---|
| `cargo test` | 244 passed, 0 failed (was 235; +9 new) |
| `cargo clippy --all-targets` | no new warnings (1 pre-existing deprecation in a test) |
| `cargo build --release` | compiles |
| release binary refuses `AUTH_MODE=disabled` | exit 1, names the variable |
| release binary refuses missing OIDC config | exit 1, names all four |
| `check_access_gate.sh` | 15 inspected, 15 cleared |
| gate self-test | passes on the tree, fails on a bad fixture with correct line numbers |
| OpenAPI document | in sync with `--print-openapi` |
| `npm run lint` | 0 errors, 1 pre-existing warning |
| `npm test` | 150 passed |
| `tsc --noEmit` | clean |
