# Stage 7 — Critic Review (FeatureRange: lines 1-137)

Target: `docs/superpowers/specs/2026-11-04-oidc-identity-isolation-design.md`
Reviewers: `egm-design` (hostile design critic), `egm-structural` (structural validator), `egm-style` (copy editor).
Standard: `feature-spec-standard.md` for design and structural; style persona applied its own standard.

Raw per-reviewer reports: `stage7_design.md`, `stage7_structural.md`, `stage7_style.md`.

---

# Part 1 — egm-design (hostile design critic)

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
---

# Part 2 — egm-structural (structural validator)

## Findings

**1. Severity: Major**
**Affected Scope:** Whole FeatureRange (lines 1–137) vs. standard's mandatory *Alternatives Considered*
**Rationale:** The standard's Alternatives Rule is called mandatory, and a fresh reader navigating by headings finds no `## Alternatives Considered` anywhere in the document — the heading list goes Overview → Requirements → Non-Goals → Current State → Decisions Summary → Architecture. The *substance* is satisfied (the Decisions Summary table at lines 89–104 carries a per-decision "Rejected alternative" column with specific reasons), so this is a discoverability/naming failure, not a content gap. Fix: add `## Alternatives Considered` (or rename the section `## Decisions Summary / Alternatives Considered`) so the mandated section is greppable by title, and state in one line that it is the complete alternatives register for this spec.

**2. Severity: Major**
**Affected Scope:** Whole FeatureRange; standard's *Risks & Mitigations*
**Rationale:** No section in lines 1–137 is named for risks, and none signposts where risks live. The document discharges the obligation under two non-standard headings far outside the range — `## Security Considerations` (line 550) and `## Known Limitations` (line 588). A reviewer using the FeatureRange as a briefing never learns the risk register exists. Fix: add `## Risks & Mitigations` inside the FeatureRange (3–6 bullets: enumeration, chokepoint discipline, `Disabled` fail-open, bootstrap handover, shared third-party quota), each linking to the corresponding Security Considerations bullet.

**3. Severity: Major**
**Affected Scope:** `## Decisions Summary` (89–104) precedes `## Architecture` (106–136)
**Rationale:** The decisions table forward-references `users.token_version`, `portfolios.owner_id`, `portfolio_members`, the `access` module, and `BOOTSTRAP_OWNER_SUBJ` — none of which are defined until Architecture (line 132) or Backend Changes §4/§5. A first-time reader meets the verdicts before the vocabulary. Fix: move Architecture above Decisions Summary, or move the data-model row of the "Three units" table into a short preamble to Decisions Summary.

**4. Severity: Major**
**Affected Scope:** `## Architecture` diagram, line 118
**Rationale:** Verified width defect in the one figure the section rests on. The box spans 57 columns (top border line 115 = `┌` + 40 `─` + `┴` + 14 `─` + `┐`; bottom line 125 = 55 `─` between corners). Lines 116, 117, 119–124 each carry exactly 55 characters between the `│` borders; line 118 (`│   └─ FromRequest guard → AuthedUser …`) carries 56, so its right border is one column right of every other row — the equivalent of a broken table border. Fix: delete one space before the closing `│` on line 118.

**5. Severity: Major**
**Affected Scope:** `## Architecture` diagram, lines 120–123
**Rationale:** The annotation "the only path to a portfolio by id" is split across three lines (120, 121, 123) that are interleaved with two `▼` flow arrows belonging to the main vertical path (121, 123). Read top-to-bottom the sentence is broken by flow glyphs, and the `▼` column and annotation column visually collide. Fix: move the annotation outside the box as a single right-hand callout, or as a caption line under the figure.

**6. Severity: Major**
**Affected Scope:** `## Overview` line 16 and `## Decisions Summary` line 96 ("ten hand-written SQL statements" / "ten places to forget")
**Rationale:** "Ten" is load-bearing for both the Overview's threat argument and the rejection rationale for per-handler checks, yet the section explicitly titled *Current State (evidence)* (66–87) contains no line establishing it. The only counts the document evidences are 10 paths / 13 operations / 14 mounted API operations — so "ten" reads as a stale number contradicting the surface count the document later defends (§6, lines 375–383). Fix: cite `file:line` for each statement in the evidence table, or restate as "every handler".

