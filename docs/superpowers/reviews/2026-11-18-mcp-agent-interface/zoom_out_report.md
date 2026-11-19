# Stage 5 — Initial Zoom Out Review

* **Target:** `docs/superpowers/specs/2026-11-18-mcp-agent-interface-design.md`
* **Ranges:** full file, 1–556
* **Reviewer:** `egm-zoom-out`, fresh context, `zoom-out-standard.md`
* **Date:** 2026-11-18

Note on the swarm: the companion `egm-inventive-catalyst` persona is not registered in this
harness, so Stage 5 ran with a single reviewer.

## Assessment

This is an unusually well-argued design: the root problem is stated in one sentence, the
three architectural constraints are operator-mandated and honestly costed, the non-goals are
unusually decisive (no `backup` scope, no stdio, no resources/prompts, no `asset_type`
constraint), and the safety instinct — *the deterministic gate sits after the model, in the
backend* — is exactly right for a system where a misread `1.5` becomes a plausible portfolio
and a wrong tax report. The Workstream 0 prerequisite is genuinely valuable on its own merits
and I would ship it even without the MCP module. Context sufficiency passes comfortably.

The problem is that the document's factual base is shakier than its reasoning, and where it
is wrong it is wrong in the direction that hides work. Four load-bearing claims do not
survive contact with the repository: the "no new HTTP framework" claim rests on `tower
0.6.11` being in the lockfile, and it is `tower 0.5.3` (`Cargo.lock:3790-3792`); the
money-as-string prerequisite rests on "`frontend/src/utils/decimal.ts` already sends
strings", and that file is a read-side normaliser that sends nothing, while the browser CSV
importer genuinely sends JSON numbers (`CsvImportModal.tsx:170-176`,
`csvValidator.ts:187-189`); the bundle-schema constraint is already implemented and
CI-diffed (`docs/backup-schema/v1.json:134-142`, `backup/schema.rs:35-40`,
`ci.yml:250-252`); and the ISIN justification points at the browser while ignoring a Rust
validator already on two write paths (`api_routes/lookup.rs:186-199`). Each of these
understates the blast radius or the remaining work, and one of them (money-as-string) breaks
a live feature.

The deeper structural gap is altitude, not accuracy. The design asserts that agent tools
"call the same `engines::` and `services::` functions the Rocket handlers call" — but the
transaction and asset write paths are not in `engines::` or `services::`; they are inline in
`api_routes/transactions.rs:221-262` and `:47-98`. So the single most important interface in
the whole design (what `staging::apply` actually calls to write a row) is unspecified, while
the document spends its detail budget on full DDL, a decimal regex and an `UPDATE …
RETURNING` statement. That inversion is the one thing I would fix before this goes to a
plan: the design needs a named shared-write-service refactor, not more SQL.

Finally, the design is bigger than its own success criteria. Nothing in Success criteria or
Requirements asks for batched update/delete of portfolios, assets and transactions, yet
`stage_edit` is a second full CRUD API that roughly doubles the surface of the one path
where a wrong write is a wrong tax report. A fifth gate script duplicates rules the existing
gate already implements. A preview renderer is re-specified in new words when
`backup/detail.rs` already builds one. Cut those three and the design gets smaller *and*
stronger.

## Findings

