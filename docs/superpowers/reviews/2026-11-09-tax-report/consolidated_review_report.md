# Consolidated Review Report — Tax Report and Portfolio Statement

*   **Design under review:** `docs/superpowers/specs/2026-11-09-tax-report-and-statement-design.md` (689 lines)
*   **Predecessor:** `docs/superpowers/specs/2026-11-09-tax-report-and-statement-concepts.md`
*   **Stages:** 5 initial zoom-out · 6 comprehension · 7 critic (design + structural + style) · 8 intermediate zoom-out · 9 implementation readiness (codebase details + readiness + zoom-out)
*   **Stage reports:** `zoom_out_report.md`, `comprehension_report.md`, `critic_report.md`, `zoom_out_2_report.md`, `implementation_readiness_report.md`
*   **Date:** 2026-11-09
*   **Overall verdict:** the architecture is sound and survived every lens. The document's **scope** and its **implementability** did not. Five independent reviewers converged on the same shape of answer: split the document, delete three subsystems, and specify the remaining half at the level of values, names and columns rather than intent.

## 1. What is sound — do not change it

Confirmed independently by two or more reviewers against source:

*   One dated replay in the engine, one public entry, two selections over it — not two matching loops.
*   Extending `GET /api/portfolios/<id>/tax-summary` with optional query parameters instead of a new route. Verified holding: `route_coverage_test.rs:140/:262/:275-278` hard-assert 15/15/13 and `to_document_path` (`:86-92`) truncates at `?`, so added parameters leave the counts intact.
*   Server owns every total; money as decimal strings end to end; renderers do not sum.
*   The realized/unrealized seam as the staging cut line.
*   No server-side PDF. The Alternatives section is unusually strong and is explicitly *not* the trimming target.
*   Hermetic seeding as a hard requirement, and `decimal_json` reuse.
*   The pricing rule's shape: last close on or before the requested date, actual date printed, silent substitution banned.

## 2. Owner decisions required (six)

These are not document fixes. Each changes what gets built, and each has a reviewer recommendation with evidence.

### D1 — Split the document in two
**Recommend: yes.** Strand A (dated reporting: `replay_at`, dated route, typed payload, CSV, `ReportControls`) is the feature. Strand B (dated FX, symbology, per-pair series cache, `historical_prices` uniqueness, honest result types) is a live bug worth shipping if the reports never exist. Strand C (print/statement) needs no backend for its first half. Strand D (P3) has no destination.

Splitting fixes at once: the "outside this feature's scope but blocks it" contradiction, Goal 7's exclusivity, the false slice-0 cells, and the P2-before-P1 ordering — which the review discovered: **P1's range fetch writes into `historical_prices`, so without the unique constraint it appends ~250 rows per symbol per fetch.** Shipping P1 first makes the duplication worse.

Corrections to the split as first proposed: Doc A's honest size is 7–8 files, not 5, because P4 breaks `stats_engine.rs:180` and `analytics.rs:73`; Doc B is *blocked on* Doc A, not a subset of it; and `analytics.rs` is edited by both and must be ordered.

### D2 — Delete the cost guard
**Recommend: delete, replace with a fail-closed elapsed deadline.** The case, in the reviewers' words and verified:
*   The in-flight slot bounds concurrency, not cost — the exact argument used to retire the `Editor` rule, in the same document. The Access paragraph refutes the guard.
*   Once the FX path is genuinely fixed, the work set is structurally ~22 calls for the flagship portfolio; a budget duplicates a bound the shrink already enforces.
*   It is per-process state resting on `replicas: 1` being a manifest value, not an invariant; a rolling update runs two pods for a window.
*   It cannot reach the code that would decrement it: `CurrencyService::new()` takes no arguments and has 24 call sites.
*   The real hazard is elapsed time. Verified: `k8s/ingress.yaml` carries **no annotations at all** (nginx default 60 s); reqwest times out at 10 s with `MAX_RETRIES = 3` and 1 s/2 s backoff ≈ 33 s worst case per unreachable host. Two bad symbols ≈ 66 s: the browser gets an HTML gateway error, never the typed 503, and the pod still holds the slot so the next click 429s.

