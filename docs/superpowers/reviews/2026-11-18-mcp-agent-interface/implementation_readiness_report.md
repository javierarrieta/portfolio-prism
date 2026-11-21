# Stage 9 — Implementation Readiness Review

* **Target:** `/home/coder/wt-pp-mcp-spec/docs/superpowers/specs/2026-11-18-mcp-agent-interface-design.md`, 1–556
* **Reviewer:** `egm-implementation-readiness`, fresh context, `implementation-readiness-standard.md`, prior stages 5/6/7/8 + `orchestrator_notes.md` in context
* **Date:** 2026-11-18

Read against the tree at `/home/coder/wt-pp-mcp-spec`. Prior stages are taken as given and not restated. Stage 8's readiness ruling was tested rather than assumed: **WS2 is confirmed not plan-ready, and WS0 is in a worse readiness state than "ready after B and C"**, because two of its items are mislocated or unverifiable as written. WS1 is closest to ready and its gaps are inventory bookkeeping the spec never lists.

The question throughout: *do I know what to code, where, and how, from this text?*

---

## Cross-cutting (applies to all three workstreams)

**C-1 · Blocker · `.sqlx` prepare-cache obligation, named nowhere**
`Cargo.toml` has no direct `hyper`/`tower`/`http`/`hyper-util` dependency (verified: rocket, sqlx, serde, reqwest, utoipa, rust_decimal, `schemars = "1.0"`, openidconnect only), and `.sqlx/` is committed. Every new `sqlx::query!` in `auth/pat.rs`, `agent/staging.rs` and the migration-adjacent code requires, in order: `sqlx migrate run --no-dotenv` against a live DB → `cargo sqlx prepare -- --all-targets` → commit `.sqlx/`. `ci.yml:142` (`cargo sqlx prepare --check`) and `ci.yml:211` (`check_sqlx_cache.sh`) both fail otherwise, and `backend_rust/Dockerfile` builds offline from that cache. **Spec fix:** add to Sequencing, per workstream: "after the migration is applied locally, run `cargo sqlx prepare -- --all-targets` and commit `.sqlx/`; this needs `DATABASE_URL` (`scripts/dev-db.sh`)."

**C-2 · Major · Migration-number reservation across three independent PRs**
The spec assigns `0002_`, `0003_`, `0004_` while Sequencing makes them three PRs. `check_migrations.sh:113-136` requires one successful ledger row per `*.sql` on disk with a matching sha384, so renumbering on rebase is safe mechanically but breaks any test or doc that names the file. **Spec fix:** one sentence — "migration numbers are reserved in merge order; a PR that lands second renumbers and updates its own prose and test names."

**C-3 · Major · The Testing section's `DATABASE_URL` claim is wrong in both directions**
"Everything DB-touching … need `DATABASE_URL` like the other five gates." Two problems: (i) `AGENTS.md:99-101` says five of eleven gates need it *including* `cargo test`, so "the other five" is four; (ii) the new `protocol.rs` deserialization table test and `tests/agent_gate_test.rs` must run **without** a URL — `tests/access_gate_test.rs` is the stated model and it is a static-tree test, and `ci.yml:223` deliberately keeps clippy URL-free to prove the offline path. An implementer who makes the gate's mutation test depend on a database has broken the property that proves the gate can fail. **Spec fix:** split the bullet into "needs `DATABASE_URL`" (staging, isolation, PAT, migration-abort, e2e) and "must pass with no `DATABASE_URL`" (protocol deserialization table, gate mutation test).

**C-4 · Major · Gate-script identity is undecided and the Testing section picks one while the body promises the other**
The body's "reuses rather than re-invents" promise plus Stage 5 ZO-9 point at extending `check_access_gate.sh`, which already parameterises its target (`HANDLER_DIR`, `scripts/check_access_gate.sh:72`) and already implements the three rules the new script would re-implement. The Testing section names a fifth script plus a new mutation test. These cannot both be true, and the choice also depends on Stage 8 alternative (a): if the tools end up mounted as Rocket routes, the new script is unnecessary and `tests/route_coverage_test.rs:181 PUBLIC_NAMESPACES` is the amendment instead. **Spec fix:** one sentence naming the file that changes and the file that does not exist.

---

## Workstream 0

