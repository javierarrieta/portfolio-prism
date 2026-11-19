# Stage 7 — Critic Review

* **Target:** `docs/superpowers/specs/2026-11-18-mcp-agent-interface-design.md`
* **Ranges:** FeatureRange 1–129, read against the full file
* **Reviewers:** `egm-design`, `egm-structural`, `egm-style` — fresh context each, `feature-spec-standard.md`
* **Date:** 2026-11-18

---

# Part 1 — egm-design (hostile design critic)

## Assessment

The design is unusually well-sourced and mostly correct about the code it touches — but its
**central safety claim is not enforced by anything in the design**. The whole document is
justified by "the agent may propose, it may not silently write" (lines 20–22, 34–40). Yet
`apply_staged` is an ordinary tool whose scope requirement is "matches batch kind" (line
465), so a token minted with `import` can stage and apply in the same turn. There is no
`apply` scope, and the Non-goals explicitly remove the only out-of-band channel: "No UI
panel for staged batches in v1. The preview is read in the agent chat" (line 74). The human
gate is therefore (a) optional at the credential level and (b) delivered *by the party whose
output is under review*. The document's own threat — a model that reads `1.5` as `15` and
produces "a plausible portfolio" (lines 36–40) — is precisely the failure a chat-relayed
preview cannot catch: the human sees a plausible `15.00`, not the literal the model read.
Nothing in the preview schema carries the source token. This is the design's load-bearing
wall and it is a convention, not a control.

Second, **Workstream 0 is presented as a no-frontend-change formalization and it is a
breaking change to a live feature**. The claim "`frontend/src/utils/decimal.ts` already sends
strings" (line 108) is a misreading: `decimal.ts` is a *response* normalizer that converts
money to JS `number` on read (`normalizeTransactionList`,
`frontend/src/utils/decimal.ts:96–104`). The browser CSV importer posts JSON **numbers**
(`frontend/src/components/CsvImportModal.tsx:170–176`, fed by
`frontend/src/utils/csvValidator.ts:247–249` → `parseFloat`). A deserializer that "rejects a
JSON number outright" (lines 105–108) breaks the importer the document says it is
formalizing, and contradicts Requirement 5 (line 63). The same problem hits the `TxType`
enum: today `transactions.rs:239` accepts `"buy"`; after the change it is a `422`, while the
migration simultaneously declares case non-semantic by normalizing with `upper(btrim(type))`.

Third, the design **prices the in-process decision only in panics**. It budgets for
`catch_unwind` and a 30-second deadline (lines 505–511) and never mentions the three
interference channels that a bulk import actually produces: the shared `PgPool` is
`max_connections(5)` with a 5 s acquire timeout (`backend_rust/src/db.rs:18–44`), the
container is capped at `cpu: 500m / memory: 256Mi` (`k8s/app-deployment.yaml:72–77`), and
the liveness probe hits the same process (`k8s/app-deployment.yaml:89–94`). A
5,000-transaction stage + preview + JSONB round-trip plus a tax-summary call that does
per-transaction FX lookups can exhaust the pool and the CPU quota, and the failure lands on
the human's browser session as 503s or a kubelet kill — the exact coupling the "module, not
process" decision creates. The caps (25/200/5,000) are stated as protection but were never
checked against these numbers, and there is no aggregate or per-token rate limit anywhere:
the realistic runaway is an agent that stages 5,000-row batches in a loop, and the only
backstop is a human who may not be looking.

Fourth, several factual claims that the design uses as justification do not survive the
repository. The `DIVIDEND` example is wrong about the mechanism (`tax_engine.rs:665`
compares `== "BUY"`, so an unknown type matches **no** branch and is silently dropped from
all tax math — a different and arguably worse bug than the one described). Workstream 0 item
3 is already implemented (`docs/backup-schema/v1.json:134–141`, generated from
`backup/mod.rs:222/244`, pinned by `backup/schema.rs:49–66`). The dependency posture claim is
wrong on `tower` (lock has 0.5.3, not 0.6.11). And the case-insensitive `CHECK` interacts
badly with the partial unique index `uq_split_asset_date ... WHERE type = 'SPLIT'`
(`migrations/0001_init.sql`), which `check_migrations.sh:171–174` pins in its case-sensitive
form — the new CHECK admits a row class that escapes the dedupe that exists to prevent bug
#50.

None of this is an argument against the operator's five chosen constraints. Every finding
above is about consequences the design asserts but does not deliver, or costs it does not
record. The document is close to shippable; findings DS-1 through DS-4 are not.

## Findings