**Critical correction from stage 9:** the replacement must **fail closed** — deadline exceeded → 503 + `retryable: true`, no partial payload. "Return what it collected" is a partial tax report, the thing Goal 5 forbids. Set it near 20 s, since it cannot preempt an in-flight call, and note it is only checkable between assets, which bounds anything only once the FX cache is genuinely per-pair.

Deleting the guard does **not** delete the RFC amendment: the amended non-goal text lives in the concepts doc at `:230-237`, a 26th file no list accounts for.

### D3 — Re-open server-generated CSV (`?format=csv` on the same route)
**Recommend: yes, before slice 3.** Every argument the design makes for server-owned numbers — strings end to end, renderers forbidden from summing, the `normalize*` bypass, "a total derived in TypeScript is a double" — is an argument for emitting the file where the decimals live. Verified feasible: auth is an HttpOnly session cookie that the guard reads and never `Authorization` (`auth/guard.rs:103`, `apiFetch.ts`), `/api` is same-origin in dev and prod (`nginx.conf:12`), so a plain `<a href>` carries the session and `curl --cookie` reproduces the file byte-for-byte — Goal 4 becomes testable for free. No route is added, so the count test stays intact.

It deletes: the browser CSV builder, its float-summed test, the `decimal.ts` question, the BOM-ownership contradiction, the copied formula-lead guard, and the `#`-header-versus-columns contradiction with `sellPnlCsv.ts:13-16`.
**UNVERIFIED:** utoipa 4.2 declaring two content types for one status, and Rocket 0.5 `Either: Responder`. In-repo pattern to cite: `auth/routes.rs:33-52`.

### D4 — Defer the statement and print slice; cut the asset subset
**Recommend: defer, and cut the subset.** The subset cannot coexist with server-owned totals: summing selected `AssetTotals` is the forbidden arithmetic, and printing the rollup above a filtered row set puts a headline describing a different asset set than the rows beneath it — on the artifact meant for an adviser. Cutting it also removes Alternative G, the IDOR row, and the only security question in a document with no risk section.

The print half has **no automated verification available today**: `ci.yml` has no browser job, `e2e:sell-pnl` is a local script needing a chromium binary, and this workspace has no chromium and no `node_modules`. Deferring strands nothing in the backend — files 19–23 have no inbound references.

Note the interaction: if print is deferred **and** CSV goes server-side, file 13 (`api.d.ts`) has no consumer. Decide explicitly whether the JSON payload stays a documented public surface or becomes an internal intermediate.

### D5 — Move P3 out of scope as a separate investigation
**Recommend: out.** No destination exists — `main.rs:63` is a plain `.launch()` with no Rocket tasks and `k8s/` has no CronJob or Job. Both readings break a stated invariant. Two facts killed its urgency: `sync_splits_for_asset` is throttled to once per symbol per 30 days and returns `Ok(0)` on a cache hit (`:541-560`), so the read-path write is rare; and **the standing record already decided this** — "considered and rejected: it turns a GET into a writer", pinned by a route test. P3 silently reverses that with no new evidence, the same pattern as the validation-matrix reversal.

Rule to adopt instead, following the `/pnl` precedent: the dated path does not sync splits; the run context states splits are as recorded; the retroactive-split hazard is a named known limitation pointing at the separate investigation. This also dissolves the SQLITE_BUSY-has-no-error-class problem, because the dated path then performs only the price write the predecessor explicitly permitted.

### D6 — Scope strict strategy validation to the dated path only; keep `Viewer`, delete its rationale
**Recommend: yes to both.** The strict-validation precedent was false — `analytics.rs:217-221` ends in `_ => "FIFO"`, the same silent fallback the design condemns — and the standing record refuses a validation matrix as over-reach with "do not re-raise without new evidence". On the undated path strict rejection also contradicts "absent parameters mean today's behaviour" and Goal 7, and has no target file.

