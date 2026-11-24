# Consolidated Review Report — PR #77, the MCP agent interface (implementation)

* **Target:** PR [#77](https://github.com/javierarrieta/portfolio-prism/pull/77) `feat/staged-batches` → `main`, head `df258e48393935330339f7ef2cb5de76d7fdcff5`, merge-base `4ab230b`. 23 commits, 81 changed files, +11225 / −318.
* **Tree reviewed:** `/home/coder/wt-pp-staging` at `df258e4`, clean. The tree was authoritative; the diff was a pointer.
* **Date:** 2026-11-24.
* **Method:** EGM / Goldfish adversarial loop — read-only reviewers, fresh context each, orchestrator adjudicates. Plus a mechanical **drift analysis**: every committed generated artifact, gate, migration and verification claim re-executed against the tree rather than read.
* **Compiled from:** `lane_implementation_readiness.md` (IR-*), `lane_correctness.md` (B/M/N-*), `lane_comprehension.md` (C-*), `lane_zoom_out.md` (Z-*), `lane_design_security.md` (SEC-*), `lane_codebase_details2.md` (C-* citations), and the orchestrator's own measurements (D-*, V-*). The `egm-design` and `egm-codebase-details` lanes timed out in the first round at 30 minutes each and were relaunched with bounded briefs; both landed, and their findings are integrated below.
* **Verification marker:** every finding below carries **[verified]** where the orchestrator re-derived it from the tree or a command, and **[lane]** where it rests on a reviewer's read that was not independently re-checked.

## 1. Bottom line

The architecture survives attack, and the security lane could not break the credential split either: no path was found by which a token writes portfolio data without a human session, all eleven tools gate on scope before any DB work, apply is single-use under concurrent approvers because of a `FOR UPDATE` on the batch row, and the expiry predicates agree across `approve`/`apply`/`stage`. Nothing here argues against merging the design.

What stops this PR is that **the shipped artifact does not match the document describing it, in nine places, and the two most load-bearing of those are not wording**. `/mcp` is unreachable in the topology this repo deploys — `frontend/nginx.conf` is not in the 81 files, so the headline feature ships dark and no step in the body turns it on. The enable gate and the rollback lever the spec documents (`AGENT_MCP_MOUNT`) **do not exist in the code**; the mount is unconditional. Both directions of that error matter: the operator has no switch, and the reviewer who trusts the body believes the feature is opt-in.

The second cluster is the body's own verification section, which is stale against its own head and, in three cases, says the opposite of what was measured: the test counts are lower than the tree produces, `cargo clippy` does warn in files this PR adds, and the `0005` non-atomicity warning describes a window that cannot occur — proven, not argued. The third cluster is comments that teach a false invariant: `pat.rs` claims the database clock decides expiry and the code uses the app clock; `mount.rs` claims mounting under `/api` would silently hand every tool to a session cookie when its own handler requires a bearer unconditionally; `currency_service.rs` describes a read-based dedupe check that `0005` deleted; `detail.rs` still lists a producer that `2692a69` deleted. Each of those is a reader's future wrong premise, which is the same class of defect `2692a69` deleted a detector for.

The fourth cluster is the one the security lane opened, and it is the most uncomfortable, because none of it is a bypass: **the approval gate approves counts, not money** (the preview shows "IWDA: 500 transactions" and never a price, so the human's consent is over a shape rather than an amount); **the one test guarding the keystone property cannot fail** (`mcp_refuses_a_session_cookie` sends no cookie and runs in `AuthMode::Disabled`, the one mode that never reads a cookie — three lanes found this independently); **`/mcp` has no timeout and no concurrency bound** while sharing the human's runtime and a 5-connection pool that `apply` can hold for ~600 statements; and **there is no kill switch for a leaked token** — `logout-all` deliberately spares tokens, and the default mint never expires. None of these is a hole in the design as documented. They are the gap between the design's promises and what the code enforces.

Two findings are the same mistake twice inside one commit: `2692a69` deleted `owned_detail`'s duplicate-symbol detector because "a detector that cannot fire is a reader's false premise", and left the bundle-side one — which is also unreachable, because `parse_and_validate` refuses the bundle before `plan_for` ever renders it. The surviving one is also case-sensitive while the validator it exists to pre-announce is case-insensitive, so it would miss the case the validator's own comment calls the real one.

**Recommendation: do not merge as-is.** The blockers are one commit each (nginx location, delete or build the enable gate, correct the body, delete the dead flag, repoint the `AGENTS.md` citation) and none of them touch the agent surface's logic. The majors are a comment pass plus six real fixes: the advisory lock in `apply`, the cookie test, the preview's money columns, a deadline and a semaphore on `/mcp`, a default token expiry, and `src/agent` inside the access gate. The drift analysis found the committed artifacts clean: `openapi.json`, `api.d.ts` and `.sqlx/` are all byte-exactly what regeneration produces, and the security lane confirmed the credential model holds under four specific attacks, so the mechanical and architectural halves are in order even though the narrative half is not.

## 2. Reviewer status

| Lane | Agent | IDs | Status |
| --- | --- | --- | --- |
| Merge & deploy readiness | `egm-implementation-readiness` | IR-1 … IR-14 | completed |
| Code correctness | builtin `reviewer` | B1–B2, M1–M6, N1–N3 | completed |
| Naive-reader comprehension | `egm-comprehension` | C-1 … C-14 | completed (cut off mid-list; frontend and `openapi.rs` halves not reached) |
| YAGNI / reuse | `egm-zoom-out` | Z1 … Z7 | completed |
| Hostile design critic | `egm-design` | — | first run **failed** (30-min timeout, 980 KB transcript, no report) |
| Citation audit | `egm-codebase-details` | — | first run **failed** — same |
| Security & runtime isolation | `egm-design` (re-run) | SEC-1 … SEC-9 | completed — 0 blockers, 6 majors, 3 nits |
| Citation audit | `egm-codebase-details` (re-run) | C1 … C6 | completed — 1 blocker, 3 majors, 2 nits |

The re-runs were scoped narrowly (four questions, ≤35 file reads, a list of already-established facts not to re-verify) after the open-ended first pair timed out. That is why they landed: the first pair burned its budget re-deriving the credential model from scratch.
| Structural / copy axes | `egm-structural`, `egm-style` | — | **deliberately skipped** — this is an implementation PR; the prose under review is the body and the module docs, which the comprehension lane audited. A copy pass would re-derive the same list at the same recall. |

No lane was dropped for convenience. The first round's two failures are recorded as failures, not as zero-finding lenses, and the re-run closed the gap it left: the runtime-isolation questions (a hung agent connection, pool contention against `db.rs` `max_connections(5)`, what a panicking tool does to the interactive app) are now examined, in SEC-2 and SEC-3.

## 3. Scorecard

| Source | Blocker | Major | Nit |
| --- | --- | --- | --- |
| readiness (IR) | 3 | 6 | 5 |
| correctness (B/M/N) | 2 | 6 | 3 |
| comprehension (C) | 3 | 4 | 7 |
| zoom-out (Z) | 1 | 2 | 4 |
| security (SEC) | 0 | 6 | 3 |
| citations (C1–C6) | 1 | 3 | 2 |
| orchestrator drift (D/V) | — | 3 | 2 |
| **Raw** | **10** | **30** | **26** |
| **After dedup** | **9** | **25** | **20** |

Dedup: Z1 = C-2 = D3 (one finding, three angles). IR-8 = C-3 = D1 (one finding). IR-10/IR-11 fold into one audit-script finding. B1 and M6 stay separate — one is reachability, the other is the case the survivor misses. **SEC-1 = MA-2** (the citations and security lanes independently reached the same conclusion as the correctness lane by three different routes; the security lane's mechanism is the sharpest and is what MA-2 now carries). **C3 = MA-1** (independently derived; it adds that `api_routes/portfolios.rs:30` has no cap at all).

## 4. Blockers

### BL-1 — `/mcp` is unreachable in the deployed topology **[verified]**
* **Scope:** `frontend/nginx.conf:30-34,53,65`, `frontend/Dockerfile:26`, `k8s/app-service.yaml:14-16`, `k8s/app-deployment.yaml:26-28,57-59`, `k8s/ingress.yaml:11-20`
* **Discrepancy:** ingress → `app-service:80` → the **frontend nginx** container. nginx proxies only `/api`, `/api-docs` and `/auth` to `127.0.0.1:8000`; everything else hits `location / { try_files $uri $uri/ /index.html; }`. `POST /mcp` never reaches Rocket.
* **Evidence:** `git diff --name-only origin/main...HEAD | grep -c nginx` → `0`. `frontend/Dockerfile:26` bakes `nginx.conf` in as the default site. The spec names the missing piece: `…design.md:918-920` prescribes `location /mcp { proxy_pass http://127.0.0.1:8000; }`.
* **Why it matters:** the feature the PR is named for is dark on merge, with no step to enable it and no signal that it is off.
* **Disposition: PROPOSE FIX** — add the `location /mcp` block (with `proxy_buffering off` if SSE is ever wanted) in this PR, or state in the body that the surface is deliberately unexposed and record the enabling step.

### BL-2 — the documented enable gate and kill switch do not exist **[verified]**
* **Scope:** `docs/superpowers/specs/2026-11-18-mcp-agent-interface-design.md:475,915-916,987` vs `backend_rust/src/lib.rs:366`, `backend_rust/src/agent/staging.rs:49`
* **Discrepancy:** the spec says the route is mounted only when `AGENT_MCP_MOUNT` is set, that `AGENT_BATCH_TTL_MINUTES` configures the TTL, and that rollback is "unset `AGENT_MCP_MOUNT` and roll". Neither variable appears anywhere in `*.rs`, `*.yaml` or `*.sh` — only in docs. `lib.rs:366` mounts unconditionally (contrast the real OIDC gate at `lib.rs:273`); the TTL is `pub const DEFAULT_TTL_MINUTES: i64 = 60`.
* **Why it matters:** on-by-default plus no lever is the inverse of what the reviewer and the operator both believe. An operator who sets the variable gets silence.
* **Disposition: PROPOSE FIX** — either implement the gate (a `from_env` read beside the OIDC one, with the same testability convention as `BackupConfig::from_env_with`), or amend the spec and the body to say the surface is always on. Do not leave a documented lever that is not wired.

### BL-3 — the body's Postgres prerequisite was executed nine days before this head **[verified]**
* **Scope:** PR body §"Not in this PR"; `docs/postgres-cutover.md:71-89`; `k8s/configmap.yaml:7-26`; `AGENTS.md`
* **Discrepancy:** the body instructs that the cluster side "must be applied **before** this image rolls". `docs/postgres-cutover.md:71-89` is an *Execution record — 2026-11-15*: step 4 deploy **Executed**, CNPG `Database` + `DatabaseRole` in k8s-titan #296, image roll and inline `DATABASE_URL` removal in #297, `Recreate` one replica. The live database is already Postgres.
* **Why it matters:** an operator blocks on a prerequisite that landed, or trusts `configmap.yaml`'s justification ("deleting this key breaks the live SQLite deployment") which now describes a deployment that does not exist. The repo's `k8s/` is no longer the deployment source of truth and nothing in the repo says so.
* **Disposition: PROPOSE FIX** — restate the prerequisite as done, with the k8s-titan PR numbers, and mark `k8s/configmap.yaml`'s `sqlite:` DSN as an artifact of a retired topology rather than a live constraint.

### BL-4 — the surviving `duplicate_symbol` detector cannot fire, and the body's reason for keeping it is false **[verified]**
* **Scope:** `backend_rust/src/api_routes/backup.rs:417,421`, `src/backup/restore.rs:82`, `src/backup/mod.rs:433`, `src/backup/detail.rs:207-221`
* **Discrepancy:** the preview route runs `parse_and_validate` (which refuses a case-insensitive duplicate symbol per portfolio at `mod.rs:433`) **before** `plan_for`, and `plan_for` is the only caller of `detail_from_bundle`. A bundle that would set the flag is refused before any plan is built, so the flag can never reach a client. `detail.rs:207-213` and the body both assert the opposite: "the flag is what tells a human, in the preview, why their bundle is about to be refused by `validate`".
* **Why it matters:** this is precisely the reasoning `2692a69` used to delete `owned_detail`'s half — "a detector that cannot fire is a reader's false premise". The same commit kept this one on a claim that is checkably false.
* **Disposition: PROPOSE FIX** — delete the bundle-side detector too, or move `detail_from_bundle` ahead of validation and render both. Operator's call, because reordering changes the preview route's contract (today a bad bundle is a 422 with paths, never a plan).

### BL-5 — the body's `0005` non-atomicity warning describes a window that cannot occur **[verified]**
* **Scope:** PR body §Migrations; `backend_rust/migrations/0005_split_index_case_insensitive.sql:96-105`; `0006_asset_symbol_unique.sql:100-112`
* **Discrepancy:** the body says the `DROP`-then-`CREATE` "is not atomic", is tolerable only because `replicas: 1` + `Recreate`, and that "at two replicas that window is a real race". sqlx wraps every migration in a transaction unless the file's **content** starts with `-- no-transaction` (`sqlx-core-0.8.6/src/migrate/source.rs:127`), and `sqlx::migrate!` uses that same resolver (`sqlx-macros-core-0.8.6/src/migrate.rs:106-108`). No migration in `migrations/` carries the marker.
* **Evidence:** measured. With `uq_assets_portfolio_symbol` dropped inside an open transaction holding `pg_sleep(6)`, a concurrent `INSERT` of the colliding symbol was still blocked after 4005 ms (`rc=124`) and only proceeded once the drop committed. `DROP INDEX` takes ACCESS EXCLUSIVE on the table; there is no observable window at any replica count.
* **Why it matters:** a false safety caveat is worse than none — it teaches that the migration is fragile under scale-out, which will be cited later to justify not scaling out, and it hides that the real constraint is the ACCESS EXCLUSIVE lock's duration.
* **Disposition: PROPOSE FIX** — correct the text in `0005`, `0006` and the body. If hand-running migrations in `psql` without `BEGIN` is a supported path, say that; otherwise delete the hedge.

### BL-6 — "`AuthFairing` accepts either [token or session]" is false **[verified]**
* **Scope:** PR body §WS1; `backend_rust/src/auth/guard.rs:126-163,232-234`; `src/agent/tools.rs:29-33`
* **Discrepancy:** `resolve_caller` reads only the session cookie; `guard.rs:232-234` states "An `Authorization` header is never consulted, only ignored", and `tools.rs:29-33` states a token "cannot reach `/api` at all" — which is exactly what makes the token-minting routes safe to mount under `/api`.
* **Why it matters:** the sentence inverts the property the design depends on. A reviewer who believes it concludes a scoped token can call `/api`, and reasons about the scope model from a false premise.
* **Disposition: PROPOSE FIX** — rewrite the sentence to say a token is accepted at `/mcp` and nowhere else, and keep the reason attached.

### BL-7 — "Every tool calls the same function the corresponding `/api` route calls" is false, and the module doc says so **[verified]**
* **Scope:** PR body §"The eleven tools"; `backend_rust/src/agent/tools.rs:8-16,166,217,398,426,576-607,653-690`
* **Discrepancy:** the body states the rule flatly. `tools.rs:8-16` — in this same PR — states it narrowly and names the exception: `list_portfolios`, `get_portfolio` and `list_transactions` "inline their own `SELECT`, because the `/api` equivalents have no function to call", and warns that "a change to `api_routes::transactions::list_portfolio_transactions` will not reach `list_transactions`". Measured across the file: `tools.rs` contains **zero** compile-time `sqlx::query_as!` macros; the route counterparts are all macros. `get_performance` and `get_transaction_pnl` share the *engine* but re-implement the three-query load.
* **Concrete divergence already present:** `GET /api/portfolios/<pid>/transactions` (`api_routes/transactions.rs:214`) has no pagination and sorts by date only (stable, so same-day ties stay asset-grouped); the tool paginates and sorts `date DESC, id DESC`. Two different answers to "the last N transactions".
* **Why it matters:** the compile-time asymmetry makes it worse than duplication — a schema change is a compile error on the `/api` side and a runtime failure on the tool side. The body's line ("a rule enforced only in a tool is a rule an `/api` caller need not follow") is the right rule; the body then claims it holds.
* **Disposition: PROPOSE FIX** — extract the three loaders (adopt the tool's single-JOIN query; it is the better one) or reproduce the module doc's narrower claim in the body. A claim the code contradicts three lines below is not a rule, it is a TODO.

### BL-8 — the new `AGENTS.md` bullet cites the wrong lines for the constraint it documents **[verified]**
* **Scope:** `AGENTS.md` (new bullet, `diff.patch:11`), `backend_rust/src/backup/mod.rs:431-434`, `:454`
* **Discrepancy:** the bullet states `asset_type` "is constrained at the bundle boundary (`backup/mod.rs:431-434`)". Lines 431-434 are the empty-symbol check and the opening of the duplicate-symbol branch. The `asset_type` check is `backup/mod.rs:454` (`if !KNOWN_ASSET_TYPES.contains(&a.asset_type.as_str())`). The companion citation, `docs/backup-schema/v1.json:6-15`, is correct.
* **Why it matters:** `AGENTS.md` is the file the next agent trusts without opening it, and the prior review in this series logged eight wrong citations, all in the direction of making work look smaller. A one-line-off pointer in the operating manual is the cheapest defect to fix and the most expensive to leave.
* **Disposition: PROPOSE FIX** — repoint to `:452-455`.

### BL-9 — the body describes a different tree than the one under review **[verified]**
* **Scope:** PR body header and §"Review round"; `commits.txt`
* **Discrepancy:** body says "21 commits, 79 files, +10718/−312" and "the last three commits are a review round" naming `250ccdf · e4bde8c · 2692a69`. Head is `df258e4`: 23 commits, 81 files, +11225/−318. `37bcdd7` ("a body cap that was never wired, and an approval that could never apply") and `df258e4` ("the manual transaction form canonicalizes money the way the importer does") appear nowhere in the body, and both change behaviour — `37bcdd7` is what makes `mount.rs:74 MAX_BODY_BYTES` real, and `df258e4` is half of the frontend side the deploy-order instruction depends on.
* **Disposition: PROPOSE FIX** — refresh the header and the review-round section before merge.

## 5. Majors

| # | Finding | Scope | Status |
| --- | --- | --- | --- |
| MA-1 | The `MAX_PORTFOLIOS_PER_USER` check inside `apply` does not serialise concurrent applies, and the comment says it does. The flip locks the **batch** row; two batches are two rows. `SELECT count(*) FROM portfolios WHERE owner_id = $1` takes no lock, and `pg_advisory_xact_lock(user_id)` is taken only in `stage` (`staging.rs:597`), never in `apply` — it is the only advisory lock in the tree. Two approved `Target::New` batches both count 9 and both insert. Independently derived by the citations lane, which adds that **`api_routes/portfolios.rs:30` inserts portfolios with no cap at all**, so the backstop binds only the agent path. Fix: take the advisory lock in `apply`, or correct the comment to say it serialises re-application of the *same* batch. | `agent/staging.rs:1156-1159`, `:981-988`, `:597`; `api_routes/portfolios.rs:30` | **[verified ×2 lanes]** |
| MA-2 | **The highest-priority test fix in the PR.** `mcp_refuses_a_session_cookie` never sends a cookie *and never runs in the only mode that reads one*. It calls `client("mcp_cookie")` → `common::build_rocket` → `AuthMode::Disabled` (`tests/common/mod.rs:115`), and `rpc(…, None)` sends no cookie and no bearer — byte-identical to `mcp_refuses_a_request_with_no_credential`. `session::read` is consulted only in the `AuthMode::Oidc` arm of `resolve_caller` (`auth/guard.rs:150-160`), so **a regression that made `/mcp` honour `pp_session` in production would leave this test green**. `mount.rs:18-22` claims both cases are tested and calls the cookie case "the one that matters"; `route_coverage_test.rs:217-222` names it as the guard on `SELF_AUTHENTICATED`. Fix: build with the existing `build_rocket_oidc` (`tests/common/mod.rs:138`), attach a signed `pp_session`, assert 401. | `tests/mcp_mount_test.rs:36-47,111-130`; `auth/guard.rs:150-160` | **[verified ×3 lanes]** |
| MA-3 | The surviving detector is case-sensitive (`a.symbol.as_str()`) while the validator it pre-announces is case-insensitive (`to_uppercase()`, with a comment saying upper-casing is deliberate because the index is `upper(symbol)`). `MSFT` beside `msft` is refused by the validator and would not be flagged. | `backup/detail.rs:214-220` vs `backup/mod.rs:433-445` | **[verified]** |
| MA-4 | `pat.rs` documents that expiry is decided by the database clock ("`lookup` … compares against `now()` in the database … One clock, one answer") and the code compares `exp <= Utc::now()` — the app clock. Only the list query uses DB `now()`. | `auth/pat.rs:115-121` vs `:314` | **[verified]** |
| MA-5 | `mount.rs`'s failure story is wrong: it says mounting `/mcp` under `/api` means "every tool would run as whoever holds a session cookie", but the handler requires a bearer unconditionally (`parts.bearer.ok_or(Status::Unauthorized)?`), so that mount would 401 loudly, not grant silently. The doc teaches that the prefix, not the route's own check, is the security boundary — contradicting its own next paragraph. | `agent/mount.rs:12-18` vs `:186` | **[verified]** |
| MA-6 | `apply_split_events`' doc still describes a read-based check ("a concurrent writer cannot slip a second row in between a check and this insert") that `0005` says was deleted ("has no read-based check of its own"). The index is the only dedupe, and the comment points a reader at a check that is gone. | `services/currency_service.rs:109-112` vs `migrations/0005` | **[verified]** |
| MA-7 | Rollback after `0006` is not clean and is undocumented. `main` looked assets up case-sensitively (`api_routes/transactions.rs:35`: `AND symbol = $2`); the branch uses `upper(symbol) = $2` (`services/assets.rs:59`). Roll the image back after `0006` applied and a row stored as `auditdup` is invisible to the old lookup, so the old binary's `INSERT` dies on a raw index violation (500) where it used to write a silent duplicate. The audit script's "N rows in a spelling the write path never produces" is exactly that count. No rollback section exists. | `services/assets.rs:59` vs `origin/main:api_routes/transactions.rs:35` | **[verified]** |
| MA-8 | The pre-flight audit has no execution path. `audit_asset_symbols.sh:32-40` needs `DATABASE_URL` and `psql`; the backend runtime image is `debian:bookworm-slim` with `ca-certificates` only, so it cannot run in the pod. The live DSN is in the `backend-secrets` Secret. Neither the body nor `AGENTS.md` says where to run it or how to get the URL. | `backend_rust/Dockerfile:60-67`, `k8s/configmap.yaml:22-26` | **[verified]** |
| MA-9 | The audit step is in no runbook. `grep audit_asset_symbols docs/postgres-cutover.md` → 0 matches; `audit_transaction_types.sh` is referenced by no doc either, while `0002`–`0006` all have raise-on-live-data shapes and `sqlx::migrate!` runs at boot. | `docs/postgres-cutover.md` | **[verified]** |
| MA-10 | "The frontend image must roll before the backend image" is not executable as committed. Images publish only from `release.yml` on `v*` tags; one run pushes `:latest` for both simultaneously; `app-deployment.yaml:26` pins the frontend to `:latest` while the backend is a digest (`:57`), and no `spec.strategy` is declared. The procedure that achieves the ordering is written nowhere. | `.github/workflows/release.yml:3-6`, `k8s/app-deployment.yaml:26,57` | **[verified]** |
| MA-11 | `parseDecimal` has no scale or precision cap, contradicting its own stated rationale ("a cell that parses here and is then refused there costs the user a failed import row"). `DECIMAL_ONLY` is `^-?\d+(\.\d+)?$`; a 16-dp cell passes the frontend and is refused by `is_canonical_decimal` (`MAX_DECIMAL_SCALE = 12`). Mirror both backend constants. | `frontend/src/utils/csvValidator.ts:206-217,301` vs `db_types.rs:110-114` | **[lane]** |
| MA-12 | `read_tools_answer_not_found_for_someone_elses_rows` asserts `matches!(code, Value::String(_))`, not the code. Delete `require_transaction` from `get_transaction_pnl` and the tool returns `refused("pricing", …)` — still a `Value::String` — so the test passes on a stranger's id. Assert the exact codes, as `mcp_mount_test.rs:345-349` already does. | `tests/mcp_mount_test.rs:585` | **[lane]** |
| MA-13 | The two race claims the body calls load-bearing have no concurrency test, while the repo already has the pattern (`split_sync_endpoint_test.rs:173-195`, `restore_concurrency_test.rs:69-71`). `staging_test.rs:381-401` and `apply_test.rs:104-121` are sequential and would pass with the advisory lock and the `FOR UPDATE` deleted. Two `tokio::join!` tests would make both falsifiable. | `tests/staging_test.rs`, `tests/apply_test.rs` | **[lane]** |
| MA-14 | `check_access_gate.sh` is a heuristic by its own admission ("heuristics and stay heuristics", Rule A is file-level, and `preview`/`restore` clear it via `export` while calling no `require`), but the body states it as a per-route guarantee. | `scripts/check_access_gate.sh:20-44` | **[lane]** |
| MA-15 | Three tools map database failures to `McpError::internal_error`, the shape the same module says never to use because it invites an identical retry, and put raw sqlx text (constraint and column names) on an agent-facing wire. | `agent/tools.rs:176,214,412,441` vs `:305-312` | **[lane]** |
| MA-16 | Migration `0005` steps 1–2 (~60 of 105 lines) are unreachable in every production-bound sequence; folding step 3 into `0002` would make the rebuild atomic and delete the caveat BL-5 corrects. Constraint: editing `0002` breaks checksums on any dev DB that applied it mid-branch. | `migrations/0005`, `check_migrations.sh:126-129` | **[lane]** |
| MA-17 | `2692a69` (migration `0006` + audit script + backup validate + detector deletion) is an independent data-integrity fix that does not belong to the agent surface; the body's "all four prerequisites had to land first" is true for WS0/WS1/WS2 and false for this one. | commit `2692a69` | **[lane]** |
| MA-18 | **The approval gate approves counts, not money.** `render_preview` emits per asset only `symbol`, `name`, `isin`, `asset_type`, `currency`, `currency_assumed`, `transactions: <count>`, `first_date`, `last_date`. No quantity, price or fee is ever shown, so the human approves "IWDA: 500 transactions" without seeing a single money value. `validate` bounds sign and precision but has no plausibility bound, so `price: "0"` or `quantity: "9999999999999999.999999999999"` passes and lands after approval. The same file already established the honest-disclosure pattern for the 25-asset cap ("NOT shown here and will still be written"); the money columns get no equivalent. Fix: per-asset min/max quantity and price in the preview, or a row-set checksum. | `agent/staging.rs:443-465` | **[verified]** |
| MA-19 | **No timeout and no concurrency bound on `/mcp`, which shares the human's runtime.** `mount.rs:250` awaits `.handle(http_req)` and `:258` awaits `resp_body.collect()` with no `tokio::time::timeout`; grep over `src/` finds **no `Semaphore`, no `spawn_blocking`, no `time::timeout` anywhere in the crate**. The CPU-bound tools (`get_tax_summary` → `analytics::build_tax_report`, `get_performance` → `StatsEngine`, `get_transaction_pnl` → `TaxLotEngine`) do their decimal math inline on the shared runtime, so one agent looping them starves `/api` in the same process. The only timeouts in the crate are the pricing client's 10 s, lookup's 10 s and the DB's per-statement 30 s — none bounds a whole `/mcp` request. The response side is unbounded too (whole body buffered into a `Vec<u8>` at `:268`). Fix: deadline around `handle`, a 2–4-slot semaphore returning 503 + `Retry-After`, engines to `spawn_blocking`. | `agent/mount.rs:250,258,268`; crate-wide grep | **[verified]** |
| MA-20 | **A token can exhaust the 5-connection pool and turn the human's UI into 503s.** `db.rs:20` `max_connections(5)`, `acquire_timeout` 5 s (`:44`), `statement_timeout = '30s'` **per statement** (`:70`) — so nothing bounds a transaction. `staging::apply` holds one connection for ~600 sequential round trips (claim, flip, revalidation, per-asset lookup + `existing_keys`, then one insert per row up to 500); `stage` holds one while blocked in `pg_advisory_xact_lock`. Five concurrent agent calls take the pool, and the human's next request then fails inside `AuthFairing::on_request` → `/_auth/unavailable` → 503 (`auth/fairing.rs:96-105`) — and `guard.rs:222-235` documents that every `/api` request makes two such reads. Note `db.rs:42-43`'s own comment: a saturated pool is a 503 *only inside the backup module*; everywhere else it surfaces as a permanent 500. | `db.rs:20,42-44,70`; `agent/staging.rs:963-1090`; `auth/fairing.rs:96-105` | **[verified]** |
| MA-21 | **No kill switch for a leaked token.** `auth/routes.rs:314-330` (`logout-all`) bumps `token_version` and clears the cookie; `pat.rs:9-12` states tokens "deliberately survive it" — a documented decision, but the consequence is that the app's only incident-response action leaves every agent token live. Compounding it: `expires_in_days: Option<i64>` with `None` = **never expires** (`api_routes/tokens.rs:37,107-109`), `MAX_EXPIRY_DAYS = 3650` (ten years, `:84`), no revoke-all, no cap on tokens per user. The default mint mints a credential that never dies. Fix: default expiry (e.g. 90 days) when the field is omitted, and "revoke all tokens" beside `logout-all`. | `auth/routes.rs:314-330`; `auth/pat.rs:9-12`; `api_routes/tokens.rs:37,84,107` | **[verified]** |
| MA-22 | **`src/agent/tools.rs` is outside the access gate.** `scripts/check_access_gate.sh:72` sets `HANDLER_DIR="src/api_routes"`, so the gate never scans `src/agent` — yet `tools.rs` resolves caller-supplied ids and issues raw SQL against the three protected tables (`:172`, `:654`). Every tool calls `access::require*` today, so this is an enforcement gap rather than a live bypass, but nothing stops the next tool from skipping it, and the body presents the gate as a per-route guarantee (MA-14). The module comment is also wrong twice: five tools re-implement route SQL, not three (BL-7), and `list_portfolios` (`:172`) / `list_transactions` (`:383`) do **not** go through `access::require*` — they use an inline `p.owner_id = $1` predicate, correct but not the chokepoint the comment claims. Fix: add `src/agent/tools.rs` to `HANDLER_DIR`. | `scripts/check_access_gate.sh:72,127`; `agent/tools.rs:12-19,172,383` | **[verified]** |
| MA-23 | **`detail.rs` still documents the producer `2692a69` deleted.** Module doc `:3-4`: "Two producers, one shape: `detail_from_bundle` … and `owned_detail`"; `:195-197`: "`owned_detail` pushes in this same order (`no_assets` first, then `duplicate_symbol`)". `owned_detail` (`:295`) no longer pushes the flag — the only push in the file is `:222`, bundle side, and the test at `:753` says so. The body and `AGENTS.md` both claim the half was deleted; the code agrees, two comments in the same file do not. Third instance of the same defect class in one file (with BL-4). | `backup/detail.rs:3-4,195-197` vs `:222,295` | **[verified]** |
| MA-24 | **`"nothing in the engines reads it" is partly false.** `tax_engine.rs:797` takes `asset_type: &str` and writes it into the response at `:873` (`AssetTaxSummary.asset_type`). No engine *branches* on it, so the substance survives, but the `AGENTS.md` justification — "which is the only reason that is tolerable" — is load-bearing and wrong: a junk stored `asset_type` is echoed to the user in every tax-summary response. | `AGENTS.md` bullet 1; `engines/tax_engine.rs:797,873` | **[verified]** |
| MA-25 | **Two money write paths validate to different rules.** `POST /api/backup/restore` types money as `String` (`backup/mod.rs:174-176`), so serde refuses a JSON number — but validation is `Decimal::from_str_exact` (`:494-498`), not `is_canonical_decimal`. A bundle can carry a fractional part longer than `MAX_DECIMAL_SCALE = 12`, which `strict_decimal_json` refuses on `/api/transactions`, and restore stores it verbatim. Not a float leak; a divergent money rule the body does not mention. | `backup/mod.rs:174-176,494-498` vs `db_types.rs:77,163-186` | **[lane]** |

## 6. Drift analysis — measured, not read

### 6.1 No drift (claims verified true)

| Claim | Measurement |
| --- | --- |
| `openapi.json` regenerated, 23 paths, no dangling `$ref` | Byte-identical to `cargo run --bin backend_rust -- --print-openapi`. 23 paths, 32 schema defs, 28 refs, 0 dangling. `/mcp` absent as stated |
| `api.d.ts` regenerated | Byte-identical to `openapi-typescript` regen of the committed spec |
| `.sqlx/` cache current | `cargo sqlx prepare --check -- --all-targets` → exit 0 |
| Migrations 0001–0006 apply to an empty DB | Clean; `uq_split_asset_date` reads `WHERE (upper(btrim(type)) = 'SPLIT'::text)`, `uq_assets_portfolio_symbol` reads `(portfolio_id, upper(symbol))` |
| `0006` checked in the failure direction | With `auditdup` + `AUDITDUP` in one portfolio: audit exits 1 naming portfolio/ids/spellings; `0006` refuses with the same detail. Script predicate ≡ migration predicate |
| `check_migrations.sh` fails with the index dropped | rc=0 → rc=1 after `drop index uq_assets_portfolio_symbol`, naming the constraint |
| All six gates pass | `access_gate` (27 handlers), `pricing_seam`, `backup_portability`, `upload_path`, `migrations`, `sqlx_cache` — all rc=0 on a freshly migrated DB |
| CI: eight checks pass | `gh pr checks 77`: 8/8 pass |
| Spec vs implementation, transport | `2026-11-18-mcp-agent-interface-design.md` is byte-identical between `main` and the branch and already specifies the Rocket mount + `NeverSessionManager` + `legacy_session_mode=false` + `json_response=true`, matching `agent/mount.rs:163-173` |
| Money wire is consistent | `decimal_json` (response, lenient) and `strict_decimal_json` (write, string-only) both serialize to strings, so route and tool emit the same shape |
| Declared out-of-scope items hold | Nothing ships backup/restore of staged batches, multi-currency, an `isin` index, or cluster config |

### 6.2 Drift found

**D1 — the Verification section is stale against its own head [verified].** Body: 664 backend tests, 361 frontend tests, 79 files. Measured at `df258e4`: **670** backend passed / 0 failed, **362** frontend passed, **81** changed files. Same root cause as BL-9.

**D2 — the clippy claim is false [verified].** Body: *"0 errors, no warning in a touched file."* Warnings exist in two files this PR **adds**: `tests/pat_test.rs:352` (`needless_borrow`), `tests/apply_test.rs:590` (`unnecessary_cast`). The `api_routes/transactions.rs:242` warning is pre-existing (same statement at `main:318`), so it is not attributable — but the two test files are.

**D3 — five of eleven tools duplicate their route's SQL [verified].** Folded into BL-7.

**D4 — operator-facing docs untouched [verified].** `docs/walkthrough.md` and `docs/auth-testing.md` mention neither PATs, `/mcp` nor staged batches; only `AGENTS.md` (+36 lines) and the spec/spike do. `k8s/` has no `/mcp` entry — harmless for routing (ingress is `path: /` Prefix) but nothing records that `/mcp` is reachable through the same public ingress as the app once nginx is fixed.

**D5 — `audit_asset_symbols.sh` can report its own failure as data corruption [verified].** `q() { psql … 2>&1; }` (line 43) folds stderr into the result and discards the exit status, so a failed collisions query prints "FOUND — same symbol twice … 0006 will refuse to run" and exits 1 instead of the documented 2. Related: on an unmigrated database it exits 2 where the honest answer is 0 — reproduced on an empty DB, where `0006` applies cleanly. Fail-safe direction, wrong diagnosis.

## 7. Conflicts between lanes, and which wins

| Disagreement | Resolution | Winner |
| --- | --- | --- |
| `comprehension` lists `mcp_mount_test.rs:118` among tests that "exist and can fail"; `correctness` files MA-2 saying it cannot fail for a cookie-specific reason | Re-read both tests and `rpc()`: byte-identical calls, no `Cookie` header anywhere in the file. The correctness lane is right; the comprehension refutation is a false positive | **correctness** |
| `zoom-out` calls the error taxonomy ceremony-adjacent in its question, then refutes itself; `correctness` independently confirms `staging_test.rs:698-703` walks variants and can fail | Taxonomy stays. It is a genuine narrowing from `e4bde8c`, and the wire codes are spec-fixed | **both refutations** |
| Body says `0005`'s window is real; `correctness` says it cannot exist | Proven empirically (BL-5): concurrent writer blocked 4005 ms behind an uncommitted `DROP INDEX`, and sqlx wraps every migration absent a `-- no-transaction` marker | **correctness** |
| `correctness`, `security` and `citations` each filed the cookie test independently | Three independent routes to one conclusion, and the security lane's mechanism is the deepest: not merely "no cookie sent" but "the harness runs `AuthMode::Disabled`, the one mode that never reads a cookie" (`guard.rs:150-160`). MA-2 carries that version. The security lane asked whether the body claims the refusal is test-pinned, which would make it a Blocker; it does not — but `mount.rs:18-22` does, in prose, which is why MA-2 is filed as the highest-priority Major rather than a nit | **converged** |
| `security` initially suspected `SELF_AUTHENTICATED` prefix-matching was a hole | Self-refuted on re-read: exact match is fail-closed, and the test documents its own dependency | **refuted** |
| `citations` rates claim 5 (pending cap serialised) CORRECT while `correctness` files MA-1 | Not a conflict: they are different checks. The pending-batch cap is inside the advisory lock; the portfolio cap in `apply` is not | **both stand** |
| Body says every tool shares the route's function; `zoom-out`, `comprehension` and the orchestrator each found the exception independently | The module doc is the tiebreaker and it agrees with the reviewers, not the body | **reviewers** |
| `readiness` IR-13 doubts the "eight checks" claim (seven jobs in `ci.yml`) | `gh pr checks 77` shows 8 passing; the eighth is GitGuardian, which is not from `ci.yml` | **refuted** |

## 8. Refutations — investigated and not filed

* **apply is single-use under two concurrent approvers.** `staging.rs:975-981` `SELECT … FOR UPDATE` blocks the second caller, which reads `applied` and returns `AlreadyApplied`; the conditional flip at `:1008-1016` is an independent second guard. Untested under concurrency (MA-13) but correct.
* **Expiry predicates agree.** `approve` puts `expires_at > now()` in the same UPDATE (`:683`); `apply` checks `expires_at <= now()` (`:996`) and repeats it in the flip (`:1012`); `stage` sweeps before counting (`:585-601`).
* **`mount.rs` header handling is correct.** `authorization`, `cookie`, `host`, `content-length`, `transfer-encoding` all dropped before rmcp (`:235-241`); bearer consumed at `:186` and never forwarded; `MAX_BODY_BYTES` enforced before rmcp sees a byte (`:196-203`), with a test asserting the >8 KiB path.
* **`0006`'s audit predicate, index expression and lookup expression all agree on `upper(symbol)`**, and the refusal to rewrite `assets.symbol` is correct: `historical_prices` is keyed by the symbol string, so a rewrite would silently detach price history.
* **The error taxonomy is necessary**, each variant mapping to a spec-fixed wire code, with a test that can fail.
* **No tool is unneeded by a stated workflow**; all eleven appear in the spec's tool table. `get_performance` and `get_transaction_pnl` are the weakest candidates and the first to cut under scope pressure.
* **Frontend reuse is real.** Both panels use `apiFetch`/`API_BASE`; no component imports the generated `api.d.ts`, so hand-written DTO interfaces match repo convention rather than duplicating it. Neither panel is deletable — `StagedBatchesPanel` is the human half of the approval gate.
* **`MAX_PORTFOLIOS_PER_USER = 10`, the 90-day staleness nudge, the 16 KB preview budget and the no-cron sweep** are all spec-stated or load-bearing, and the "ten is arbitrary" note at `staging.rs:36-44` is honest disclosure rather than a gap.
* **`is_canonical_decimal` accepts nothing `Decimal::from_str_exact` rejects** (28 total digits ⇒ ≤ 9.99e27, inside rust_decimal's range).
* **The frontend-first window is survivable.** `AccessTokensPanel.tsx:85,95,106` catch a failed `/api/tokens` list and render an error rather than crashing, so new-frontend + old-backend degrades to two unhappy panes.
* **A panicking tool does not take the process down.** Rocket 0.5.1 wraps the handler future in `std::panic::catch_unwind` and maps a panic to a 500 (`rocket-0.5.1/src/server.rs:51-58`); an in-flight `apply` rolls back because the sqlx transaction is dropped uncommitted.
* **The oversized-body path is correct.** `mount.rs:75,204-208` enforces a 1 MiB cap with a 413 before rmcp sees a byte, and taking `Data` rather than `Vec<u8>` correctly avoids Rocket's 8 KiB `bytes` default. The 16 KiB preview budget is a separate, smaller bound.
* **Magnitude overflow via staged money is not reachable.** `is_canonical_decimal` (`db_types.rs:95-118`) caps total significant digits at 28 and scale at 12, so a 200-digit quantity is refused in `validate` before storage.
* **The non-constant-time token compare is not exploitable.** `pat::lookup` matches on `secret_hash` equality rather than a constant-time compare, but the secret is 43 base62 chars from a CSPRNG with rejection sampling (`pat.rs:130-150`) — 256 bits, no dictionary.
* **`AuthMode::Disabled` opening all of `/api` is compiled out of release.** `auth/mod.rs:64-70` gates the variant on `debug_assertions` and `:98-103` refuses `AUTH_MODE=disabled` in a release build. (This is what makes MA-2 a test gap rather than a live bypass.)
* **Handle enumeration is not an oracle.** `new_handle` is 32 CSPRNG bytes hex (`staging.rs:556-562`) and `require_own_staged_batch` answers 404 for both "gone" and "not yours" (`access.rs:239-256`).
* **Clock skew between app and DB in `apply` is handled.** The flip compares `expires_at` against the DB's `now()` while the pre-check uses `Utc::now()`, and the `rows_affected() == 0` branch re-reads the locked row and answers `Expired` rather than `AlreadyApplied` (`staging.rs:1020-1043`).
* **`SELF_AUTHENTICATED = ["/mcp"]` is exact-match, and that is fail-closed** — a `/mcp/…` variant would trip the assert (`route_coverage_test.rs:223`). The citations lane confirmed the test ignites the **production** builder (`build_rocket_oidc` → `lib_build_rocket`) and iterates `rocket.routes()` after `.ignite()`, so drift in `lib.rs` mounts is genuinely observed, and the `path == "/api"` panic at `:226` is reachable.
* **The `get_staged` scope test can fail.** `mcp_mount_test.rs:352-370` mints a real `import`-scoped token, stages a real batch and asserts `status == "staged"`; under `Scope::Read` the gate returns `scope_missing` and the assert fires.
* **The pending-batch cap really is serialised.** The advisory lock at `staging.rs:597` covers every writer that can raise the count — the only `INSERT INTO agent_staged_batch` in the tree is `:637`, on the same transaction, and `approve`/`discard`/`apply` only move status. (MA-1 is about the *portfolio* cap, which is a different check.)
* **`check_migrations.sh` pins the expressions `AGENTS.md` claims** — `(portfolio_id, upper(symbol))` with `portfolio_id` leading at `:287-296`, and `upper(btrim(type)) = 'SPLIT'` at `:247-252`.

## 9. Nits

`db_types.rs:80-82` documents `MAX_DECIMAL_PRECISION` as an integer-part bound ("16 integer digits") while `:114` applies it as a total digit count — the code is right, the comment is stale **[verified]**. `audit_transaction_types.sh` is mode `100644` while its sibling is `100755`, so `./audit_transaction_types.sh` fails with permission denied **[verified]**. Body describes the audit script as two exit codes where it has three. Body says "`eslint src`" where CI runs `npm run lint` = `eslint .` (CI is stricter, so this understates). Two hand-written `YYYY-MM-DD` validators in one module (`tools.rs:104 exact_date`, `staging.rs:191 is_calendar_date`) — the PR's own "one home for a rule" argument applies. Three refusal-rendering helpers where one suffices (`refused`, `refused_report` with a single caller, `refused_from`). "gate" carries three unmarked senses (approval gate, access-gate script, `gate()`). `mount.rs:3-5` cites a spike commit a fresh clone may lack while the in-tree spike doc goes unlinked. `staging.rs:116-118` describes the `what:"preview"` refusal as both live and dead with no sentence marking it a tripwire. `resolve_asset` silently discards a staged ISIN/name/currency when the symbol already exists, after the preview showed the human the ISIN — same class as the currency-default bug `250ccdf` fixed, one level down.

From the security and citation lanes: **a token can undo a human's approval** — `discard` accepts `status IN ('staged','approved')` (`staging.rs:697-703`) and the `discard_staged` tool needs only `Import` scope, so a token can force a re-stage/re-approve loop against the 5-pending cap; it cannot cause a write, but refusing to discard an `approved` batch from a token would close it **[verified]**. Header forwarding at `mount.rs:236-241` is a five-name denylist on the app's only non-session door — correct today, but an allowlist (`content-type`, `accept`, `mcp-*`) would fail closed instead **[verified]**. `tools/list` is ungated, so any valid token learns all eleven tool names whatever its scopes (no data leaks; filter by scope or accept) **[lane]**. `pg_advisory_xact_lock($1)` is keyed on bare `user_id` with no namespace tag (`staging.rs:597`) — the only advisory lock in the tree today, so no collision now, but a two-key form costs nothing **[verified]**.

## 10. Unresolved — operator decisions

1. **Is `/mcp` meant to be reachable in this release?** If "merged but dark", BL-1 and BL-2 drop to documented-and-deferred; if "live on merge", they are release blockers. Settled by the operator's intent, not by the tree.
2. **Does the live cluster still consume this repo's `k8s/` at all**, or only k8s-titan's copy? Settles whether `configmap.yaml`'s `sqlite:` DSN is inert or live, and where BL-3's fix belongs. Read k8s-titan #297.
3. **Is hand-running migrations in `psql` a supported path?** If not, BL-5's fix is to delete the non-atomicity hedge; if yes, the hedge is right for the wrong reason and needs rewriting.
4. **Has any dev or CI database already applied `0002` from a mid-branch build?** Determines whether MA-16's fold is available or whether `0005` stays as-is.
5. **BL-4: delete the dead flag or reorder the preview.** Deleting matches the commit's own principle; reordering changes the preview route's contract.
6. **MA-1's fix choice.** Take `pg_advisory_xact_lock(user_id)` in `apply` (serialises all applies for a user — a behaviour change), or correct the comment to say it only serialises re-application of the same batch (leaves a narrow way past the cap). Settled by whether 10 is meant as a ceiling or only an agent-path backstop — note `POST /api/portfolios` has no cap either way.
7. **Whether Rocket bounds a body opened via `Data::open`.** The security lane did not verify a socket-level read timeout, so the "hung client" arm of MA-19 may or may not be partly bounded by the server. Settled by `rocket-0.5.1/src/config/limits.rs` or a half-open-body integration test. Either way the *tool* side has no bound.
8. **Whether `services::transactions::insert` performs any side effect outside the `&mut PgConnection` it is handed.** The security lane read the call site (`staging.rs:1078`) but not the function; if it writes anything off the transaction handle, `staging.rs`'s "all or nothing" claim breaks. One file settles it.
9. **A deterministically unappliable `approved` batch is retryable until TTL.** The flip rolls back with the failed write (documented as intentional) and `approve` cannot re-approve a non-`staged` row, so nothing burns the batch. Judged correct-but-sharp; flagging in case a failed-apply counter that expires the batch is wanted.

## 11. Commands run for the drift analysis

```sh
cargo run --bin backend_rust -- --print-openapi            # → byte-identical to docs/openapi/openapi.json
npx openapi-typescript ../docs/openapi/openapi.json        # → byte-identical to src/types/api.d.ts
cargo sqlx prepare --check -- --all-targets                # exit 0
cargo test --workspace                                     # 670 passed, 0 failed
npx vitest run                                             # 362 passed
npx tsc --noEmit ; npx eslint src                          # clean
for g in check_access_gate check_pricing_seam check_backup_portability \
         check_upload_path check_migrations check_sqlx_cache audit_asset_symbols; do bash scripts/$g.sh; done
sqlx migrate run --no-dotenv                               # fresh DB, 0001–0006
# 0006 failure direction: auditdup + AUDITDUP in one portfolio → audit exits 1, migration refuses
# check_migrations mutation: drop index → rc=1 naming uq_assets_portfolio_symbol
# 0005 atomicity: DROP INDEX in an open transaction; concurrent INSERT blocked 4005 ms (rc=124)
gh pr checks 77                                            # 8/8 pass
# security-lane verification:
grep -rn "Semaphore\|spawn_blocking\|time::timeout" backend_rust/src/   # → no matches
grep -n "HANDLER_DIR" backend_rust/scripts/check_access_gate.sh         # → src/api_routes only
grep -n "status IN ('staged', 'approved')" backend_rust/src/agent/staging.rs  # discard accepts approved
```

All regeneration went to `/tmp`; the reviewed worktree was left clean at `df258e4`.

## 12. Disposition — what was fixed, and how it was proved

Written 2026-11-24 after the operator asked for the findings to be addressed. Fixes are on
branch `fix/pr77-review-findings`, commit `34482ba`, worktree `/home/coder/wt-pp-fix77`
(off `df258e4`). The reviewed tree `/home/coder/wt-pp-staging` was left untouched.

**Code fixes** are in `34482ba`. **Prose fixes** — the claims that were false about the tree,
which no commit can correct — are in `pr_body_corrected.md` beside this report, paste-ready.

| Finding | Disposition | How it was proved |
| --- | --- | --- |
| BL-1 `/mcp` unreachable | **fixed** | `location /mcp` added; `check_upload_path.sh` rule 3 fails on a missing block or a proxy narrower than the route's `MAX_BODY_BYTES`. Three tests in `upload_path_test.rs`, including the shape that shipped. |
| BL-2 no enable gate / TTL knob | **fixed** | `AgentConfig` with `AGENT_MCP_MOUNT` and `AGENT_BATCH_TTL_MINUTES`, threaded to the expiry actually used. Unit tests on the parsing and clamping. |
| BL-3 stale Postgres prerequisite | **body** | Corrected in `pr_body_corrected.md`; the cutover ran 2026-11-15. |
| BL-4 dead `duplicate_symbol` detector | **fixed + documented** | Comparison upper-cased to match `validate` and the index; the flag's doc now states it is a tripwire unreachable through `preview`, not a message to a human. Test rewritten to assert the flag now fires on a case variant. |
| BL-5 `0005` non-atomicity claim | **docs** | Cannot edit the migration — checksum-locked against the live DB. Correction written into `docs/postgres-cutover.md`, with the measurement (writer blocked 4005 ms behind an uncommitted `DROP INDEX`). |
| BL-6 "`AuthFairing` accepts either" | **body** | Corrected; `guard.rs` reads the session, PAT lookup is separate. |
| BL-7 "every tool calls the same function" | **body + gate** | Body names the 7 that do and the 4 that do not. `check_access_gate.sh` rule D now requires `.gate(` in every `#[tool]`; mutation (strip a gate) → exit 1. |
| BL-8 `AGENTS.md` wrong line refs | **fixed** | Refs repointed; the `asset_type` bullet corrected (the tax engine does echo it). |
| BL-9 body describes another tree | **body** | Counts recomputed against the real head. |
| MA-1 portfolio cap not serialised | **fixed** | `pg_advisory_xact_lock(user_id)` in `apply`. Proved by holding `pg_advisory_lock(user_id)` from a second session and asserting `apply` does not complete — **the first version of this test was a `tokio::join!` race and it passed with the lock deleted; it was thrown away for that reason.** |
| MA-2 cookie test cannot fail | **fixed** | Test now builds the OIDC rocket and sends a real decryptable `pp_session` with no bearer. |
| MA-18 preview approves counts, not money | **fixed** | Per-asset and total `quantity_min/max`, `price_min/max`, `fee_max`. |
| MA-19/20 no timeout, no bound, pool | **fixed** | 1 MiB body cap wired, `Semaphore(3)` → 503, 120 s → 504. |
| MA-21 no kill switch | **fixed** | `POST /api/tokens/revoke-all`, session-only, in the route inventory and the published document. |
| MA-22 `src/agent` outside the access gate | **fixed** | Rule D, as BL-7. |
| MA-23 stale `owned_detail` doc | **fixed** | Comments now say the DB-side producer is gone. |
| MA-24 `asset_type` echoed | **fixed (docs)** | `AGENTS.md` corrected rather than the engine changed — the engine's behaviour is out of this PR's scope. |
| MA-25 restore decimal scale | **fixed** | Restore rejects above `MAX_DECIMAL_SCALE`; frontend `parseDecimal` now refuses the same shapes, so a cell that parses in the browser is a cell the API accepts. |
| MA-7/8/9/10 deploy + audit + rollback docs | **fixed (docs)** | New migrations section in `docs/postgres-cutover.md`: where to run each audit and with which DSN, why there is no clean rollback past `0006`, and how to order the frontend-before-backend roll given that the pipeline publishes both images at once. |
| D1 stale counts | **fixed** | 685 backend / 363 frontend now, and the body says so. |
| D2 clippy claim false | **fixed** | The two warnings in files this PR adds (`tests/pat_test.rs`, `tests/apply_test.rs`) are gone. The rest are pre-existing in files the PR does not touch. |
| D3 query duplication in tools | **body** | See BL-7. |
| D4 operator docs untouched | **fixed** | See MA-7/8/9/10. |
| D5 audit script folds stderr into `q()` | **open** | Operator decision — see §10. |
| SEC-4 default token expiry | **open** | Operator decision: changing the default expiry is a product choice, not a defect fix. |
| MA-21 expiry clock consistency | **fixed** | `pat::lookup` filters `expires_at > now()` in SQL, matching every other expiry check. |

### Gate and test state after the fixes

```
cargo test                 685 passed, 0 failed   (was 670)
vitest                     363 passed, 0 failed   (was 362)
six gate scripts           all pass
cargo sqlx prepare --check clean
cargo clippy --all-targets 0 errors; no warning in a file this PR adds
tsc --noEmit / eslint src  clean
```

Every new test was mutation-checked — the fix removed, the test must fail. Two of them did not
fail on the first attempt and were rewritten rather than left in: the `join!` race for the
advisory lock (see MA-1), and a row-lock race that passed with `FOR UPDATE` deleted because
the flip's `UPDATE` takes the same row lock. That second test is now named for what it
actually pins — that `apply` cannot proceed while another session holds the batch row — and
says in its comment that it does not distinguish the two lock sites.
