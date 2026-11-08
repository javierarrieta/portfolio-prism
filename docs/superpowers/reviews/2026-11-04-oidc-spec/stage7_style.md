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