**7. Severity: Major**
**Affected Scope:** `## Non-Goals` (52–64)
**Rationale:** Asymmetric entries. Three bullets carry a destination pointer ("— Spec 2", "— Spec 3", "Spec 4"); three do not (roles/admin panels/quotas, `is_active` sync, replacing SQLite / `replicas: 1`). The reader cannot tell "assigned to a later spec" from "permanently unowned", and the Deferred Work table (576–586) lists only Specs 2–4, leaving the latter three with no home. Fix: give every Non-Goal either a Spec pointer or an explicit "no planned spec" marker; cross-link the SQLite item to Known Limitations, where it is duplicated verbatim.

**8. Severity: Nit** — Front matter (3–5) uses loose `**Label**: value` lines while every other metadata structure in the document is a table; no table of contents despite 20 H2 sections and four intra-document anchors. Fix: three-column metadata table plus a short TOC.
**9. Severity: Nit** — `## Current State (evidence)` (66) embeds a parenthetical qualifier, unlike every other heading; heading naming is otherwise inconsistent about ampersands (`Configuration & Deployment`, `OpenAPI, CI & Frontend Impact` vs `Backend Changes`). Fix: `## Evidence: Current State`, and pick one conjunction style.
**10. Severity: Nit** — Requirement 7 (line 44) states the `/api-docs` *prefix* requires auth; §6 (line 373) delivers two enumerated routes (`/api-docs/openapi.json`, `/openapi.json`). Prefix-vs-enum is the same drift class the document elsewhere builds a test against, and `/openapi.json` is not evidenced in the Current State table. Fix: align wording and extend the "every mounted route is in the table" test to `/api-docs`.
**11. Severity: Nit** — Requirement 6 (line 41) says "the members table"; the table is named `portfolio_members` only at line 55. Use the real name on first use.
**12. Severity: Nit** — Requirements (28–50) are a flat 1–12 list mixing functional (1–7), operational (8–10) and forward-compat (11–12) concerns, with no stable IDs despite later references to "requirement 4" and "requirement 10", and no requirement→test traceability table. Fix: group under subheads, use `R1…R12`, add a traceability table.
**13. Severity: Nit** — Fence info strings are inconsistent: the Rust and SQL blocks are tagged, the Architecture fence (108–128) is bare. Also the bottom box border (125) has no `┴` where the Authentik connector (line 126) leaves the box, and line 116 uses an ASCII `|` for OR inside a box drawn with `│`.
**14. Severity: Nit** — Overview (10–12) asserts `utoipa_swagger_ui` is an unused dependency; this is the document's only evidence-free factual claim inside a section that otherwise cites `file:line`. Add it to the evidence table.

**No Blockers found.** Verified clean: heading hierarchy is strictly H1 → H2 with no skipped or out-of-order levels (1, 7, 28, 52, 66, 89, 106); all three FeatureRange tables are well-formed with header/separator/cell counts matching (2, 3, 3 columns); all four FeatureRange anchors resolve (`#deferred-work`, `#security-considerations`, plus `#configuration--deployment` and `#rollout--rollback` used later); all code fences are closed; and all twelve Requirements are genuinely delivered in the DetailedRange (§1–§8, Configuration, Testing) — no orphaned promises.

## Not assessed
- Line-level correctness of the DetailedRange (138–614) beyond the promise-vs-delivery check above.
- Factual accuracy of cited `file:line` references (read-only structural pass; no source files opened).
- Semantic security review of the design (out of Structural role).
---

# Part 3 — egm-style (copy editor)

## Findings

**1. Blocker — Requirements 5–6 (lines 38–42) and Overview (line 14)**
> "A user cannot read or mutate a portfolio, asset, or transaction they do not own. The response for any non-owned resource id is `404`, never `403`." / "no user can read or mutate another user's portfolios"

Requirement 6 and the Access table (§5) grant `Editor`/`Viewer` read access to a portfolio the caller does **not** own. As written, Requirement 5 forbids exactly that, and "non-owned" is the operative term in both. An implementer can build the wrong rule from Requirement 5 alone.
Rewrite: "A user cannot read or mutate a portfolio, asset, or transaction outside their granted access level (owner, editor, or viewer). The response for any resource id outside the caller's access is `404`, never `403`." Overview: "so that no user can read or mutate another user's data without an explicit grant."

**2. Major — Overview (lines 19–20)**
> "ownership checks scattered through ten hand-written SQL statements"