| ID | Severity | Location | Finding | Why it matters |
|---|---|---|---|---|
| ZO-1 | 🔴 Blocker | L396–405 (`tools.rs`), L405 ("Apply. Re-validates…") | The claim that tools "call the same `engines::` and `services::` functions the Rocket handlers call" is false for the write path. Transaction creation lives inline in `api_routes/transactions.rs:221-262` (`require_asset_in` at `:221`, asset read `:225-235`, SPLIT ratio check `:239-244`, fee zeroing `:244`, date bound `:248-252`, INSERT `:254-262`); asset creation likewise at `:47-98` (ISIN check `:49`, duplicate-ISIN `:55-65`). Neither is in `engines::` or `services::`. | The document's central safety claim — the deterministic gate is in the backend and *reuses rather than re-invents* (L171-173) — is unfunded. Either the agent path duplicates that validation (two write paths that drift, which is precisely the failure Workstream 0 exists to prevent) or an unmentioned refactor extracts a shared write service. This is the design's main open interface and it is absent. |
| ZO-2 | 🔴 Blocker | L166–170 (WS0 item 4) | "`frontend/src/utils/decimal.ts` already sends strings" is refuted twice. `frontend/src/utils/decimal.ts` is read-side only — every export is a `normalize*` function converting responses to JS numbers (`:20-23 toNumber`, `:96-104 normalizeTransactionList`); it contains no request code. And the browser CSV importer **does** send JSON numbers: `CsvImportModal.tsx:170-176` posts `quantity: row.quantity` where `ValidatedRow.quantity/price/fee: number` (`csvValidator.ts:187-189`, built by `parseNumeric` at `:247-249`); the demo seed at `App.tsx:509,537` does the same. `TransactionCreate` (`schemas.rs:18-27`) uses `decimal_json`, whose `visit_f64` arm (`db_types.rs:56-59`) is exactly what those callers depend on. | Rejecting JSON numbers "outright" on the shared `TransactionCreate` breaks the existing browser CSV importer and the seed path — a live behaviour change that contradicts Requirement 5 (L66) and the document's own "No existing endpoint changes behaviour" preamble (L6). Strictness must be scoped to the agent DTOs, or a frontend change must land first and be stated. |
| ZO-3 | 🟡 Major | L161–165 (WS0 item 3), L529 | "the bundle schema … must constrain `type`" — already done. `docs/backup-schema/v1.json:134-142` already carries `"enum": ["BUY","SELL","SPLIT"]`. It is generated (`backup/schema.rs:3-5`) from `KNOWN_TRANSACTION_TYPES` (`backup/mod.rs:222`) via `transaction_type_schema` (`backup/mod.rs:244-245`) and `enumerated_string` (`:226-229`); the validator already rejects an unknown type (`backup/mod.rs:467-470`, test at `:871-884`); CI already diffs the artifact (`ci.yml:250-252`, drift test `backup/schema.rs:35-40`). | Workstream 0 item 3 and the WS0 test bullet "a restore bundle with a junk type is a `422` at preview" are no-ops. Specifying already-shipped work as prerequisite work inflates the plan and will read as a defect when an implementer finds it done. |
| ZO-4 | 🟡 Major | L144–150 (WS0 item 1) | A new `TxType` enum in `models.rs` would be the **second** definition of the same closed set. `backup::KNOWN_TRANSACTION_TYPES` (`backup/mod.rs:222`) is already the single source feeding the bundle schema and validator, and `frontend/src/types.ts:13` is a third. `backup/mod.rs:231-237` documents this exact drift class as a bug that was just fixed for `asset_type`. | Introducing a second source of truth for the same vocabulary re-opens a bug the codebase has already named and closed. Pick one source and derive the other; the document should say which. |
| ZO-5 | 🟡 Major | L349–351 | "ISIN … a mod-10 checksum implemented in Rust. `isinValidator.ts` lives in the browser and this path cannot depend on it." A Rust ISIN validator already exists and is already on two write paths: `api_routes/lookup.rs:186-199` (`is_valid_isin`), used by `api_routes/transactions.rs:5,49` and `backup/mod.rs:440-444`. (It is shape-only — length 12 + alphanumeric — not mod-10; and the browser's `isValidIsin` is a bare regex, `frontend/src/utils/isinValidator.ts:1-3`, so neither checks a checksum today.) | A second, stricter validator in `agent/protocol.rs` gives the repo two ISIN rules: the agent would reject rows the REST write path and the restore both accept. Decide once — strengthen `is_valid_isin` centrally so all three paths get it, or do not add it to the agent path. |
| ZO-6 | 🟡 Major | L99–101 | "`tower 0.6.11` … already in `Cargo.lock`" is refuted. The lockfile contains exactly one `tower`, at **0.5.3** (`Cargo.lock:3790-3792`). The other three check out: `hyper 1.10.1` (`:1349-1351`), `hyper-util 0.1.20` (`:1385-1388`), `http 1.4.2` (`:1258-1260`). | "No new HTTP framework is added — `rmcp` is the only new dependency" is the argument that makes the second-listener design cheap. If `rmcp` requires tower 0.6, the design adds a second major version of an HTTP framework — the thing L100-101 claims is not happening. The Verification list hedges on `hyper` (L549-551) and `schemars` (L551-552) but not on `tower`. |
| ZO-7 | 🟡 Major | L136–139 | "`DIVIDEND` is stored and the tax engine reads it as a positive-quantity lot." Refuted. Both engines use an `if / else if` chain over the three literals — `tax_engine.rs:665,680,771` and `stats_engine.rs:250,253,256` — so an unknown type falls through every branch and is a silent no-op, not a positive lot. | The real defect (a ledger row that renders in the UI and moves no figure) still justifies Workstream 0, but the stated hazard overstates the tax damage and is the sentence a skeptic will use to dismiss the migration. Fix the description, keep the fix. |
| ZO-8 | 🟡 Major | L89–91, L152–160 | The proposed `CHECK` is case-insensitive (`upper(btrim(type))`) but the existing unique index is `CREATE UNIQUE INDEX uq_split_asset_date ON transactions (asset_id, ((date AT TIME ZONE 'UTC')::date)) WHERE type = 'SPLIT'` (`migrations/0001_init.sql:89-91`) — a **case-sensitive** predicate. Rows stored as `split` are invisible to that index today; the normalization step at L153-156 pulls them into it. | The migration can fail on a unique-index violation rather than the intended `CHECK` violation, and the pre-flight audit query (L157-159) looks only for out-of-set values — never for post-normalization `(asset_id, day)` duplicates. Two notions of "is a SPLIT" in one schema is exactly what `check_migrations.sh` exists to catch. |
| ZO-9 | 🟡 Major | L490–496 | A **fifth gate script** `scripts/check_agent_gate.sh` plus `tests/agent_gate_test.rs` plus a new AGENTS.md/CI entry, to check that `#[tool]` handlers take a caller and call `require*`. `check_access_gate.sh` already implements exactly those rules (Rule A/B/C at `:8-14`, marker at `:79`, handler scan at `:169-184`) and already parameterises its target directory (`HANDLER_DIR` at `:72`). Its own comments say the isolation matrix, not the script, is the real gate (`:26-27`, `:44-45`) — a point the MCP document itself concedes at L488-489. | Two gate scripts that must agree about the same three rules is a new drift surface with two mutation tests to maintain. Extending the existing script with a second directory and a second attribute pattern delivers the same guarantee at a fraction of the long-term cost. |
| ZO-10 | 🟡 Major | L376, L408–424 | `stage_edit`: batched create/update/delete of portfolios, assets and transactions, with six operation kinds, per-element scope matching and delete-echoed-by-name. Nothing in Success criteria (L46-58) or Requirements (L60-69) asks for it — they name read questions and full-portfolio import. | It is a second full CRUD API duplicating `/api` routes that already exist and already pass the access gate, and it roughly doubles the surface of the one path where a wrong write is a wrong tax report. This is the largest YAGNI item in the document. |
| ZO-11 | 🟡 Major | L353–361, L404 | "The preview" re-specifies a renderer that already exists. `backend_rust/src/backup/detail.rs` already implements the described discipline: display caps as constants (`:20 MAX_PORTFOLIOS_SHOWN = 25`, `:22 MAX_ASSETS_SHOWN = 50`), stable flag codes (`:27-33`: `no_assets`, `no_transactions`, `zero_price`, `future_dated`, `currency_mismatch`, `duplicate_symbol`), and `PlanPortfolio`/`PlanAsset` carrying exact counts over every row plus `shown`/`more_assets` (`:35-62`, tests at `:575-585`). | The MCP section invents a parallel anomaly vocabulary ("unusual price scale", "an assumed currency", "duplicate-looking row") and a second cap regime (25/200/5000). Two preview vocabularies for one product means the agent chat and the web UI will describe the same anomaly differently. Reuse the types and the `flag` codes. |
| ZO-12 | 🟡 Major | L455–465, L553–554 | The entire safety story for "module, not process" rests on `catch_unwind` plus a 30-second per-call deadline — and the document's own Verification item admits it does not know whether `catch_unwind` is sufficient given tokio task panics. | That is not an API detail; it is the load-bearing assumption of the in-process decision. If the answer is "must run in its own spawned task", the design has rebuilt a poor man's process isolation inside the API process and the trade-off should be revisited on the record. Resolve before Workstream 2 is committed. |
| ZO-13 | 🟢 Nit | L166–170, L338–341 | A third decimal mechanism. `db_types::decimal_json` (`db_types.rs:21-67`) is already the shared string-or-number deserializer at ~55 call sites in `schemas.rs`; `Decimal::from_str_exact` is already the string-exactness rule (`backup/mod.rs:472,532`); and `rust_decimal` (`Cargo.toml:19`) already ships strict string serde modules. | A hand-written `^-?\d+(\.\d{1,12})?$` regex plus a bespoke visitor is a fourth spelling of "is this a decimal". Prefer `from_str_exact` plus a `scale() <= 12` check, in one place. |
| ZO-14 | 🟢 Nit | L200–217, L300–315, L327–331, L338 | Full DDL for both new tables, the exact decimal regex, the exact `UPDATE … RETURNING` statement and the exact serde attribute are plan altitude, not design altitude (zoom-out standard §3, Premature Detail) — while the one interface that actually matters (ZO-1) is unspecified. | Detail budget is spent where risk is low and withheld where risk is high. Move the SQL and regex down to the plan; lift the shared-write-service interface up into the design. |
| ZO-15 | 🟢 Nit | L234–235 | "`last_used_at` updates at most once a minute per token, fire-and-forget." On `replicas: 1` (`k8s/app-deployment.yaml:9`) with a single operator, a per-token write throttle plus a fire-and-forget task is premature optimization for a UI badge. | Just write it on every call, or drop the column and keep only the 90-day marker. The throttle is complexity bought for a bottleneck that cannot exist at this deployment's scale. |
| ZO-16 | 🟢 Nit | L262–266, L505, L509 | `tests/route_coverage_test.rs:291` hard-asserts `PROTECTED_ROUTES.len() == 21`, so adding three routes requires editing that count — the document says "must be added there" but not that the count is pinned. `check_migrations.sh:188-280` pins per-column types, not-null, identity and PK facts per table, so both new tables need entries; only the junk-`type` case is mentioned. "the other five gates" (L509) is also loose: `AGENTS.md:99-101` names five things needing `DATABASE_URL` and `cargo test` is one of them. | Small, but each is a CI failure an implementer discovers rather than is told. The document is otherwise scrupulous about exactly this class of thing (cf. L262-266). |

## What should be cut

* **`stage_edit` (L376, L408–424).** No requirement asks for it. Ship `stage_import` +
  `apply_staged` + `discard_staged` + `get_staged`; let edits go through the UI until a
  stated need exists. This is the single largest reduction in the design and it removes the
  whole "per-element scope matching + delete-echoed-by-name" sub-mechanism with it.
* **Workstream 0 item 3 (L161–165) and its test bullet.** Already implemented and
  CI-enforced (`docs/backup-schema/v1.json:134-142`, `backup/mod.rs:222,467-470`,
  `ci.yml:250-252`). Replace with one sentence: "the bundle schema already enumerates the
  set from `KNOWN_TRANSACTION_TYPES`; `TxType` must become that constant's source (ZO-4)."
* **`scripts/check_agent_gate.sh` + `tests/agent_gate_test.rs` as a separate gate
  (L490–496).** Extend `check_access_gate.sh` — it already has the three rules, the marker
  and a parameterised `HANDLER_DIR` (`:72`). One gate, one mutation test, one CI entry.
* **The bespoke decimal regex and deserializer (L166–170, L338–341).** Use
  `Decimal::from_str_exact` plus a scale check, or `rust_decimal`'s own strict string serde.
  Do not add a fourth spelling.
* **The second ISIN validator (L349–351).** Reuse and, if the checksum is wanted, strengthen
  `api_routes/lookup::is_valid_isin` in place so the REST path, the restore and the agent
  agree.
* **The new preview vocabulary and the second cap regime (L353–361, L404).** Reuse
  `backup::detail`'s `PlanPortfolio`/`PlanAsset` and `flag` codes; add the two genuinely new
  flags to that module rather than coining a parallel set.
* **The `last_used_at` write throttle (L234–235).** Write it plainly or drop the column.
* **The third scope.** `read` / `write` / `import` with `apply_staged` "requires the scope
  matching the batch's kind" (L377) puts scope logic in two places for a distinction that
  buys nothing — both kinds already pass the same human preview gate. Two scopes (`read`,
  `write`) is enough and one of the two failure modes the design fears disappears.

## What is missing at this altitude

1. **The shared write-service refactor.** The design never says what `staging::apply` calls
   to write a row. Given ZO-1, there is an unstated prerequisite: extract the validation and
   insert logic from `api_routes/transactions.rs:47-98` and `:221-262` into a service both
   the Rocket handlers and the agent tools call. That refactor is the actual mechanism by
   which "the gate sits after the model, in the backend" becomes true, and it belongs in
   Workstream 0 or at the head of Workstream 2.
2. **The `.sqlx` prepare-cache obligation.** Every new `sqlx::query!` requires
   `cargo sqlx prepare -- --all-targets` and a committed `.sqlx/` refresh, or
   `check_sqlx_cache.sh` fails CI (`docs/superpowers/plans/2026-11-16-import-preview.md:17`,
   `AGENTS.md:103-104`); the Dockerfile builds offline from that cache
   (`AGENTS.md:120-125`). The design adds many queries and mentions this in neither
   Sequencing (L527–540) nor the Verification items (L543–556).
3. **Forensic attribution on the staged batch.** `agent_staged_batch` records `user_id` only
   (L300–315). With a model in the loop, the fact you want after the fact is *which token
   staged this and which token applied it* — the logging section (L467) already records a
   token id for live calls, but the durable row cannot answer the question later. Two
   nullable `BIGINT` columns now; unrecoverable later.
4. **External-quota exposure of the read tools.** `get_performance` reaches
   `currency_service.get_price` (`engines/stats_engine.rs:195`), which consumes a
   rate-limited external provider quota (`services/currency_service.rs:577-600`; see also
   `lib.rs:179` on the per-symbol ISIN lookup quota). The repo treats this seam as
   gate-worthy (`scripts/check_pricing_seam.sh`, `AGENTS.md:94`). An agent in a retry loop
   can spend the day's quota; the 30-second deadline (L462) bounds latency, not quota. The
   document names the risk in passing ("a hung pricing lookup") and draws no conclusion.
5. **The Kubernetes path for the second port.** `k8s/app-service.yaml:12-16` is a
   single-port ClusterIP (80→8000), `k8s/ingress.yaml:7-17` routes only `/`, and the backend
   container declares one port (`app-deployment.yaml:58-60`). Exposing the listener needs a
   containerPort, a Service port and an ingress route; L523–526 mentions only the ingress.
   Separately, every probe targets 8000 (`app-deployment.yaml:84-99`), so a dead agent
   listener is invisible to the kubelet — state whether that is intended.
6. **Migration-number reservation across three independently shippable PRs.** Workstreams
   0/1/2 claim `0002_`, `0003_`, `0004_` (L152, L198, L299) while L527–540 makes them
   separate PRs. Three PRs racing for the same numbers with no stated reservation rule,
   against a gate that requires the ledger to hold one row per `*.sql` on disk
   (`check_migrations.sh` header, item 4).
7. **Any size discipline on the read side.** The write side gets four caps (L404); the read
   side gets the word "paginated" (L369) and nothing else. For a design whose whole thesis
   is that the model normalizes and the gate checks, there is no stated bound on what
   `list_transactions` can return into a model's context window, and no stated pagination
   contract.

## Reuse opportunities found in the repository

| Existing thing | Path | Use it for |
|---|---|---|
| `is_valid_isin` | `backend_rust/src/api_routes/lookup.rs:186-199` (used at `api_routes/transactions.rs:5,49`, `backup/mod.rs:440-444`) | The agent's ISIN check — strengthen in place if a mod-10 checksum is wanted, rather than adding a second validator (ZO-5). |
| `KNOWN_TRANSACTION_TYPES` | `backend_rust/src/backup/mod.rs:222`, consumed at `:244-245`, `:467-470` | The single source for the closed transaction-type set; make `TxType` derive from it or replace it, but do not run two (ZO-4). |
| `backup::detail` preview renderer | `backend_rust/src/backup/detail.rs:20-62` incl. `flag` module `:27-33`; tests `:575-585` | The staged-batch preview: display caps, exact-count-over-every-row shape, stable flag codes (ZO-11). |
| `db_types::decimal_json` / `Decimal::from_str_exact` | `backend_rust/src/db_types.rs:21-67`; `backup/mod.rs:472,532` | The money/quantity boundary rule — one mechanism, not a new regex (ZO-13). |
| `check_access_gate.sh` | `backend_rust/scripts/check_access_gate.sh` (`HANDLER_DIR` `:72`, marker `:79`, rules `:8-14`, scan `:169-184`) | Extend for `#[tool]` handlers instead of a fifth gate script (ZO-9). |
| `tests/isolation_test.rs` + `tests/access_gate_test.rs` | `backend_rust/tests/` | The document already commits to both; the isolation matrix is the real gate per `check_access_gate.sh:26-27,44-45`, which is the argument for *not* adding a parallel script. |
| `src/test_db.rs` | `backend_rust/src/test_db.rs` | Correctly cited at L475 — one real Postgres per test; no change needed. |
| `auth::guard::resolve_caller` | `backend_rust/src/auth/guard.rs:99-157` (user lookup `:128`, `is_active` `:140`, `token_version` `:145`) | Correctly reused. Note `guard.rs:181` already documents "An `Authorization` header is never consulted, only ignored" — that comment is the strongest available evidence for the two-listener invariant and should be cited by it. |
| `BackupPanel.tsx` | `frontend/src/components/BackupPanel.tsx` | Correctly cited as the shape for `AccessTokensPanel.tsx`; the nav-rail anchor at `App.tsx:657` checks out. |
| Conditional-mount precedent | `backend_rust/src/lib.rs:269` (`let oidc_routes = matches!(auth, AuthMode::Oidc(_));`) | Correctly cited as the precedent for `AGENT_LISTEN` gating. |
| `csvValidator.ts` closed set | `frontend/src/utils/csvValidator.ts:99` | Correctly cited — the `BUY\|SELL\|SPLIT` contract is already enforced in the browser. |
| `check_migrations.sh` column pinning | `backend_rust/scripts/check_migrations.sh:188-280` | Extend with `api_token` and `agent_staged_batch` column/identity/PK facts (ZO-16). |
| `backup/schema.rs` + CI diff | `backend_rust/src/backup/schema.rs:3-5,35-40`; `.github/workflows/ci.yml:250-252` | The established pattern for a generated, drift-checked artifact — the model to follow if tool input schemas are ever published as an artifact. |
| `frontend/src/types.ts` | `frontend/src/types.ts:13` (`TransactionType = 'BUY' \| 'SELL' \| 'SPLIT'`) | Keep in lockstep with `TxType`; it is already the third copy of the set. |

## Orchestrator verification note

Every refutation above was re-checked in the worktree before this report was filed, and all
of them held: `Cargo.lock` carries `tower 0.5.3` only (the spec's `0.6.11` came from a
malformed grep that matched `tower-http`); `docs/backup-schema/v1.json` already enumerates
`BUY|SELL|SPLIT`; `frontend/src/utils/decimal.ts` exports only `toNumber` and `normalize*`
read-side helpers; `CsvImportModal.tsx:170-176` posts numeric `quantity`/`price`/`fee`;
`migrations/0001_init.sql:89-91` predicates the SPLIT unique index on case-sensitive
`type = 'SPLIT'`; and `route_coverage_test.rs:291` pins `PROTECTED_ROUTES.len() == 21`.