The Access rationale is fictional: `access.rs:37-57` returns `Owner` or `None`; `Viewer`/`Editor` are unattainable and there is no membership table. The accountant/adviser argument describes a sharing feature that does not exist, and file 9's Viewer-collaborator test cannot be written. The decision is a no-op and costs nothing; the fake precedent is the cost.

## 3. Blocking specification gaps (must be filled before implementation)

1.  **Single-parameter dates are undefined, and `to`-alone is now load-bearing.** "Cumulative from inception to a chosen date" is `to` present, `from` absent. With the statement deferred this becomes the CSV's cumulative column, so it blocks the thing that *is* shipping. State: `to` alone means from inception; `from` alone means open-ended; both-or-neither if that is the rule; future `from` handled like future `to`.
2.  **The current-year default collides with the future-date 422.** Default to current-year-**to-date**, not Dec 31, or the default request is rejected in February. Goal 3 also counts four cases against five presets, and "no calendar component" sits next to two `<input type="date">` fields.
3.  **No CSV column names, order, or header text** — and the CSV *is* the deliverable. File 15 tests a `lot_pnl` column the FeatureRange never defines.
4.  **`totals: portfolio rollup` is the one field group never enumerated**; `fx_rate_date_used` nullability unstated; `decimal_json` is the wrong helper for nullable money (`decimal_opt_json`, `db_types.rs:73`).
5.  **Five names for one date** (`to`/`end`/`as_of`/`as-of date`/`valuation_date`) and three for the pricing date. Adopt the canonical-name table from `critic_report.md`. `latent_gain_loss` vs `unrealized_at_end` must be reconciled or mapped.
6.  **Every number the plan depends on is named and never valued:** bounded lookback, outbound budget, missing-item cap, FX cache bound, elapsed deadline, page count, and the definition of "per user" (suggest the `sub` claim). The lookback length *is* the pricing rule.
7.  **`replay_at`'s signature is elided** and as printed cannot produce open-lot values — `market_value` and gain/loss come from `current_price_base` in `calculate_lots` (`tax_engine.rs:549-551`, `:580-590`).
8.  **`Transaction.date` is `DateTime<Utc>`, not a date** (`models.rs:35`). "No conversion is involved" and "a sell at any time on `to` is inside the window" cannot both hold.
9.  **Goal 5 says "every", the contract caps the list.** Goal 7 says "only the FX fix" and P2/P3/P4 falsify it. Goal 1's equality is stated for any window when `tax-summary` has no window. Goal 4 is falsified by the app's own read-path writes and the dropped `price_source` flag.
10. **P2's migration shape**: dedupe → `CREATE UNIQUE INDEX IF NOT EXISTS`, idempotent in `init_db`, with a stated tie-break (`max(rowid)` is the only deterministic choice and it *is* a figure decision), ordered against `migrate_decimal_columns`, plus a migration test in the `decimal_migration_test.rs` shape. Rename the "migrations" heading.
11. **The hermetic FX problem**: `get_rate` has no DB read path and there is no FX table, so no dated multi-currency test can be hermetic, and **nothing in the suite can distinguish a correct dated-FX implementation from the current broken one** — every fixture is USD/USD. Land FX on `historical_prices` under `EURUSD=X` or the figure-moving change ships with zero coverage.
12. **P4 inverts the mechanism the only hermetic pricing test depends on**: `analytics_test.rs:169-171` seeds prices and works *because* the network-first path falls back to the DB; P4 turns that fallback into an error. State the DB-first rule and the fallback explicitly, and note that seeding every date leaves no gaps for the non-trading-day test.
13. **No provider seam**: `CurrencyService::new()` takes no config and the Yahoo URL is hardcoded four times, so "unreachable provider returns 503" cannot be tested.
14. **422 carries two meanings** (validation vs no-data) with no body rule for which fields are populated.

## 4. Mechanical corrections (no judgement needed)