**W0-1 · Blocker · WS0 item 1 names the wrong file and leaves the read-side type undecided**
The spec says "In `models.rs`: … `TransactionCreate.type` becomes `TxType`". `TransactionCreate` is at `backend_rust/src/schemas.rs:18-27` (verified); `models.rs:44-51` holds `Transaction`, whose `r#type: String` is the `FromRow` decode target for every `query_as!(Transaction, …)` in `api_routes/transactions.rs` (list, create-return) and `backup/`. If an implementer follows the sentence literally and also converts `models::Transaction.r#type` to `TxType`, every read of a stored lowercase row fails to decode — and the spec's own `CHECK (upper(btrim(type)) …)` explicitly permits those rows. **Spec fix:** "`TxType` is defined in `models.rs`; `schemas::TransactionCreate.r#type` becomes `TxType`; `models::Transaction.r#type` and `schemas::TransactionOut.r#type` **stay `String`** because they decode stored rows, which the `CHECK` allows in any case. `frontend/src/types.ts:13` is unchanged." Also: the spec's literal `#[serde(rename_all = "uppercase")]` is a compile error (ON-2) — the plan must write `"UPPERCASE"`.

**W0-2 · Blocker · WS0 item 4 is three deliverables in one bullet with an unnamed prerequisite migration**
"Rejects a JSON number outright" applied to `TransactionCreate` breaks the browser importer, which posts numbers today: `frontend/src/utils/csvValidator.ts:106 parseNumeric` → `parseFloat`, `:187-189 ValidatedRow.quantity/price/fee: number`, `:247-249`; `frontend/src/components/CsvImportModal.tsx:163` (dedup key built from the numbers) and `:170-176` (payload); the demo seed at `frontend/src/App.tsx:537` (`quantity: tx.qty`). `frontend/src/components/PortfolioDetail.tsx:17-18,374` already sends strings, so the manual form is not the blocker. **Spec fix — the instruction, not the decision:** "Prerequisite PR (frontend only): change `parseNumeric` to return a canonical decimal `string` (or add `parseNumericString` and use it at `csvValidator.ts:247-249`), change `ValidatedRow.quantity/price/fee` to `string`, update the dedup key at `CsvImportModal.tsx:163`, update `frontend/src/test/mocks/handlers.ts:105-111` (which currently `String(body.quantity)`), update the assertions at `frontend/src/utils/csvValidator.test.ts:15,141`, and rewrite the contract comment at `frontend/src/types.ts:8` which currently says 'may use either string or number'. Land and deploy it before the backend rejects numbers." Then the backend item must say **which** deserializer: `db_types::decimal_json` (`db_types.rs:21-67`) is the *response* serializer at ~55 sites in `schemas.rs` and its `visit_f64` arm is load-bearing for `TaxLot`/`AssetTaxSummary` round-trips — so "add `db_types::strict_decimal_json` (string-only, `^-?\d+(\.\d{1,12})?$`, error names the fix) and apply it to the three `TransactionCreate` fields and the agent DTOs; do not modify `decimal_json`."

**W0-3 · Major · The case-sensitivity triangle has no resolution instruction**
Three sources of truth disagree and the spec picks none: the proposed case-insensitive `CHECK`; the case-sensitive partial index `uq_split_asset_date … WHERE type = 'SPLIT'` (`migrations/0001_init.sql:89-91`); and `scripts/check_migrations.sh:169-175`, which pins that predicate **literally** (`case "$pred" in *"type = 'SPLIT'"*)`). Additionally the only lowercase writer in the tree is a test seed: `src/access.rs:260` inserts `'buy'` (verified — it is the sole `'buy'|'sell'|'split'` literal in `src/` and `tests/`). An implementer must choose: (a) exact `CHECK` after normalization → must also change the index predicate, `check_migrations.sh:169-175`, and `access.rs:260`; or (b) case-insensitive `CHECK` → must state that a `'split'` row escapes the #50 dedupe and add a post-normalization `(asset_id, day)` duplicate check to the pre-flight audit (Stage 5 ZO-8). **Spec fix:** pick one and list those four files.