Unverified count, and it collides with the 13/14-operation counts in the evidence table two paragraphs later. The actual count of `WHERE id = ?` statements against `portfolios`/`assets`/`transactions` is 12 (`analytics.rs:26,112`; `portfolios.rs:158,196,204,236,257`; `transactions.rs:103,112,146,174,283`). Same unsupported "ten" at line 99 ("ten places to forget").
Rewrite: "twelve hand-written statements across `analytics.rs`, `portfolios.rs`, `transactions.rs` select these rows by caller-supplied id."

**3. Major — Overview (line 16)**
> "The authentication half is comparatively small. The isolation half is the substance of this spec"

"Comparatively small" is unmeasurable (small in code? risk? weeks?), and "half … half" asserts a 50/50 split the next sentence denies.
Rewrite: "Authentication is a bounded integration against a running identity provider. Isolation is the design work, and the majority of the code change."

**4. Major — Overview (lines 19–22)**
> "a leak waiting to happen" … "makes the cross-user test matrix the primary acceptance gate"

Idiomatic rhetoric; and "primary" implies a secondary gate that is never named (Testing Strategy lists four undifferentiated gates). Also "makes … and makes" repetition.
Rewrite: "…so an omitted ownership check is invisible in normal use. The design therefore routes every id-based lookup through one module and treats the cross-user test matrix as the acceptance criterion for isolation."

**5. Major — Overview (line 25)**
> "stores a shadow record per identity and nothing else"

"Shadow record" is undefined shorthand at first use, and "nothing else" is literally false — the application stores portfolios, assets, and transactions.
Rewrite: "stores one local `users` row per identity and no other identity data."

**6. Major — Overview (lines 12–13) and Architecture (line 130)**
> "**existing, already operated Authentik instance**" … "(out of cluster, already running)"

Synonym redundancy: *existing* = *already operated*. Also "**hard per-user data isolation**" (line 14) — "hard" is an intensifier with no defined meaning.
Rewrite: "an Authentik instance the operator already runs"; drop "hard".

**7. Major — Overview (lines 10–12)**
> "(No Swagger UI is mounted today — `utoipa_swagger_ui` is a dependency but is never used in `build_rocket` — but the document alone enumerates every route and payload.)"

A parenthetical carrying two clauses and a second adversative "but" inside a sentence that already has one. Misparse risk. "Payload" is imprecise.
Rewrite as a separate sentence: "No Swagger UI is mounted: `utoipa_swagger_ui` is an unused dependency. The document alone still enumerates every route and its request and response schemas."

**8. Major — Requirements 3 (line 34) vs schema and Config**
> "keyed on `(issuer, subject)`"

Three spellings for one concept across the document: `subject` (line 34), `subj` (column, lines 237, 282), `sub` (ID-token claim, lines 291, 419).
Rewrite: define once at first use — "the ID-token `sub` claim, stored as `users.subj`" — then use `sub` for the claim and `subj` for the column.

**9. Major — Requirement 7 (line 43)**
> "`/api-docs` requires authentication."

`/api-docs` is not a route; §6 protects `/api-docs/openapi.json` and `/openapi.json`. A prefix-scoped requirement reads as already-satisfied by a non-existent path.
Rewrite: "`GET /api-docs/openapi.json` and `GET /openapi.json` require authentication."

**10. Major — Non-Goals (lines 58–59)**
> "The auth guard **is** shaped to accept a bearer token here; no key is issued yet."

Scope ambiguity: it does not say whether the guard accepts, rejects, or ignores an `Authorization: Bearer` header in this spec. That is precisely the kind of line that seeds scope creep into Spec 3.
Rewrite: "The guard resolves credentials through one function so a bearer source can be added in Spec 3. In this spec a bearer header is rejected with `401`; no key is issued or accepted."

**11. Major — Non-Goals (line 61)**
> "Roles, admin panels, per-user quotas beyond the lookup rate limit."

"Roles" means two things in this document: the out-of-scope admin roles, and `portfolio_members.role` (`editor`/`viewer`), which **is** created and enforced here.
Rewrite: "Administrator or staff roles. The `editor`/`viewer` values in `portfolio_members` are in scope and are not affected by this exclusion."

**12. Major — Current State (line 76)**
> "including a crash-safe column rebuild helper"

A quality claim about existing code with no cited mechanism; §4 supplies the mechanism later, making the adjective here redundant.
Rewrite: "including a column rebuild helper (`migrate_one_column`)".

**13. Major — Current State (lines 85–87)**
> "Same-origin in both prod and dev is load-bearing for this design: it means the browser never handles a token"

