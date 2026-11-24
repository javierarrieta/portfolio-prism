# Zoom-Out Review — PR #77 (MCP agent interface), target `/home/coder/wt-pp-staging` @ `df258e4`

**What I read (verified in tree):** `pr_body.md`, `diffstat.txt`, `agent/tools.rs` (full), `agent/staging.rs` (validate/stage/apply/error enums/preview), `agent/mount.rs`, `agent/protocol.rs`, `api_routes/tokens.rs`, `api_routes/staged_batches.rs`, `api_routes/analytics.rs` (tax-summary/performance/pnl routes), `api_routes/transactions.rs`, `api_routes/portfolios.rs`, `auth/pat.rs` (head + Scope), `auth/guard.rs`, `services/error.rs`, `db_types.rs`, `backup/mod.rs` validate hunks, migrations 0002–0006, `check_migrations.sh`, `AccessTokensPanel.tsx`, `StagedBatchesPanel.tsx`, `App.tsx` hunks, `api.d.ts` greps, spec tool table, test names in `staging_test.rs`/`mcp_mount_test.rs`.

**Not reached (no findings filed on these):** test-suite execution (664/361 counts unverified), `csvValidator.ts`/WS0 frontend diff, `backup/detail.rs` deletion hunks read only via grep context, `audit_*.sh` script bodies (line counts from diffstat only), `access.rs` full read (hunks only).

## Blockers