**W0-4 · Major · "abort, listing the offending values" does not say how, and the atomicity consequence is unstated**
A `.sql` migration cannot "abort and list" without plpgsql: the implementer needs `DO $$ DECLARE bad TEXT; BEGIN SELECT string_agg(DISTINCT type, ', ') INTO bad FROM … ; IF bad IS NOT NULL THEN RAISE EXCEPTION '…: %', bad; END IF; END $$;`. sqlx runs each migration file in one transaction, so the `upper(btrim(type))` normalization rolls back with the abort — probably intended, but unstated. And the blast radius is concrete: `main.rs:74-76` turns a failed `sqlx::migrate!` into `refuse()` → `exit(1)` → crashloop, with all three probes on the same container (`k8s/app-deployment.yaml:84-99`). **Spec fix:** state the plpgsql mechanism, the all-or-nothing consequence, and the boot consequence in item 2.

**W0-5 · Major · The named verification for item 2 cannot be implemented where the spec puts it**
"`check_migrations.sh` gains a case proving it fails on a junk value" contradicts that script's own contract, which `AGENTS.md` states in as many words: *"`check_migrations.sh` inspects a **database**, not `migrations/`"*. It cannot observe a migration aborting, and CI's migrate step (`ci.yml:132`) runs against an empty service database. The real home is a Rust test, and it cannot use `src/test_db::test_pool` — `test_db.rs:201` runs the embedded `sqlx::migrate!` before the test body, so junk can never be present *before* migration. **Spec fix:** name the file and the recipe: `tests/migration_type_check_test.rs` — create a scratch database via `test_db::test_pool_url`, load migrations at runtime with `sqlx::migrate::Migrator::new(Path::new("./migrations"))`, apply only `0001`'s `up()`, insert a `'DIVIDEND'` row, apply `0002`'s `up()`, assert the error text names the offending value; then a second case asserting a clean run leaves the `CHECK` in place.

**W0-6 · Major · "the runbook" is not an artifact that exists**
Item 2 makes the audit query "a pre-flight step in the runbook"; Sequencing step 1 makes it a gate; nothing names the artifact. `backend_rust/scripts/pre_cutover_audit.sh` exists but is SQLite-only by design (its header says so) and cannot host a Postgres query. **Spec fix:** name a new `backend_rust/scripts/audit_transaction_types.sh` with the repo's existing exit-code convention (`0` clean / `1` out-of-set rows found, printing `SELECT DISTINCT type` / `2` could not run), and reference it from Sequencing.

**W0-7 · Major · The shared write service (Stage 8 root D) is absent from WS0 text**
If it lands here, as Stage 8 recommends, the instruction form is: "Extract `services::transactions::insert(pool, tx, asset_id, &TxCreate) -> Transaction` from `api_routes/transactions.rs:239-262` — type canonicalisation, the SPLIT quantity/price rule (`:240-243`), fee zeroing for SPLIT (`:244`), the 1900-01-01…2100-01-01 bound (`:248-252`), the INSERT (`:254-262`) — and `services::assets::insert(...)` from `api_routes/transactions.rs:31-96` (ISIN shape check, duplicate-ISIN conflict, currency detection, INSERT). Call them from the Rocket handlers (no behaviour change) and from `staging::apply`. `services/mod.rs` currently declares only `currency_service`." **Gate consequence the spec must also state:** moving INSERTs out of `src/api_routes/` is harmless to `check_access_gate.sh` (Rule A is file-level and the handler keeps its `require*`), but moving any INSERT out of `src/backup/` shrinks `check_backup_portability.sh` Rule 2/Rule 3 scope — see W2-3.

**W0-8 · Nit · WS0 item 3 is already shipped**
`docs/backup-schema/v1.json` already enumerates the set (ON-3). Reword to "verify the existing constraint and confirm no shipped fixture violates it", and drop the matching test bullet, or the plan carries a no-op task.

**W0-9 · Slice size.** Item 4 as written is one task that touches two languages and two deployable artifacts. Split at: **0a** `TxType` + `TransactionCreate` + serde test (backend only, no migration); **0b** migration `0002` + audit script + abort test (needs `DATABASE_URL`, deploy-order sensitive, own PR); **0c-i** frontend money-as-string migration; **0c-ii** backend strict deserializer (blocked by 0c-i); **0d** write-service extraction (no behaviour change, own PR, `refactor/` branch per `AGENTS.md`). 0a and 0b are independent; 0c-ii cannot merge before 0c-i is deployed.