| ID | Severity | Location | Finding | Why it matters |
|---|---|---|---|---|
| DS-1 | 🔴 Blocker | lines 74, 465, 55–57 (scope list) | The scope lattice cannot express "may stage, may not apply". Scopes are `read`/`write`/`import`; `apply_staged` "requires the scope matching the batch's kind". An `import`-scoped token stages and applies in one turn with no second factor. | The document's thesis — "the agent is allowed to propose. It is not allowed to silently write" — is unenforceable as designed. Add an `apply` scope (or require `Credential::Session` for apply), and make the mint UI not grant it by default. |
| DS-2 | 🔴 Blocker | lines 20–22, 34–40, 74, 443–452 | The human review is mediated by the untrusted proposer, and the preview carries no provenance. "The preview is read in the agent chat" means the model relays, may truncate ("showing 25 of 1,204") and may summarize its own proposal; `get_staged` re-renders through the same channel. No preview field carries the source literal the model read. | The stated central risk is a *plausible* misread. A preview that shows `15.00` cannot distinguish it from `1.5`; the human has no evidence to compare. Add a per-row `source_text` echo for flagged rows, and at minimum a read-only pending-batch affordance outside the agent. |
| DS-3 | 🔴 Blocker | lines 63, 105–108, 118 | Money-as-string enforcement breaks the existing browser CSV importer. `CsvImportModal.tsx:170–176` posts `quantity`/`price`/`fee` as JSON numbers (`csvValidator.ts:247–249` → `parseFloat`); `decimal.ts:96–104` is a read-path normalizer, not a send path. Only the manual form (`PortfolioDetail.tsx:15–20,171–177`) sends strings. | Requirement 5 ("every current `/api` route keeps working unchanged") is violated by Workstream 0 as written, and the evidence offered for "no frontend change required" is a misread file. Either migrate the importer first, or accept numbers with an explicit `Decimal` path that bypasses `f64`. |
| DS-4 | 🔴 Blocker | lines 486–490 (merge/dedupe), 105–108 (scale rule) | Exact-duplicate detection on `TEXT` money columns is scale-sensitive. Stored values preserve input scale byte-for-byte (the design's own test requirement, line 528), so a re-imported `10` will not string-match a stored `10.0`. The browser precedent cited is also not the same rule: `CsvImportModal.tsx:164–168` dedupes in-memory within one import run and never compares against rows already in the database. | The flagship guarantee — "re-importing an updated statement does not double the ledger" — silently fails for the common case of a differently-formatted re-export, and the preview will show no duplicate flag. Define duplicate matching as `Decimal` equality (and state whether `fee`/`date` participate). |
| DS-5 | 🟡 Major | line 461 (`get_performance`), stats_engine.rs:323–329 | The honesty warning covers volatility/Sharpe/beta but not `unrealized_pnl` and `realized_pnl`, which are hardcoded `"0.0"` in the non-empty path (`stats_engine.rs:328–329`); `AGENTS.md:63–68` documents only the first four. | Success criterion #1 is "answer 'what is my realized gain in 2025?'". A model that calls `get_performance` instead of `get_tax_summary` gets a confident `0.0`. Either strip those fields from the tool or make the tool description name them too. |
| DS-6 | 🟡 Major | lines 130–134 ("module not process"), 505–511 | No connection or concurrency budget for the agent listener. `db.rs:18–44` sets `max_connections(5)`, `acquire_timeout(5s)`, `statement_timeout=30s`; tools call `engines::` directly, and `tax_engine::replay` does a per-transaction FX call (`tax_engine.rs:648–656`). | A handful of concurrent agent reads starves the human's session to 503s on a `replicas: 1` deployment. The in-process decision's real cost is pool contention, which the fault-isolation section does not mention. Add a semaphore (1–2 concurrent tool calls) or a reserved pool. |
| DS-7 | 🟡 Major | lines 496–499 (hard caps), 505–511 | Caps were never checked against the container: `k8s/app-deployment.yaml:72–77` gives the backend `cpu: 500m / memory: 256Mi`. A 5,000-transaction payload exists as DTO graph + canonical JSONB + rendered preview JSON + the `serde_json::Value` of both, inside a process that also serves the API. Rocket's per-route body limits (`lib.rs:272–275`) have no counterpart on a raw `hyper` listener, so the cap is enforced only after full deserialization. | OOM-kill or CPU starvation takes the API down with it — the same blast radius the design claims to have paid for, but unmitigated. Add an explicit body-size limit on the listener and re-derive the caps from the memory limit. |
| DS-8 | 🟡 Major | lines 419–423, 499–503 | Ordering of the single-use flip versus the write is unspecified. "Single use is a conditional update" is described as a step, then "Apply … writes in one Postgres transaction". If the flip commits before the write, a failed or deadline-killed write (the 30 s deadline equals the DB `statement_timeout`) leaves a batch marked `applied` with zero rows written and no way to retry. | Unrecoverable data loss on the one path a human approved. The flip and the write must be the same transaction, and the doc should say so in one sentence. |
| DS-9 | 🟡 Major | lines 96–101 (case-insensitive CHECK) | The `CHECK (upper(btrim(type)) IN (...))` is looser than every writer and inconsistent with `uq_split_asset_date ... WHERE type = 'SPLIT'` (`migrations/0001_init.sql`), which is case-sensitive and is *pinned* case-sensitively by `check_migrations.sh:171–174`. A `'split'` row passes the new CHECK and escapes the one-SPLIT-per-asset-per-day dedupe that exists to prevent bug #50. | The design calls case-insensitivity a feature without noting it weakens an existing guarantee, and the gate that would catch the fix is itself pinned to the broken form. Either make the CHECK exact after normalization, or change the index predicate and the gate together. |
| DS-10 | 🟡 Major | lines 88–95, 118 | The `TxType` enum makes lowercase input a `422` on a live endpoint (today `transactions.rs:239` uppercases). The migration simultaneously treats case as non-semantic by rewriting rows with `upper(btrim(type))`. `src/access.rs:260` seeds a lowercase `'buy'` row, which is presumably why the CHECK had to be case-insensitive. | An unstated breaking API change, and an internal contradiction: case is non-semantic for stored data but semantic at the boundary. State the client impact and either accept case-insensitive deserialization or declare the break. |
| DS-11 | 🟡 Major | lines 68–71 (backup non-goal), 508–513 (`stage_edit` deletes) | The backup non-goal is justified because `mode=replace` deletes everything and "stays in the web UI" with a typed-word confirm. But `stage_edit` carries `delete_portfolios`, `delete_assets`, `delete_transactions`, confirmed only by "echoed back by name" — text the model itself types from the preview it generated. | The rationale that excludes `backup` argues against agent-side deletes, and the confirmation is *weaker* on the more autonomous surface. Ship `stage_edit` as create/update in v1 and keep deletes in the UI. |
| DS-12 | 🟡 Major | lines 415–423, 543–546 | No retention and no aggregate limit. `payload` and `preview` JSONB (up to 5,000 transactions each) are retained forever after `applied`/`discarded`; the sweep runs only "when that user stages the next batch"; there is no per-user or per-token rate limit. | A looping agent is the realistic failure mode, not the oversized single call. Postgres growth is unbounded and the only brake is human attention. Add a per-user pending-batch cap, a rate limit, and a retention rule for terminal rows. |
| DS-13 | 🟡 Major | lines 44–49, 136–143 | Dependency claims are wrong or unproven. `tower 0.6.11` is **not** in the lock — `Cargo.lock:3792` has `tower 0.5.3` and no 0.6. `schemars` already appears twice (`Cargo.lock:2867` 0.9.0, `:2879` 1.2.3) against `Cargo.toml:20`'s `schemars = "1.0"`. `rmcp` is absent from the lock entirely. | "No new HTTP framework is added — `rmcp` is the only new dependency" is the sentence that buys the supply-chain argument, and it is inaccurate. The `schemars`/`tower` questions are unresolved version-resolution risk, not implementation-time trivia. |
| DS-14 | 🟡 Major | lines 549–556 (Configuration), 570 (sequencing step 4) | The exposure path is one line. `k8s/app-service.yaml:12–15` maps 80→80 only; `k8s/ingress.yaml` routes `invest.example.com` path `/` to that service and carries no TLS block. Exposing a second port means Service + Deployment `containerPort` + Ingress changes, a TLS decision, and — if it is a path on the same host — whatever provider-side auth policy the host already carries. | "Enable `AGENT_LISTEN` and mint the first token" is not a deployable step. Also unstated: whether the listener binds cluster-internal only, and what happens if the port fails to bind at startup. |
| DS-15 | 🟡 Major | lines 517–541 (Testing and gates) | Two CI gates are omitted. `cargo sqlx prepare --check` and `check_sqlx_cache.sh` (`AGENTS.md:93–104`) require the committed `.sqlx/` prepare cache to be regenerated for every new `sqlx::query!` in `auth/pat.rs` and `agent/staging.rs` — the image builds offline from that cache. `check_migrations.sh` pins "all 36 columns of the six tables" and will need entries for `api_token` and `agent_staged_batch`. `tests/route_coverage_test.rs:291` asserts `PROTECTED_ROUTES.len() == 21`. | The document enumerates new gates in detail while missing that existing gates will fail on the new code. Cheap to fix, embarrassing in CI. |
| DS-16 | 🟡 Major | lines 443–452, 74 | No token economics. The preview must be read in chat, but no size budget is stated; the reused display caps (25 portfolios / 50 assets per portfolio, `2026-11-16-import-preview-design.md:68–69`) plus an anomaly list can be tens of thousands of tokens, and `apply_staged` requires the model to echo names back from it. `list_transactions` page size is unspecified. | The cost of the feature to the operator's LLM budget is unquantified, and an over-large preview degrades exactly the review step the design depends on. |
| DS-17 | 🟢 Nit | lines 31–34, 96 | Citation errors: `to_uppercase()` is at `transactions.rs:239`, not `:238`; "the only three literals in `src/`" ignores `backup/mod.rs:222` `KNOWN_TRANSACTION_TYPES`, which is already the enforced set. | Small, but this document is the citation others will trust. |
| DS-18 | 🟢 Nit | lines 100–103 (WS0 item 3) | Already implemented: `docs/backup-schema/v1.json:134–141` enumerates `type`, generated from `backup/mod.rs:222/244`, validated at `backup/mod.rs:467–470`, pinned by `backup/schema.rs:49–66`. | Workstream 0 is scoped partly on work that is done; the real remaining gap is the case-sensitivity mismatch in DS-9. |
| DS-19 | 🟢 Nit | line 344 (zero price) | The justification cites the import-preview design's stock-dividend rationale (`2026-11-16-import-preview-design.md:65`), which in turn cites `docs/roadmap/stock-dividend-transactions.md` — a file that does not exist in the repo — while `2026-11-09-tax-report-and-statement-design.md:123` states the app does not record dividends. | Accepting zero-price rows is defensible, but the cited authority is a roadmap for a flow the app cannot produce today. |
| DS-20 | 🟢 Nit | lines 419, 468 | TTL is "checked lazily at apply"; `get_staged` "re-renders a preview" with no stated expiry check. | A human can be shown a preview of an already-expired batch and then get `stale_handle`. State that `get_staged` applies the same predicate. |
| DS-21 | 🟢 Nit | lines 417–423 | The single-use `UPDATE ... WHERE handle=$1 AND status='staged'` omits `user_id`; ownership is a separate `require_own_staged_batch` read. | Unguessable handles make this safe today, but making ownership and state transition one statement removes a class of future mistake. |
| DS-22 | 🟢 Nit | lines 15–17, 44–49 | "There is no Node.js anywhere in the design" is true of the runtime path but not the repository (`frontend/package.json` drives the build, `openapi-typescript`, vitest, playwright). The real constraint is "no Node in the backend runtime image". | Stated too broadly, it makes the supply-chain rationale look like a preference rather than a scoped decision. |
| DS-23 | 🟢 Nit | lines 122–127 vs 150–152 | "A token yields a `uid` and falls through to that same lookup" (which keeps the `token_version` comparison) sits awkwardly with "tokens deliberately survive `logout-all`". `guard.rs:150–155` rejects on `tv != token_version`. | As written, a literal implementation either breaks the stated requirement or forks the lookup. State that the converged function takes `expected_tv: Option<i64>`, `None` for tokens. |
| DS-24 | 🟢 Nit | line 519 | "the other five gates" — `AGENTS.md:99` says five of eleven gates need `DATABASE_URL`, of which four are not `cargo test`. | Precision only. |

## Rejected alternatives the document never considered

1. **rmcp in stateless / JSON-response mode, mounted inside Rocket.** The document rejects a
   Rocket↔tower adapter on the grounds that "hand-rolling SSE through a Rocket `Responder`
   fights the framework" (lines 142–143). But nothing in Requirements 1–6 needs
   server-initiated streaming — Non-goals already exclude resources, prompts, sampling and
   subscriptions (line 78). Streamable HTTP can answer each POST with a direct JSON response,
   which removes the SSE objection almost entirely and with it the entire cost stack the
   second listener creates: a second credential type, a second gate script
   (`check_agent_gate.sh`), hand-paid fault isolation, ingress work, and the loss of
   `check_access_gate.sh` as the single authoritative surface. Trade-off bought: one
   listener, one gate, Rocket's limits and probes for free. The document's own Verification
   item about `StreamableHttpServerConfig` "stateful mode and SSE keep-alive" (line 578) shows
   SSE was assumed, not required.
2. **Reusing the existing preview/restore machinery as the single staging path.**
   `POST /backup/preview` + `/backup/restore` already implement canonicalized
   preview-before-write with one validation module (`backup/mod.rs:467` enforces the closed
   type set today), `MAX_BUNDLE_BYTES`, a row-count ceiling and a SERIALIZABLE single
   transaction. Building `agent/staging.rs` as a second preview implementation guarantees two
   rule sets that drift — the document itself is already inconsistent with `backup/` about
   transaction types (DS-18). Trade-off bought: one validator, one preview renderer, one
   anomaly vocabulary; the operator's "no restore for agents" constraint is preserved by
   adding an append-only mode rather than a parallel subsystem.
3. **An `apply` scope, or apply restricted to `Credential::Session`.** The `Credential` enum
   already distinguishes session from token (lines 124–127); making apply require a session is
   a one-line policy that turns the human gate from a convention into a structural property,
   at the cost of needing some minimal UI affordance — which is alternative 4.
4. **A read-only pending-batch affordance in the existing UI** (a count and a rendered
   preview, no apply button). This is a strict subset of the deferred "UI panel" and is the
   cheapest way to break the "the proposer renders its own review" loop in DS-2.
5. **Per-row source provenance in the proposal DTO** (`source_text: "1.5"` echoed in the
   preview for flagged rows). The single highest-value control for the design's own stated
   threat, and it costs one optional field.
6. **A concurrency semaphore or reserved connection budget for the agent listener**, sized
   against `max_connections(5)`. Cheaper than a second process and directly protects the
   interactive surface.
7. **Rate limiting / per-user pending-batch caps** as part of the token model. The document
   explicitly reasons about throttling for *secret guessing* and correctly declines it (lines
   145–147), then never considers throttling for the request patterns that are actually cheap
   for an agent and expensive for the deployment.
8. **A narrow, deterministic dialect parser for the rows the preview flags**, rather than an
   absolute "no parser" non-goal. The non-goal is defensible as a v1 scope call, but the
   document never weighs the middle position where the model normalizes and the server
   re-checks only the flagged subset.

## Risks stated but not mitigated

* **"A model that misreads `1.5` as `15` produces a plausible portfolio and a wrong tax
  report, and nothing downstream would notice"** (lines 36–40). The named mitigation is the
  preview — which is chat-relayed (line 74), carries no source literal, and can be applied by
  the same token that staged it. This is the document's founding risk and it is not mitigated;
  see DS-1, DS-2.
* **"The listener runs in the same process as the API, so a panic in a tool handler would take
  the whole application down"** (lines 505–511). Mitigated for panics and hung calls only. Not
  mitigated: memory exhaustion against the 256 Mi limit, CPU starvation against the 500m
  limit, pool exhaustion against `max_connections(5)`, and liveness-probe failure caused by
  any of the three — all of which take down the API exactly as the quoted sentence fears,
  without being a panic. See DS-6, DS-7.
* **"LocalSessionManager is in-memory … acceptable only because all semantic state lives in
  Postgres"** (lines 528–533). Correct as far as it goes, but the same paragraph is the only
  place `replicas` is discussed, and the staged-batch table's growth (unbounded, swept only on
  next stage) is a Postgres-state risk that this reasoning does not cover. See DS-12.
* **"No `backup` scope exists, so it is not a policy an agent can forget — it is a capability
  that cannot be minted"** (lines 68–71). The principle is sound and then immediately not
  applied to `stage_edit`'s `delete_*` operations, which give an agent a strictly easier
  deletion path than the web UI's typed-word confirm. See DS-11.
* **"256 bits of entropy means there is no guessing attack, so no attempt throttling is added
  — stated so nobody adds one later"** (lines 145–147). Fine for the secret. The document
  generalizes from "no throttling for auth" to no throttling anywhere, and never revisits
  throttling for tool calls. See DS-12, DS-16.
* **Requirement 6, "an existing deployment must not grow a public MCP endpoint by upgrading"**
  (lines 61–63). Delivered by the `AGENT_LISTEN` default, but the corollary — that when an
  operator *does* set it, the second port becomes reachable through the same host/ingress as
  the app, with whatever auth policy that host carries — is left as one sentence in
  Configuration. See DS-14.

## Claims checked against the repository

