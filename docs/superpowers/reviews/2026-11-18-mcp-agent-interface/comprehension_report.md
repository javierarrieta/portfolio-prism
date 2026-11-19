# Stage 6 — Comprehension Test

* **Target:** `docs/superpowers/specs/2026-11-18-mcp-agent-interface-design.md`
* **Ranges:** FeatureRange 1–129, DetailedRange 130–556
* **Reviewer:** `egm-comprehension`, fresh context, no rubric applied
* **Date:** 2026-11-18

## What this proposes

Add a Model Context Protocol (MCP) server to Portfolio Prism so an external AI agent can
answer read questions about a user's portfolios and write data into them — including
importing a whole portfolio from rows the agent read off a broker statement. The server is
not a new process: it is a module inside the existing `backend_rust` binary that opens a
second HTTP listener (hyper + the `rmcp` Rust SDK, because Rocket is not tower-based) next
to the existing Rocket API. Writes are never direct: the agent calls `stage_import`/
`stage_edit`, the backend validates deterministically and persists the proposal plus a
rendered preview in Postgres under an opaque handle, and a human reads that preview in the
agent chat before `apply_staged` runs. Access is by a new scoped, individually revocable
personal access token (`read`/`write`/`import`, no `backup`) minted self-service in the web
UI; the token is structurally unable to reach the interactive `/api` surface because that
surface only reads session cookies. The whole thing is off unless `AGENT_LISTEN` is set.
Workstream 0 is a prerequisite hardening pass on the live API (closed `TxType` enum + `CHECK`
migration, money-as-string at the deserializer) that MCP depends on.

## How the existing system works, per this document

Portfolio Prism is a self-hosted, multi-user web app: a TypeScript/React frontend
(`frontend/`, with `App.tsx` nav rail, `csvValidator.ts`, `isinValidator.ts`,
`utils/decimal.ts`) against a Rust Rocket backend (`backend_rust`) on Postgres, with
migrations run by `sqlx::migrate!` at startup and an OpenAPI document generated via `utoipa`
and consumed by `npm run generate-types`. Authentication is session-cookie based for `/api`,
plus an OIDC flow; `auth/guard.rs::resolve_caller` turns a request into an `AuthedUser` and
keeps deny (`Ok(None)`) distinct from outage (`Err` → 503), while `access.rs` is — and is
enforced by `check_access_gate.sh` to be — the only module allowed to resolve a
caller-supplied id, answering not-found rather than forbidden. Money and quantity are stored
as `TEXT` and written from `Decimal::to_string()` to preserve exactness (a migration test and
a `check_migrations.sh` case forbid `NUMERIC`), and transaction `type` is unconstrained
`TEXT` uppercased at write time, with only `BUY`/`SELL`/`SPLIT` actually understood by the
tax engine. Read-side capabilities already exist as engines/services (tax summary, dated
report, performance/TWR, sell P&L, asset lookup), some of which are partly fake
(volatility/Sharpe/beta are constants). There is a backup export/preview/restore surface
with a destructive `mode=replace`, a browser CSV importer that enforces the closed type set
and auto-detects date formats and hardcodes USD, a deployment with `replicas: 1`, a
`build-and-test` CI job with five DB-touching gates, and a proxy that has previously refused
an oversized request body.

## Findings