---

## Workstream 1

**W1-1 · Blocker · The scope set is undecided, and it is load-bearing for four deliverables**
The `Scope` enum, `require_scope`, the `api_token.scopes` array contents, and the mint form's checkboxes all depend on whether the set is `read|write|import` (spec) or `read|import|apply` (Stage 8 ruling). Nothing in WS1 can be written without it. This is the one WS1 blocker; everything below is mechanical.

**W1-2 · Major · Three files the spec never names must change, or CI goes red**
Adding `api_routes/tokens.rs` is not enough: the routes must be added to the mount list in `src/lib.rs:320-333` (`.mount("/api", routes![…])`) and to the `paths(...)` inventory in `src/openapi.rs:12-61`, and the request/response types belong in `src/schemas.rs` with `ToSchema`. Then two generated artifacts are committed and CI-diffed: `docs/openapi/openapi.json` (`ci.yml:231`) and `frontend/src/types/api.d.ts` (`ci.yml:341`, generated by `npm run generate-types`, which exists at `frontend/package.json:14`). **Spec fix:** name `lib.rs`, `openapi.rs`, `schemas.rs`, and both committed artifacts.

**W1-3 · Major · Three hidden count pins; the spec names one**
`tests/route_coverage_test.rs:291` asserts `PROTECTED_ROUTES.len() == 21` → 24. `tests/route_coverage_test.rs:304` asserts `with_level == 15` with the message *"six routes authenticate without checking an id"* → 16 and "eight" (POST/GET carry no level, DELETE carries `Owner`). `tests/access_gate_test.rs:87` asserts the gate prints `handlers inspected: 21` → 24. Two prose copies of that number also go stale: `docs/auth-testing.md:45-46` and `scripts/check_access_gate.sh:48`. The spec mentions only the first. **Spec fix:** list all five edits in the WS1 file list. *(Orchestrator: all three pins verified verbatim.)*

**W1-4 · Major · Marker placement is a rule the spec states incompletely**
`check_access_gate.sh` honours `// access-gate: global-config (no caller-supplied id)` only as the *whole line immediately above the route attribute* — i.e. **below** `#[utoipa::path(...)]`, as `api_routes/backup.rs:153` demonstrates — and refuses it outright if the route has any path parameter. Put it above the utoipa attribute and Rule A fails with a message that will read as a gate bug. **Spec fix:** one sentence, citing `backup.rs:153` as the pattern.

**W1-5 · Major · `AuthedUser` gains a field: the blast radius is unstated and the sub-decisions are open**
Construction sites: `auth/guard.rs:150`, `auth/oidc.rs:257`, `:295`, `:331`. Signature change: `resolve_caller` (`guard.rs:99`), used by `AuthFairing` (`fairing.rs:40` prefix rule) and the `/auth/me` guard (`guard.rs:160-186`). Unspecified and required: which module owns `Scope` and `CredentialKind` (`auth/pat.rs` vs `auth/guard.rs` — `pat.rs` is listed as "the only implementation of every operation", but `require_scope` is called by handlers, so put the enum in `guard.rs` and say so); and what `require_scope` returns when a token lacks a scope — `access.rs::deny()` doctrine says never-forbidden for *ownership*, but a missing scope is not an ownership question, so state 403 vs 404 explicitly. Also unresolved and previously noted (DS-23): the converged lookup must take `expected_tv: Option<i64>` with `None` for tokens, or `logout-all` kills tokens and contradicts the spec's own sentence. **Split:** 1a = pure refactor (`user_by_id` extraction + `credential` field, no behaviour change, all four sites, existing tests green); 1b = token branch + `require_scope` + PAT routes.

**W1-6 · Major · The named two-listener test cannot be written the way the neighbouring file does it**
"a request to `/api/portfolios/` carrying only an `Authorization` header is denied" is vacuous in `disabled` mode: `tests/common/mod.rs:115` builds `AuthMode::Disabled { as_user }`, which authenticates every request regardless of headers, and `tests/isolation_test.rs` — the file the spec tells you to extend — is entirely disabled-mode. The assertion needs `common::build_rocket_oidc(pool)` (`tests/common/mod.rs:138`). **Spec fix:** name the harness.