| Claim (line) | Verdict | Evidence |
|---|---|---|
| `transactions.type` is `TEXT` with no `CHECK`; no Rust enum (lines 31–33) | Confirmed | `backend_rust/migrations/0001_init.sql:62–69` |
| `create_transaction` at `transactions.rs:238` does `to_uppercase()` (line 32) | Confirmed, off by one | `backend_rust/src/api_routes/transactions.rs:239` |
| A stored `"DIVIDEND"` is read by the tax engine "as a positive-quantity lot" (lines 33–35) | **Refuted** | `engines/tax_engine.rs:665,680,771` branch on `== "BUY"` / `"SELL"` / `"SPLIT"`; an unknown type matches no branch and is silently dropped. Same shape in `engines/stats_engine.rs:250–256`, `api_routes/analytics.rs:389` |
| "The only three literals in `src/` are BUY, SELL, SPLIT" (line 35) | Refuted | `backup/mod.rs:222` `KNOWN_TRANSACTION_TYPES`, enforced at `backup/mod.rs:467–470` |
| `quantity`/`price`/`fee` are `TEXT`; app stores `Decimal::to_string()` (lines 37–40) | Confirmed | `migrations/0001_init.sql:65–67`; `api_routes/transactions.rs:255–257` |
| `frontend/src/utils/decimal.ts` already sends strings (line 108) | **Refuted** | `frontend/src/utils/decimal.ts:96–104` is a read-path normalizer (`num()`); the CSV importer sends numbers: `components/CsvImportModal.tsx:170–176` + `utils/csvValidator.ts:247–249` |
| `csvValidator.ts:99` rejects everything outside BUY/SELL/SPLIT (line 92) | Confirmed | `frontend/src/utils/csvValidator.ts:98–101` |
| `docs/backup-schema/v1.json` must be made to constrain `type` (WS0 item 3) | **Refuted — already done** | `docs/backup-schema/v1.json:134–141`; generated from `backup/mod.rs:222,244`; pinned by `backup/schema.rs:49–66` |
| `backup/restore.rs:367` inserts the same column (line 99) | Confirmed | `backup/restore.rs:367`; case-sensitive SPLIT test at `:361` |
| `services/currency_service.rs:138` inserts literal `'SPLIT'` (line 100) | Confirmed | `services/currency_service.rs:139` |
| `check_migrations.sh` fails if those columns are not `text` (line 112) | Confirmed | `backend_rust/scripts/check_migrations.sh:239–242` |
| `hyper 1.10.1`, `hyper-util 0.1.20`, `http 1.4.2` already in the lock (line 138) | Confirmed | `Cargo.lock:1351,1387,1260` |
| `tower 0.6.11` already in the lock (line 138) | **Refuted** | `Cargo.lock:3792` is `tower 0.5.3`; no 0.6 present |
| "`rmcp` is the only new dependency"; "no new HTTP framework is added" (lines 139–140) | Unproven | `rmcp` absent from `Cargo.lock`; `schemars` already duplicated at `Cargo.lock:2867` (0.9.0) and `:2879` (1.2.3) vs `Cargo.toml:20` |
| 449 packages already in `Cargo.lock` (line 48) | Plausible, not counted | `Cargo.lock` is ~4.6k lines at ~9 lines/package |
| 21 existing route handlers, unchanged by the two-listener split (line 152) | Confirmed | 21 route attributes under `src/api_routes`; `scripts/check_access_gate.sh:48`; `tests/route_coverage_test.rs:291` |
| `check_access_gate.sh` anchors on `^#[(get\|post\|…)]` in `src/api_routes` and is blind to tool handlers (lines 531–535) | Confirmed | `scripts/check_access_gate.sh:46–51,72` |
| `access.rs::deny()` answers not-found, never forbidden (line 500) | Confirmed | `src/access.rs:76–81` |
| `resolve_caller` owns the conversion, keeps `is_active` and `token_version` in one place, keeps `Ok(None)` ≠ `Err` (lines 120–127) | Confirmed | `src/auth/guard.rs:100–160` |
| The `/api` surface never reads an `Authorization` header (lines 26–28, 148–152) | Confirmed | `src/auth/guard.rs:181–183`; `resolve_caller` reads only `session::read(req.cookies())` |
| `lib.rs:268` precedent: OIDC routes mounted only when a provider exists (line 407) | Confirmed | `src/lib.rs:268–269` |
| `stats_engine.rs:108–110` returns constants for volatility/Sharpe/beta (line 461) | Confirmed, and incomplete | `:108–110` (empty path) and `:323–325` (real path); `unrealized_pnl`/`realized_pnl` are also constants at `:328–329`; `AGENTS.md:63–68` |
| `replicas: 1` makes the in-memory session manager acceptable (lines 528–533) | Confirmed as stated | `k8s/app-deployment.yaml:9` |
| "The ingress needs a route for the second port" (line 556) | Understated | `k8s/app-service.yaml:12–15` (80→80 only), `k8s/ingress.yaml` (single host, path `/`, no TLS block) |
| Browser importer's duplicate rule is "the same rule" as the agent's merge-skip (lines 487–489) | **Refuted** | `components/CsvImportModal.tsx:164–168` dedupes in-memory within one run; it never compares against existing rows |
| `2026-11-16-import-preview-design.md` treats zero-price as valid stock-dividend output (line 344) | Partially confirmed | `2026-11-16-import-preview-design.md:65` says so and cites `docs/roadmap/stock-dividend-transactions.md`, which does not exist; `2026-11-09-tax-report-and-statement-design.md:123` says the app does not record dividends |
| Five existing gates need `DATABASE_URL` (line 519) | Confirmed | `AGENTS.md:99–101` |
| `tests/route_coverage_test.rs` holds `PROTECTED_ROUTES` and fails on drift (line 176) | Confirmed | `tests/route_coverage_test.rs:31,285–291` (also asserts `len() == 21`) |
| "This deployment's proxy already refused one oversized body (`af17064`)" (line 499) | Unverifiable | No git history access with read-only tools |
| Pool/timeout posture implied by "module not process" | Contradicts the design's silence | `src/db.rs:18–44`: `max_connections(5)`, `acquire_timeout(5s)`, `statement_timeout=30s`; `k8s/app-deployment.yaml:72–77`: `cpu 500m / memory 256Mi` |

## Orchestrator verification note

The reviewer's new load-bearing claims were re-checked in the worktree and all held:
`stats_engine.rs:328-329` hardcodes `unrealized_pnl` and `realized_pnl` to `"0.0"`;
`db.rs:20,44` sets `max_connections(5)` and a 5 s acquire timeout with
`statement_timeout = '30s'` at `:70`; `k8s/app-deployment.yaml:72–77` limits the container to
`cpu 500m / memory 256Mi`; `access.rs:260` seeds a lowercase `'buy'` row; `Cargo.lock` carries
`schemars` twice (0.9.0 and 1.2.3); and `check_migrations.sh:169–175` pins the SPLIT index
predicate literally as `type = 'SPLIT'`.

---

# Part 2 — egm-structural (structural validator)

## Assessment

The document has no heading-level violations — H1 → H2 → H3 is clean and sequential
throughout, both tables are well-formed with consistent cell counts, and all five code fences
are balanced. The structural problem is not correctness of the outline, it is **completeness
of the outline against the standard and against the repo's own sibling convention**. The
standard mandates an explicit alternatives evaluation and an explicit risks section; the two
sibling specs in the same directory both carry dedicated headers for them
(`2026-11-04-oidc-identity-isolation-design.md` has `## Decisions and Alternatives Considered`
and `## Risks and Mitigations`; `2026-11-16-import-preview-design.md` has `## Failure modes`
and `## Rollout`). This document has neither header. Rejection rationale exists but is buried
as a trailing `Rejected:` clause inside one of five bold-lead paragraphs in
`## Decisions and rationale`, and risk content is scattered across four unrelated sections. A
fresh reader scanning the heading list cannot locate "alternatives" or "risks" at all, and
cannot locate "the implementation plan" either, because the three `## Workstream N` headers do
not announce themselves as that section.

The second structural problem is **workstream asymmetry**. Workstream 0 carries zero H3
subsections and expresses its five decisions as a numbered list, one of which (item 4)
contains a nested second paragraph and so is structurally deeper than its siblings.
Workstreams 1 and 2 use 5 and 7 H3 subsections respectively, including a parallel
`### Schema — 000N_….sql` pair that Workstream 0 breaks despite owning migration `0002`. The
asymmetry is partly deliberate (Workstream 0 is a prerequisite change, not a module), but
nothing in the document says so, and the effect is that the three workstreams cannot be read
against each other. Separately, real content in Workstreams 1 and 2 is carried as bold-lead
paragraphs (`**Scopes:**`, `**Expiry**`, `**Handle rules.**`) rather than headings, so the
outline does not reflect the actual concept set.