"Load-bearing" is metaphor; "it means" is filler; and the causal claim is overstated — same-origin permits a cookie design, the cookie choice is what keeps tokens out of the browser.
Rewrite: "Both deployments are same-origin, so the backend can authenticate with a cookie. The browser never stores or sends a token: no token storage, no refresh logic, no new frontend auth dependency."

**14. Major — Decisions Summary (line 102)**
> "the forgotten variable the check exists to catch is the check's own default"

Garden-path sentence; requires re-reading. "Literally" is filler.
Rewrite: "Rocket 0.5 has no `ROCKET_ENV`. With the variable unset the profile resolves to `release`, so a `!= production` check fails open in exactly the state it was written to catch."

**15. Major — Decisions Summary (lines 96–104)**
> "the thing not to hand-roll" · "needless new stateful service" · "for no gain" · "free enumeration oracle" · "a shared failure" · "Authentik-for-this-app"

Wit and bare assertions in place of reasons. Rewrites: "requires password storage, reset flows, and a password policy — all out of scope"; "adds a second stateful service to operate"; "a session table buys immediate revocation, which `token_version` already provides"; "`403` confirms the id exists, letting an attacker enumerate valid ids"; "one user can exhaust the quota for every user"; "a second, dedicated Authentik deployment".

**16. Major — Decisions Summary (line 103)**
> "Matching the ID token's bare `email`"

"Bare" is not the hazard; unverified is, and §4 uses "unverified". Use "an `email` claim not gated by `email_verified`".

**17. Major — Architecture (lines 106–130, 132)**
Diagram shows `/auth/*` proxied by nginx, which Current State (line 79) says does not exist today; the diagram is unlabeled as target state. Line 132: "Three units, each independently understandable" is a subjective quality judgment.
Rewrite: title the diagram "Target state after this spec"; replace line 132 with "Three units, with dependencies in one direction:".

**18. Nit — scattered**
- Line 8 "no authentication of any kind", line 9 "reachable by anyone who can reach the host", line 9 "the complete OpenAPI document": drop "of any kind", "complete"; rewrite as "reachable by any client that can reach the host".
- Line 25–26 "**There is no user-management UI in this application.**" duplicates Non-Goals "admin panels"; keep one.
- Line 54 "Explicitly out of scope": drop "Explicitly".
- Line 36 "during migration" vs §4 "On startup": pick "at startup".
- Line 46 "pass without an OIDC provider reachable": "without a reachable OIDC provider".
- Line 47 "the final access model": "the access model planned for Spec 2".
- Lines 31, 96, 60: `PKCE`, `JIT`, `MCP` never expanded; expand at first use (`MCP` is scope-defining).
- Line 72 "mounted surface … under-reports the surface": use "route set" once.
- Line 71 "the published spec": say "the committed OpenAPI document".
- Line 73: "the file is lowercase `configmap.yaml`" is a naming note unrelated to profile selection; move to Configuration.
- Line 83 "~14 raw `fetch` calls": give an exact count or drop the tilde; "raw" → "direct".
- Line 84 "already needed 429 handling": "required 429 handling (added in `2753049`)".
- Line 91 "Decisions Summary" → "Summary of Decisions"; rename third column "Rejected alternative — why".
- Line 93 "dev diverges from prod", line 94 "no benefit given same-origin": restore full clauses; also standardize "prod/dev" → "production/development".
- Line 118–120: the "◀── the only path to a portfolio by id" annotation wrapped across three diagram lines; move it to a caption under the box.
- Line 134 "→ `Access`, or nothing": "→ `Access`, or `None` (callers return `404`)".
- Line 135 "never resolves ownership themselves": "must not resolve ownership" (normative, and the plural pronon has no plural antecedent).
- Requirement 8 is a constraint on Requirement 1, not a functional requirement; mark it as such.
- "Dependency" is used for both npm/crate dependencies and architectural "Depends on"; disambiguate.

## Not assessed
- Lines 138 to end (Backend Changes, Configuration & Deployment, OpenAPI/CI, Testing, Rollout, Security, Deferred Work, Known Limitations, Open Questions) — read for orientation only.
- British/American spelling mix ("behaviour", "defence" at lines 535, 556) — outside FeatureRange, noted for the orchestrator.
- Technical correctness of the OIDC, Rocket, and SQLite claims beyond the wording issues cited; no build or test was run.
- Whether "14 API operations" and "10 paths / 13 operations" match the current router and committed spec (not re-derived from source).