**W1-7 · Nit · `last_used_at` throttle is an unspecified mechanism**
"at most once a minute per token, fire-and-forget" does not say where the throttle state lives (a global map? a `tokio::spawn` per call?). At `replicas: 1` with one operator this is complexity with no beneficiary (Stage 5 ZO-15). **Spec fix:** "write it inline on every successful lookup; no throttle", or drop the column and keep only the 90-day marker.

**W1-8 · Nit · `require_own_token` and `pat::revoke` both filter by `user_id`**
State which is authoritative (the `access.rs` one, per the chokepoint rule), that a foreign id is a `404` via `deny()` and not a `403`, and that `pat::revoke(pool, user_id, token_id)` keeps its predicate as defence in depth. Otherwise the implementer picks one and the gate inventory (`Owner`) disagrees with the code.

**W1-9 · Nit · Slice size.** `AccessTokensPanel.tsx` is three deliverables: list + 90-day staleness marker, mint form (scope checkboxes + expiry select — blocked on W1-1), one-time reveal modal. Split there, and note the placement trap: the Access view must render like `backup` at `App.tsx:791` — **outside** the `currentPortfolio ? …` branch — or it is a blank page whenever no portfolio is selected. `NAV_ITEMS` is at `App.tsx:53`, not `:657` (that is the `<ul>` that maps it).

---

## Workstream 2

**W2-1 · Blocker · Apply mechanism (Stage 8 root D) — confirmed, and it blocks five of the eight WS2 tasks**
Nothing in the document says what `staging::apply` calls to write a row, and the claim that tools "call the same `engines::` and `services::` functions the Rocket handlers call" is false for writes: `services/` contains only `currency_service.rs` and `mod.rs`. Until this is written, `staging.rs`, `tools.rs` (write half), the isolation-test extension, the e2e test and the golden preview fixture are all unbuildable. See Decision 2 below for the paragraph that closes it.