*   **P4 is referenced in four places and defined in none of the in-scope ones**; slice 0 under-scopes by one reviewed change.
*   **Slice 0 does not compile**: file 2 registers `report_guard` from file 3, a slice-4 file.
*   **Slice 2 fails CI at its own finish line**: the `TaxLot` fix lives in `schemas.rs` (slice 4), `TaxLot` is in the committed OpenAPI contract, and CI diffs it byte-for-byte while regeneration is slice 4. It also ships an undeclared change to a live endpoint's response shape.
*   **Slice 3 cannot compile its payload** — `AssetTotals`/`ReportRow` are file 5, slice 4.
*   **`P4` belongs in slice 4**, not slice 0: its only consumer is the error contract, and the undated path should be stated to keep today's permissive behaviour.
*   **`get_historical_prices` already returns `Result`**; the plan describes the change backwards. Five swallow sites at `:429/:442/:449/:467/:502`.
*   **`get_price` hardcodes `Utc::now()`** (`:268`) — a past-close lookup cannot be built on it; `get_historical_prices` already does the range fetch and already writes the DB.
*   **`api.d.ts` has zero importers**; `frontend/src/types.ts` is the file that changes.
*   **File 7 describes the wrong edit** — utoipa derives query parameters from the Rocket attribute; what's needed is registering the new schemas in `openapi.rs:26-40`.
*   **File 10's sweep has no hook** — one route, so a dated pass is a second parameterised pass, not a special case. The comment at `auth_test.rs:43-45` is already false.
*   **File 21's MSW change is a branch on the existing handler**, not a new route.
*   **`check_print_styles.sh` should be deleted** — a green gate that can pass while printing is broken.
*   **Add the 14 omitted files** listed in `implementation_readiness_report.md`, including `stats_engine.rs`, `analytics_test.rs`, `PortfolioDetail.tsx` and its test, `main.rs`, `tests/common/mod.rs`, `cors_test.rs`, `decimal_migration_test.rs`, `types.ts`, `package.json`, `main.tsx`, the review record, a shared CSV-cell module, and the concepts doc.
*   **Terminology and duplication:** adopt the canonical-name table; state each rationale once with an owner section; move provenance narratives to `docs/superpowers/reviews/2026-11-09-tax-report.md` per the existing convention. The duplication is not cosmetic — it is the mechanism behind the stale diagram, the Access paragraph arguing against a retired rule, and the Reconciliation paragraph defending a guard that guards nothing.
*   **Add `## Risks & Non-Functional Requirements`** with four rows: existing-figure movement, outbound cost bound, access posture, print fidelity.
*   **Fix the Sequencing table**: `Network?` means "does verification reach the network" (say so); `Risk` mixes magnitudes with scopes and grades the highest-blast-radius item `low`.
*   **Restore `## Open Questions`** with `*None open.*` and five pointers — the predecessor lists five, the design says four.
*   **Move Alternative D to `### Future Improvements`** — it is a deferral filed as a rejection.
*   **Cite the standing record** wherever this design reverses it (validation matrix, split sync), or scope the reversal away.

## 5. Recommended shape after remediation

**Doc A — pricing-layer truthfulness** (7–8 files, ships first, own release note naming which figures move):
P2 (dedupe with stated tie-break → unique index → migration test) → P1 (dated FX, `EURUSD=X`, per-pair series cache with stated window, cache bound and key versioning, FX persisted to `historical_prices`) → first hermetic FX test → P4 with the undated path stated permissive, plus `stats_engine.rs` and `analytics.rs` compile fixes → `SellPnlModal.tsx` footnote.

**Doc B — dated reporting** (blocked on Doc A):
`replay_at` with explicit `valuation_date` and `date <= to` before matching (network-free) → dated `tax-summary` with both-or-neither dates, collected pricing failures, fail-closed elapsed deadline, `?format=csv` → `ReportControls` with a current-year-to-date default.

**Doc C — statement and print** (deferred, own short design where selectors, page count and an entry point can be named).

**Explicitly out:** cost guard, asset subset, P3, undated-path strict validation, `check_print_styles.sh`, file 16.

## 6. Verification debt