The third problem is **duplication density**. The money-string regex
`^-?\d+(\.\d{1,12})?$` appears verbatim in two sections. The `AGENT_LISTEN` flag, the 60-minute
TTL, the scope model, the `access.rs` chokepoint rule, single-use apply semantics, and the
decimal/comma rejection rule each appear in three or more places with different wording and no
authoritative home. Several of these pairs already disagree: the dependency claim in
`## Decisions and rationale` ("no new HTTP framework is added — `rmcp` is the only new
dependency") is contradicted by the second Verification bullet, which asks whether `hyper`
needs a version bump. One paragraph contradicts itself internally: `pp_` + 32 random bytes
**base62** is roughly 190 bits, not the "256 bits of entropy" asserted two sentences later.

Finally, navigation. A 556-line, 27-heading document ships with no table of contents and zero
anchor links, while making three prose cross-references ("see *Sessions*", "See Workstream 0,
item 5", "the flagship, below"). Two source citations are already stale or path-ambiguous:
`api_routes/transactions.rs:238` is off by one (the `to_uppercase()` call is line 239), and
`stats_engine.rs:108-110` omits the `engines/` prefix that every other citation in the
document carries. `## Sequencing` asserts each workstream is "independently revertible" but no
rollback content exists anywhere in the document — an asserted property with no corresponding
section.

## Heading hierarchy

```
H1  L1    MCP Agent Interface Design
H2  L8      Overview                                   ← carries Problem Statement, no dedicated header
H2  L22     Why this shape                             ← architecture rationale, part 1 of 2
H2  L43     Success criteria                           ← overlaps ## Requirements (see ST-12)
H2  L58     Requirements
H2  L72     Non-goals                                  ← matches standard ("Non-Goals")
H2  L87     Decisions and rationale                    ← architecture rationale, part 2 of 2;
                                                          holds the only "Rejected:" clause
        ✗ MISSING (standard-mandated): Alternatives Considered
        ✗ MISSING (standard-mandated): Risks and Mitigations
        ✗ MISSING (repo convention):   Rollout & Rollback
        ✗ MISSING: any header naming the implementation plan
   --- L128
H2  L130    Workstream 0 — prerequisite: close the transaction-type and number boundary
        (no H3 children — content is a numbered list 1–5; item 4 nests a second paragraph)
        (no `### Schema` header despite owning migration 0002)
   --- L185
H2  L187    Workstream 1 — personal access tokens
H3  L189      The invariant
H3  L203      Schema — `0003_api_token.sql`
                (unheaded: **Scopes:**, **Expiry**, **`last_used_at`** — three concepts, no heading)
H3  L237      `auth/pat.rs` — the only implementation of every operation
H3  L247      `api_routes/tokens.rs` — session-authenticated
H3  L268      The Access screen
   --- L283
H2  L285    Workstream 2 — the agent module
H3  L287      Layout — `backend_rust/src/agent/`
                (unheaded: "Off by default: AGENT_LISTEN" — feature flag, no heading)
H3  L303      Schema — `0004_agent_staged_batch.sql`
                (unheaded: **Handle rules.**, **Single use is a conditional update**)
H3  L333      Validation, deterministic and server-side
H3  L353      The preview
H3  L364      Tool inventory
H3  L384      `stage_import`
H3  L422      Sessions
   --- L429
H2  L431    Error handling
   --- L470
H2  L472    Testing and gates
   --- L514
H2  L516    Configuration
H2  L527    Sequencing                                 ← no preceding `---`
H2  L543    Verification items to confirm during implementation   ← no preceding `---`
```

No level skips. No duplicate heading text. No out-of-order sections. `---` rules precede 6 of
the 8 post-front-matter H2 boundaries and are absent before the last two.

## Findings

| ID | Severity | Location | Finding | Why it matters |
|---|---|---|---|---|
| ST-1 | 🟡 Major | whole document; `## Decisions and rationale` L87–127 | No `Alternatives Considered` header. The standard's Alternatives Rule is mandatory. Rejection rationale exists only as a trailing `Rejected:` clause in the second of five bold-lead paragraphs; the other four decisions (module-not-process, server-side staging, opt-in expiry, self-service minting) state rationale but never name the path not taken. | A fresh reviewer cannot verify alternatives were evaluated without reading every paragraph linearly. Both sibling specs in the same directory carry this header, so the omission is a local convention break, not a house style. |
| ST-2 | 🟡 Major | `## Non-goals` L72, `### Sessions` L422, `## Error handling` L461, `## Verification items` L543 | No `Risks and Mitigations` header. Risk content is distributed across four sections with no aggregation: multi-replica session loss, in-process fault blast radius, migration-aborts-boot, live-data audit. | The standard requires the spec to call out its own constraints explicitly. A reader doing a risk pass has to reconstruct the risk register from four places, and will miss some. |
| ST-3 | 🟡 Major | headings L8, L22, L87, L130/L187/L285 | No `Proposed Architecture` header, and no header naming the implementation plan. Architecture is split across `Overview` / `Why this shape` / `Decisions and rationale` with no heading a fresh reader would look for; the three `Workstream N` headers are the plan but do not say so. | Heading text is the document's only navigation surface here (no TOC). Two of the standard's five required areas are undiscoverable from the outline. |
| ST-4 | 🟡 Major | whole document; L83, L85, L375 | No table of contents and no anchor links in a 556-line, 27-heading document. Three in-document references are prose pointers: "see *Sessions*" (italics used as a pseudo-link), "See Workstream 0, item 5", "the flagship, below". | The reader must scroll to build a mental map. Item-number references ("item 5") break the moment an item is inserted into that list. |
| ST-5 | 🟡 Major | `## Workstream 0` L130–184 vs `## Workstream 1` L187, `## Workstream 2` L285 | Workstream 0 has zero H3 subsections; its five decisions are a numbered list. Item 4 contains a nested second paragraph (**The `TEXT` columns are not to be changed**), making it structurally deeper than items 1–3 while rendering at the same level. Workstream 0 also owns migration `0002_transaction_type_check.sql` but has no `### Schema —` header, breaking the `0003`/`0004` pattern. | The three workstreams cannot be compared section-for-section, and the migration inventory is not derivable from the outline. The asymmetry may be deliberate but is never declared as such. |
| ST-6 | 🟡 Major | `## Sequencing` L539–542 | "Each workstream is independently shippable and independently revertible" is asserted with no rollback content anywhere in the document. Workstream 0's migration adds a `CHECK` that aborts boot on a failing row; its revert path is the least obvious of the three and is undocumented. | An asserted safety property with no corresponding section is a structural gap, not a prose omission. Sibling spec has `## Rollout & Rollback`. |
| ST-7 | 🟡 Major | `### Tool inventory` L364–382 | The `Scope` column mixes types: seven rows carry scope literals (`read`, `write`, `import`), three carry the prose phrase "matches batch kind". The `Notes` cell for `discard_staged` is empty while all eleven other rows are populated. `apply_staged`'s Notes cell restates its Scope cell. | A table column that holds either an enum or a sentence is not a column. The scope model should be enumerable from this table; currently three rows require prose resolution elsewhere (L229). |
| ST-8 | 🟡 Major | `## Decisions and rationale` L96–104 vs `## Verification items` L547–549 | Direct contradiction between two sections. Decisions states `hyper 1.10.1`, `hyper-util 0.1.20`, `tower 0.6.11`, `http 1.4.2` "are already in `Cargo.lock` … **no new HTTP framework is added**". Verification item 2 asks whether `TowerToHyperService` composes with `hyper 1.10.1` "without a version bump, or whether `rmcp` requires a newer `hyper`". | The document asserts a conclusion in one section and lists it as unresolved in another. A reader cannot tell which is current, and the bolded claim is the one that will be quoted. |
| ST-9 | 🟡 Major | `### Schema — 0003_api_token.sql` L219–221 | Internal numeric contradiction inside one paragraph: "Secret: `pp_` + 32 random bytes, base62" yields ≈190 bits of entropy, but the next sentence asserts "256 bits of entropy means there is no guessing attack". The security conclusion is load-bearing on the number. | Two statements of the same quantity in adjacent sentences disagree. Whichever is wrong, the justification for omitting throttling is unsupported as written. **CONTESTED — see orchestrator note below.** |
| ST-10 | 🟡 Major | `## Workstream 0` L134, `### Tool inventory` L373 | Stale / path-ambiguous source citations. `api_routes/transactions.rs:238` — the `tx.r#type.to_uppercase()` line is actually **239** (function starts at 214). `stats_engine.rs:108-110` omits its directory; the file is `backend_rust/src/engines/stats_engine.rs`, while every other citation in the document carries a path prefix (`backup/restore.rs:367`, `services/currency_service.rs:138`, `frontend/src/App.tsx:657`, `lib.rs:268`). | Line-number citations into mutable source are the most drift-prone reference form, and one has already drifted. The document cites by symbol elsewhere (`create_transaction`, `resolve_caller`); the numeric citations should match that convention. |
| ST-11 | 🟡 Major | `## Workstream 0` item 2, L155–158 | Dangling reference: the audit query "is a pre-flight step in **the runbook**". No runbook is named, linked, or produced by any workstream, and `## Sequencing` step 1 ("confirm the live database passes the audit") does not name one either. `af17064` (L404) is cited bare with no link or description. | The mitigation for the document's highest-blast-radius failure (app does not boot) points at an artifact that does not exist in the document's reference set. |
| ST-12 | 🟡 Major | `## Success criteria` L43–56 vs `## Requirements` L58–70 | Two near-duplicate lists stating the same six capabilities in different vocabulary (read surface, staged write, bulk import, scoped revocable tokens, no regression, off-by-default), with no cross-reference between them and no traceability from either to the workstreams. | The standard treats Requirements as the traceable spine. Two unlinked restatements will diverge, and neither can be checked off. |
| ST-13 | 🟢 Nit | L1–6 front matter | The metadata bullet list mixes status/date/lineage with a policy assertion: "**Supersedes nothing.** No existing endpoint changes behaviour except where Workstream 0 says so explicitly." | Front matter should not carry requirements, and this one forward-references a section 124 lines away. |
| ST-14 | 🟢 Nit | L128, L185, L283, L429, L470, L514 | `---` rules appear before six H2 boundaries but not before `## Sequencing` (L527) or `## Verification items` (L543). | Inconsistent section delimiter implies the last two H2s are subordinate to Configuration, which they are not. |
| ST-15 | 🟢 Nit | L205, L305, L386, L410, L446 | All five code fences are unlabeled. Two are SQL DDL (should be ` ```sql `), three are payload sketches (should be ` ```json ` at most). The three payload blocks are not valid JSON — they use `\|` union syntax, `<portfolio_id>` placeholders, and `?` optional markers. | Unlabeled fences hide which blocks are executable contracts (`0003`, `0004` DDL) versus informal sketches, and disable highlighting. The DDL blocks in particular are the migration source of truth. |
| ST-16 | 🟢 Nit | L130 vs L187 vs L285 | Workstream H2 titles are not parallel: `Workstream 0 — prerequisite: close the transaction-type and number boundary` (id + imperative sentence) vs `Workstream 1 — personal access tokens` and `Workstream 2 — the agent module` (id + noun phrase). | Breaks scannability of the three-item spine that the document is organized around. |
| ST-17 | 🟢 Nit | `## Testing and gates` L472–512 | Test content is centralized rather than per-workstream, which is a defensible choice, but the labelling is inconsistent: one bullet is scoped by workstream ("**Workstream 0 tests**"), the rest are scoped by file (`protocol.rs`, `staging.rs`, `tests/isolation_test.rs`, `tests/route_coverage_test.rs`). No bullet is labelled "Workstream 1 tests" despite PAT tests existing there. | The reader cannot answer "does Workstream 1 have test coverage?" from the section structure. |

## Workstream symmetry matrix

| Subsection role | Workstream 0 (L130) | Workstream 1 (L187) | Workstream 2 (L285) |
|---|---|---|---|
| H2 lead line stating scope/ordering | ✅ "Ships and deploys **before**…" | ❌ none | ❌ none |
| Current-state findings / motivation | ✅ "Two findings from the current schema" | ⚠️ inline in *The invariant* ("already owns…") | ❌ none |
| Stated invariant / governing rule | ⚠️ implicit (chokepoint, via Decisions) | ✅ `### The invariant` | ❌ none (lives in *Decisions and rationale*) |
| `### Schema — 000N_….sql` heading | ❌ **missing** (owns `0002`, buried in list item 2) | ✅ `0003_api_token.sql` | ✅ `0004_agent_staged_batch.sql` |
| File / module layout inventory | ⚠️ inline (`models.rs`, deserializer, `restore.rs`) | ✅ two H3s (`auth/pat.rs`, `api_routes/tokens.rs`) | ✅ `### Layout` (4 files) |
| Validation / rules subsection | ⚠️ list items 1, 4 | ⚠️ unheaded bold paras (`**Scopes:**`, `**Expiry**`) | ✅ `### Validation` |
| Payload / interface contract block | ❌ none | ⚠️ route signatures only | ✅ 3 fenced payload blocks + tool table |
| UI surface subsection | ❌ none | ✅ `### The Access screen` | ❌ none (deliberate — Non-goals L86) |
| Feature-flag / configuration subsection | ❌ none | ❌ none | ⚠️ inline in *Layout* (`AGENT_LISTEN`) |
| Cross-cutting risk subsection | ❌ none | ❌ none | ✅ `### Sessions` |
| Per-workstream tests | ❌ (in global section, labelled "Workstream 0 tests") | ❌ (in global section, unlabelled) | ❌ (in global section, by file name) |
| Per-workstream rollback | ❌ | ❌ | ❌ (asserted in Sequencing, documented nowhere) |
| **H3 child count** | **0** | **5** | **7** |
| **Body form** | **numbered list 1–5 (item 4 nests a paragraph)** | **H3 subsections + bold paras** | **H3 subsections + bold paras** |

Deliberate asymmetries, and defensible: no UI panel in Workstream 2 (declared in Non-goals), no
schema change in Workstream 1's UI section. **Undeclared** asymmetries: Workstream 0's total
lack of H3 structure, its missing `### Schema` header despite owning a migration, and the
absence of any per-workstream rollback or test subsection in all three.

## Duplicated content that will drift

| Concept | Places stated | Divergence risk |
|---|---|---|
| Money/quantity regex `^-?\d+(\.\d{1,12})?$` | `## Workstream 0` item 4 (L166); `### Validation` (L338) | **Highest.** Two verbatim copies of a numeric validation rule with no single owner. A scale change (12 → 8) applied to one and not the other silently accepts values the other rejects, and the `protocol.rs` table-driven test asserts only one of them. |
| Dependency footprint of `rmcp` | `### Why this shape` item 2 (L31–40); `## Decisions and rationale` (L96–104); `## Verification items` bullets 1–3 (L545–552) | **Already diverged.** Decisions states the dependency conclusion as settled and bolded; Verification lists the same question as open. Three sections, three framings, no owner. |
| `access.rs` is the only caller-supplied-id resolver | L36, L106, L253, L322, L418, L491 | Six statements of one invariant, each phrased for its local context. The gate script's actual anchor is described only in Testing, so the six statements differ on whether the rule is enforced or intended. |
| Two-listener / credential separation | L12–15, L106–112, L189–201, L500 | Overview says the listener is "authenticated by a new scoped personal access token"; Decisions says Rocket "accepts only session cookies" and the listener "accepts only bearer tokens"; the enforceable form (`CredentialKind` parameter) appears only in Workstream 1. The strongest version is the least prominent. |
| Staged-batch single use / cannot double-apply | L50, L114, L327, L452, L484 | Five places. Success criteria adds a fourth condition ("cannot be applied by the user who did not stage it") that the schema section implements via `user_id` binding + `access.rs` but never states in those words. |
| Scope model (`read`/`write`/`import`, no `backup`) | L74, L229, L366, L452 | Four places, none authoritative. The tool table's three "matches batch kind" rows make the enum non-enumerable from the table, so the authoritative source is the weakest one. |
| `AGENT_LISTEN` off-by-default | L299, L519, L538 | Three places with three different phrasings ("binds only when set" / "unset means the module does not exist" / "Enable in the cluster"). The k8s/ingress precondition exists only in Configuration. |
| Staged-batch TTL = 60 minutes | L322 ("TTL 60 minutes"); L521 (default `60`) | A hardcoded literal in prose duplicating a configurable default. Changing the default invalidates the schema section silently. |
| Comma-decimal / non-canonical-number rejection | L339–342, L452–456, L476 | Three places, each with a different example string (`142,30` vs `"142.30", not 142.3`). The error-message contract and the validation rule are specified in separate sections that never cite each other. |
| Date bound 1900-01-01 … 2100-01-01 | L181, L346 | Two places, both attributing the bound to `create_transaction`. If the bound moves, both must change. |
| Closed transaction-type set `BUY\|SELL\|SPLIT` | L143–160, L336, L508 | Five places. Workstream 0's three internal mentions are at least co-located; the Workstream 2 mention is a correct back-reference. |
| Preview discipline (exact totals, bounded display, flagged-not-blocking) | L47, L355, L362 | The display-cap rule is stated abstractly and then as a literal string in the same section, and again as "exact counts" in Success criteria. Three phrasings of one contract. |
| Delete-echo requirement | L377, L418 | Two places; the second carries the `mode=replace` justification, the first does not, so the table reads as an unexplained rule. |
| `deny_unknown_fields` | L289, L334 | Two places, one as a file description, one as a rule. Minor, but it is the same sentence twice. |
| Pre-flight audit of live `type` rows | L155, L533, L554 | Three places. Only item 2 names the query; only Verification frames it as an unknown; only Sequencing makes it a gate. The one authoritative statement is the least operational. |

## Orchestrator note on ST-9 — contested

ST-9 is incorrect and should not be actioned. The spec says the secret is `pp_` + **32 random
bytes**, base62-encoded. Entropy is a property of the random source, not of the encoding: 32
random bytes are 256 bits regardless of alphabet, and base62 renders them in
`ceil(256 / log2(62)) = 43` characters. The reviewer appears to have computed the entropy of
32 base62 *characters* (`32 × 5.954 ≈ 190`), which is not what the document specifies. The
"256 bits" statement is correct and the throttling conclusion stands.

The finding does expose one real ambiguity worth a one-word fix: "32 random bytes, base62"
can be misread as "32 characters", which is exactly how it was misread here. State the encoded
length or say "32 bytes of entropy, base62-encoded (43 characters)".

ST-8 is accepted in part: the `tower 0.6.11` claim is false (confirmed, `Cargo.lock` has
0.5.3 only), so the bolded "no new HTTP framework is added" conclusion is overstated and must
be corrected. The framing of "direct contradiction" is harsher than the text warrants —
asserting that `hyper 1.10.1` is present and asking whether `rmcp` needs a newer `hyper` are
different claims — but the underlying point, that the document states a conclusion as settled
that it elsewhere lists as open, is fair.

---

# Part 3 — egm-style (copy editor)

## Assessment

The document contains no marketing adjectives — no "robust", "seamless", "elegant", "simply",
"obviously" anywhere. Its prose problem is the opposite and more expensive: it is written in
aphorisms. Roughly one sentence per section makes a rhetorical point instead of a technical one
("each was chosen deliberately rather than fallen into", "Its cost is honest and bounded",
"would make that gate decorative", "teaches the owner to mint with their eyes shut", "cheap to
pay knowingly and expensive to discover", "the nudge that gets acted on", "the failure mode
that actually happens"). These read as conviction transfer. A fresh implementer gets a mood,
not a rule, and in several places the aphorism is load-bearing: it stands in for a decision the
document has not made (what "swept" does to a row, what "echoed back by name" means for a
transaction, what "unusual price scale" measures, who runs the pre-flight audit and where the
runbook lives).

The second problem is claim calibration around precedent and numbers. The document justifies
choices by pointing at existing behaviour, and three of those pointers do not hold: `tower
0.6.11` is not in `Cargo.lock` (the lock has `tower 0.5.3`; `0.6.11` is `tower-http`), the
exact-duplicate skip rule is attributed to `csvValidator.ts`, which has no duplicate-row rule
at all, and the panic-isolation paragraph asserts both that a tool panic "would take the whole
application down" and that `catch_unwind` is the decided fix, while the verification list at
553–555 asks whether `catch_unwind` is sufficient. The verifiable numbers do check out — 449
packages in `Cargo.lock` is exactly right, 21 route handlers is right, `guard.rs:181` really
does ignore `Authorization` — which makes the wrong ones worse, because the reader has earned
reason to trust the digits.

The third problem is accountability grammar. The central safety claim is stated in the passive
throughout: writes "are applied", a preview "is shown", a delete "must be echoed back", stale
rows "are swept", a token is "revoked", a route "needs a route for the second port". Across
556 lines the document never once names who confirms an apply, who can revoke when the owner
cannot reach the UI, who sweeps, who runs the audit query, or who edits the ingress. Where the
actor is knowable, make it the subject: the backend validates, the account owner revokes, the
operator enables. Where the actor is not yet chosen, that is a missing decision, not a style
problem, and it is listed below as one.

## Findings

| ID | Severity | Location | Finding | Rewrite |
|---|---|---|---|---|
| SY-1 | 🔴 Blocker | l.99–101 | `tower 0.6.11` is not in `Cargo.lock` — the lock pins `tower 0.5.3`; `0.6.11` is the `tower-http` version. The sentence then asserts "`rmcp` is the only new dependency", which the document's own verification item contradicts. A precise-looking version list that is wrong is worse than a vague one. | "`hyper 1.10.1`, `hyper-util 0.1.20`, `http 1.4.2` and `tower 0.5.3` are already in `Cargo.lock` via reqwest. Whether rmcp needs `tower` ≥ 0.6 is a verification item, not an assumption. The new dependency is `rmcp` plus whatever version bumps its resolution forces." |
| SY-2 | 🔴 Blocker | l.392–394 | "the same rule `csvValidator.ts` applies to browser CSV imports" — `csvValidator.ts` contains no duplicate-row rule (only `unknownSymbols` at `:262`). The dedupe lives in `CsvImportModal.tsx:164–168` and is in-memory within one run. The existing preview vocabulary has `duplicate_symbol`, a *presence* flag (`2026-11-16-import-preview-design.md:190`), which is a different concept. The rule is new behaviour presented as precedent. | "Exact-duplicate rows are listed in the preview and skipped on apply. This is new behaviour: the browser importer has no cross-run duplicate-row rule. It is distinct from the existing `duplicate_symbol` flag, which reports that a symbol repeats, not that a row repeats." |
| SY-3 | 🔴 Blocker | l.19–20, l.46–48, l.377 | The human-in-the-loop guarantee is stated only in the passive: "a human sees before it is applied", "apply it on confirmation", "destructive elements must be echoed back by name". No sentence names who confirms, over which channel, or what the backend can prove about that confirmation. If the confirmation travels through the agent, the server holds no evidence that a human saw anything. | "Every agent write creates a staged batch and returns a preview. A human reads that preview and tells the agent to apply; the agent then calls `apply_staged`, echoing each delete by name. The backend enforces handle ownership, single use and expiry; it cannot verify that a human read the preview, and this spec does not claim it does." |
| SY-4 | 🔴 Blocker | l.143–147 | `#[serde(rename_all = "uppercase")]` — serde's style name is `UPPERCASE`; the lowercase spelling is rejected by serde at compile time. "canonical by construction" also hides a behaviour change: a closed enum rejects `"buy"`, while the SQL `CHECK` at l.163 stays case-insensitive, so the two layers are deliberately not the same width. | "`#[serde(rename_all = \"UPPERCASE\")] enum TxType { Buy, Sell, Split }` with `Display`. `TransactionCreate.type` becomes `TxType`, so an unknown value is a serde `422`. Note the asymmetry: the DTO accepts only `BUY\|SELL\|SPLIT` exactly, while the `CHECK` at item 2 stays case-insensitive to cover rows written before this change." |
| SY-5 | 🔴 Blocker | l.460–465 vs l.553–555 | "a panic in a tool handler would take the whole application down" and "Every tool call **is** wrapped in `catch_unwind`" are stated as settled, then the verification list asks whether `catch_unwind` is sufficient. The document asserts the mitigation and questions it on the same page. | l.461–464: "A panic on the listener's task can take down the API process, because the listener and Rocket share it. The intended mitigation is `catch_unwind` plus a 30-second deadline per call; whether `catch_unwind` is sufficient for a `tokio` task panic is an open verification item, and if it is not, the call runs in its own spawned task." |
| SY-6 | 🟡 Major | l.44–56 | Success criteria are not measurable. Two quoted user questions have no stated oracle or tolerance; "exact counts, exact decimal values" does not say what is compared against what; "the new gate has a mutation test proving it can fail" over-states what a mutation test shows. | "An agent holding a token with `scopes = ['read']` completes `get_tax_summary` and `list_transactions` calls, and every write tool returns `scope_missing`. A staged import applies with the row count and every decimal string byte-identical to the preview. A second `apply_staged` on one handle returns `already_applied` and writes zero rows. `check_agent_gate.sh` fails on a handler with an injected missing `require*` call." |
| SY-7 | 🟡 Major | l.254 | "Two consequences, stated now rather than discovered in CI:" is followed by three bullets. A stated count that the list contradicts reads as unproofed. | "Three consequences, listed here rather than found in CI:" |
| SY-8 | 🟡 Major | l.94–96 | "PR #228 … PR #642" — bare numbers with no repository. A fresh reader cannot resolve which repo's #228. | "…`modelcontextprotocol/rust-sdk` PR #228 reshaped it as `StreamableHttpService` … and PR #642 removed the unused axum dependency. (Links: <full URLs>.)" |
| SY-9 | 🟡 Major | l.219–222 | Three problems in one sentence: "256 bits of entropy means there is no guessing attack" is stronger than shown; "stated so nobody adds one later" speaks for future readers; and "`pp_` + 32 random bytes, base62" can be misread as 32 base62 *characters*, which is ~190 bits, not 256. | "Secret: the `pp_` prefix followed by 32 random bytes encoded base62 (≈43 characters). 32 bytes is 256 bits of entropy, which makes online guessing impractical, so the agent listener adds no attempt throttling. Do not add throttling without a stated reason in a new spec." |
| SY-10 | 🟡 Major | l.274–275 | "Anything idle more than 90 days is marked **unused for 90 days** — the nudge that gets acted on" asserts a behavioural outcome nobody measured, and the 90-day threshold has no stated basis or knob. | "The list marks a token **unused for 90 days** when `last_used_at < now() - interval '90 days'`. 90 days is a chosen default, not a measured threshold; make it a constant in the panel component." |
| SY-11 | 🟡 Major | l.322–325 | "TTL 60 minutes, checked lazily at apply; stale rows are swept when that user stages the next batch, so there is no cron." — "swept" does not say whether the row is deleted or marked, "lazily" is undefined jargon, the 60-minute value is stated here and again in the config table with no cross-reference, and a user who never stages again keeps rows forever. | "Lifetime: `AGENT_BATCH_TTL_MINUTES` (default 60). `apply_staged` compares `expires_at` and rejects an expired handle. When a user stages a new batch, the same transaction deletes that user's rows with `status = 'staged' AND expires_at < now()`. There is no scheduled job, so rows for a user who never stages again remain until that user stages again." |
| SY-12 | 🟡 Major | l.359–362 | "unusual price scale" is an anomaly flag with no definition — no threshold, no comparison target. An implementer cannot build it and a tester cannot assert it. | Replace with a rule, e.g. "price more than 10× or less than 1/10 of the asset's other rows in the same batch" — or drop the flag. |
| SY-13 | 🟡 Major | l.346–347 | "Dates are strict ISO `YYYY-MM-DD`, resolved to UTC midnight, bounded 1900-01-01 … 2100-01-01" — a date has no time zone; "resolved to UTC midnight" hides whether the column stores a `date`, a timestamp, or a string, and which value the bound is applied to. | "Dates are strict ISO `YYYY-MM-DD` and are stored as `TIMESTAMPTZ` at UTC midnight, as `create_transaction` does today. The 1900-01-01 … 2100-01-01 bound is applied to the date itself." |
| SY-14 | 🟡 Major | l.181–183 | "It gets a roadmap note." — a promise with no artifact: no file, no text, no owner. A decision hidden by a short sentence. | "`asset_type` stays free text. Add a roadmap entry to `AGENTS.md` → *Roadmap / Current Focus*: `asset_type` is unconstrained; constraining it needs an inventory of live values first." |
| SY-15 | 🟡 Major | l.427 | "Any future change to `replicas` must revisit this." — a change cannot revisit anything, and an instruction with no gate will not survive. Name the actor and the mechanism. | "The operator raises `replicas` only after replacing `LocalSessionManager` or adding session affinity. Add this precondition to the runbook line that changes `.spec.replicas`." |
| SY-16 | 🟡 Major | l.524–526 | "the ingress needs a route for the second port before the endpoint is reachable from outside the cluster" — passive, and the reader must guess which manifest, which path, and who applies it. | "`k8s/ingress.yaml` gains a path for the agent port. Until the operator adds it, the listener is reachable only inside the cluster. This spec does not add that route; enabling external access is a separate, explicit change." |
| SY-17 | 🟡 Major | l.163–165, l.554–556 | "the audit query … is a pre-flight step in the runbook" — the runbook is never named, and no one is named as the person who runs it. The same audit reappears at l.555 as the answer to a verification item. | "The operator runs the audit query from `docs/<runbook file>` before the deploy that carries migration `0002`. It must return zero rows; if it returns rows, the migration aborts and the values are listed." |
| SY-18 | 🟡 Major | l.170, l.530 | Subjectless imperatives: "Confirm no shipped bundle fixture violates it." and "Merge, deploy, confirm the live database passes the audit." No actor, no command, no artifact. | l.170: "The implementer greps the fixture directories and adds a test asserting every shipped bundle fixture passes the new constraint." l.530: "The developer merges; the operator deploys and runs the audit query, which must return zero rows." |
| SY-19 | 🟡 Major | l.536–541 | "it may just as well be executed as its own plan and its own PR" — hedging that hides an open decision the sequencing section exists to close. "independently revertible" also outruns evidence for Workstream 0, which changes live API behaviour and adds a `CHECK`. | "Workstream 0 ships as its own PR: it changes live API behaviour and MCP only needs the enum. Workstreams 1 and 2 ship separately and revert by removing config. Workstream 0 reverts by dropping the `CHECK`; rows written after deploy are unaffected because the accepted set does not widen." |
| SY-20 | 🟡 Major | l.386–388 | `{ target: { new: {...} } \| { existing: <portfolio_id> } }` — the `\|` inside a JSON-shaped block does not say whether this is an externally tagged object, an internally tagged one with a discriminator field, or a nullable pair. A code generator and a non-native reader will each pick a different one. | Spell the wire form: `target: { mode: "new", name, base_currency } \| { mode: "existing", portfolio_id }`, and state that an unknown `mode` is a `422`. |
| SY-21 | 🟡 Major | l.408–413 | `delete_portfolios: [<id>]` versus `delete_assets: [{id}]` — two shapes for the same operation in one payload, and `create_portfolios: [...]` / `create_assets: [...]` elide their shape without saying they mean the `stage_import` shape. | Use one shape for every list: `delete_portfolios: [{id}]`, `delete_assets: [{id}]`, `delete_transactions: [{id}]`; and write "`create_portfolios` and `create_assets` reuse the element shapes from `stage_import`". |
| SY-22 | 🟡 Major | l.418–421 | "Any delete must be echoed back by name in the `apply_staged` call" — undefined for a transaction, which has no name; and "an absent mode is a `422` because neither mode is safe to guess" refers to a `mode` field that does not exist in this spec, so the analogy reads as a missing requirement. | "For each delete, `apply_staged` must repeat the resource's human-readable name (portfolio name, asset symbol) or, for a transaction, its `date`, `symbol` and `quantity`. The point is the same one behind `mode=replace` in `2026-11-13-backup-export-restore-design.md`: that design requires an explicit `mode` and returns `422` when it is absent." |
| SY-23 | 🟡 Major | l.466–468 | "the batch fingerprint" is logged but never defined — hash of what, which algorithm, what question it answers in a log line. The trailing "which is someone's financial history" is pathos where a policy statement belongs. | "Logging records tool name, user id, token id, batch size, outcome, and a SHA-256 prefix of the canonical payload for correlation. The payload itself is never logged: it holds a user's holdings and tax history." |
| SY-24 | 🟡 Major | l.402 | "Not hypothetical: this deployment's proxy already refused one oversized body (`af17064`)." — a bare hash is not evidence a reader can read; the sentence spends its emphasis on a rebuttal nobody made. | "Caps are enforced server-side and return `batch_too_large`. The cluster's proxy already rejects oversized request bodies (see the commit message of `af17064`), so a larger batch would fail before reaching the backend anyway." |
| SY-25 | 🟡 Major | l.431 | "The repo already has the right discipline and this module inherits it." — a subjective quality verdict about existing code plus a vague verb. | "This module reuses two existing rules from `auth/` and `access.rs`:" |
| SY-26 | 🟡 Major | l.493–496 | "leaving the highest-privilege new code in the repo ungatechecked" — a superlative with no basis and a coined word; "Added to the CI gate list in `AGENTS.md` order." is a fragment with no subject and an undefined ordering rule. | "…and would be blind to tool handlers, so the code that touches rows for an agent would be the only such code with no automated check. `tests/agent_gate_test.rs` runs the real script against a throwaway tree, mirroring `tests/access_gate_test.rs`. Add the script to the gate list at `AGENTS.md:91–98`, after `check_upload_path.sh`." |
| SY-27 | 🟡 Major | l.504–507 | "The only test that proves the tower-service wiring rather than the logic behind it." — fragment, and "only … proves" is stronger than a single end-to-end test supports. | "This is the only test that exercises the tower-service wiring itself; every other test calls the logic behind it." |
| SY-28 | 🟡 Major | l.489 | "The gate script's own comments say the isolation matrix is the real gate, not the script." — an appeal to a code comment as authority, and a fourth meaning of "gate". | "The isolation matrix in `tests/isolation_test.rs` is the check that matters; `check_access_gate.sh` only proves a `require*` call is textually present." |
| SY-29 | 🟢 Nit | l.474–476 | "need `DATABASE_URL` like the other five gates" — `AGENTS.md:99–101` says five of eleven gates need it, and one of those five is `cargo test` itself, i.e. four others. | "…and needs `DATABASE_URL`, as four other gates in `AGENTS.md:99–101` do." |
| SY-30 | 🟡 Major | l.545 | "These are unknowns about external APIs, not open design questions." — contradicted by its own list: item 4 (`catch_unwind` sufficiency) is an internal design question and item 5 (live rows violating the `CHECK`) is a data audit. | "The first three are questions about external crate behaviour. The last two are not: `catch_unwind` scope is an internal design choice, and the `type` audit is a query against live data." |
| SY-31 | 🟡 Major | l.441, l.454 | Model-as-actor with a hidden requirement: "content the model **should** read and act on", "the model's correct response differs". Neither is verifiable, and the second states a guarantee about model behaviour. | l.441: "…and the agent receives it as tool output it can act on." l.454: "`already_applied` and `stale_handle` are distinct codes so the client can branch: the first means report the earlier success, the second means re-stage." |
| SY-32 | 🟡 Major | l.113 | "The gate cannot be skipped by a client that chooses to" — anthropomorphic ("chooses") and vague about the mechanism. | "A client cannot bypass it by omitting a step: `apply_staged` accepts only a handle whose row is still `status = 'staged'`." |
| SY-33 | 🟡 Major | l.116–121 | The expiry rationale is three metaphors and one filler: "the risk this feature **actually** creates", "teaches the owner to mint with their eyes shut", "the UI shows staleness instead of the server enforcing a calendar". The argument underneath is good and short. | "Expiry limits a forgotten credential. It does not limit the risk this feature adds, which is wrong rows staged by a model. Mandatory expiry on a token stored in an agent client's config produces an outage at an unpredictable time and trains the owner to mint without reading the scopes. The controls that carry the risk are scopes, revocation, and a list that shows what exists and when it was last used." |
| SY-34 | 🟡 Major | l.231–232 | "revocation is the answer to a leaked token" — the actor is never named, and no path exists for the case where the owner cannot reach the UI (the realistic leak scenario). | "The account owner revokes a token in the Access panel (`DELETE /api/tokens/<id>`). `logout-all` does not revoke tokens, by design: a token is not a browser session. If the owner cannot reach the UI, revocation requires a database update by an operator — record that as the documented fallback." |
| SY-35 | 🟢 Nit | l.234–235 | "fire-and-forget" is jargon and hides who sees the failure of the `last_used_at` write. | "`last_used_at` is updated at most once a minute per token on a spawned task; a failed update is logged and ignored, so a tool call never waits on it." |
| SY-36 | 🟢 Nit | l.340–341 | "a German broker statement is exactly what produces `142,30`" — rhetorical, and names one locale for a general case. | "European decimal-comma statements produce values such as `142,30`." |
| SY-37 | 🟡 Major | l.355–362 | Telegraphic and mixed-level: "exact totals always, display bounded, anomalies flagged and never blocking", then an anomaly list mixing noun phrases with adjectives. "Display caps always read 'showing 25 of 1,204'" is ambiguous about whether the string is literal. | "The preview follows `2026-11-16-import-preview-design.md`: totals are always complete, the displayed list is bounded, and anomalies are flagged without blocking apply. Anomaly conditions: unknown symbol; exact-duplicate row (same date, asset, type, quantity, price); currency differing from the portfolio base currency; date later than today; price outside the asset's batch range; currency assumed rather than supplied. When a list is truncated, the label reads `showing 25 of 1,204`." |
| SY-38 | 🟡 Major | l.366–382 | The tool table's Scope column mixes real scope names with the non-scope value "matches batch kind"; the Notes column mixes fragments with full sentences; `discard_staged` has an empty note; and the `apply_staged` note restates its own Scope cell. | Scope column: `read` / `write` / `import` / `read\|write\|import (batch kind)`. Give `discard_staged` a note ("discards without writing; no confirmation echo required") and shorten `apply_staged` to "echoes each delete by name". |
| SY-39 | 🟢 Nit | l.392–393 | "and there exact-duplicate rows are flagged" — "there" is doing the work of a conjunction and a referent; "flagged" is used with three meanings across the document. | "In the merge case, the backend lists exact-duplicate rows in the preview's anomaly section and skips them on apply." Then define once: *flagged = appears in the preview's anomaly list with a code*. |
| SY-40 | 🟡 Major | l.397–400 | "Currency defaults to the portfolio's `base_currency` and the preview flags the assumption — deliberately different from the browser importer, which hardcodes USD, and wrong for a EUR statement." — the final clause dangles: grammatically "wrong" attaches to the default, but the intended referent is the importer's hardcoding. | "Currency defaults to the portfolio's `base_currency`, and the preview names that assumption. This differs from the browser importer, which hardcodes USD — wrong for a EUR statement." |
| SY-41 | 🟢 Nit | l.404–407 | "**Apply.** Re-validates — … — then writes …" — subjectless; the reader must infer the backend. | "**Apply.** The backend re-validates (the target portfolio may have been deleted between stage and apply), writes in one Postgres transaction, and returns the created portfolio ids, asset ids and transaction count." |
| SY-42 | 🟢 Nit | l.149 | "the contract the browser importer has always enforced" — an absolute historical claim; the file's current state is the evidence, its whole history is not. | "…so this formalizes the contract `csvValidator.ts:99` enforces today." |
| SY-43 | 🟡 Major | l.74–78, l.84 | Three issues in Non-goals: "it is not a policy an agent can forget — it is a capability that cannot be minted" (anthropomorphic, and "minted" appears before the term is introduced); "`restore` … stays in the web UI" (ambiguous); "The preview is read in the agent chat" (passive — the user reads it) with an undefined "v1". | "There is no `backup` scope, so no token can be issued with backup access; the capability cannot be granted by configuration. `restore` with `mode=replace` deletes everything the user owns and remains available only in the web UI." / "There is no staged-batch panel in this release. The user reads the preview in the agent chat." |
| SY-44 | 🟢 Nit | l.39–41 | Heading "The chokepoint stays the chokepoint" is a tautology, and "would make that gate decorative" is a metaphor for a concrete failure. | Heading: "`access.rs` stays the only id resolver." Body: "…would let an agent write rows without the check that `check_access_gate.sh` exists to enforce." |
| SY-45 | 🟢 Nit | l.24–25 | "Three constraints decided the architecture, and each was chosen deliberately rather than fallen into:" — the second clause asserts the author's care and no technical fact. | "Three constraints decided the architecture:" |
| SY-46 | 🟡 Major | l.34–38 | "the npm incident class that bites hardest" (superlative, uncited), "Its cost is honest and bounded" (two subjective adjectives), and "~43 direct dependencies" — a tilde welded to an exact integer with no command shown. The 449 figure is exactly right and worth keeping; the 43 reads as invented. | "The concern is supply-chain exposure and code execution at install time. `rmcp` is the Rust SDK published under `modelcontextprotocol` (`modelcontextprotocol/rust-sdk`), v3.11.0 as of 2026-11-17, Apache-2.0, MSRV 1.88, implementing the `2026-07-28` spec revision; the local toolchain is `rustc 1.98.1`. Cost: `cargo tree -p rmcp -e normal` lists 43 direct dependencies, against the 449 packages already in `backend_rust/Cargo.lock`, under the same lockfile and audit posture as the rest of the crate." |
| SY-47 | 🟡 Major | l.460–465 | "Fault isolation must be paid for explicitly." and "This is the price of the in-process decision; it is cheap to pay knowingly and expensive to discover." — an aphorism closing a safety paragraph that asserts nothing beyond the previous sentence. | Keep a plain heading ("Fault isolation is not free in-process.") and delete the closing aphorism; the preceding sentence already gives the mechanism and the deadline. |
| SY-48 | 🟢 Nit | l.46–47, l.355, l.509 | "exact counts, exact decimal values", "exact totals always", and 'Guards "exact totals are always exact"' — the last is a tautology in quotation marks. | l.509: "The fixture asserts the counts, date spans and flag set byte-for-byte, which is what makes a display cap safe to add." |
| SY-49 | 🟢 Nit | l.520, l.512 | "unset means the module does not exist" (a module exists in the binary; only the socket is absent) and "PDF parsing, which does not exist" (ambiguous: not in scope, or not implemented anywhere). | "`AGENT_LISTEN` unset: the listener never binds and no agent tool is reachable." / "…and PDF extraction, which this design does not implement." |
| SY-50 | 🟡 Major | l.168, l.173 | "Money arrives as a string, enforced." — a dangling absolute; and "The `TEXT` columns are not to be changed to `NUMERIC`" — a prohibition with no actor, in a document whose implementer is an AI session that will not know whom it binds. | "**Money arrives as a JSON string; the deserializer rejects JSON numbers.**" / "**Do not change the `TEXT` columns to `NUMERIC`.** `tests/decimal_migration_test.rs` explains why … and `scripts/check_migrations.sh` fails the build if any of those columns is not `text`." |
| SY-51 | 🟢 Nit | l.164–165 | "keeps the engine's existing `.to_uppercase()` comparisons honest" — wrong rationale. Once the DTO is a closed enum (item 1), the case-insensitive `CHECK` exists for rows written *before* the migration, not for engine comparisons. | "Case-insensitive on purpose: rows written before this migration may hold `buy`, and the `CHECK` must accept them without a data rewrite." |
| SY-52 | 🟢 Nit | l.201 | "`require_scope(&caller, Scope::Write)` allows `Session` always and `Token` only when the scope is present" — "allows Session always" can be misread as "always allows sessions". | "`require_scope(&caller, Scope::Write)` returns ok for a session credential, and for a token credential only when that scope is present." |
| SY-53 | 🟢 Nit | l.436 | "because a 403 is an existence oracle" — jargon for a general reader. | "…because a 403 confirms the row exists in someone else's account." |
| SY-54 | 🟢 Nit | l.457–459 | "Never leaked: SQL text, stack traces, another user's data." — fragment with no actor; "Rows that do not exist yet are named by index in the submitted payload" is ambiguous about which rows. | "The error mapper never emits SQL text, stack traces, or another user's data. A proposed row is identified by its zero-based index in the submitted payload, never by a database id." |
| SY-55 | 🟢 Nit | l.132, l.6 | "Ships and deploys **before** the agent module" (subjectless) and "**Supersedes nothing.** … except where Workstream 0 says so explicitly" (a section cannot say anything). | "Workstream 0 ships and deploys before the agent module, because it changes live API behaviour." / "- **Supersedes:** nothing. No existing endpoint changes behaviour, except the endpoints listed in Workstream 0." |
| SY-56 | 🟢 Nit | l.19 | "The agent is allowed to *propose*. It is not allowed to *silently write*." — permission framed as temperament; the italics carry the meaning. | "No agent tool call writes a row directly. Every agent write creates a staged batch that a human sees before it is applied." |
| SY-57 | 🟢 Nit | l.51 | "cannot be applied by the user who did not stage it" — a double negative that reads as its opposite. | "…cannot be applied by a different user, and cannot be applied after it expires." |
| SY-58 | 🟢 Nit | l.72–73 | "The whole feature is off unless explicitly configured on. An existing deployment must not grow a public MCP endpoint by upgrading." — a riddle plus an ambiguous "by upgrading". | "The feature is disabled until `AGENT_LISTEN` is set. Upgrading must not expose a public MCP endpoint on an existing deployment." |
| SY-59 | 🟢 Nit | l.335, l.447, l.293 | Fragment connectors: "`deny_unknown_fields` on every DTO. Then:", "Payload shape, defined once in `protocol.rs`:", "Owns `agent_staged_batch` and nothing else." | "`deny_unknown_fields` is set on every DTO. The rules on top of it:" / "The payload shape is defined once in `protocol.rs`:" / "`staging.rs` is the only module that reads or writes `agent_staged_batch`." |
| SY-60 | 🟡 Major | l.12, l.70–71, l.79–80 | The input channel for the broker statement is never defined. "a broker statement the agent has read" assumes the agent already holds it; "No PDF extraction" then rules out the most likely source without saying what the user does instead. This determines whether the non-goal is even reachable. | Add one sentence to Overview: "The statement reaches the agent outside this system — the user pastes text or the agent's own client reads a file. Portfolio Prism receives only structured rows and never a document." |
| SY-61 | 🟢 Nit | l.66, l.433, l.31, l.333 | "the same preview discipline" and "the right discipline" use *discipline* as a virtue word; "deterministic" is used as a term of art without ever being defined as "implemented in code, not by the model". | Replace "discipline" with the rule it means ("the same preview rules"). Define once at l.31: "*deterministic* below means *implemented as code in the backend, so it cannot be talked out of it*." |
| SY-62 | 🟢 Nit | l.68–73 | Requirements mix grammatical levels: items 1–3 are noun phrases with a modal buried in the middle, item 4 is a bare noun phrase, items 5–6 are clauses. | Give every requirement the same shape: "The system SHALL …" or "The agent can …". E.g. "4. The owner can mint and revoke individually scoped tokens from the web UI without operator help." |
| SY-63 | 🟢 Nit | l.72–85 | Non-goals mix two patterns: "**No X.**" for six bullets and a full sentence for the first. | Standardize: bold "**No X.**" lead, then one sentence of consequence. E.g. "**No backup access.** No export, preview or restore; no `backup` scope exists." |
| SY-64 | 🟢 Nit | l.168–176 | Workstream 0 item 4 contains a nested bold paragraph (the `TEXT`/`NUMERIC` note) at a different list level inside a numbered item, so the list reads as four items with an appendix. | Move the `TEXT`-columns note to its own numbered item (or a "Not changing" sub-list under Workstream 0) so every item is one kind of statement. |
| SY-65 | 🟢 Nit | l.330 | "which is the failure mode that actually happens" — a frequency claim with no incident behind it, unlike the caps bullet which at least cites a commit. | "so a retry after a timeout cannot post the batch twice." |
| SY-66 | 🟢 Nit | l.30 | "and nothing downstream would notice" — an absolute about a system the document elsewhere describes as full of checks. | "…and the schema accepts it: the error surfaces only in the tax report." |
| SY-67 | 🟢 Nit | l.108 | "an agent cannot curl a raw write endpoint" — *curl* used as a verb, and "raw write endpoints" is a third name for the `/api` surface. | "an agent cannot call a write endpoint on `/api`, because those handlers do not look at bearer tokens." |
| SY-68 | 🟢 Nit | l.452–453 | "`hint` exists because the consumer retries" — *consumer* is a fourth name for the agent (agent / model / client / consumer). | "`hint` exists because the agent retries on error: …" |
| SY-69 | 🟢 Nit | l.462–463 | "a 30-second per-call deadline — so a hung pricing lookup cannot wedge a session" — the deadline value has no stated basis, and no tool in the inventory is named as the one that can hang. | "a per-call deadline (default 30s, configurable with the other agent settings) so a slow upstream pricing call in `get_performance` cannot hold a session open." |
| SY-70 | 🟢 Nit | l.14, l.98–103, l.299, l.547 | "streamable HTTP" is written four ways: "MCP streamable HTTP", "the streamable-HTTP transport", "the streamable-HTTP tower service", "streamable-HTTP-server transports". | Use the unhyphenated noun ("streamable HTTP") and the hyphenated modifier ("streamable-HTTP transport"), consistently. |

## Rhetoric that carries no information

| Line | Phrase | What it actually asserts |
|---|---|---|
| 24–25 | "each was chosen deliberately rather than fallen into" | Nothing. The three constraints follow and speak for themselves. |
| 34 | "the npm incident class that bites hardest" | A superlative about the world with no citation. Assert: install-time code execution is the risk being avoided. |
| 36 | "Its cost is honest and bounded" | Nothing; "honest" and "bounded" are praise for the author. The numbers after the colon are the content. |
| 41 | "would make that gate decorative" | That a bypass path would exist. Say that. |
| 39 | Heading "The chokepoint stays the chokepoint" | Nothing; tautology. |
| 118 | "the risk this feature **actually** creates" | "actually" is filler; the risk named after it is the whole content. |
| 120 | "teaches the owner to mint with their eyes shut" | That expiry friction encourages careless minting. An inference dressed as a punchline. |
| 121 | "instead of the server enforcing a calendar" | That expiry is not enforced by default. Already stated in the heading. |
| 149 | "the contract the browser importer has always enforced" | That `csvValidator.ts:99` rejects other values today. "always" is unprovable and unnecessary. |
| 165 | "keeps the engine's … comparisons honest" | Nothing, and the stated rationale is wrong (see SY-51). |
| 176 | "The fix is at the boundary, never in the storage type." | Half content, half slogan. The `check_migrations.sh` sentence next to it is the enforceable part. |
| 222 | "stated so nobody adds one later" | Mind-reading about future contributors; the instruction itself is the content. |
| 254 | "stated now rather than discovered in CI" | Nothing about the consequences; it praises the document for listing them. |
| 275 | "the nudge that gets acted on" | An unmeasured claim about user behaviour, used to justify a UI label. |
| 330 | "which is the failure mode that actually happens" | A frequency claim with no incident cited. |
| 347 | "the agent has no dropdowns" | That there is no interactive correction step. The metaphor makes a reader decode it first. |
| 362 | "Display caps always read 'showing 25 of 1,204'" | Unclear whether the string is literal; "always" adds nothing beyond "when truncated". |
| 375 | "the flagship, below" | Marketing filler plus a positional pointer. The tool name is the pointer. |
| 396 | "two portfolios named 'Depot' is a real thing" | That duplicate portfolio names occur. Fine as an example, stated as a wisecrack. |
| 402 | "Not hypothetical:" | Nothing; the cited commit is the evidence, the colon is theatre. |
| 433 | "The repo already has the right discipline and this module inherits it." | A subjective verdict about existing code plus a vague inheritance. |
| 464–465 | "it is cheap to pay knowingly and expensive to discover" | Nothing. The mechanism and the deadline are already given. |
| 468 | "which is someone's financial history" | Emotional framing of a rule already stated ("never the payload"). |
| 489 | "the isolation matrix is the real gate, not the script" | A value ranking attributed to a code comment. |
| 507 | "The only test that proves the tower-service wiring" | That it is the only end-to-end test of the wiring — minus "only" and "proves". |
| 509 | 'Guards "exact totals are always exact"' | A tautology quoted as if it were a requirement. |
| 520 | "unset means the module does not exist" | That the listener does not bind. The module is compiled in either way. |
| 545 | "These are unknowns about external APIs, not open design questions." | Half true, and the half that is false is contradicted by items 4 and 5 of the same list. |
| 556 | "the pre-flight audit answers this before deploy, not after" | That the audit runs pre-deploy. The "not after" is rhythm, not information. |

## Terms with three names

| Concept | Competing terms in the document | Keep |
|---|---|---|
| The access chokepoint | "the auth chokepoint" (l.4), "the chokepoint" (l.39), "the gate" (l.41, l.107, l.113), "the access gate" (l.253), "the real gate" (l.489), "the staging gate" (l.107) | `access.rs` for the module, "the access chokepoint" for the rule. Never "the gate" — it is used for four different things. |
| CI check scripts | "gate" (l.56, l.476, l.490, l.496), "the new agent gate", "the gate list", "the five gates" | "CI gate" for scripts; never for `access.rs` or for the isolation matrix. |
| The new network endpoint | "the MCP server" (whole feature), "the agent module" (WS2 title), "the agent listener" (credential boundary), "the second listener" / "second port", "the public MCP endpoint" (l.73) | "agent listener" for the socket, "agent module" for the code, "MCP server" only for the protocol role. |
| The interactive surface | "the interactive `/api` surface" (l.52), "raw write endpoints" (l.108), "every current `/api` route" (l.67), "the `/api` routes" (l.252) | "the `/api` surface"; "write endpoints on `/api`" when the subset matters. |
| The approving human | "a human" (l.20, l.312), "the user" (l.51, l.76), "the owner" (l.120, l.274), "the caller" (l.189, l.296), "consumer" (l.453), "client" (l.113, l.382) | "the account owner" for a person; "the agent" for the model; "the MCP client" for the software. Retire "consumer" and bare "caller". |
| The model | "the model" (l.27, l.31, l.118, l.441, l.454), "the agent" (l.12, l.19, l.66, l.108), "an AI agent" (l.10), "the consumer" (l.453) | "the agent" for the actor making tool calls; "the model" only for the text-normalization step that produces rows. |
| The staged write | "staged batch" (l.20, l.49), "proposal" (l.112, l.293, l.311), "batch" (l.227, l.377, l.466), "payload" (l.112, l.311) | "staged batch" for the row, "proposal" for its content, "payload" for the JSON bytes. |
| Duplicate | "duplicates flagged rather than silently dropped" (l.48), "duplicate-looking row" (l.360), "exact-duplicate rows" (l.393), `duplicate_symbol` in the existing preview spec | Two distinct rules need two names: "exact-duplicate row" (row-level, skip on apply) and the existing `duplicate_symbol` flag (symbol-level, presence only). |
| Error naming | "named error" (l.340, l.401), "named rejection" (l.350), "stable codes" (l.450), "error code" (l.449) | "stable error code from the list in `protocol.rs`". |
| Money in / money out | "money" (l.91, l.168, l.289, l.452), "Money and quantity" (l.339), "decimal values" (l.47), "financial figures" (l.175) | "monetary value" for the field class; keep `rust_decimal` and the regex for the mechanics. |
| Sell profit | "sell P&L" (l.62), "the sell P&L breakdown" (l.372), "realized gain" (l.45), `get_transaction_pnl` (l.372) | "realized gain per sale" in prose, `get_transaction_pnl` as the tool name. Define the mapping once. |
| Tax report | "the dated tax report" (l.62), "the dated report" (l.371), "tax report" (prior spec title) | "the dated tax report" everywhere after first mention. |
| Making data standard-form | "normalized" (l.66, l.289), "canonicalized" (l.112, l.311), "Normalize existing rows" (l.152), "canonical by construction" (l.146) | "normalize" for agent-produced rows and legacy rows; "canonical payload" for the stored JSON. Define each once. |
| Flagged | "duplicates flagged" (l.48), "anomalies flagged and never blocking" (l.356), "the preview flags the assumption" (l.399), "accepted and flagged" (l.344), "flagged in the preview and skipped" (l.393) | Define once: *flagged = listed in the preview's anomaly section with a code, and does not block apply*. |
| Token | "personal access token" (l.16), "PAT" (l.491), "bearer token" (l.106), "scoped credential" (l.16), "secret" (l.219), "api_token" (schema) | "personal access token (PAT)" at first use, then "token"; "token secret" for the plaintext string only. |
| Idle token signal | "shows staleness" (l.121), "Anything idle more than 90 days" (l.274), "**unused for 90 days**" (l.274) | "unused for 90 days" as the label; "last used" as the column. |
| Backend | "the backend" (l.31), "the server" (l.121, l.231), "server-side" (l.111, l.333), "the app" (l.140, l.158), "the application" (l.462) | "the backend" for the actor; "server-side" only as an adjective ("server-side state"). |
| "kind" | batch `kind` = `edit`\|`import` (l.308, l.377), `CredentialKind` = `SessionOnly`\|`TokenOnly` (l.193) | Keep both (they are column and type names) but never write bare "kind" in prose; write "batch kind" or "credential kind". |

## Orchestrator note on SY-4

Confirmed against serde's documented `rename_all` values: the style is `UPPERCASE`, not
`uppercase`, so the attribute as written in the spec would not compile. Accepted without
reservation.