**Z1 — PR body claim "Every tool calls the same function the corresponding `/api` route calls" is false for 5 of 11 tools.**
`agent/tools.rs:6-16` (module doc) admits `list_portfolios`, `get_portfolio`, `list_transactions` inline their own `SELECT` (`tools.rs:167`, `tools.rs:390-440`), while the routes are different queries entirely (`api_routes/portfolios.rs:211`; `api_routes/transactions.rs:214`, which is N+1). Beyond the doc's confession: `get_performance` (`tools.rs:562-633`) and `get_transaction_pnl` (`tools.rs:637-730`) re-issue their routes' 3–4 loader queries verbatim (`analytics.rs:316-345`, `analytics.rs:379-437`); only the engine call is shared. The pair has **already diverged**: the tool strips `realized_pnl`/`unrealized_pnl` (`tools.rs:620-625`), the route does not.
**Delete/fix cost:** extracting 3–5 loader functions (adopt the tool's single-JOIN query for transactions; it is the better one) breaks nothing — routes and tools return identical bytes today except the intentional PNL strip. Keeping two implementations costs silent drift; the module comment is a confession, not a control.

## Majors

**Z2 — Migration `0005` steps 1–2 (~60 of its 105 lines) are unreachable in every production-bound sequence; fold step 3 into `0002`.**
`0002:81-83` already runs `SET type = upper(btrim(type))` on every row, and `0002:30-80` refuses case-variant SPLIT collisions with the *identical* query `0005:37-84` repeats. Both files are new in this PR, so every database runs 0002 first; after 0002, `0005:88-92`'s UPDATE is a no-op and its audit cannot fire (all SPLITs are `'SPLIT'`, which the old case-sensitive index already dedupes per asset/UTC-day). The hole 0005 says it closes never exists in any deployed state, because migrations run sequentially before Rocket serves a request.
**Delete cost:** deleting 0005 steps 1–2 breaks nothing reachable. Folding step 3's DROP/CREATE (`0005:100-105`) into 0002 makes it atomic inside sqlx's per-file transaction and deletes the non-atomic race caveat the PR body itself flags. Constraint: editing 0002 breaks checksums on any dev DB that already applied it mid-branch (`check_migrations.sh:126-129`) — those DBs reset; production never saw either file.

**Z3 — Commit `2692a69` (migration 0006 + `audit_asset_symbols.sh` + `backup/mod.rs` validate + `backup/detail.rs` detector deletion) is an independent data-integrity fix; deferring it to its own PR loses no guarantee the MCP surface depends on.**
`staging`'s `resolve_asset` works under the pre-existing software-only check exactly as `services::assets::insert` always did; nothing in token/staging/approve/apply needs the DB constraint. The unit is self-contained: `0006` (114 lines), audit script (131), `backup/mod.rs` +78, `backup/detail.rs` ±47, plus tests.
**Delete cost:** the only thing lost is the closed race window for concurrent asset writers — a bug that predates this PR and is not widened by the agent surface (staging goes through the same `insert` seam). The PR body's "all four prerequisites had to land first" is true for WS0/WS1/WS2 and false for this one.

## Nits

**Z4 — Two hand-written strict `YYYY-MM-DD` validators in the same module.** `tools.rs:104` `exact_date` and `staging.rs:191` `is_calendar_date` implement the same rule with the same rationale comment ("chrono accepts `2024-3-1`"). One should delegate to the other — this is the PR's own "one home for a rule" argument against the scope CHECK in `0003`. Delete cost: nothing; both are internal.

**Z5 — Three refusal-rendering helpers where one suffices.** `refused` (`tools.rs:80`), `refused_report` (`tools.rs:88`, exactly one caller: `get_tax_summary`), `refused_from` (`tools.rs:120`). Inline `refused_report`; delete cost: nothing observable.

**Z6 — Two style dialects in one PR.** `AccessTokensPanel.tsx` uses `glass-card` + inline styles + hand-rolled overlay modal (matching `SellPnlModal.tsx:132` precedent — no shared Modal exists, so not a reinvention); `StagedBatchesPanel.tsx` uses `card`/`data-table`/`muted`. Cosmetic; neither panel should be deleted (see refutations).

**Z7 — Audit scripts re-state migration predicates with nothing pinning script SQL to migration SQL.** `check_migrations.sh` pins the live DB against the migration files, not the `audit_*.sh` pre-flights against either. Drift un-ships the pre-flight guarantee silently. Nit only because the failure mode is the pre-script status quo (crashloop with the reason in a container log).

## Refutations and accepted risk

- **Error taxonomy (`ValidationError`/`StageError`/`ApplyError`) is necessary narrowing, not ceremony.** Each variant maps to a spec-fixed wire code; `e4bde8c` was a genuine deletion (removed the arm claiming `apply` could answer `too_many_pending`/`database`), and `staging_test.rs:698-703` walks variants and **can fail** — it asserts against staging-only codes with real fixtures. Deleting the taxonomy collapses "go ask a human" / "fix your document" / "never retry" into one stringly-typed code an agent cannot act on.
- **No tool is unneeded by a stated workflow.** All eleven appear in the spec's tool table (`2026-11-18-mcp-agent-interface-design.md:678-686`). `get_performance`/`get_transaction_pnl` are the weakest candidates (reporting already served to humans via `/api`; realized P&L comes from `get_tax_summary`) — first two to cut under scope pressure, but the spec settled them; deleting them breaks only agent-side reporting, nothing in the import loop.
- **Frontend reuse is real.** Both panels use `apiFetch`/`API_BASE` (`utils/apiFetch.ts:20-30`); hand-written DTO interfaces match repo convention (no component imports generated `api.d.ts` — verified by grep across `frontend/src`). Deleting `AccessTokensPanel` breaks token mint/revoke entirely; deleting `StagedBatchesPanel` breaks the human approval half of the gate — the design's central control becomes unusable. Neither is deletable.
- **`MAX_PORTFOLIOS_PER_USER = 10`, 90-day stale nudge, 16 KB preview budget, `pg_advisory_xact_lock`, sweep-in-stage-transaction**: all spec-stated or load-bearing for a stated cap; the "ten is arbitrary" note (`staging.rs:36-44`) is honest disclosure. The no-scheduled-job sweep (`staging.rs:565-590`) is the *simpler* alternative.
- **`mount.rs` bearer-only auth + header stripping** is the minimum for the token≠session invariant, each pinned by a mutation test (`mcp_mount_test.rs:111-131`).

## Unresolved

- **Whether any live/dev database already applied `0002` from a mid-branch build.** If yes, Z2's merge requires editing an applied migration (checksum break); honest options become "leave 0005 as-is" or "reset dev DBs". Settled by: operator confirms which databases ran any commit of this branch's migrations.
- **PR body verification claims** (664 backend / 361 frontend tests, six gates, `sqlx prepare --check`): not executed here (read-only, time-boxed). Nothing read contradicts them.