*   `cargo clippy` is in the plan and in `AGENTS.md` but **CI never runs it**.
*   No browser job in CI, so no print verification is possible without first adding one.
*   This workspace: no `frontend/node_modules`, no chromium-family binary, no `backend_rust/target/` (cold full build). `bash scripts/check_access_gate.sh` is the only listed gate that runs unconditionally.
*   The access gate's real constraints on a refactored `analytics.rs` should be quoted in the plan: Rule B ends a handler at the first column-0 `}` and requires `require*` before any `WHERE id` query; Rule C inspects the signature only up to the first `->`; a new file under `src/api_routes/` must import `crate::access` to satisfy Rule A.
*   The Verification Plan cites "invariants I1–I5" and "I1, I2, I4 from the predecessor RFC" while the FeatureRange names only Goal 1–7. Inline the three equalities.

## 7. UNVERIFIED — carried forward

1.  `tests/access_gate_test.rs` and `tests/access_test.rs` never read; either may hold hard counts or tax-summary assertions that move.
2.  Rocket local-client panic behaviour (underpins the guard's panic-release test being unwritable).
3.  ~~Whether `e2e/run.sh` supplies `AUTH_MODE=disabled`.~~ **Answered during the statement review:** it does not, and the harness therefore does not start. `run.sh:74-76` exports only `DATABASE_URL`, `BASE_URL`, `API_URL`; the server defaults to OIDC and bails. Tracked on issue #45.
4.  `frontend/src/test/mocks/handlers.test.ts` may assert the exact handler list and break when the tax-summary mock is branched.
5.  Whether `cargo test` passes offline here.
6.  utoipa / Rocket content negotiation for `?format=csv`.

## 8. Process note

Two stage-9 reviewers hit the 30-minute timeout at 71 turns / ~116 tool calls and were recovered by resuming with forked context and instructing a write-up of verified material only. For a 25-file audit against a repository, scope the reviewer tighter or set an explicit `maxRuntimeMs` — the default is too long to be a useful budget and too short for the task as posed.

## 9. Values left open at the end of drafting — all four settled

Recorded here because each was a fork, not an oversight. Full rationale in the spec's *Open questions*.

| Value | Decision | Why |
| :-- | :-- | :-- |
| Single-parameter dates | **Either alone is legal.** `to` alone = cumulative from first transaction; `from` alone = through today. | Both-or-neither forces an invented start date, and an invented start silently changes the number. The dominant quarterly query is `to` alone. The effective range is echoed in the response and CSV, so an open end is visible rather than inferred. |
| Missing-item cap | **20 symbols, 10 rate pairs**, with `missing_prices_total` / `missing_rates_total`. | The list is for a human; 200 unpriced symbols need the count, not the list. Truncation is never silent. |
| Elapsed deadline | **20 s, checked between outbound calls.** | The spec said "between assets", and that is a hole: one asset can make two unreachable calls (price + FX pair), so 66 s can follow a clean check and the ingress 60 s default returns a 504 instead of the typed 503 — losing the body, `retryable`, and the point of failing closed. Between calls bounds it at 53 s. Testable without waiting: the deadline is a parameter alongside the start `Instant`. |
| CSV summable-column convention | **Row 1 is always headers; caveats ride as columns.** | Evidence, not taste: `detectColumns` (`csvParser.ts:25`) normalizes row 1 against `FIELD_ALIASES`, so a leading `#` row maps every column to `null` and the import silently yields nothing. `lot_pnl` is the column that totals, separated by `row_type`; the disclosure goes in the release note. |

Also closed as a side effect: the lookback and cache-bound constants were already owned and merged by
the pricing-layer work (`FX_LOOKBACK_DAYS = 30`, `FX_SERIES_WINDOW_DAYS = 365`,
`MAX_FX_SERIES_CACHED = 64`), the per-report outbound budget died with the cost guard (D2), and the
print page count became a manual check in the statement document.

One consequence worth carrying into the release note: a cold large portfolio can legitimately exceed
20 s and 503 once, then succeed on retry because the first attempt warmed the per-pair series cache.
Without that sentence a first request after a deploy reads as a bug.