**W2-2 · Blocker · Single-use flip vs write ordering is stated as two steps**
"Single use is a conditional update" is presented as its own step, then "Apply … writes in one Postgres transaction". An implementer following the text writes the flip outside the write transaction; a failed write (the 30-second per-call deadline equals the pool's `statement_timeout`) then leaves a batch marked `applied` with zero rows and no retry path. **Spec fix:** "the flip and the write are the same `SERIALIZABLE` transaction: `BEGIN; UPDATE … WHERE handle=$1 AND status='staged' AND user_id=$2 RETURNING id; <writes>; COMMIT`. Zero rows → `already_applied`, and nothing else in the transaction runs." Note the flip must also carry `user_id` (DS-21) so ownership and state transition are one statement.

**W2-3 · Major · The listener is unspecified at exactly the points an implementer must decide**
`listener.rs` "binds … at launch" does not say: where (the only viable place is `main.rs` before `.launch()` at `:109`, inside `#[rocket::main]`, and it must not bind on the `--print-openapi` / `--print-backup-schema` early-return paths at `:36-43`); what it returns (the e2e test needs `start(pool, cfg) -> anyhow::Result<(SocketAddr, tokio::task::JoinHandle<()>)>` bound to port 0); what a bind failure does (refuse boot like `refuse()`, or log and continue — the spec is silent and the probes cannot see it); and **that `hyper`, `hyper-util`, `tower` and `http` must be added as direct dependencies to `Cargo.toml`** — they are transitive-only today (verified: no `hyper`/`tower`/`http` line in `Cargo.toml`), so "rmcp is the only new dependency" is false at the manifest level even where the versions are already in the lock. **Spec fix:** name the function signature, the bind-failure policy, the four manifest entries with their required features (`hyper` `server,http1`; `hyper-util` `tokio,server,service`), and the `--print-openapi` exclusion.

**W2-4 · Major · The k8s path is three manifests and a frontend image, not one ingress line**
Verified: `k8s/app-service.yaml:12-16` is `port: 80 → targetPort: 80`, and `k8s/app-deployment.yaml` has **two containers** — `frontend` (`containerPort: 80`, probes on 80) and `backend` (`containerPort: 8000`, probes on 8000). The Service therefore targets the **frontend nginx container**, not the backend; the backend's 8000 is reachable only through `frontend/nginx.conf:34-35` (`location /api { proxy_pass http://127.0.0.1:8000; }`). So there is no existing Service port to reuse, and the operator faces a real choice: (i) add `location /mcp` to `frontend/nginx.conf` (frontend image rebuild, one Service, one host, inherits whatever auth policy the host carries), or (ii) add a `containerPort` (`k8s/app-deployment.yaml:58-60`), a second Service port (`k8s/app-service.yaml:12-16`) and an ingress path (`k8s/ingress.yaml`), bypassing nginx and needing its own TLS/policy decision. Either way the probes at `k8s/app-deployment.yaml:83-99` all target 8000 and a dead agent listener stays invisible. **Spec fix:** name the five files and record which of (i)/(ii) is chosen, plus the probe statement. (If Stage 8 alternative (a) lands, this whole item disappears — which is the strongest argument in the document for resolving that decision first.)

**W2-5 · Major · The isolation-test extension has no stated harness**
`tests/isolation_test.rs` is HTTP-shaped (`Client::tracked(build_rocket(pool, as_user))`, `tests/common/mod.rs:87`). Agent tools have no HTTP surface under the second-listener design, so "extend `isolation_test.rs` with the agent tools" requires either calling `tools::` functions directly with a `Credential::Token` `AuthedUser` minted through `pat::mint`, or driving the listener over the transport with two tokens. Those are different tests with different setup. **Spec fix:** name the harness (recommend: direct `tools::` calls for the matrix, transport e2e for wiring only), and state that the matrix cases are B-applies-A's-handle, B-stages-into-A's-portfolio, B-`get_staged`-A's-handle — each paired with an A-succeeds case, per that file's existing convention.

**W2-6 · Major · ISIN validation location is an open decision the review list never named as one**
The spec puts a mod-10 checksum in `agent/protocol.rs`, while `api_routes/lookup.rs:186-199 is_valid_isin` (shape-only) is already on the REST write path (`api_routes/transactions.rs:5,49`) and the restore path (`backup/mod.rs:440-444`). Two ISIN rules in one repo means the agent rejects rows the API accepts. **Spec fix (one sentence either way):** "strengthen `is_valid_isin` in place with the mod-10 check so all three paths share it, and add `tests/` cases for the two paths' existing acceptance behaviour" — or "the agent path is deliberately stricter; `is_valid_isin` is unchanged; the divergence is intentional and named here."

**W2-7 · Major · `stage_import` is not one task**
It is five: (a) DTO + `deny_unknown_fields` + validation rules; (b) asset matching (ISIN → symbol+currency → auto-create, currency default + flag); (c) preview render; (d) apply (re-validate, one transaction, return ids); (e) caps + error codes. The seam between (b) and (d) matters because `target.existing` duplicate-skip is the capability `backup::restore` explicitly refused (`restore.rs:22-24`, `Mode` is `Copy|Replace` only) and needs its own review, and because Stage 8 made the merge path conditional on defining duplicate matching as `Decimal` equality. **Spec fix:** state the five sub-deliverables and mark (b)-merge as gated on the `Decimal`-equality definition.

**W2-8 · Major · `get_performance` cannot answer the spec's own success criterion**
Success criterion #1 is "what is my realized gain in 2025?". The tool note disclaims volatility/Sharpe/beta (`stats_engine.rs:108-110`) but `unrealized_pnl` and `realized_pnl` are also hardcoded `"0.0"` (`stats_engine.rs:328-329`, orchestrator-verified). An implementer following the body ships a read tool that answers the flagship question with a confident zero. **Spec fix:** "`get_performance`'s response omits `unrealized_pnl` and `realized_pnl`; the tool description names the three constants; realized gain is answered only by `get_tax_summary`."

**W2-9 · Nit · Config parsing convention unstated**
`AGENT_BATCH_TTL_MINUTES` should follow `BackupConfig::from_env_with(&dyn Fn)` (`backup/mod.rs:103-114`) so it is testable without mutating the process environment (edition-2024 `unsafe`). One sentence.

**W2-10 · Nit · `schemars` version pin belongs in Verification, not in an implementer's hands**
`Cargo.toml:20` declares `schemars = "1.0"` and the lock already carries both 0.9.0 (`Cargo.lock:2867`, via utoipa 4.2) and 1.2.3 (`:2879`). The Verification item should state the acceptable outcome ("rmcp must resolve to the existing 1.2.3; a third copy is a blocker for the PR"), otherwise an implementer will accept a third `schemars` as fine.

---

## Where the spec's verification list contradicts its body (implementer silently gets it wrong)

1. `check_migrations.sh` "gains a case proving it fails on a junk value" vs that gate's documented database-inspection contract — W0-5.
2. "need `DATABASE_URL` like the other five gates" vs the static tests that must pass without it — C-3.
3. Testing names `scripts/check_agent_gate.sh` + `tests/agent_gate_test.rs` while the body's reuse promise (and `check_access_gate.sh:72 HANDLER_DIR`) points at extending the existing gate — C-4. If the extension is chosen, `tests/access_gate_test.rs:87`'s `handlers inspected: 21` assertion changes meaning, not just value.
4. Non-goals: "No UI panel for staged batches in v1" will be read as "skip the pending-batches list" if Decision 1 adopts it. The non-goal must be re-scoped to "no apply from the UI" or the control will be silently dropped.
5. The body still specifies `stage_edit`, `update_*`, `delete_*` and the third scope, which Stage 8 ruled cut. As written the document **instructs** the implementer to build the two largest YAGNI surfaces in it.
6. Sessions asserts `replicas: 1` is the precondition, while Verification asks about stateful mode and SSE keep-alive; ON-1 shows `2026-07-28` is stateless by default. An implementer will configure session stickiness for a requirement that does not exist.
7. WS0 item 4's "a deserializer used by both the agent DTOs and `TransactionCreate`" reads as "change `decimal_json`", which is the response serializer at ~55 sites — W0-2.

---

## The three open decisions, as spec text

**Decision 1 — what makes apply a control rather than a convention.**
"`apply_staged` requires `Scope::Apply`, which is a distinct scope from `read` and `import`, is not pre-checked in the mint form, and cannot be added to an existing token (re-mint to obtain it). The mint UI shows it in a separate group labelled as the one that writes. In addition, the Access screen carries a read-only *Pending batches* list — `GET /api/staged-batches` (session-authenticated, `PROTECTED_ROUTES` entry at `Owner`-none), rendering the stored `preview` JSONB verbatim with its handle, kind, row counts and expiry — so a human can read the preview out of band rather than only through the model that produced it; this replaces the v1 non-goal's blanket 'no UI panel' with 'no apply from the UI'. Each flagged row may carry an optional `source_text` string, echoed by the agent from the statement it read, and the preview renders it next to the parsed value so `1.5` versus `15` is comparable by eye. The backend does not and cannot prove a human read anything; it guarantees only that a token which can apply is a token a human deliberately minted for that, and that the preview exists somewhere the model did not write."

**Decision 2 — the shared write and preview service.**
"`services::transactions::insert(pool, &mut tx, asset_id, &TxCreate)` and `services::assets::insert(pool, &mut tx, portfolio_id, &AssetCreate)` are extracted from `api_routes/transactions.rs:239-262` and `:31-96` in a no-behaviour-change refactor PR (Workstream 0d) and become the only code in the repo that inserts a transaction or an asset outside `backup::restore`. `staging::apply` calls them inside one `SERIALIZABLE` transaction together with the single-use flip, so the flip, every insert and the commit are atomic. The preview is rendered by `backup::detail` — `detail_from_bundle` over a one-portfolio sub-bundle if the spike shows it is not coupled to whole-bundle shape, otherwise the `PlanSide`/`PlanAsset` types and the `flag` codes (`detail.rs:27-33`) with a thin agent-side adapter — and no second anomaly vocabulary is introduced: the two genuinely new flags (`assumed_currency`, `duplicate_candidate`) are added to `detail::flag`. `target.new` reuses the `Mode::Copy` insert path shape; `target.existing` with duplicate-skip is new code, is precisely the merge capability `backup/restore.rs:22-24` refused, and is reviewed as such with duplicate matching defined as `Decimal` equality over `(date, asset_id, type, quantity, price, fee)`. If the extraction moves any INSERT out of `src/backup/`, `check_backup_portability.sh` Rule 2/Rule 3 scope is extended to the new directory in the same PR; if it does not, say so here so the gate's coverage is not silently reduced."

**Decision 3 — whether the second listener exists at all.**
"Before Workstream 2 is planned, a half-day spike runs against `rmcp` 3.11 with `legacy_session_mode(false)` and `json_response(true)`: stand up one Rocket route that bridges a single POST (method, URI, headers, buffered body in; status, headers, bytes out) to `StreamableHttpService`, and drive it with rmcp's own `StreamableHttpClientTransport` plus one real third-party client through `initialize` → `tools/list` → one tool call. Adopt the in-Rocket mount only if all four hold: no SSE and no `Mcp-Session-Id` anywhere in the exchange; the adapter stays in one bounded file; `AuthFairing`'s prefix rule (`auth/fairing.rs:40`) needs no exemption for the mount point; and `tests/route_coverage_test.rs:181 PUBLIC_NAMESPACES` admits the namespace by explicit amendment. If any fails, record the specific failure in *Alternatives Considered* and the second listener becomes a decided design — at which point W2-3 and W2-4 (direct `hyper`/`hyper-util`/`tower`/`http` dependencies, bind-failure policy, containerPort, Service port or nginx location, probe invisibility) become mandatory Workstream 2 deliverables rather than open questions. Either outcome is recorded in the same PR that adds `agent/`."

---

## Verdicts

**Workstream 0 — PLAN-READY AFTER LISTED EDITS.** Shortest list: (1) correct the file location and state that the read-side `type` stays `String` (W0-1); (2) rule on money-as-string scope and write the frontend migration as a named prerequisite PR (W0-2); (3) pick the case-sensitivity resolution and name the four files it touches (W0-3); (4) name the abort-test file + runtime-`Migrator` recipe and the audit-script artifact (W0-5, W0-6); (5) say whether the write-service extraction lands here (W0-7). Items 1–4 are document edits, not design work.

**Workstream 1 — PLAN-READY AFTER LISTED EDITS.** Shortest list: (1) fix the scope set (W1-1 — this is Decision 1's scope half, and it is the only true blocker); (2) add the unnamed files and the five inventory pins (W1-2, W1-3); (3) state `expected_tv: Option<i64>` for tokens and the `require_scope` status code (W1-5); (4) name the OIDC-mode harness for the two-listener test (W1-6). With those, WS1 is the most executable part of the document.

**Workstream 2 — NOT PLAN-READY.** Blocking: Decision 1 (apply control), Decision 2 (shared write/preview service), Decision 3 (listener), plus the apply/write transaction-atomicity sentence (W2-2). Non-blocking but required before the relevant task starts: ISIN location (W2-6), gate-script identity (C-4), `get_performance` field policy (W2-8), k8s exposure route (W2-4), isolation harness (W2-5). Decisions 1 and 2 are the operator's to make; Decision 3 is a four-hour spike that can run in parallel with all of Workstreams 0 and 1, so nothing outside WS2 needs to wait for it.

---

## Orchestrator adjudication

Claims checked against the tree; all confirmed:

| Claim | Verdict |
|---|---|
| `TransactionCreate` at `schemas.rs:18`, not `models.rs` | ✅ spec citation is WRONG |
| `Cargo.toml` has no direct `hyper`/`tower`/`http`/`hyper-util` | ✅ grep returns nothing — "rmcp is the only new dependency" is false at manifest level |
| `route_coverage_test.rs:291` `PROTECTED_ROUTES.len() == 21` | ✅ exact |
| `route_coverage_test.rs:304` `with_level == 15`, message "six routes authenticate without checking an id" | ✅ exact |
| `access_gate_test.rs:87` `"handlers inspected: 21"` | ✅ exact |
| Service targets the frontend container | ✅ `app-service.yaml` `targetPort: 80`; deployment has `frontend` (`containerPort: 80`, probes 80) and `backend` (`containerPort: 8000`, probes 8000); `frontend/nginx.conf:34-35` `location /api { proxy_pass http://127.0.0.1:8000; }` |

The W2-4 finding is the most consequential single fact in this stage and it materially changes the spec's deployment section: **the Kubernetes Service does not point at the backend at all.** The spec's one-line "the ingress needs a route for the second port" (SY-16) understates the work by an order of magnitude, and the choice between an nginx `location` (frontend image rebuild) and a new Service port (own TLS and policy decision) is an operator decision the document does not know it has to make.
