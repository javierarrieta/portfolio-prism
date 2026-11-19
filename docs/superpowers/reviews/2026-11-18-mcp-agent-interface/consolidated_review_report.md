# Consolidated Review Report — MCP Agent Interface Design

* **Target:** `/home/coder/wt-pp-mcp-spec/docs/superpowers/specs/2026-11-18-mcp-agent-interface-design.md` (556 lines, 27 headings)
* **Ranges reviewed:** full file 1–556; FeatureRange 1–129, DetailedRange 130–556
* **Date:** 2026-11-18
* **Compiled from:** Stage 5 (`zoom_out_report.md`), Stage 6 (`comprehension_report.md`), Stage 7 (`critic_report.md` — design / structural / copy axes), Stage 8 (`intermediate_zoom_out_report.md`), Stage 9 (`implementation_readiness_report.md`, `citation_audit_report.md`), plus `orchestrator_notes.md` (ON-1, ON-2, ON-3 — evidence, not findings)
* **Organising spine:** Stage 8's nine root problems A–I. Per-axis IDs appear only as citations back to the source reports.

## 1. Bottom line

The architecture is sound and worth building: staging behind a deterministic server-side gate, scoped revocable tokens, opt-in by config, and a prerequisite hardening pass that is valuable on its own merits. It is **not plan-ready**, and the reason is not taste — four of the document's load-bearing justifications are false as written, its central safety guarantee is asserted in the passive and enforced by nothing, and the one interface the whole design depends on (what `staging::apply` calls to write a row) is never named. Eight citations are wrong and sixteen are partly wrong, and every one of those errors points the same direction: making the change look smaller and safer than it is. Workstream 0 is sold as a no-frontend-change formalization; it is a breaking live-API change whose prerequisite is a frontend PR that does not exist yet, and one of its five items is already shipped. Workstream 1 is close to executable and blocked on exactly one decision (the scope set). Workstream 2 should not be planned at all until three things are recorded: what makes `apply` a control rather than a convention, what shared write-and-preview service the tools call, and whether the second listener exists. Two four-hour spikes settle most of that and cost almost nothing if run on day one. Cutting `stage_edit`, all `update_*`/`delete_*` operations and the third scope makes Workstream 2 roughly a third smaller and removes the two places where the design contradicts its own reasoning about destructive writes. Of the nine root problems, three (B, C, G) are document work and can start immediately; six require design decisions, and two of those decisions are the operator's to make, not the implementer's.

## 2. Reviewer status

| Stage | Axis / reviewer | Finding IDs | Status |
| --- | --- | --- | --- |
| 5 | `egm-zoom-out` | ZO-1 … ZO-16 | completed |
| 5 | `egm-inventive-catalyst` | — | not registered in this harness; Stage 5 ran single-reviewer. Not counted as a zero-finding lens. |
| 6 | `egm-comprehension` | CP-1 … CP-24 | completed |
| 7 | `egm-design` (hostile design critic) | DS-1 … DS-24 | completed |
| 7 | `egm-structural` | ST-1 … ST-17 | completed |
| 7 | `egm-style` (copy editor) | SY-1 … SY-70 | completed |
| 8 | `egm-zoom-out` (intermediate, prior stages in context) | root problems A–I, tool-surface ruling, two structural-alternative rulings | completed |
| 9 | `egm-implementation-readiness` | C-*, W0-*, W1-*, W2-* | completed |
| 9 | `egm-codebase-details` (citation audit) | 8 WRONG / 16 PARTLY / 1 half-confirmed / external unknowns | completed |
| 9 | second copy pass (A/B recall doubling) | — | **deliberately skipped** — the document was byte-unchanged since Stage 7 filed 70 copy findings; a second pass would re-derive the same list at the same recall. |

No requested reviewer failed, timed out, or returned nothing. Every refutation in these reports was re-checked in the worktree by the orchestrator before it was filed, and the adjudications in each report's trailing section are final for this consolidation.

## 3. Scorecard

| Stage / axis | 🔴 Blocker | 🟡 Major | 🟢 Nit | Total |
| --- | --- | --- | --- | --- |
| 5 — zoom-out (ZO) | 2 | 10 | 4 | 16 |
| 6 — comprehension (CP) | 2 | 11 | 11 | 24 |
| 7 — design (DS) | 4 | 12 | 8 | 24 |
| 7 — structural (ST) | 0 | 12 (11 actionable; ST-9 withdrawn per ON-3) | 5 | 17 |
| 7 — copy (SY) | 5 | 36 | 29 | 70 |
| 9 — implementation readiness (C/W0/W1/W2) | 6 | 19 | 6 | 31 |
| **Raw total** | **19** | **100** (99 actionable) | **63** | **182** (181 actionable) |
| 9 — citation audit | 8 WRONG | 16 PARTLY (+1 half-confirmed) | — | 25 verdicts on ~120 extracted references |

**Dedup.** Stage 8 collapsed roughly 130 findings from Stages 5–7 into **nine root problems**; fixing a root closes everything folded into it. Stage 9's 31 findings fold into the same nine (mapping below) and add no new root. Net actionable structure: **9 root problems + 3 open decisions + 2 standing rulings + 25 citation errata**.

| Root | Absorbs from Stage 9 |
| --- | --- |
| A | W1-1 (scope half) |
| B | W0-2, W0-3, W0-8 |
| C | W0-1, audit WRONG/PARTLY rows |
| D | W0-7, W2-1, W2-2, W1-5, C-4 |
| E | W2-3, W2-4 |
| F | (Stage 9 adds no new limit; W2-8 is the read-side symptom) |
| G | C-1, C-2, C-3, W0-4, W0-5, W0-6, W1-2, W1-3, W1-4, W1-6 |
| H | W2-6, W2-9, W2-10 |
| I | W2-5, W2-7, W0-1 (contract half) |

### 3.1 Conflicts between stages, and which one wins

| Disagreement | Resolution | Winner |
| --- | --- | --- |
| ST-9 "256 bits vs ≈190 bits is an internal contradiction" | **Withdrawn.** Entropy is a property of the 32 random bytes, not of the base62 rendering (~43 characters). Only SY-9's wording fix survives. | ON-3 |
| DS-19 (zero-price rationale cites a roadmap file that does not exist) | **Out of scope.** A missing roadmap document is not this spec's defect. | ON-3 |
| WS0 item 3 "the bundle schema must constrain `type`" | **Already shipped.** `docs/backup-schema/v1.json:134-142` enumerates `BUY\|SELL\|SPLIT`, generated from `backup/mod.rs:222`, enforced at `backup/mod.rs:467-470`, pinned artifact↔constant at `backup/schema.rs:49-64`. ZO-3, DS-18, W0-8 and audit WRONG L162-163 all agree. | ON-3 / citation audit |
| `af17064` marked UNVERIFIABLE by the citation audit | **CONFIRMED.** Local git works offline; the commit exists: `af17064 2026-11-15 fix(backup): the proxy refused a bundle the API would have accepted (#68)`. The claim is true. SY-24's style point (name the incident, don't drop a bare hash) stands; the implication that it is unsupported does not. | Orchestrator |
| Stage 8 verdict "WS0 is ready after B and C" vs Stage 9 "WS0 is in a worse state than that" | Stage 9 tested WS0 item-by-item against the tree and found two items mislocated or unverifiable (wrong file named; verification placed in a gate that cannot observe it). | Stage 9 |
| Stage 7 alternative 2 "reuse the backup preview/restore machinery as the single staging path" vs Stage 8 | Stage 8 is correct: `POST /backup/preview` is stateless and restore re-sends the whole bundle, so the staged-batch row, handle, TTL, single-use flip and user binding must be written regardless. Adopt the **validate → plan → apply** implementation; reject **merge into existing rows** on the record. | Stage 8 |
| Stage 5/7 "extend `check_access_gate.sh`" (ZO-9) vs the spec's fifth gate script | Unresolved by the spec, and Stage 9 (C-4) shows both cannot be true. Ruling: the extension is the default recommendation; Decision 3 can moot the question entirely. Must be stated either way. | Stage 9 (as an open item) |
| CP-2 "a `read`-only token can preview an import" vs the tool table | The tool table is authoritative as written; both sentences change under Decision 1. | Tool table, then Decision 1 |
| ST-8 "direct contradiction" between Decisions and Verification | Accepted in part: the `tower 0.6.11` claim is false and the bolded conclusion must be corrected, but "asserting `hyper 1.10.1` is present" and "asking whether rmcp needs newer hyper" are different claims. | ON-3 |
| SY-24 "a bare hash is not evidence" vs audit CONFIRMED | Keep the rewrite (name the incident); drop the "unhypothetical/unsupported" framing. | Both, split |
| Anything the citation audit marks WRONG vs an earlier stage repeating the claim | The audit overrides. Where an earlier stage already refuted the same claim (ZO-6/ZO-7/ZO-3/ZO-2), the reports agree. | Citation audit |

## 4. The nine root problems

Each row is one problem. Closing the row closes everything folded into it.

### A — The confirming signal travels through the party under review. *Design work.*
**What it is.** The founding sentence — every write is staged and "a human sees it before it is applied" — is enforced by nothing the document describes. Same token, same session, same user, same channel. `apply_staged` requires only "the scope matching the batch's kind", and the Non-goals explicitly remove the only out-of-band channel ("No UI panel for staged batches in v1. The preview is read in the agent chat"). The preview is the proposer's own output relayed by the proposer; no field carries the source literal, so a human reading `15.00` has nothing to compare against the `1.5` the model may have read.
**Collapses:** CP-1, CP-3, CP-20, DS-1, DS-2, DS-11, DS-20, SY-3, SY-22, W1-1 (scope half).
**Single change.** Make apply require something the agent does not automatically hold: a distinct `apply` scope, not pre-checked in the mint form, plus a read-only *Pending batches* list in the Access screen that renders the stored `preview` JSONB out of band. Add one optional per-row `source_text` echo for flagged rows. Then state plainly that the backend cannot prove a human read anything, and stop writing as if it does. (Paste-ready text in §10, Decision 1.)

### B — Workstream 0 is sold as a no-op formalization; it is a breaking live-API change, and part of it is already shipped. *Document work* (plus one operator approval and one code-level choice).
**Collapses:** ZO-2, ZO-3, ZO-4, ZO-7, ZO-8, DS-3, DS-9, DS-10, DS-18, SY-4, SY-19, SY-51, CP-22, W0-2, W0-3, W0-8.
**Single change.** Split WS0 into three separately-decided items: (i) strictness scoped to **agent DTOs only**; (ii) an explicit, separately-approved live-API change with the frontend money-as-string migration named as a prerequisite PR; (iii) delete the bundle-schema item as done. Make `TxType` the single source behind `KNOWN_TRANSACTION_TYPES` rather than a second copy, and resolve the case-sensitivity triangle (serde `UPPERCASE` / case-insensitive `CHECK` / case-sensitive `uq_split_asset_date`) in one place, naming the four files it touches.

### C — Load-bearing claims that do not survive the repository, and they are all the claims that make a decision look cheap. *Document work.*
**Collapses:** ZO-3, ZO-5, ZO-6, ZO-7, DS-13, DS-17, DS-18, ST-8 (in part), SY-1, SY-2, SY-9, SY-46, CP-7, CP-9, CP-22, W0-1, and the eight WRONG audit rows.
**Single change.** One citation pass with a hard rule: every claim used to justify a choice is either verified with a path a reviewer can re-run, or demoted to a Verification item. The three that matter are the dependency posture (the sentence that buys the supply-chain argument), "frontend already sends strings", and the duplicate-skip precedent. Work top to bottom from §8.

### D — The design's central mechanism is unspecified: nothing says what `staging::apply` calls to write a row, or what renders a preview. *Design work.*
**Collapses:** ZO-1, ZO-9, ZO-11, ZO-14, CP-13, DS-8, W2-1, W2-2, W0-7, W1-5, C-4, Stage 7 alternative 2.
**Single change.** Name the shared **validate → preview → write** service as a deliverable and put its extraction in WS0 or at the head of WS2. `staging.rs` then owns only persistence, handle, single use and expiry; validation and preview delegate to `backup::detail` and its flag codes; the single-use flip and the write are one `SERIALIZABLE` transaction. This is the one thing that must move *up* into the design; the DDL and the decimal regex should move down to the plan.