| ID | Severity | Location | Finding | What a fresh reader cannot resolve |
|---|---|---|---|---|
| CP-1 | 🔴 Blocker | Overview l.19–20; Success criteria l.36–44; `apply_staged` l.377; staging rules l.110–115, 316–322 | The central safety claim — "every write goes through a staged batch **that a human sees before it is applied**" — is not enforced by anything the document describes. Staging and applying use the same token, the same session and the same user; the only stated cross-user rule is the opposite one ("cannot be applied by the user who did not stage it" = must be the *same* user). Nothing requires a delay, a second credential, a UI action, or a human-originated signal between `stage_import` and `apply_staged`. | Whether a compliant agent that stages and immediately applies has violated the design or satisfied it. What "a human sees" mechanically means, given there is explicitly no staged-batch UI panel (l.84) and the preview is relayed by the model itself. |
| CP-2 | 🔴 Blocker | l.223–226 vs Tool inventory l.370–379 | "a `read`-only token can preview an import and do nothing with it" contradicts the tool table, where `stage_import` requires the `import` scope and `get_staged`/`apply_staged` require "matches batch kind". A `read`-only token cannot create a batch, so it has nothing to preview. | Which sentence is authoritative. Also unresolved: the mapping from batch `kind` (`edit`/`import`) to scope (`write`/`import`) is never stated — the reader infers it. |
| CP-3 | 🟡 Major | l.19–20, 84, 377, 419–420 | The preview and the "echo destructive elements back by name" confirmation both travel *through* the model: the server renders a preview, but the model is the only channel that shows it to the human, and the model is also the one that must echo names back. The document never states this trust boundary or says why it is acceptable, even though its own justification for staging (l.26–31) is that a model producing plausible-but-wrong output is the actual risk. | Whether the design considers model-mediated preview a real gate or a convenience, and what stops a model from summarizing away the anomaly list. This is the same gap as CP-1 but on the information path rather than the write path. |
| CP-4 | 🟡 Major | `stage_import` payload l.390–396 vs Hard caps l.401–403 | The hard cap of "25 portfolios per stage" is unreachable given the documented payload, which accepts exactly one `target` (`new` *or* `existing`). Either the payload is incomplete (multiple targets allowed) or the cap belongs to `stage_edit`. | Whether one `stage_import` call can create many portfolios, and therefore which tool the 25-portfolio cap protects. |
| CP-5 | 🟡 Major | `stage_import` payload l.390–396 vs anomaly list l.360–362 | The anomaly list flags "currency differing from the portfolio base currency" and "an assumed currency", but the documented transaction row is `{ type, date, quantity, price, fee }` — no currency field — and asset `currency` is optional with a stated default. | Whose currency is being compared to what, and whether a transaction can carry a currency at all. Without this, the "German broker statement in EUR" scenario the document motivates cannot be evaluated. |
| CP-6 | 🟡 Major | Tool table l.376 vs `stage_edit` payload l.411–417 | The table says `stage_edit` does "create / update / delete of portfolios, assets, transactions", but the payload only offers `update_portfolios`. There is no `update_assets` and no `update_transactions`. | Whether asset and transaction updates are out of scope or just omitted from the sketch. `create_assets: [...]` and `create_portfolios: [...]` are also elided, so their required fields (including an asset's portfolio association) are unspecified. |
| CP-7 | 🟡 Major | l.16 vs l.279–281 | "There is no Node.js anywhere in the design" is stated absolutely, but the same document mandates `npm run generate-types` in `frontend/` for the Access screen contract, and the frontend it describes is TypeScript. | Whether the claim means "no Node at runtime/deploy time" or "no new Node tooling". A fresh reader sees a contradiction between a stated hard constraint and a mandated step. |
| CP-8 | 🟡 Major | l.159 | "the audit query … is a pre-flight step in **the runbook**" — no runbook is named, linked, or created by any workstream. | Where the pre-flight audit actually lives, and who runs it before the Workstream 0 deploy (Sequencing step 1 says "confirm the live database passes the audit" without saying where the procedure is recorded). |
| CP-9 | 🟡 Major | l.401–403 | "Not hypothetical: this deployment's proxy already refused one oversized body (`af17064`)" — a bare commit hash with no link, no explanation of what the proxy limit was, and no statement of whether the caps derive from that limit. | Whether 25/200/5,000 are proxy-driven (and thus must be re-checked if the proxy changes) or arbitrary safety numbers, and why they are hardcoded rather than configured alongside `AGENT_BATCH_TTL_MINUTES`. |
| CP-10 | 🟡 Major | l.466 | "the batch fingerprint" is listed as a logged field, next to the rule "never the payload". The fingerprint is never defined (hash of what, over what canonicalization, is it joinable to a staged-batch row?). | What is actually in the log line, and whether a fingerprint of financial data is itself sensitive under the document's own privacy rule. |
| CP-11 | 🟡 Major | l.475–476 | "need `DATABASE_URL` like the other five gates" — the five existing gates are never enumerated, and the new `check_agent_gate.sh` is added to "the CI gate list in `AGENTS.md` order" without saying what that list is. | Which five gates exist, where the list lives, and what "AGENTS.md order" means for placement of the new one. |
| CP-12 | 🟡 Major | l.346–352 vs l.360 vs l.450 | Three overlapping notions of "bad asset": `unknown_asset` as an error code, "unknown symbol" as a *non-blocking* anomaly, and "missing assets auto-create". | Whether an unknown symbol is created, flagged, or rejected — and which of the three applies in each tool. The document says a bad ISIN is "a named rejection, not a silently unmatched asset", which implies unknown symbols are not rejected, but the error code says otherwise. |
| CP-13 | 🟡 Major | l.110–112, 311 | "canonicalized payload" / "canonicalized proposal" is load-bearing (the preview and the apply both read it, and the decimal-scale test asserts byte-for-byte round-trip), but canonicalization is never defined — key order, defaults filled in at stage time vs apply time, and whether the 60-minute re-validation can disagree with what was previewed. | What "canonicalized" guarantees, and whether the human sees exactly what will be written (the document says apply re-validates, which implies it may differ). |
| CP-14 | 🟢 Nit | l.11–20, 271–277 | "Portfolio Prism" is never described in one sentence, and "the agent" / "the model" / "the client" are used as if defined: nothing says who hosts the MCP client, whether one token serves one user's agent, or that the agent is external software outside the deployment. | The actor model. A fresh reader assembles it from context rather than from a definition. |
| CP-15 | 🟢 Nit | l.101–103, 548 | `SSE` is never expanded; "streamable HTTP" is never explained (HTTP POST + SSE streams) beyond a mention that hand-writing it means owning "protocol negotiation, session ids and SSE". | What the transport is, for a reader who has not met MCP before. No link to the MCP spec revision `2026-07-28` is given either. |
| CP-16 | 🟢 Nit | l.96–100 | "PR #228" and "PR #642" are cited without URLs. The repository name appears earlier, so the reader can guess, but the two PRs are the load-bearing evidence for the "cannot mount into Rocket" decision. | Where to read the rationale. |
| CP-17 | 🟢 Nit | l.192, 231, 253, 257 | Existing identifiers are used without a gloss: `AuthFairing` (what it covers, what it rejects), `/auth/me`, `logout-all`, and the `Access::*` level set (only `Owner` appears; "no resource level" is used as a level name). | The full set of access levels and what `AuthFairing` does to an unauthenticated request. |
| CP-18 | 🟢 Nit | l.358 | "Per security:" introduces a second word for "asset" without saying they are the same thing. | Whether "security" is a distinct entity from `assets`. |
| CP-19 | 🟢 Nit | l.372 | `TWR` is not expanded (nor is ISIN). Sharpe/beta are fine as domain vocabulary; TWR is the one the tool's *output contract* depends on, especially next to "these three are constants". | Nothing critical, but the one place the document tells the model "this is not a measurement" would benefit from naming the measure. |
| CP-20 | 🟢 Nit | l.487–489 vs l.401–403 | Success criteria promise duplicates are "flagged rather than silently dropped"; Workstream 2 says duplicates are "flagged in the preview and skipped on apply". Skipping after a flag is defensible, but the two sentences read as opposite behaviours to a skimming reader. | Whether apply drops duplicate rows silently when the preview was not read (which, per CP-1, is possible). |
| CP-21 | 🟢 Nit | l.491 | The gate-script regex is quoted as `^#[(get\|post\|...)]` — malformed (unbalanced bracket, ellipsis inside the group). | Exactly what the existing script matches, which matters because the new gate is defined by contrast with it. |
| CP-22 | 🟢 Nit | l.66 vs Workstream 0 l.130–166 | Requirement 5 says "every current `/api` route keep working unchanged", while Workstream 0 makes `/api` transaction creation reject JSON-number money and unknown types — a live behaviour change. The Overview's exception clause ("except where Workstream 0 says so explicitly") covers it, but the Requirements list carries no such caveat. | Whether Requirement 5 is a constraint Workstream 0 violates or a constraint it satisfies because the frontend already sends strings. |
| CP-23 | 🟢 Nit | l.551–553 vs l.461–464 | The design asserts "every tool call is wrapped in `catch_unwind` with a 30-second deadline" as a settled decision, while the Verification list asks whether `catch_unwind` is even sufficient under tokio. | Whether fault isolation is designed or still open; the two sections state different confidence levels about the same mechanism. |
| CP-24 | 🟢 Nit | l.316–322 | Sweeping stale batches "when that user stages the next batch" means rows (containing financial payloads) persist indefinitely for a user who stops staging, despite `expires_at NOT NULL`. | Whether unbounded retention of staged payloads is intended, given the document's own "never log someone's financial history" posture. |

## Terms used without definition

| Term | First used | Defined in the document? |
|---|---|---|
| Portfolio Prism | l.11 | ❌ Never described; inferable only from context |
| the agent / the model / the MCP client | l.12, l.26 | ❌ Used interchangeably, never identified as a component or host |
| MCP streamable HTTP | l.15 | ⚠️ Named, never explained; no link to the `2026-07-28` spec revision |
| `rmcp` | l.29 | ✅ Defined inline (official Rust SDK, repo, version, licence, MSRV) |
| Rocket | l.94 | ⚠️ Named with version and "not tower-based"; external, not linked |
| tower / hyper / hyper-util | l.96–103 | ⚠️ Named with versions; the tower-service concept is assumed knowledge |
| SSE | l.101 | ❌ Never expanded |
| `access.rs` / `check_access_gate.sh` | l.34 / l.35 | ✅ Function stated ("only module permitted to resolve a caller-supplied id", "enforces it"); script's mechanics only much later at l.491 |
| `engines::` / `services::` | l.90 | ⚠️ Named as module namespaces; contents never enumerated |
| `AuthFairing` | l.192 | ❌ Never described |
| `/auth/me` | l.192 | ❌ Never described |
| `token_version` / `is_active` | l.195 | ✅ Behaviour described in context |
| `logout-all` | l.231 | ⚠️ Effect given, identity (endpoint? UI action?) not |
| `Access::Owner`, "resource level" | l.257, l.262 | ❌ The access-level set is never enumerated |
| `mode=replace` | l.76 | ✅ Effect given inline ("deletes everything a user owns"); full semantics live in a linked spec |
| `backup/*` | l.74 | ⚠️ Named as an endpoint group; never listed |
| `api_token`, `agent_staged_batch` | l.207, l.303 | ✅ Full schema given |
| canonicalized payload / proposal | l.112, l.311 | ❌ Load-bearing, never defined |
| batch fingerprint | l.466 | ❌ Never defined |
| the runbook | l.159 | ❌ Never named or linked |
| `af17064` | l.403 | ❌ Bare commit hash, no link or explanation |
| "the other five gates" / "the CI gate list" / `AGENTS.md` order | l.476, l.495 | ❌ Not enumerated |
| `build-and-test` job | l.475 | ⚠️ Named CI job; not described |
| TWR | l.372 | ❌ Not expanded |
| ISIN | l.352 | ❌ Not expanded (domain-standard, but unexpanded) |
| security (vs asset) | l.358 | ❌ Introduced as a synonym without saying so |
| `unknown_asset` vs "unknown symbol" vs auto-create | l.450 / l.360 / l.398 | ⚠️ All three present, their precedence unresolved (CP-12) |
| `TxType`, `Scope`, `CredentialKind` | l.141, l.199, l.191 | ✅ Defined where introduced |
| `PROTECTED_ROUTES`, `test_db.rs`, `check_migrations.sh`, `LocalSessionManager` | l.260, l.474, l.159, l.423 | ✅ Described where used |
| `utoipa`, `schemars`, `deny_unknown_fields`, `catch_unwind`, `#[tool]` | l.279, l.299, l.345, l.461, l.305 | ✅ External toolchain names, self-evident from context |
| `docs/backup-schema/v1.json`, `docs/auth-testing.md`, `stats_engine.rs:108-110`, `lib.rs:268` | l.153, l.509, l.372, l.293 | ✅ Concrete paths; adequate as pointers |

## Sections that could not be paraphrased

Every section was paraphrasable, including the two architecture-rationale paragraphs and the
tool inventory. Two places required me to *choose* a reading rather than restate one, and
both are already logged as blockers rather than paraphrase failures:

* **Overview / Success criteria, the "a human sees before it is applied" claim (CP-1)** —
  paraphrasable only if I silently assume the human step is enforced. It is not, anywhere in
  the document.
* **Workstream 1, "a `read`-only token can preview an import" (CP-2)** — paraphrasable only
  by contradicting the tool table two sections later.

One further paraphrase caveat, not a blocker: the `stage_edit` payload sketch (l.411–417) had
to be restated with its `[...]` elisions intact, because the field contents are genuinely
unspecified — I could describe what it does, not what it accepts.