### E — The in-process decision is priced only in panics. *Design work.*
**Collapses:** ZO-12, DS-6, DS-7, DS-14, SY-5, SY-15, CP-23, W2-3, W2-4, Stage 5 gaps 5 and 6.
**Single change.** Replace the fault-isolation paragraph with a **"costs of sharing a process"** subsection that enumerates, with numbers: a concurrency semaphore sized against `max_connections(5)`, an explicit body-size limit on the listener (Rocket's per-route limits have no counterpart on raw `hyper`), caps re-derived from the 256 Mi / 500 m limits rather than asserted, probe visibility for a dead listener, bind-failure behaviour, and the real Kubernetes path for the second port — which is not one ingress line (W2-4). This section is also where the second-listener decision gets recorded.

### F — Nothing bounds an agent that is working normally. *Design work.*
**Collapses:** DS-7, DS-12, DS-16, DS-20, CP-9, CP-24, DS-5/W2-8, Stage 5 gaps 4 and 7.
**Single change.** One **limits** table with one owner: per-token rate limit, per-user pending-batch cap, `list_transactions` page size and hard row ceiling, a preview token budget, terminal-row retention rule, and pricing-quota exposure on `get_performance`. Derive the write caps from the container limits and say so; a bare commit hash is not a derivation.

### G — The document has no home for its own decisions and never names an actor. *Document work.*
**Collapses:** ST-1 … ST-6, ST-11, ST-12, CP-8, CP-11, DS-15, DS-24, ZO-16, SY-6, SY-10, SY-12, SY-14, SY-15, SY-17, SY-18, SY-25, SY-34, SY-50, SY-55, SY-26, C-1, C-2, C-3, W0-4, W0-5, W0-6, W1-2, W1-3, W1-4, W1-6, and the whole "rhetoric that carries no information" table.
**Single change.** Add `## Alternatives Considered`, `## Risks and Mitigations`, `## Rollout & Rollback` (both sibling specs in the same directory already carry these — this is a local convention break, not house style), a TOC with anchors, and one Operations paragraph naming the operator for each passive obligation. Then do a sentence pass: where an aphorism was carrying a decision, write the decision. The copy reviewer already supplied most of these rewrites; this is mechanical.

### H — Multiple sources of truth for one rule, and parallel vocabularies for one product. *Document work with two code-level consequences.*
**Collapses:** ST duplication table, ZO-4, ZO-11, ZO-13, CP-12, CP-18, SY-37, SY-39, SY-61, W2-6, W2-9, and the terms-with-three-names table.
**Single change.** One authoritative home per rule with back-references, and reuse instead of new spellings: `backup::detail`'s flag codes for anomalies, `db_types::decimal_json` / `Decimal::from_str_exact` for money, one gate script extended rather than a fifth added, `KNOWN_TRANSACTION_TYPES` as the one set, two names for two different duplicate rules.

### I — Wire contracts and validation rules that cannot be implemented or tested as written. *Design work (small, well-specified).*
**Collapses:** CP-4, CP-5, CP-6, CP-10, CP-12, CP-13, DS-4, DS-21, SY-11, SY-12, SY-13, SY-20, SY-21, SY-23, ST-7, ST-15, W2-5, W2-7.
**Single change.** A definitions-and-wire-contracts pass: spell the tagged unions as real JSON, give every anomaly flag a computable rule or delete it, define duplicate matching as `Decimal` equality over named fields (not string equality — scale-sensitive as written), settle the transaction-currency question, state what `canonicalized` guarantees between preview and apply, and label the DDL fences as the executable contracts they are.

## 5. Blocking findings

Deduplicated across all axes. Citation blockers are listed compactly; their full corrections are in §8, which is built to be worked top to bottom in one editing pass.

### BLK-1 — Apply is a convention, not a control
* **IDs:** CP-1, DS-1, DS-2, SY-3, CP-3, CP-20, W1-1
* **Scope:** L19–20, L46–48, L74, L377, L465 (scope list + tool table)
* **Discrepancy.** The scope lattice cannot express "may stage, may not apply". An `import`-scoped token stages and applies in one turn with no second factor, and the preview the human is supposed to read is delivered by the model whose output is under review. The document's own threat — a plausible misread producing a plausible portfolio — is exactly the failure a chat-relayed preview cannot catch.
* **Recommendation.** Decision 1 (§10). Distinct `Scope::Apply`, not pre-checked in the mint form and not addable to an existing token; read-only *Pending batches* list rendering the stored `preview` JSONB; optional per-row `source_text` echo; and one honest sentence that the backend guarantees a deliberately-minted apply credential and an out-of-band copy of the preview, not proof that a human read anything.

### BLK-2 — The write path the whole design rests on is unspecified
* **IDs:** ZO-1, W2-1, DS-8, CP-13, Stage 7 alternative 2, root D
* **Scope:** L396–405, L419–423, L499–503
* **Discrepancy.** "Tools call the same `engines::` and `services::` functions the Rocket handlers call" is false for writes: transaction creation is inline at `api_routes/transactions.rs:221-262`, asset creation at `:47-98`, and `services/` contains only `currency_service.rs` and `mod.rs`. Either the agent path duplicates that validation — two write paths that drift, the precise failure WS0 exists to prevent — or an unmentioned refactor extracts a shared service.
* **Recommendation.** Decision 2 (§10): name `services::transactions::insert` / `services::assets::insert` as extracted, no-behaviour-change deliverables, and make `backup::detail` the only preview renderer.

### BLK-3 — Money-as-string enforcement breaks a live feature
* **IDs:** ZO-2, DS-3, W0-2, CP-22, audit WRONG L166–168
* **Scope:** L166–170 (WS0 item 4), L66 (Requirement 5), L6
* **Discrepancy.** "`frontend/src/utils/decimal.ts` already sends strings" is refuted: that file is a read-side normaliser (`toNumber` `:20-23`, `normalizeTransactionList` `:96-104`) with no request code. The browser CSV importer posts JSON **numbers** (`csvValidator.ts:187-189` → `parseFloat` at `:247-249`; `CsvImportModal.tsx:170-176`; demo seed `App.tsx:509,537`). Only the manual form sends strings (`PortfolioDetail.tsx:15-21,171-177`). Rejecting numbers on the shared `TransactionCreate` breaks a live feature and contradicts Requirement 5. W0-2 adds that the bullet is three deliverables in one and names no prerequisite migration.
* **Recommendation.** Scope strictness to the agent DTOs, **or** land the frontend migration first and name it: `parseNumeric` → canonical decimal string, `ValidatedRow.quantity/price/fee: string`, dedup key at `CsvImportModal.tsx:163`, `frontend/src/test/mocks/handlers.ts:105-111`, assertions at `csvValidator.test.ts:15,141`, and the contract comment at `frontend/src/types.ts:8`. Separately: add `db_types::strict_decimal_json`; do **not** modify `decimal_json`, which is the response serializer at ~55 sites whose `visit_f64` arm is load-bearing for `TaxLot`/`AssetTaxSummary`.

### BLK-4 — The duplicate-skip rule is undefined and its cited precedent is misattributed
* **IDs:** DS-4, SY-2, audit WRONG L392–396
* **Scope:** L392–396, L358, L105–108
* **Discrepancy.** Exact-duplicate detection over `TEXT` money columns is scale-sensitive: stored values preserve input scale byte-for-byte (the spec's own test requirement), so a re-imported `10` will not string-match a stored `10.0`. The flagship guarantee — re-importing an updated statement does not double the ledger — silently fails in the common case, and the preview shows no duplicate flag. The attributed precedent is also not the same rule: `csvValidator.ts` has no duplicate rule at all; the dedupe lives at `CsvImportModal.tsx:163-168`, is in-memory within one run, never compares against rows already in the database, and its key **includes `fee`** while the spec's anomaly key omits it.
* **Recommendation.** Define duplicate matching as `Decimal` equality over `(date, asset_id, type, quantity, price, fee)`; state that this is new behaviour, distinct from the existing presence-only `duplicate_symbol` flag.

### BLK-5 — The serde attribute as written does not compile
* **IDs:** SY-4, W0-1 (part), audit WRONG L144, ON-2
* **Scope:** L143–147
* **Discrepancy.** `#[serde(rename_all = "uppercase")]` is rejected at compile time. serde's `RENAME_RULES` (`serde_derive-1.0.229/src/internals/case.rs:33-53`) lists `UPPERCASE` and compares case-sensitively before returning `ParseError`. "Canonical by construction" also hides a behaviour change: a closed enum rejects `"buy"` while the SQL `CHECK` stays case-insensitive.
* **Recommendation.** `#[serde(rename_all = "UPPERCASE")] enum TxType { Buy, Sell, Split }` with `Display`, and one sentence naming the deliberate asymmetry between DTO and `CHECK`.

### BLK-6 — The dependency posture claim is false
* **IDs:** ZO-6, DS-13, SY-1, ST-8 (in part), audit WRONG L99, W2-3
* **Scope:** L99–101, L136–139
* **Discrepancy.** `Cargo.lock` carries `tower 0.5.3` (`:3791-3792`); `0.6.11` is **`tower-http`** (`:3806-3807`). `hyper 1.10.1`, `hyper-util 0.1.20`, `http 1.4.2` do check out, all via reqwest. `rmcp` is absent from the lock entirely, and `schemars` already appears twice (0.9.0 at `:2867`, 1.2.3 at `:2879`) against `Cargo.toml:20`'s `1.0`. W2-3 sharpens it: `hyper`/`tower`/`http`/`hyper-util` are transitive-only, with no direct `Cargo.toml` entries, so "rmcp is the only new dependency" is false at the manifest level even where the versions are already in the lock. The sentence "no new HTTP framework is added" is the one that buys the supply-chain argument.
* **Recommendation.** SY-1's rewrite verbatim; add tower to the Verification items alongside hyper; if the second listener survives Decision 3, name the four direct manifest entries with their required features (`hyper` `server,http1`; `hyper-util` `tokio,server,service`).

### BLK-7 — Fault isolation is asserted as settled and questioned two sections later
* **IDs:** SY-5, ZO-12, CP-23, DS-6, DS-7
* **Scope:** L460–465 vs L553–555
* **Discrepancy.** "Every tool call **is** wrapped in `catch_unwind` with a 30-second deadline" is stated as decided; the Verification list then asks whether `catch_unwind` is even sufficient for a tokio task panic. That is the load-bearing assumption of the in-process decision. Worse, the design prices only panics: pool (`db.rs:20` `max_connections(5)`, `:44` `acquire_timeout(5s)`, `:70` per-statement `statement_timeout = '30s'`), container (`cpu 500m / memory 256Mi`), and a liveness probe on the same process are all unpriced, and each can take the API down without being a panic.
* **Recommendation.** SY-5's rewrite (mitigation stated as intended, sufficiency as open), plus root E's "costs of sharing a process" subsection.

### BLK-8 — The `.sqlx` prepare-cache obligation is named nowhere
* **IDs:** C-1, DS-15
* **Scope:** Sequencing L527–540, Verification L543–556, Testing L472–512
* **Discrepancy.** Every new `sqlx::query!` in `auth/pat.rs` and `agent/staging.rs` requires, in order: migrate against a live DB → `cargo sqlx prepare -- --all-targets` → commit `.sqlx/`. `ci.yml:142` (`--check`) and `ci.yml:211` (`check_sqlx_cache.sh`) both fail otherwise, and `backend_rust/Dockerfile` builds offline from that cache. The design adds many queries and mentions this in none of the three places.
* **Recommendation.** Add to Sequencing per workstream: "after the migration is applied locally, run `cargo sqlx prepare -- --all-targets` and commit `.sqlx/`; this needs `DATABASE_URL` (`scripts/dev-db.sh`)."

### BLK-9 — WS0 item 1 names the wrong file and leaves the read-side type undecided
* **IDs:** W0-1, audit PARTLY L144
* **Scope:** L143–146
* **Discrepancy.** `TransactionCreate` is at `schemas.rs:18-27`, not `models.rs`. `models.rs:44-51` holds `Transaction`, whose `r#type: String` is the `FromRow` decode target for every `query_as!(Transaction, …)` in `api_routes/transactions.rs` and `backup/`. An implementer who follows the sentence literally and also converts the read-side field will fail to decode every stored lowercase row — rows the spec's own `CHECK (upper(btrim(type)) …)` explicitly permits.
* **Recommendation.** W0-1's replacement text: `TxType` defined in `models.rs`; `schemas::TransactionCreate.r#type` becomes `TxType`; `models::Transaction.r#type` and `schemas::TransactionOut.r#type` **stay `String`** because they decode stored rows; `frontend/src/types.ts:13` unchanged.

### BLK-10 — The scope set is undecided
* **IDs:** W1-1, CP-2, ST-7
* **Scope:** L223–226, L366–382, L229
* **Discrepancy.** The `Scope` enum, `require_scope`, the `api_token.scopes` array contents and the mint form's checkboxes all depend on whether the set is `read|write|import` (spec) or `read|import|apply` (Stage 8 ruling). Nothing in WS1 can be written without it, and the tool table's three "matches batch kind" rows make the enum non-enumerable from the table. L223–226 ("a `read`-only token can preview an import") additionally contradicts the table: a read-only token cannot create a batch, so it has nothing to preview.
* **Recommendation.** Decision 1's scope half. This is the only true WS1 blocker.

### BLK-11 — The single-use flip and the write are specified as two steps
* **IDs:** W2-2, DS-8, DS-21
* **Scope:** L419–423, L499–503, L327
* **Discrepancy.** "Single use is a conditional update" is described as its own step, then "Apply … writes in one Postgres transaction". An implementer following the text puts the flip outside the write; a failed or deadline-killed write (the 30 s per-call deadline equals the pool's per-statement `statement_timeout`) then leaves a batch marked `applied` with zero rows written and no retry path — unrecoverable loss on the one path a human approved. The flip also omits `user_id`, so ownership and state transition are two statements instead of one.
* **Recommendation.** "The flip and the write are the same `SERIALIZABLE` transaction: `BEGIN; UPDATE … WHERE handle=$1 AND status='staged' AND user_id=$2 RETURNING id; <writes>; COMMIT`. Zero rows → `already_applied`, and nothing else in the transaction runs."

### BLK-12 — The eight WRONG citations block, because each one carries a decision
Full corrections in §8. In one line each:

| # | Spec line | What breaks |
| --- | --- | --- |
| 1 | L99 | `tower 0.6.11` → `tower 0.5.3`; 0.6.11 is `tower-http`. The "no new HTTP framework" argument collapses. |
| 2 | L135 | `transactions.rs:238` → `:239` (function starts at `:214`). |
| 3 | L137–139 | `DIVIDEND` is not read as a positive-quantity lot; the engine chain has no `else`, so the row is **silently dropped** from all tax math. |
| 4 | L162–163 | `backup-schema/v1.json` already constrains `type`; WS0 item 3 is a no-op. |
| 5 | L166–168 | `decimal.ts` is a read-side normaliser; the CSV importer sends JSON numbers. See BLK-3. |
| 6 | L372 | The published series is **weekly** (`stats_engine.rs:317-320`), not daily. |
| 7 | L392–396 | The duplicate-skip precedent is `CsvImportModal.tsx:163-168`, not `csvValidator.ts`, and its key includes `fee`. See BLK-4. |
| 8 | L144 | `rename_all = "uppercase"` → `"UPPERCASE"`. See BLK-5. |

## 6. Major findings, by theme

### 6.1 Transport & deployment
* **The second listener's real cost stack is unpriced** (DS-6, DS-7, ZO-12, root E). No connection or concurrency budget against `max_connections(5)` / 5 s acquire / 30 s statement timeout; a 5,000-row stage exists simultaneously as DTO graph + canonical JSONB + rendered preview + `serde_json::Value` of both inside a 256 Mi process that also serves the API; Rocket's per-route body limits (`lib.rs:272-275`) have no counterpart on a raw `hyper` listener, so the cap is enforced only after full deserialization. Fix: semaphore (1–2 concurrent tool calls) or a reserved pool, an explicit listener body limit, caps re-derived from the container limits.
* **Kubernetes exposure is five files and an operator choice, not one ingress line** (W2-4, DS-14, SY-16, audit PARTLY L523–526). Verified: `k8s/app-service.yaml:12-16` is `port: 80 → targetPort: 80`, and the Deployment has **two containers** — `frontend` (`containerPort: 80`, probes on 80) and `backend` (`containerPort: 8000`, probes on 8000). The Service therefore targets the **frontend nginx container**; the backend's 8000 is reachable only through `frontend/nginx.conf:34-35`. There is no existing Service port to reuse. The operator must choose: (i) `location /mcp` in `frontend/nginx.conf` (frontend image rebuild, one Service, one host, inherits whatever policy that host carries), or (ii) new `containerPort` + second Service port + ingress path, bypassing nginx and needing its own TLS/policy decision. Either way every probe targets 8000, so a dead agent listener is invisible to the kubelet — state whether that is intended. This is the single most consequential fact in Stage 9 and it changes the spec's deployment section by an order of magnitude.
* **`listener.rs` is unspecified at exactly the points an implementer must decide** (W2-3). Where it binds (the only viable place is `main.rs` before `.launch()` at `:109`, and it must not bind on the `--print-openapi` / `--print-backup-schema` early-return paths at `:36-43`); what it returns (the e2e test needs `start(pool, cfg) -> anyhow::Result<(SocketAddr, JoinHandle<()>)>` bound to port 0); what a bind failure does (refuse boot, or log and continue — the spec is silent and the probes cannot see it); and the four direct `Cargo.toml` entries with features.
* **Version-resolution risk is unresolved, not trivia** (DS-13, ZO-6, W2-10, audit PARTLY L547–556). `schemars` already resolves twice (0.9.0 via utoipa 4.2, and 1.2.3) against `Cargo.toml:20`'s `1.0`. The Verification item must state the acceptable outcome: rmcp must land on the existing 1.x; a third copy is a blocker for the PR.
* **External evidence is unciteable as written** (SY-8, CP-16). "PR #228" and "PR #642" carry no repository; these two PRs are the load-bearing evidence for the cannot-mount-in-Rocket decision. Give full URLs.
* **"There is no Node.js anywhere in the design" is stated too broadly** (CP-7, DS-22). The same document mandates `npm run generate-types` at L279–281, and the frontend it describes is TypeScript. The real constraint is *no Node in the backend runtime image*; stated as an absolute it makes the supply-chain rationale read as preference.

### 6.2 Auth & scopes
* **`AuthedUser` gains a field and the blast radius is unstated** (W1-5, DS-23). Construction sites: `auth/guard.rs:150`, `auth/oidc.rs:257`, `:295`, `:331`. Signature change to `resolve_caller` (`guard.rs:99`), used by `AuthFairing` and the `FromRequest` guard. Open sub-decisions the spec must make: which module owns `Scope` and `CredentialKind` (recommend `guard.rs`, since `require_scope` is called by handlers), and what `require_scope` returns for a missing scope — `access.rs::deny()` doctrine settles *ownership*, not scope, so 403-vs-404 must be stated explicitly. And the converged lookup must take `expected_tv: Option<i64>`, `None` for tokens, or `logout-all` kills tokens and contradicts the spec's own sentence.
* **The gate is blind to token-id queries, so the spec's claim about it is false** (audit PARTLY L253–262). Rule B's regex is `(FROM|UPDATE|DELETE FROM) (portfolios|assets|transactions) … WHERE id` (`check_access_gate.sh:224`); an `api_token … WHERE id` query is invisible to it and Rule A passes file-level. The isolation matrix, not the script, is the only real check on `DELETE /api/tokens/<id>`.
* **Revocation has no path when the owner cannot reach the UI** (SY-34) — the realistic leaked-token scenario. Record the operator-with-a-database-update fallback, or state that none exists.
* **`require_own_token` and `pat::revoke` both filter by `user_id`** (W1-8). State which is authoritative (the `access.rs` one), that a foreign id is a 404 via `deny()` and not a 403, and that `pat::revoke` keeps its predicate as defence in depth.
* **The `last_used_at` throttle is an unspecified mechanism with no beneficiary** (ZO-15, W1-7, SY-35). "At most once a minute per token, fire-and-forget" does not say where the state lives. At `replicas: 1` with one operator, write it inline or drop the column and keep the 90-day marker.
* **The 90-day marker asserts unmeasured behaviour** (SY-10). "The nudge that gets acted on" is a behavioural claim nobody measured; the threshold has no basis or knob.
* **Entropy wording** (SY-9; ST-9 withdrawn per ON-3). Say "32 bytes of CSPRNG output (256 bits at the source), base62-encoded, ≈43 characters", and replace "stated so nobody adds one later" with an instruction.

### 6.3 Staging & apply
* **`stage_edit` is a second full CRUD API with no requirement behind it** (ZO-10, DS-11, CP-6, SY-21, SY-22). Nothing in Success criteria or Requirements asks for batched update/delete of portfolios, assets and transactions; it roughly doubles the surface of the one path where a wrong write is a wrong tax report. The payload sketch offers only `update_portfolios` while the table promises update of all three kinds, and `create_*` shapes are elided. **Ruled cut** — see §9.
* **Agent-side deletes are weaker than the thing the design excluded** (DS-11). The non-goal that keeps `mode=replace` in the web UI because a typed-word confirm is the only honest gate applies with *more* force to a surface where the confirmer is the proposer.
* **No retention and no aggregate limit** (DS-12, CP-24, SY-11). `payload` and `preview` JSONB (up to 5,000 transactions each) are retained forever after `applied`/`discarded`; the sweep runs only when that user stages the next batch, so a user who stops staging keeps rows indefinitely despite `expires_at NOT NULL`. A looping agent, not an oversized single call, is the realistic failure mode.
* **`get_staged` states no expiry check** (DS-20). A human can be shown a preview of an already-expired batch and then receive `stale_handle`. Same predicate, both tools.
* **"Canonicalized payload" is load-bearing and undefined** (CP-13). Key order, whether defaults are filled at stage or apply time, and whether the 60-minute re-validation can disagree with what was previewed — the preview and the apply both read it and a test asserts byte-for-byte round-trip.
* **`stage_import` is five deliverables, not one** (W2-7): DTO+validation, asset matching, preview render, apply, caps/codes. The `(b)`-merge seam is separately gated on the `Decimal`-equality definition because `target.existing` is precisely the capability `backup/restore.rs:22-24` refused.
* **The 25-portfolio cap is unreachable from the documented payload** (CP-4): `stage_import` accepts exactly one `target`, `new` *or* `existing`. Either the payload is incomplete or the cap belongs to a tool that is now cut.
* **Preview token economics are unquantified** (DS-16). Reused display caps (25 portfolios / 50 assets per portfolio) plus an anomaly list can be tens of thousands of tokens, and `apply_staged` requires the model to echo names back out of that text. The cost of the feature to the operator's LLM budget is unstated, and an over-large preview degrades the one review step the design depends on.
* **Cap provenance is a bare hash** (CP-9, SY-24). The incident is real — `af17064 fix(backup): the proxy refused a bundle the API would have accepted (#68)` — but a hash is not a derivation. State whether 25/200/5,000 are proxy-driven (and must be re-checked if the proxy changes) or memory-driven, and why they are hardcoded while the TTL is configurable.

### 6.4 Money & types
* **WS0 item 3 is already shipped** (ZO-3, DS-18, W0-8, audit WRONG L162–163, ON-3). Reword to "verify the existing constraint and confirm no shipped fixture violates it", and drop the matching test bullet, or the plan carries a no-op task.
* **`TxType` would be a second source of truth for one closed set** (ZO-4). `backup::KNOWN_TRANSACTION_TYPES` (`backup/mod.rs:222`) already feeds the bundle schema and validator; `frontend/src/types.ts:13` is a third copy; `backup/mod.rs:231-237` documents this exact drift class as a bug that was just fixed for `asset_type`. Make `TxType` that constant's source and say so.
* **The `DIVIDEND` hazard is described wrongly** (ZO-7, audit WRONG L137–139). Both engines use an `if/else if` chain over the three literals with no terminal `else` (`tax_engine.rs:665,680,771`; `stats_engine.rs:250-256`), so an unknown type matches nothing and produces no lot at all. The real defect — a ledger row that renders in the UI and moves no figure — still justifies WS0. Fix the description, keep the fix.
* **The case-sensitivity triangle has no resolution instruction** (ZO-8, DS-9, DS-10, W0-3). The proposed `CHECK (upper(btrim(type)) IN …)` is case-insensitive; the existing partial unique index `uq_split_asset_date … WHERE type = 'SPLIT'` (`migrations/0001_init.sql:89-91`) is case-sensitive and is pinned **literally** by `check_migrations.sh:169-175`; `access.rs:260` seeds a lowercase `'buy'` row. A `'split'` row passes the new CHECK and escapes the one-SPLIT-per-asset-per-day dedupe that exists to prevent bug #50, and the migration can then fail on a unique-index violation rather than the intended CHECK violation. Pick one: (a) exact CHECK after normalization — must also change the index predicate, `check_migrations.sh:169-175` and `access.rs:260`; or (b) case-insensitive CHECK — must state the escape and add a post-normalization `(asset_id, day)` duplicate check to the pre-flight audit. Name the four files either way.
* **`TxType` makes lowercase a 422 on a live endpoint** (DS-10). Today `transactions.rs:239` uppercases, so `"buy"` is accepted; the migration simultaneously declares case non-semantic. State the client impact and either accept case-insensitive deserialization or declare the break.
* **A fourth spelling of "is this a decimal"** (ZO-13). `db_types::decimal_json` is already the shared string-or-number deserializer at ~55 sites, `Decimal::from_str_exact` is already the string-exactness rule (`backup/mod.rs:472,532`), and `rust_decimal` ships strict string serde. Prefer `from_str_exact` + `scale() <= 12` in one place over a hand-written regex plus bespoke visitor.
* **"Abort, listing the offending values" does not say how, and the atomicity consequence is unstated** (W0-4). A `.sql` migration needs plpgsql (`DO $$ … RAISE EXCEPTION … $$`); sqlx runs each file in one transaction, so the `upper(btrim(type))` normalization rolls back with the abort — probably intended, but unstated. Blast radius is concrete: `main.rs:74-76` turns a failed `sqlx::migrate!` into `refuse()` → `exit(1)` → crashloop, with all three probes on the same container.
* **The two paths cannot share a date deserializer as implied** (audit PARTLY L345–347). The range bound is reused, but `TransactionCreate.date` is `DateTime<Utc>` and a bare `YYYY-MM-DD` is rejected **by design** (`tests/date_boundary_test.rs:46-57`: "a bare date must not silently become midnight UTC"). The agent path wants the bare form. Also SY-13: "resolved to UTC midnight" hides whether the column stores a `date`, a timestamp or a string.
* **The `asset_type` claim is incomplete** (audit PARTLY L177–180). No engine reads it, but `backup/mod.rs:431-434` already rejects an out-of-set `asset_type` at bundle validation and `v1.json:6-15` already enumerates it.
* **`get_performance` promises daily and ships weekly** (audit WRONG L372). `stats_engine.rs:317` `aggregate_weekly(history)`, `:320` `"history": weekly_history` — items still carry a field literally named `daily_return`.
* **Two more P&L fields are constants, and they are the flagship question** (DS-5, W2-8). `unrealized_pnl` and `realized_pnl` are hardcoded `"0.0"` at `stats_engine.rs:328-329`; the tool note disclaims only volatility/Sharpe/beta. Success criterion #1 is literally "what is my realized gain in 2025?", so an agent that calls `get_performance` instead of `get_tax_summary` gets a confident zero. Omit both fields from the tool and name the three constants in its description.

### 6.5 Validation & contracts
* **Three overlapping notions of a bad asset** (CP-12, SY-12). `unknown_asset` as an error code, "unknown symbol" as a *non-blocking* anomaly, and "missing assets auto-create" — the document says a bad ISIN is "a named rejection, not a silently unmatched asset", which implies unknown symbols are not rejected, while the error code says otherwise. Separately, "unusual price scale" has no threshold and no comparison target: an implementer cannot build it and a tester cannot assert it. Give it a rule or delete it.
* **No transaction currency field, yet currency anomalies are flagged** (CP-5). The documented row is `{type, date, quantity, price, fee}`; asset `currency` is optional with a stated default. The "German broker statement in EUR" scenario the document motivates cannot be evaluated as written.
* **A second ISIN validator where one already exists on two write paths** (ZO-5, W2-6, audit PARTLY L349–351). `api_routes/lookup.rs:186-199 is_valid_isin` is shape-only (length 12 + alphanumeric, no checksum) and is already used by `api_routes/transactions.rs:5,49` and `backup/mod.rs:440-444`; the browser's `isValidIsin` is a bare regex (`isinValidator.ts:1-3`), so neither checks a checksum today. Either strengthen it in place so all three paths agree — noting that adding a checksum also changes lookup and restore behaviour for bundles that pass today — or declare the divergence intentional and name it.
* **Symbol resolution is new work, not existing capability** (audit PARTLY L373). `lookup.rs:45` is ISIN-only (`/assets/lookup?<isin>` → OpenFIGI, then Yahoo).
* **The payload blocks are not wire contracts** (SY-20, SY-21, ST-15). `{ target: { new: {...} } | { existing: <portfolio_id> } }` does not say whether the union is externally tagged, internally tagged with a discriminator, or a nullable pair; `delete_portfolios: [<id>]` versus `delete_assets: [{id}]` is two shapes for one operation; `create_*` shapes are elided without saying they reuse `stage_import`'s; all five fences are unlabeled, and three of them are not valid JSON. Spell the tagged unions, use one shape per list, label the two DDL fences ` ```sql ` as the executable contracts they are.
* **"Batch fingerprint" is logged and never defined** (CP-10, SY-23). Hash of what, which algorithm, over what canonicalization, joinable to what. Say SHA-256 prefix of the canonical payload for correlation, and say why a fingerprint of financial data is acceptable under the document's own "never the payload" privacy rule.
* **The tool table's columns are not columns** (ST-7, SY-38). The Scope column holds seven enum literals and three prose sentences; `discard_staged`'s Notes cell is empty while eleven others are filled; `apply_staged`'s note restates its own scope cell.
* **Success criteria are not measurable** (SY-6). Two quoted user questions have no oracle or tolerance; "exact counts, exact decimal values" does not say what is compared against what; "a mutation test proving it can fail" overstates what a mutation test shows.
* **Error codes are specified as advice to a model instead of a client contract** (SY-31). "Content the model *should* read and act on" and "the model's correct response differs" are unverifiable. State the branch: `already_applied` → report the earlier success; `stale_handle` → re-stage.
* **The isolation-test extension has no stated harness** (W2-5). `tests/isolation_test.rs` is HTTP-shaped (`Client::tracked(build_rocket(pool, as_user))`), and agent tools have no HTTP surface under the second-listener design. Either call `tools::` directly with a `Credential::Token` `AuthedUser` minted through `pat::mint`, or drive the listener over the transport with two tokens — different tests, different setup. Name it, and list the matrix pairs (B-applies-A's-handle, B-stages-into-A's-portfolio, B-`get_staged`-A's-handle, each with an A-succeeds case).
* **The named two-listener test is vacuous in the harness the spec points at** (W1-6). `tests/common/mod.rs:115` builds `AuthMode::Disabled { as_user }`, which authenticates every request regardless of headers, and `tests/isolation_test.rs` is entirely disabled-mode. The assertion needs `common::build_rocket_oidc(pool)` (`:138`).
* **The gate marker rule is stated incompletely** (W1-4). `check_access_gate.sh` honours `// access-gate: global-config (no caller-supplied id)` only as the whole line immediately above the route attribute — i.e. **below** `#[utoipa::path(...)]`, as `api_routes/backup.rs:153` demonstrates — and refuses it outright for routes with a path parameter. Put it in the wrong place and Rule A fails with a message that reads as a gate bug.

### 6.6 Testing & gates
* **Existing gates will fail on the new code, and the spec enumerates new gates instead** (DS-15, C-1, W1-2). Adding `api_routes/tokens.rs` is not enough: the routes must be added to the mount list in `src/lib.rs:320-333`, to the `paths(...)` inventory in `src/openapi.rs:12-61`, with request/response types in `src/schemas.rs` carrying `ToSchema`; then two committed artifacts are CI-diffed — `docs/openapi/openapi.json` (`ci.yml:231`) and `frontend/src/types/api.d.ts` (`ci.yml:341`).
* **Three hidden cardinality pins; the spec names one** (W1-3, audit PARTLY L263–267, ZO-16). `route_coverage_test.rs:169` `assert_eq!(checked, 21)`, `:291` `PROTECTED_ROUTES.len() == 21`, `:304` `with_level == 15` with the message "six routes authenticate without checking an id" → 24 / 24 / 16 and "eight". Plus `access_gate_test.rs:87` `"handlers inspected: 21"` → 24, and two prose copies that go stale: `docs/auth-testing.md:45-46` and `check_access_gate.sh:48`.
* **The `DATABASE_URL` claim is wrong in both directions** (C-3, SY-29, DS-24). `AGENTS.md:99-101` says five of eleven gates need it *including* `cargo test`, so "the other five" is four; and the new `protocol.rs` deserialization table test and `tests/agent_gate_test.rs` must pass **without** a URL — `ci.yml:223` deliberately keeps clippy URL-free to prove the offline path. An implementer who makes the gate's mutation test depend on a database breaks the very property that proves the gate can fail. Split the bullet into "needs `DATABASE_URL`" and "must pass with none".
* **Gate-script identity is undecided and the document picks both answers** (C-4, ZO-9). The body's reuse promise points at extending `check_access_gate.sh`, which already parameterises its target (`HANDLER_DIR`, `:72`) and already implements the three rules the new script would re-implement; Testing names a fifth script plus a new mutation test. If Decision 3 lands on the in-Rocket mount, the new script is unnecessary and `route_coverage_test.rs:181 PUBLIC_NAMESPACES` is the amendment instead. One sentence must name the file that changes and the file that does not exist.
* **WS0's named verification cannot live where the spec puts it** (W0-5). "`check_migrations.sh` gains a case proving it fails on a junk value" contradicts that gate's documented contract — it inspects a **database**, not `migrations/` — and CI's migrate step (`ci.yml:132`) runs against an empty service database. Nor can it use `src/test_db::test_pool`, because `test_db.rs:201` runs the embedded `sqlx::migrate!` before the test body, so junk can never be present *before* migration. Home: `tests/migration_type_check_test.rs` with a runtime `sqlx::migrate::Migrator::new(Path::new("./migrations"))`, apply `0001`'s `up()`, insert a `'DIVIDEND'` row, apply `0002`'s `up()`, assert the error names the offending value.
* **"The runbook" is not an artifact that exists** (CP-8, ST-11, SY-17, W0-6). The mitigation for the design's highest-blast-radius failure (app does not boot) points at a document nobody names; `backend_rust/scripts/pre_cutover_audit.sh` exists but is SQLite-only by design and cannot host a Postgres query. Name a new `backend_rust/scripts/audit_transaction_types.sh` with the repo's exit convention (0 clean / 1 out-of-set rows, printing `SELECT DISTINCT type` / 2 could not run) and reference it from Sequencing.
* **Migration numbers are contested across three independently shippable PRs** (C-2, Stage 5 gap 6). `0002_`, `0003_`, `0004_` are all assigned while Sequencing makes three PRs, against a gate requiring one ledger row per `*.sql` with a matching sha384 (`check_migrations.sh:113-136`). One sentence: numbers are reserved in merge order; a PR that lands second renumbers and updates its own prose and test names.
* **`check_migrations.sh` needs more than the one case the spec mentions** (ZO-16, audit PARTLY L158–159). It pins per-column types, not-null, identity and PK facts for "all 36 columns of the six tables" and asserts no `CHECK` constraints today; both new tables need entries or they pass it silently.
* **WS0 is four slices, not one** (W0-9). 0a `TxType` + `TransactionCreate` + serde test (backend only, no migration); 0b migration `0002` + audit script + abort test (needs `DATABASE_URL`, deploy-order sensitive, own PR); 0c-i frontend money-as-string migration; 0c-ii backend strict deserializer (**cannot merge before 0c-i is deployed**); 0d write-service extraction (no behaviour change, own PR).

### 6.7 Documentation & accountability
* **Two standard-mandated sections are missing and no header names the plan** (ST-1, ST-2, ST-3). No `Alternatives Considered`, no `Risks and Mitigations`; rejection rationale survives only as a trailing `Rejected:` clause inside one of five bold-lead paragraphs, and risk content is scattered across four sections. Both sibling specs in the same directory carry these headers, so this is a local convention break.
* **No TOC and no anchors in 556 lines / 27 headings** (ST-4). Three in-document references are prose pointers ("see *Sessions*", "See Workstream 0, item 5", "the flagship, below"), and item-number references break the moment an item is inserted.
* **Workstream asymmetry** (ST-5, SY-64). WS0 has zero H3 subsections, expresses five decisions as a numbered list whose item 4 nests a second paragraph, and owns migration `0002` without a `### Schema —` header, breaking the `0003`/`0004` pattern. H3 child counts are 0 / 5 / 7, so the three workstreams cannot be read against each other.
* **Rollback is asserted and documented nowhere** (ST-6, SY-19). "Each workstream is independently revertible" — WS0's revert path (drop the `CHECK`; rows written after deploy are unaffected because the accepted set does not widen) is the least obvious of the three and is exactly the sentence that is missing.
* **Success criteria and Requirements are two unlinked restatements** (ST-12). Six capabilities in two vocabularies, no cross-reference, no traceability to workstreams.
* **The actor is never named** (SY-15, SY-16, SY-17, SY-18, SY-34, SY-50, SY-55, SY-26). Across 556 lines nobody confirms an apply, sweeps a row, runs the audit query, edits the ingress, revokes when the UI is unreachable, or is bound by the `TEXT`→`NUMERIC` prohibition. "Any future change to `replicas` must revisit this" names a change, not an actor and not a gate.
* **Aphorisms are standing in for decisions** (SY-25, SY-27, SY-28, SY-32, SY-33, SY-43, SY-46, SY-47, SY-60, SY-61 and the rhetoric table). Where the rhetorical sentence is load-bearing, a decision is missing: what "swept" does to a row, what "echoed back by name" means for a transaction, what "unusual price scale" measures, who runs the audit. The copy reviewer supplied most rewrites verbatim; this pass is mechanical.
* **The input channel for the broker statement is never defined** (SY-60). "A broker statement the agent has read" assumes the agent already holds it, and "No PDF extraction" then rules out the most likely source without saying what the user does instead. One Overview sentence: the statement reaches the agent outside this system; Portfolio Prism receives only structured rows, never a document.
* **The MCP client is never named** (Stage 8, gap 1). Four passes reviewed a design whose central mechanism — the human reads the preview in chat — depends entirely on software the document does not identify. Does it truncate long tool output? Relay a 20k-token preview verbatim or summarize it? Retry a timed-out `apply_staged`? DS-16's token hole is a symptom; the cause is an unspecified integration partner. One paragraph, plus one end-to-end run against that client as an acceptance gate.
* **The premise is never tested** (Stage 8, gap 2). The founding bet is "the model normalizes the broker statement", and every test assumes the rows are already structured: the e2e test posts a synthetic payload, the golden fixture asserts the *preview*, not the model's accuracy. A ten-row manual accuracy run against two or three real anonymized statements, recorded in the spec, is the cheapest viability check available and nothing proposes it.
* **The threat model names two adversaries, not three** (Stage 8, gap 3). The document reasons about a misreading model and a leaked token and never names the third: the broker statement is untrusted input to an agent that can write rows. Text that reads as instructions is the actual prompt-injection path, and the staged-batch gate is its containment boundary — but the design never says so, so it will be built as a UX nicety.
* **The Verification list mislabels its own items** (SY-30). "These are unknowns about external APIs, not open design questions" is contradicted by item 4 (`catch_unwind` sufficiency — internal design) and item 5 (live-row audit — a data query).
* **Structural nits that read as accountability gaps** (ST-13, ST-14, ST-16, ST-17). Front matter carries a policy requirement that forward-references a section 124 lines away; `---` rules precede six H2 boundaries but not the last two, implying subordination; workstream H2 titles are not parallel; the Testing section labels one bullet by workstream and the rest by file, so "does WS1 have test coverage?" is unanswerable from the structure.

## 7. Nits

`→` marks a nit folded into a blocker or major above; it is listed here for traceability, not re-litigated.

| ID | Location | Nit | Fix |
| --- | --- | --- | --- |
| ZO-13 | L166–170, L338–341 | Fourth spelling of "is this a decimal" | → §6.4 — `from_str_exact` + `scale() <= 12`, one place |
| ZO-14 | L200–217, L300–315, L327, L338 | Full DDL, exact regex, exact `UPDATE … RETURNING`, exact serde attribute are plan altitude while the real interface is unspecified | Move SQL/regex down to the plan; lift the shared-write-service interface up |
| ZO-15 | L234–235 | Per-token write throttle + fire-and-forget at `replicas: 1` | → §6.2 — write inline or drop the column |
| ZO-16 | L262–266, L505, L509 | `PROTECTED_ROUTES.len()` pin and per-column `check_migrations.sh` entries not mentioned; "the other five gates" is loose | → §6.6 |
| CP-14 | L11–20, L271–277 | "Portfolio Prism" never described in one sentence; agent / model / client used as if defined | Add a one-line product description and an actor gloss |
| CP-15 | L101–103, L548 | SSE never expanded; "streamable HTTP" never explained; no link to spec revision `2026-07-28` | Expand once; link the revision |
| CP-16 | L96–100 | PR #228 / #642 cited without URLs | → §6.1 (SY-8) |
| CP-17 | L192, L231, L253, L257 | `AuthFairing`, `/auth/me`, `logout-all`, the `Access::*` level set used without a gloss | Gloss each at first use |
| CP-18 | L358 | "security" introduced as a second word for "asset" without saying so | Say they are the same, or use "asset" |
| CP-19 | L372 | TWR (and ISIN) unexpanded, at the one place the tool's output contract depends on it | Expand TWR at first use |
| CP-20 | L487–489 vs L401–403 | "flagged rather than silently dropped" vs "flagged in the preview and skipped on apply" read as opposite behaviours | → BLK-1 / §6.3; define *flagged* once |
| CP-21 | L491 | Gate regex quoted malformed (`^#[(get\|post\|...)]`) | Quote the real awk pattern |
| CP-22 | L66 vs L130–166 | Requirement 5 carries no WS0 caveat while WS0 changes live behaviour | → BLK-3 |
| CP-23 | L551–553 vs L461–464 | Two sections state different confidence about `catch_unwind` | → BLK-7 |
| CP-24 | L316–322 | Stale batches persist indefinitely for a user who stops staging, despite `expires_at NOT NULL` | → §6.3 retention rule |
| DS-17 | L31–34, L96 | `to_uppercase()` is `:239` not `:238`; "the only three literals in `src/`" ignores `KNOWN_TRANSACTION_TYPES` | → §8 errata rows 2 and 3 |
| DS-18 | L100–103 | WS0 item 3 already implemented | → §6.4 / §8 row 4 |
| DS-19 | L344 | Zero-price rationale cites a nonexistent roadmap file | **Out of scope per ON-3** — not carried forward |
| DS-20 | L419, L468 | `get_staged` states no expiry check | → §6.3 |
| DS-21 | L417–423 | Single-use `UPDATE` omits `user_id` | → BLK-11 |
| DS-22 | L15–17, L44–49 | "No Node.js anywhere" too broad | → §6.1 |
| DS-23 | L122–127 vs L150–152 | Token lookup vs `logout-all` contradiction; state `expected_tv: Option<i64>` | → §6.2 |
| DS-24 | L519 | "the other five gates" precision | → §6.6 (C-3, SY-29) |
| ST-13 | L1–6 | Front matter carries a policy assertion that forward-references L130 | Move to Requirements; keep front matter metadata-only |
| ST-14 | L128, L185, L283, L429, L470, L514 | `---` before six H2s, absent before `## Sequencing` and `## Verification items` | Add the two missing rules |
| ST-15 | L205, L305, L386, L410, L446 | Five unlabeled fences; three payload blocks are not valid JSON | → §6.5 — label DDL ` ```sql `, sketches ` ```json ` |
| ST-16 | L130 vs L187 vs L285 | Workstream H2 titles not parallel | Make all three `Workstream N — noun phrase` |
| ST-17 | L472–512 | Test bullets mix workstream-scoped and file-scoped labels; no "Workstream 1 tests" | Label consistently |
| SY-29 | L474–476 | "the other five gates" is four | → §6.6 |
| SY-35 | L234–235 | "fire-and-forget" hides who sees a failed `last_used_at` write | Say it is logged and ignored; a call never waits on it |
| SY-36 | L340–341 | "a German broker statement is exactly what produces `142,30`" — one locale for a general case | "European decimal-comma statements produce values such as `142,30`" |
| SY-39 | L392–393 | "there exact-duplicate rows are flagged"; *flagged* used with three meanings | Define *flagged* once; then use it |
| SY-41 | L404–407 | "**Apply.** Re-validates … then writes …" — subjectless | "**Apply.** The backend re-validates …, writes …, and returns …" |
| SY-42 | L149 | "the contract the browser importer has always enforced" — unprovable absolute | "formalizes the contract `csvValidator.ts:99` enforces today" |
| SY-44 | L39–41 | Heading "The chokepoint stays the chokepoint" is a tautology; "decorative" is a metaphor | "`access.rs` stays the only id resolver"; state the bypass concretely |
| SY-45 | L24–25 | "each was chosen deliberately rather than fallen into" asserts care, not a fact | Delete |
| SY-48 | L46–47, L355, L509 | "exact totals are always exact" is a tautology in quotation marks | State what the fixture asserts byte-for-byte |
| SY-49 | L520, L512 | "unset means the module does not exist"; "PDF parsing, which does not exist" | "the listener never binds and no agent tool is reachable"; "PDF extraction, which this design does not implement" |
| SY-51 | L164–165 | Wrong rationale for the case-insensitive `CHECK` | It exists for rows written before the migration, not for engine comparisons |
| SY-52 | L201 | "`require_scope` allows `Session` always" is misreadable | "returns ok for a session credential, and for a token credential only when that scope is present" |
| SY-53 | L436 | "a 403 is an existence oracle" — jargon | "a 403 confirms the row exists in someone else's account" |
| SY-54 | L457–459 | "Never leaked: …" fragment; "named by index" ambiguous | Name the actor; say zero-based index in the submitted payload |
| SY-55 | L132, L6 | Subjectless "Ships and deploys **before**"; "Workstream 0 says so explicitly" | Name the workstream as the actor in both |
| SY-56 | L19 | "The agent is allowed to *propose*" — permission framed as temperament | "No agent tool call writes a row directly." |
| SY-57 | L51 | "cannot be applied by the user who did not stage it" — double negative reading as its opposite | "cannot be applied by a different user, and cannot be applied after it expires" |
| SY-58 | L72–73 | "The whole feature is off unless explicitly configured on" is a riddle | "The feature is disabled until `AGENT_LISTEN` is set." |
| SY-59 | L335, L447, L293 | Fragment connectors ("Then:", "Payload shape, defined once:", "Owns X and nothing else.") | Complete the sentences |
| SY-61 | L66, L433, L31, L333 | *discipline* as a virtue word; *deterministic* never defined | Define *deterministic* once at L31; replace *discipline* with the rule |
| SY-62 | L68–73 | Requirements mix grammatical levels | One shape per requirement |
| SY-63 | L72–85 | Non-goals mix "**No X.**" bullets with a full sentence | Bold lead + one consequence sentence |
| SY-64 | L168–176 | WS0 item 4 nests a bold paragraph at a different list level | Give the `TEXT`-columns note its own item |
| SY-65 | L330 | "which is the failure mode that actually happens" — frequency claim with no incident | "so a retry after a timeout cannot post the batch twice" |
| SY-66 | L30 | "and nothing downstream would notice" — absolute about a system full of checks | "…and the schema accepts it: the error surfaces only in the tax report" |
| SY-67 | L108 | "curl" as a verb; "raw write endpoints" a third name for `/api` | "cannot call a write endpoint on `/api`, because those handlers do not look at bearer tokens" |
| SY-68 | L452–453 | "*consumer* retries" — fourth name for the agent | "`hint` exists because the agent retries on error" |
| SY-69 | L462–463 | 30-second deadline has no stated basis; no tool named as the one that can hang | Name `get_performance`'s upstream pricing call; make the value configurable |
| SY-70 | L14, L98–103, L299, L547 | "streamable HTTP" written four ways | Unhyphenated noun, hyphenated modifier, consistently |
| W0-8 | WS0 item 3 | Already-shipped work specified as prerequisite | → §6.4; drop the matching test bullet |
| W1-7 | L234–235 | `last_used_at` throttle mechanism unspecified | → §6.2 |
| W1-8 | `require_own_token` / `pat::revoke` | Both filter by `user_id`; authority unstated | → §6.2 |
| W1-9 | `AccessTokensPanel.tsx` | Three deliverables in one task; Access view must render outside the `currentPortfolio ? …` branch (like `backup` at `App.tsx:791`) or it is a blank page; `NAV_ITEMS` is `App.tsx:53`, not `:657` | Split into list / mint form / reveal modal; record the placement trap |
| W2-9 | Config | `AGENT_BATCH_TTL_MINUTES` parsing convention unstated | Follow `BackupConfig::from_env_with(&dyn Fn)` (`backup/mod.rs:103-114`) so it is testable without mutating process env |
| W2-10 | Verification | `schemars` pin left to the implementer | → §6.1 — state the acceptable outcome: rmcp must resolve to the existing 1.2.3; a third copy blocks the PR |

## 8. Citation errata

Work this table top to bottom in one editing pass. Line numbers are the spec's.

### 8.1 WRONG — eight items, each load-bearing

| Spec line | Claim | Correction |
| --- | --- | --- |
| 99 | "`hyper 1.10.1`, `hyper-util 0.1.20`, **`tower 0.6.11`**, `http 1.4.2` are already in `Cargo.lock` via reqwest" | `tower` is **0.5.3** (`Cargo.lock:3791-3792`); 0.6.11 is **`tower-http`** (`:3806-3807`). Write "`tower 0.5.3` (and `tower-http 0.6.11`)", and add tower to the Verification items, not just hyper. hyper/hyper-util/http do check out (`:1351`, `:1387`, `:1260`). |
| 135 | "`create_transaction` at `api_routes/transactions.rs:238`" | Function starts at `:214`; the `tx.r#type.to_uppercase()` line is **`:239`**. |
| 137–139 | "a stored `DIVIDEND` is read by the tax engine as a positive-quantity lot" | The engine chain is `if BUY / else if SELL / else if SPLIT` with **no terminal `else`** (`tax_engine.rs:665,680,771`; the `} else {` at `:822` belongs to an outer block). A `DIVIDEND` row matches nothing and produces **no lot at all** — silently ignored, same shape at `stats_engine.rs:250-256`. Say "invisible tax data", which is quieter than a wrong lot, not a wrong lot. |
| 144 | "In `models.rs`: `#[serde(rename_all = "uppercase")] enum TxType {…}`; `TransactionCreate.type` becomes `TxType`" | `models.rs` holds row structs only; `TransactionCreate` is `schemas.rs:18` with `r#type: String` at `:19`. serde's rule is `"UPPERCASE"` (ON-2). Write "`schemas.rs:18` `TransactionCreate.r#type: String` becomes `TxType`" and `rename_all = "UPPERCASE"`. |
| 162–163 | "`docs/backup-schema/v1.json` **must** constrain `type`" | It **already** does: `v1.json:134-142` enumerates `type` from `backup/mod.rs:222` via `transaction_type_schema` (`:242-246`); `validate()` refuses an unknown type at preview (`backup/mod.rs:467-470`); `backup/schema.rs:49-64` pins artifact↔constant; CI diffs the artifact (`ci.yml:250-252`). Remaining work is only the DB `CHECK` and confirming no shipped fixture violates it. |
| 166–168 | "`frontend/src/utils/decimal.ts` already sends strings" | `decimal.ts` is a **response** normaliser (`num`/`toNumber` `:16-23`, `normalizeTransaction` `:97-104`). The string-sending write path is `PortfolioDetail.tsx:15-21` + `:171-177`. The CSV importer sends JSON **numbers** (`csvValidator.ts:187-189` → `CsvImportModal.tsx:170-176`). Rejecting numbers on `TransactionCreate` is a breaking change to the browser importer and needs the importer changed first. |
| 372 | "`get_performance`: **daily** value series and cumulative TWR" | The published series is **weekly**: `stats_engine.rs:317` `aggregate_weekly(history)`, `:320` `"history": weekly_history` (items still carry a field named `daily_return`). Say "weekly value series, aggregated from a daily internal series". The constants claim is confirmed at `:108-110` **and again at `:323-325`**. |
| 392–396 | "exact-duplicate rows skipped on apply — the same rule `csvValidator.ts` applies" | The rule is `CsvImportModal.tsx:163-168` (`dedupKey = ${isoDate}-${asset.id}-${type}-${quantity}-${price}-${fee}`, "Duplicate transaction skipped"). `csvValidator.ts` has no duplicate rule. The browser key **includes `fee`**; the spec's anomaly key (L358) omits it. And the browser dedupe is in-memory within one run — it never compares against rows already in the database. |

### 8.2 PARTLY — sixteen items

| Spec line | Claim | Correction |
| --- | --- | --- |
| 35 | "The toolchain here is `rustc 1.98.1`" | No `rust-toolchain.toml`; the only pin is `rust:1.92-slim` (`backend_rust/Dockerfile:4`), CI uses `rust-toolchain@stable` (`ci.yml:44`). Say "local toolchain 1.98.1; the build image pins `rust:1.92-slim`, which is the binding floor and clears MSRV 1.88". |
| 40 | "`access.rs` is the only module permitted to resolve a caller-supplied id" | The module's own header scopes it to "a **portfolio, asset or transaction** from a caller-supplied id" (`access.rs:1-2`), and Rule B's regex matches only those three tables. Extending it to tokens and staged batches is a widening of its stated job — say so. |
| 144 | `models.rs` / `TransactionCreate` placement | See §8.1 row 4; also state that `models::Transaction.r#type` and `schemas::TransactionOut.r#type` stay `String` (BLK-9). |
| 158–159 | "`check_migrations.sh` gains a case proving it fails on a junk value" | The gate inspects a database, not `migrations/`, and asserts no `CHECK` constraints today (`:5`, `:239-242`). It needs a constraint check **and** entries for `api_token` / `agent_staged_batch`, or the spec must state that extra tables pass it silently. The abort test belongs in Rust (W0-5). |
| 177–180 | "`asset_type` … nothing computes on it — no engine reads it" | No engine reads it, but `backup/mod.rs:431-434` rejects an out-of-set `asset_type` at bundle validation and `v1.json:6-15` enumerates it. |
| 191–197 | "`resolve_caller` is the only path shared by `AuthFairing` and the `/auth/me` guard" | Call sites are `fairing.rs:83` and the **`FromRequest` impl at `guard.rs:180`** — that impl is the guard for all 21 handlers, not just `/auth/me`. |
| 220–221 | "32 random bytes, base62 … 256 bits of entropy" | Self-contradictory as read: 32 base62 *characters* encode ≈190 bits. Say "32 bytes of CSPRNG output (256 bits at the source), base62-encoded (≈43 characters)". The 256-bit conclusion is correct (ON-3); only the wording changes. |
| 253–262 | "token routes are ordinary `/api` routes so `check_access_gate.sh` sees them" | The gate will count the handler (Rule C) but is blind to an `api_token … WHERE id` query — Rule B's regex is `(FROM\|UPDATE\|DELETE FROM) (portfolios\|assets\|transactions) … WHERE id` (`check_access_gate.sh:224`). The isolation matrix, not the script, is what pins `DELETE /api/tokens/<id>`. |
| 263–267 | "all three routes must be added to `PROTECTED_ROUTES`" | Three hardcoded cardinalities also break: `:169` `checked == 21`, `:291` `PROTECTED_ROUTES.len() == 21`, `:304-306` `with_level == 15` ("six routes authenticate without checking an id"). They move to 24 / 24 / 16 and "eight". Also `access_gate_test.rs:87` `"handlers inspected: 21"`. |
| 300 | "the precedent at `lib.rs:268`" | `:268` is the comment; the code is `let oidc_routes = matches!(auth, AuthMode::Oidc(_));` at **`:270`**. |
| 345–347 | "Dates are strict ISO `YYYY-MM-DD` … bounded 1900-01-01…2100-01-01 **as `create_transaction` already does**" | The range check is reused (`transactions.rs:248-252`); the **format** is not — the interactive endpoint requires RFC 3339 and rejects a bare date by design (`tests/date_boundary_test.rs:46-57`: "a bare date must not silently become midnight UTC"). The two paths cannot share a date deserializer. |
| 349–351 | "ISIN … mod-10 checksum implemented in Rust; `isinValidator.ts` lives in the browser and this path cannot depend on it" | A Rust validator already exists server-side — `api_routes::lookup::is_valid_isin` (`lookup.rs:186-198`), called from bundle validation (`backup/mod.rs:442`) and the REST write path. It does shape only; `isinValidator.ts:1-3` is likewise a bare regex. Extend the existing one, and note that adding a checksum also changes lookup and restore behaviour for bundles that pass today. |
| 373 | "`lookup_asset`: symbol / ISIN resolution" | ISIN resolution exists (`lookup.rs:45`, OpenFIGI then Yahoo). **Symbol resolution is new work.** |
| 433–435 | "`guard.rs` keeps `Ok(None)` apart from `Err(db)` (503 via `UNAVAILABLE_PATH`)" | `UNAVAILABLE_PATH` lives in `auth/fairing.rs:32` with the rewrite at `:110-113`; the **guard's own** `Err` arm answers **500** (`guard.rs:187-190`). Attribute the 503 to the fairing. |
| 523–526 | "`k8s/configmap.yaml` gains no `AGENT_LISTEN`; the ingress needs a route for the second port" | Rewrite per W2-4: the Service targets the **frontend** container (`targetPort: 80`), so there is no backend Service port to extend. Name the five files and record the nginx-`location` vs new-Service-port choice. |
| 547–556 | "five verification items … unknowns about external APIs" | Correctly labelled as unknowns, but `schemars = "1.0"` (`Cargo.toml:20`) already resolves twice (0.9.0 at `Cargo.lock:2867`, 1.2.3 at `:2879`) — rewrite as "confirm rmcp lands on the existing 1.x; a third copy blocks the PR" — and add a tower-0.5.3 compatibility item. Items 4 and 5 are not external-API unknowns (SY-30). |

### 8.3 Half-confirmed, and one reversal

| Spec line | Claim | Verdict |
| --- | --- | --- |
| 404–406 | "apply writes in one Postgres transaction" | **CONFIRMED as precedent** (`backup/mod.rs:457-459`, one SERIALIZABLE transaction), **PARTLY as completeness**: the uncited constraints the apply inherits are `db.rs:20` `max_connections(5)`, `:44` `acquire_timeout(5s)`, `:70` per-statement `statement_timeout = '30s'`. State that a 5,000-row apply shares a 5-connection pool and that the 30 s per-call deadline interacts with the existing per-statement timeout. |
| 401–403 | "this deployment's proxy already refused one oversized body (`af17064`)" | **CONFIRMED** — the audit's UNVERIFIABLE verdict was wrong; local git works offline. `af17064 2026-11-15 fix(backup): the proxy refused a bundle the API would have accepted (#68)`. Keep SY-24's rewrite (name the incident, don't drop a bare hash); drop the implication that the claim is unsupported. |

## 9. Rulings already made

These are decided. Do not re-open them in a plan; implement them or record a superseding reason.

### 9.1 v1 tool surface

Operator priority is read + natural-language data entry + full-portfolio import from a statement. Nothing in that asks for a general CRUD API.

**Stays.**
* All seven read tools, with two conditions: `list_transactions` gets a stated page size and hard ceiling (root F), and `get_performance` must not surface `unrealized_pnl` / `realized_pnl`, which are constants (DS-5, W2-8) — naming only three fake fields is not enough when success criterion #1 is literally "what is my realized gain in 2025?".
* `stage_import`, `apply_staged`, `discard_staged`, `get_staged`.
* **New:** the read-only *Pending batches* list in the Access screen (root A). It is a strict subset of the deferred UI panel, reuses the `preview` JSONB already being stored, and is the cheapest thing that breaks the proposer-renders-its-own-review loop.

**Goes — and what the spec must then say.**

| Cut | Reason the spec must state |
| --- | --- |
| `stage_edit` as a separate tool | Its create half is already one `stage_import` call with one asset and one transaction, so `stage_import` becomes the single staging entry point. The spec must say "single-row entry uses this tool" rather than leaving implementers to discover it. Removes CP-6, SY-21, the prose rows in the tool table, the per-element scope matching and half the payload surface. |
| All `update_*` | No requirement asks for it, and an update by a model that picked the wrong id is unrecoverable without a delete — which is also cut. State the reinstatement trigger: "an update tool needs the same delete-echo discipline, and needs a reason to exist first." |
| All `delete_*` | DS-11 is decisive and the design already owns the argument: the non-goal that keeps `mode=replace` in the web UI because a typed-word confirm is the only honest gate applies with *more* force where the confirmer is the proposer. Say: deletes stay in the web UI, an agent that needs one tells the human to do it, and the capability is **absent rather than policy-gated** — the same "cannot be minted" logic the backup non-goal uses, finally applied consistently. |
| The third scope | With create-only staging, `write` has no distinct meaning. Scopes become `read` and `import`, plus `apply` from Decision 1. The `kind`→scope prose mapping disappears and the tool table's Scope column becomes enumerable. |

**Kept but gated.** `target.existing` merge stays — re-importing an updated statement is a real need — but it cannot ship until duplicate matching is defined as `Decimal` equality over named fields (BLK-4). As written the rule silently fails on a re-exported `10` versus a stored `10.0`, which is the flagship guarantee failing in the common case.

**Consequence worth naming.** Cutting update and delete removes every caller-supplied id from the staging payload except the staged-batch handle. That shrinks the `access.rs` surface, shrinks the gate script's job, and makes the staging payload structurally identical to the backup bundle nesting — which is what makes §9.2(b) cheap.

### 9.2 Structural alternative (a) — rmcp stateless JSON mode inside Rocket: **investigate, timeboxed, before Workstream 2 is planned**

Neither adopt sight-unseen nor reject. The design's rejection rests on one argument — "hand-rolling SSE through a Rocket `Responder` fights the framework" — and stateless mode removes SSE from the problem entirely. A rejection whose sole stated reason has been removed is a defect in the document regardless of which way the decision lands.

**Evidence (ON-1, verified against primary sources).**
1. `2026-07-28` is stateless **by default** per SEP-2567: no `Mcp-Session-Id`, no standalone GET/DELETE stream, no `Last-Event-ID` resumption; `legacy_session_mode` only affects pre-`2026-07-28` clients.
2. `StreamableHttpServerConfig::json_response` (rust-sdk PR #683) makes the server prefer `application/json` for simple request-response tools; every tool in this inventory is one, so the SSE fallback never triggers.
3. In stateless mode the endpoint is **POST-only** (`tower.rs`: `allowed_methods` is `"POST"` when `legacy_session_mode` is false and stateless replay is supported). There is no long-lived connection to buffer at the proxy.
4. `NeverSessionManager` exists — "disables sessions entirely (stateless mode)".
5. `session_store: Option<Arc<…>>` allows external session storage for cross-instance recovery, so the `replicas: 1` precondition does not account for this field.
6. Third-party corroboration: a production MCP server's accepted ADR (ADR-0067, 2026-04-12) reports that stateful sessions behind a Traefik sticky cookie failed because "the real MCP clients we tested … don't persist cookies", and settled on stateless streamable HTTP with JSON responses. Portfolio Prism's ingress is Traefik.

**What it removes if it works:** the second listener, `hyper`/`hyper-util` usage and the version-bump question, the ingress/Service/containerPort/TLS work (DS-14, W2-4, SY-16), the fifth gate script (ZO-9, C-4), the invisible-to-probes problem, Rocket's body limits as a freebie against DS-7, and — if statelessness is real — the `replicas: 1` session coupling and the "no multi-replica session sharing" non-goal.

**What it does not remove:** PATs, staging state, pool/CPU/memory contention (same process either way), panic isolation, the liveness-probe coupling, and the `tower`/`http` version question (narrower than `hyper`'s, still open). **What it adds:** a hand-written request/response adapter, and a change to the most sensitive inventory in the repo.

**Acceptance conditions for the half-day spike.** Adopt the in-Rocket mount only if all hold: (1) rmcp 3.11 round-trips `initialize` → `tools/list` → one tool call with its own `StreamableHttpClientTransport` **and** one real third-party client, with no SSE and no `Mcp-Session-Id`; (2) a buffered adapter (method/URI/headers/body in, status/headers/bytes out) passes that round-trip in one bounded file — if it needs streaming anywhere, reject; (3) `AuthFairing`'s prefix rule (`auth/fairing.rs:40`, protects by `path.starts_with("/api/")`) is confirmed **inert** for the mount point — if the only way to make it work is to touch the fairing, reject, because a bearer token read on the same socket as the cookie surface is exactly what the two-listener design exists to make impossible; (4) `tests/route_coverage_test.rs::no_mounted_route_escapes_the_prefix_rule` (`PUBLIC_NAMESPACES`, `:181`) admits the new namespace by explicit amendment; (5) the chosen MCP client does not require server-initiated notifications.

**Refinement recorded by Stage 8.** `PUBLIC_NAMESPACES` is asserted over *mounted Rocket routes*, so it constrains alternative (a) only — a second listener on a separate port never appears in that inventory. The gate argument therefore favours (a) relative to the status quo, but the status quo does not fail the gate today. Amending one maintained inventory is cheaper and less drift-prone than writing and maintaining a fifth gate script.

**Status: investigate.** If all five pass, adopt and delete the second listener's whole cost stack. If any fails, record the specific failure in the new *Alternatives Considered* section and the second listener becomes a decided design rather than a default — at which point W2-3 and W2-4 become mandatory Workstream 2 deliverables.

### 9.3 Structural alternative (b) — reuse the backup preview/restore machinery: **partially adopt as a constraint on Workstream 2; reject the part that cannot work, on the record**

This is not the alternative Stage 7 framed. `POST /backup/preview` is **stateless** — it returns a `Plan` and restore re-sends the whole bundle. So the staged-batch row, opaque handle, TTL, single-use flip and user binding still have to be written no matter what. `staging.rs` is not avoidable.

**Adoptable, and to be written as a constraint.** The **validate → plan → apply** triple that `backup/mod.rs`, `backup/detail.rs` and `backup/restore.rs` already implement must be the single implementation the agent path calls. No second preview renderer, no second anomaly vocabulary, no second write path. This closes ZO-1, ZO-11 and most of root D at once, and it is the answer to the design's own "reuses rather than re-invents" promise, which is currently unfunded. `backup::detail` already carries display caps as constants (`:20`, `:22`), stable flag codes (`:27-33`) and `PlanPortfolio`/`PlanAsset` shapes that carry exact counts over every row (`:35-62`, tests `:575-585`).

**Rejected on the record: merge into existing rows.** `backup::restore::Mode` is exactly `Copy` and `Replace`, and the source states the reason — merge is "deliberately absent: 'which transaction is a duplicate?' has no clean answer, and a wrong guess silently corrupts a tax ledger" (`restore.rs:22-24`, orchestrator-verified). Restore also always mints new ids and never matches an existing row (`restore.rs:10-11`, verified). The agent's `target.existing` + exact-duplicate-skip is precisely the capability the backup module refused, and BLK-4 shows the refusal was well-founded because the duplicate rule as specified is not even well-defined. The spec must say: `target.new` reuses the existing insert path (it is nearly `Mode::Copy` on a sub-bundle); `target.existing` is new code, it is the case backup declined, and it gets reviewed as such.

**Spike condition.** Show that `backup::detail::detail_from_bundle` renders a plan from a one-portfolio sub-bundle rather than a whole-user snapshot, and that `MAX_BUNDLE_BYTES` / the row ceilings and the agent caps can be one regime. If `detail_from_bundle` turns out to be hard-coupled to whole-bundle shape, downgrade to "reuse the flag codes and the plan shape, not the code path" and say which.

**Status: partially adopt.**

## 10. The three open decisions

Each paragraph below is ready to paste into the spec as written by Stage 9.

### Decision 1 — what makes apply a control rather than a convention
* **Question.** What stops a single token, in a single turn, from staging and applying without a human in the loop?
* **Options.** (i) Status quo — scope "matching the batch's kind", preview relayed by the model. (ii) A distinct `apply` scope, not granted by default and not addable to an existing token. (iii) Apply restricted to `Credential::Session` (the `Credential` enum already distinguishes session from token). (iv) A read-only pending-batches affordance in the existing UI, no apply button.
* **Recommendation.** (ii) + (iv). (i) is not a control. (iii) is a one-line policy but forces a UI apply path the operator has ruled out for v1.
* **Spec text.**

> `apply_staged` requires `Scope::Apply`, which is a distinct scope from `read` and `import`, is not pre-checked in the mint form, and cannot be added to an existing token (re-mint to obtain it). The mint UI shows it in a separate group labelled as the one that writes. In addition, the Access screen carries a read-only *Pending batches* list — `GET /api/staged-batches` (session-authenticated, `PROTECTED_ROUTES` entry at `Owner`-none), rendering the stored `preview` JSONB verbatim with its handle, kind, row counts and expiry — so a human can read the preview out of band rather than only through the model that produced it; this replaces the v1 non-goal's blanket "no UI panel" with "no apply from the UI". Each flagged row may carry an optional `source_text` string, echoed by the agent from the statement it read, and the preview renders it next to the parsed value so `1.5` versus `15` is comparable by eye. The backend does not and cannot prove a human read anything; it guarantees only that a token which can apply is a token a human deliberately minted for that, and that the preview exists somewhere the model did not write.

* **Follow-on edit the spec must not miss.** Non-goals currently read "No UI panel for staged batches in v1"; if this decision is adopted, that sentence will be read as "skip the pending-batches list". Re-scope it to **"no apply from the UI"** or the control gets silently dropped.

### Decision 2 — the shared write and preview service
* **Question.** What does `staging::apply` call to write a row, and what renders the preview?
* **Options.** (i) Status quo — unspecified; tools "call the same `engines::`/`services::` functions", which is false for writes. (ii) Duplicate the inline validation from `api_routes/transactions.rs` into the agent path. (iii) Extract a shared `services::transactions::insert` / `services::assets::insert` and delegate the preview to `backup::detail`.
* **Recommendation.** (iii). (ii) creates two write paths that drift — the exact failure class WS0 exists to prevent, and one the codebase has already named and closed for `asset_type`.
* **Spec text.**

> `services::transactions::insert(pool, &mut tx, asset_id, &TxCreate)` and `services::assets::insert(pool, &mut tx, portfolio_id, &AssetCreate)` are extracted from `api_routes/transactions.rs:239-262` and `:31-96` in a no-behaviour-change refactor PR (Workstream 0d) and become the only code in the repo that inserts a transaction or an asset outside `backup::restore`. `staging::apply` calls them inside one `SERIALIZABLE` transaction together with the single-use flip, so the flip, every insert and the commit are atomic. The preview is rendered by `backup::detail` — `detail_from_bundle` over a one-portfolio sub-bundle if the spike shows it is not coupled to whole-bundle shape, otherwise the `PlanSide`/`PlanAsset` types and the `flag` codes (`detail.rs:27-33`) with a thin agent-side adapter — and no second anomaly vocabulary is introduced: the two genuinely new flags (`assumed_currency`, `duplicate_candidate`) are added to `detail::flag`. `target.new` reuses the `Mode::Copy` insert path shape; `target.existing` with duplicate-skip is new code, is precisely the merge capability `backup/restore.rs:22-24` refused, and is reviewed as such with duplicate matching defined as `Decimal` equality over `(date, asset_id, type, quantity, price, fee)`. If the extraction moves any INSERT out of `src/backup/`, `check_backup_portability.sh` Rule 2/Rule 3 scope is extended to the new directory in the same PR; if it does not, say so here so the gate's coverage is not silently reduced.

### Decision 3 — does the second listener exist at all
* **Question.** Is the agent endpoint a second `hyper` listener on its own port, or rmcp's stateless JSON service mounted inside Rocket behind a thin buffered adapter?
* **Options.** (i) Second listener, as specified. (ii) In-Rocket stateless mount. (iii) Decide during implementation.
* **Recommendation.** Run the spike first (it is four hours and can run in parallel with all of Workstreams 0 and 1, so nothing outside WS2 waits for it), then record the outcome. (iii) is what the document currently permits and it is how the design acquired a fifth gate script and a second preview engine.
* **Spec text.**

> Before Workstream 2 is planned, a half-day spike runs against `rmcp` 3.11 with `legacy_session_mode(false)` and `json_response(true)`: stand up one Rocket route that bridges a single POST (method, URI, headers, buffered body in; status, headers, bytes out) to `StreamableHttpService`, and drive it with rmcp's own `StreamableHttpClientTransport` plus one real third-party client through `initialize` → `tools/list` → one tool call. Adopt the in-Rocket mount only if all four hold: no SSE and no `Mcp-Session-Id` anywhere in the exchange; the adapter stays in one bounded file; `AuthFairing`'s prefix rule (`auth/fairing.rs:40`) needs no exemption for the mount point; and `tests/route_coverage_test.rs:181 PUBLIC_NAMESPACES` admits the namespace by explicit amendment. If any fails, record the specific failure in *Alternatives Considered* and the second listener becomes a decided design — at which point W2-3 and W2-4 (direct `hyper`/`hyper-util`/`tower`/`http` dependencies, bind-failure policy, containerPort, Service port or nginx location, probe invisibility) become mandatory Workstream 2 deliverables rather than open questions. Either outcome is recorded in the same PR that adds `agent/`.

## 11. Per-workstream verdicts

### WS0 — prerequisite hardening: **plan-ready after the listed edits**
Stage 8's "ready after B and C" was tested and is too generous (Stage 9 wins): two items are mislocated or unverifiable as written. Shortest list that makes it plan-ready — all but one are document edits:
1. Correct the file location and state that the read-side `type` stays `String` (BLK-9 / W0-1).
2. Rule on money-as-string scope and write the frontend migration as a named prerequisite PR (BLK-3 / W0-2). **This is the one operator decision.**
3. Pick the case-sensitivity resolution and name the four files it touches (§6.4 / W0-3).
4. Name the abort-test file and runtime-`Migrator` recipe, and the audit-script artifact (W0-5, W0-6).
5. Say whether the write-service extraction lands here (W0-7 / Decision 2).
Plus mechanical edits: delete the already-shipped bundle-schema item and its test bullet (W0-8), add the `.sqlx` step to Sequencing (BLK-8), state the plpgsql abort mechanism and boot consequence (W0-4), and slice as 0a / 0b / 0c-i / 0c-ii / 0d with the merge-order constraint (W0-9).

### WS1 — personal access tokens: **plan-ready after the listed edits**
The token model is sound and the Access screen is well-shaped; this is the most executable part of the document.
1. Fix the scope set (BLK-10 / W1-1) — Decision 1's scope half, and the only true blocker.
2. Add the unnamed files (`lib.rs` mount list, `openapi.rs`, `schemas.rs`, both committed generated artifacts) and the five inventory pins (W1-2, W1-3).
3. State `expected_tv: Option<i64>` for tokens and the `require_scope` status code (W1-5).
4. Name the OIDC-mode harness for the two-listener test (W1-6).
Plus: marker-placement rule citing `backup.rs:153` (W1-4), the revocation fallback when the UI is unreachable (SY-34), and the `Access` view placement trap outside the `currentPortfolio` branch (W1-9).

### WS2 — the agent module: **not plan-ready**
Blocking: Decision 1, Decision 2, Decision 3, and the apply/write atomicity sentence (BLK-11 / W2-2). Required before the relevant task starts, not before the workstream: ISIN location (W2-6), gate-script identity (C-4), `get_performance` field policy (W2-8), k8s exposure route (W2-4), isolation harness (W2-5), the tool-surface cuts (§9.1). Add one sentence to Sequencing: **Workstream 2's plan is not started until the apply-confirmation mechanism, the shared write/preview service and the listener decision are recorded in the Alternatives and Costs sections.** Cutting `stage_edit`, `update_*`, `delete_*` and the third scope makes WS2 roughly a third smaller and removes the two places where the design contradicts its own reasoning about destructive writes.

## 12. Remediation order

Ordered so each step unblocks the next. The two spikes go first because they cost calendar time only if discovered late, and both are four hours of work that can run in parallel with everything else.

1. **Kick off both spikes on day one, in parallel with all document work.**
   * *Spike A — stateless mode* (Decision 3, ~4 h): settles the listener question, and can moot W2-3, W2-4, DS-14, SY-16, the fifth gate script, the session section and the `replicas: 1` precondition in one stroke. Nothing outside WS2 depends on it, so running it early is free.
   * *Spike B — backup-preview reuse* (Decision 2's open half, ~4 h): decides whether the shared-service paragraph says "code path" or "types + flag codes", and whether the caps can be one regime. Also independent of WS0/WS1 work.
   Running both before any editing begins is the only ordering in which neither becomes a calendar-time blocker.
2. **Citation errata pass (§8).** 8 WRONG + 16 PARTLY + the half-confirmed row. No decisions required, and every later step quotes these numbers — do it first so later edits do not re-bake a false figure. Closes root C.
3. **Re-scope Workstream 0 (root B).** Delete the already-shipped item; split strictness into agent-DTO-only vs an explicitly-approved live-API change; name the frontend prerequisite PR; make `TxType` the source behind `KNOWN_TRANSACTION_TYPES`; resolve the case triangle and name the four files. Requires one operator approval (the live-API change) and nothing else.
4. **Record Decision 1 (root A).** The scope set falls out of it, which unblocks the tool table, the Non-goals re-scope, the mint form and all of WS1.
5. **Record Decision 2 and write root D.** Name the shared validate → preview → write service (using Spike B's outcome) and add the flip-and-write atomicity sentence. This is the step that makes half of WS2 buildable.
6. **Write root E — "costs of sharing a process".** Semaphore, listener body limit, caps re-derived from 256 Mi / 500 m, probe visibility, bind-failure policy, and the k8s path per W2-4 (using Spike A's outcome). Also where the second-listener decision is finally recorded.
7. **Write root F — one limits table with one owner.** Rate limit, pending-batch cap, page size and row ceiling, preview token budget, terminal-row retention, pricing-quota exposure. Derive the write caps from the container limits and say so.
8. **Root I — wire-contract pass.** Tagged unions as real JSON, computable anomaly rules or deletions, `Decimal` equality for duplicates, the transaction-currency question, the canonicalization guarantee, labeled fences.
9. **Root H — single-source-of-truth pass.** One home per rule with back-references (regex, TTL, scope model, caps, date bound, closed type set, anomaly flags), plus the reuse substitutions: `backup::detail` flag codes, `decimal_json`/`from_str_exact`, one gate script, `KNOWN_TRANSACTION_TYPES`, two names for two duplicate rules.
10. **Root G — structure and actors.** Add `## Alternatives Considered`, `## Risks and Mitigations`, `## Rollout & Rollback`, a TOC with anchors, and the Operations paragraph naming the operator for each passive obligation; then the aphorism→decision sentence pass (the copy reviewer's rewrites are ready).
11. **WS1 inventory edits** (W1-2 … W1-6, SY-34, W1-9). WS1 becomes plan-ready.
12. **WS0 plan-ready edits** (W0-1, W0-3 … W0-7, BLK-8, W0-9). WS0 becomes plan-ready and can ship while WS2 is still being decided.
13. **WS2 re-scope.** Apply the §9.1 cuts, rewrite the tool table and Non-goals, add the three "must add" items (name the MCP client, the premise accuracy run, the three-adversary threat model), and add the Sequencing gate that WS2's plan does not start until Decisions 1–3 are recorded.

## 13. What remains unverified

Each item is labelled with how to settle it. None of these can be closed by reading the repository.

| Unknown | How to settle |
| --- | --- |
| Whether `StreamableHttpService` can be driven **without** `hyper` — the documented paths are `TowerToHyperService` (needs a hyper server) and `axum::nest_service`; calling `tower::Service::call` from a Rocket route is plausible for one POST/JSON exchange but the body-type bridge (Rocket body → `http_body::Body`) is unbuilt and Rocket 0.5 is not tower-based | Spike A. This is the crux of Decision 3. |
| Whether `rmcp` requires `tower` ≥ 0.6 (lock has 0.5.3) | Add `rmcp` to a scratch branch and read the resolver output; record the answer in Verification, not in the Decisions section. |
| Whether `rmcp` needs a `hyper` newer than 1.10.1, and whether `TowerToHyperService` composes with it | Same scratch-branch resolution; only relevant if Spike A rejects the in-Rocket mount. |
| Whether `rmcp` resolves `schemars` onto the existing 1.2.3 or forces a third copy | Same scratch branch. Acceptable outcome is stated in W2-10: a third copy blocks the PR. |
| rmcp v3.11.0 facts as cited — Apache-2.0, MSRV 1.88, spec revision `2026-07-28`, "~43 direct dependencies" | Crate metadata is external. Show `cargo tree -p rmcp -d` output or drop the 43; keep the licence/MSRV line but cite the crate page. Note the binding floor is the `rust:1.92-slim` build image, not the local 1.98.1. |
| Whether `readOnlyHint` / `destructiveHint` behave as the tool table assumes | Drive them with a real MCP client (same run as the acceptance gate in §13 "MCP client"). |
| Whether `StreamableHttpClientTransport` end-to-end behaves as the e2e test assumes | Covered by Spike A step 1; re-run against the chosen client. |
| Whether live rows violate the new `CHECK` (out-of-set `type` values), and — if the `CHECK` stays case-insensitive — whether post-normalization `(asset_id, day)` SPLIT duplicates exist | The pre-flight audit script (W0-6), extended with the duplicate query per ZO-8. Run against production before the deploy that carries `0002`. |
| Whether `backup::detail::detail_from_bundle` can render a plan from a one-portfolio sub-bundle rather than a whole-user snapshot | Spike B. Determines whether Decision 2 says "code path" or "types + flag codes". |
| Whether `AuthFairing`'s prefix rule is genuinely inert for an in-Rocket mount point | Spike A step 3, on a branch — not from reading `fairing.rs:40`. |
| Whether `PUBLIC_NAMESPACES` can be amended to admit the new namespace without weakening `no_mounted_route_escapes_the_prefix_rule` | Spike A step 4; the amendment must be explicit and reviewed. |
| Which MCP client is in scope, and whether it truncates long tool output, relays a large preview verbatim or summarizes it, and whether it retries a timed-out `apply_staged` | Name it in the spec, then run one end-to-end session against it as an acceptance gate (Stage 8 gap 1). |
| Whether a real model turns a real broker statement into correct rows — the design's founding premise, currently untested | A ten-row manual accuracy run against two or three real anonymized statements, recorded in the spec with an error budget (Stage 8 gap 2). |
| Whether the operator exposes the agent port via nginx `location /mcp` (frontend image rebuild, inherits host policy) or a new containerPort + Service port + ingress path (own TLS and policy decision) | Operator decision; W2-4. Blocked on Spike A. |
| What a bind failure on the agent listener should do (refuse boot vs log and continue), given no probe can see it | Operator decision; record in the root E subsection. |
| Whether the 30-second per-call deadline is right, given it equals the pool's per-statement `statement_timeout` | Measure `get_performance` and a 5,000-row apply against the real pool; then set the value and make it configurable. |
| Whether the 25 / 200 / 5,000 caps are proxy-driven or memory-driven, and whether they must be re-checked if the proxy changes | Re-derive from the 256 Mi limit and the proxy body limit (`af17064`'s incident); state the derivation in the limits table. |
