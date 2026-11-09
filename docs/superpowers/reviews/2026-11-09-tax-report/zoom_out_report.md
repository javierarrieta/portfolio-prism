# Stage 5 — Initial Zoom Out Review

*   **Target:** `docs/superpowers/specs/2026-11-09-tax-report-and-statement-design.md` (689 lines, full file)
*   **Reviewer:** `egm-zoom-out`, fresh context, zoom-out standard
*   **Date:** 2026-11-09
*   **Verdict:** Conditional pass

Context sufficiency passes: the objective, the root problem, and the motivation are stated
and traceable. The root-problem focus is sound — the design correctly identifies one
dated-calculation change plus two renderers, and correctly refuses the obvious wrong answer
of adding `as_of` to `calculate_lots`. Alternatives are honest and every rejection checked
holds against the code. Of the four prerequisites, three are genuine and one is mislabelled.
The cost guard is not load-bearing as written.

## Blockers

### B1 — The architecture diagram contradicts the settled access decision and names a file that does not exist

The diagram still reads `Viewer when undated; Editor when dated`, which is the rule the
"Access level" decision explicitly retired. It also names
`frontend/src/components/ReportView.tsx`, which appears nowhere in the 25-item file list;
the list creates `ReportControls.tsx` and `PortfolioStatement.tsx`.

The diagram is the highest-signal artifact in the document and is read before the prose. On
a tax route, a stale parameter-dependent access rule is the worst possible leftover, because
access is the one thing the grep gate cannot see.

**Fix:** replace the access annotation with `Viewer, dated or not`, and reconcile component
names with the file list. **Applied during this review.**

### B2 — P3's fallback rule requires the network call P3 forbids

P3 says a report "detects an unrecorded split inside the window and names it rather than
fixing it". Detecting an unrecorded split means learning about a split absent from the
`transactions` table, and the only source is the Yahoo split feed that
`sync_splits_for_asset` fetches (`currency_service.rs:541-627`). A report that does not call
it cannot know a split is missing; a report that calls it to find out has put the outbound
split fetch back on the read path with its throttle slot and retry ladder.

**Fix — a deletion, not an addition:** drop the detection clause. The app already supports
manually recorded `SPLIT` transactions, and `apply_split_events` dedupes against them
(`currency_service.rs:46-56`). The rule becomes: the dated path does not sync splits; splits
are whatever is recorded; the run context states that.

## Major

### M1 — The cost guard is speculative, and it guards the cheap path while leaving the expensive one unguarded

Four independent problems.

**(a) The budget duplicates a bound the design already establishes.** The pricing section
shrinks the work set to one close per asset and one FX range per pair. For the design's own
20-asset example that is roughly 25 outbound calls, about five seconds at the 200 ms floor.
The "hundreds of requests" figure that justifies the budget is the *pre-shrink*
per-transaction-date behaviour, which the shrink removes. If the cap is the same one the
shrink already enforces structurally, the counter is a second mechanism for one bound.

**(b) The in-flight slot bounds concurrency, not cost — the argument the design used to
reject the access rule.** The access decision rejects `Editor`-for-dated because an `Editor`
can issue the same expensive request as often as a `Viewer`. The same reasoning applies to
the slot: a user told 429 clicks again, and N sequential reports cost N times the same
thing. The slot restricts how many at once, which is not the cost question the amendment was
written to answer.

**(c) The guard is on the dated path, and the dated path is the cheap one.** The undated
screen is what actually grinds: `analytics.rs:65` runs split sync per asset and
`tax_engine.rs:403` calls `get_rate` per transaction inside `replay`. A 20-asset
multi-currency portfolio with 10 transactions each is about 200 `get_rate` calls through a
200 ms process-wide floor — roughly 40 seconds of serialized waiting against the ingress
cutoff — on a path the guard explicitly declines to touch. P1 does not fix this; it makes
each call a range fetch, still one HTTP call per cache miss.

**(d) Standing decision.** The prior review record refuses "max-transactions limit" and
"p50/p99 targets" with the reason **"No measured bottleneck."** The design amends the
predecessor RFC's non-goal but does not acknowledge that the same family of cost machinery
was already refused once, and offers a rationale rather than a measurement.

**Recommendation:** delete `report_guard.rs`, its managed-state registration, the 429
variant, the guard test cases and the 429 UI state. Keep the work-set shrink. If a
belt-and-braces assertion is wanted, make it a local constant in the pricing resolver that
fails with the existing 503 variant when the resolved work set exceeds it — no shared state,
no `Drop` guard, no panic-leak failure mode, no new status code, no new UI state. The design
itself concedes the hazard: the slot must be released on every exit path including a panic,
or one slow report permanently blocks that user.

If a guard is kept anyway, the honest version bounds the **undated** path too, because that
is where the measured cost is.

### M2 — P4 is an undefined label, mis-sequenced, and its blast radius contradicts Goal 7

`P4` is referenced twice as if defined; the Prerequisites section defines only P1–P3. The
slice table says slice 0 is "P1, P2, P3" while target file 1 puts P4 in "slices 0, 4".

Substantively: changing `get_price` to a result and `get_historical_prices` to stop swallowing
errors changes what the **existing undated screen** does for a delisted symbol — today it
renders market value 0 and a large negative unrealized figure; after P4 it can fail. Goal 7
says the dated-FX fix is the only change to numbers on existing endpoints. That is false the
moment P4 lands, unless P4 is specified to stay permissive on the undated path.

**Fix:** promote P4 to a numbered fourth prerequisite with its own paragraph, assign it to
exactly one slice, and state which way the undated path behaves. "Undated stays permissive"
is a legitimate choice and keeps Goal 7 true — but it must be said.

### M3 — Strict strategy validation rests on a false claim about the code, reverses a standing rejection, and contradicts the Route decision

**(a) The precedent claim is wrong.** The design says the single-sell P&L route "already
normalizes strictly". It does not. `analytics.rs:218-222` is:

```rust
let strategy_val = match strategy.unwrap_or_default().to_uppercase().as_str() {
    "LIFO" => "LIFO".to_string(),
    "HYBRID" => "HYBRID".to_string(),
    _ => "FIFO".to_string(),
};
```

`_ => FIFO` is a silent fallback, behaviourally identical to `analytics.rs:59`. Neither route
rejects anything. They differ only in what they echo back, which is the defect the prior
record describes: *"The real defect was echoing unvalidated input as fact; one normalisation
fixes it."* There is no existing strict route to align with. **Verified during this review.**

**(b) It reverses a standing rejection.** The prior record lists "Input validation matrix for
`strategy` / `threshold_days` (400s, ranges)" under *Rejected as over-reach — do not re-raise
without new evidence*. The design applies strict rejection on both paths and reconciles
against the RFC, but the standing record it silently reverses is the review record, and that
one is not reconciled.

**(c) It contradicts the Route decision.** "Route" says absent `from`/`to` the endpoint
behaves exactly as today. A 200→422 change on the undated path is not "exactly as today", and
Goal 7 says the FX fix is the only existing-endpoint change.

**Fix:** scope strict validation to the dated path only, or keep it on both and amend Goal 7
and the Route sentence and record the reversal of the standing rejection explicitly. Either
way, delete the false-precedent sentence.

### M4 — Goal 1 is stated over-broadly and a literal test will fail

Goal 1 says the window sum equals `tax-summary`'s `realized_pnl` "for any `[start, end]`".
`tax-summary` computes realized P&L over **all** transactions and has no date parameter, so
the equality holds only when the window covers every sell. The predecessor stated this
correctly as invariant I1: "for any `end` at or after every sell in the window". The design
dropped the qualifier from the normative Goals section, which is what a test author reads
first.

### M5 — `check_print_styles.sh` is a second gate for a property the design concedes only a browser can check

File 22 proposes a new `frontend/scripts/` directory (does not exist today) plus CI wiring to
grep that the print stylesheet exists and hides chrome selectors. File 23 proposes a
Playwright smoke asserting chrome is `display: none` under print emulation, and the design
says of it: "This is the only honest check that printing works."

If that is true, the grep is a gate that passes on CSS text matching nothing — it asserts the
presence of a selector string, not the absence of chrome. Two gates for one property, one of
which can be green while the property is broken, manufactures confidence. The `ci.yml` nginx
grep precedent does not carry over: that grep exists because the nginx rule is outside every
test suite; print CSS is inside one.

**Fix:** delete file 22. Also note file 23 adds `npm run e2e:statement`, which needs a
`frontend/package.json` script and a parameterization of `e2e/run.sh` (currently hardcoded to
`node e2e/sell-pnl.smoke.mjs`) — `package.json` is not in the target list.

### M6 — The design asserts reuse of CSV guards that are module-private

"Reuses the formula-lead guard and the exponent-free formatting rule from `sellPnlCsv.ts`" —
neither is exported. `FORMULA_LEAD` (`:55`), `text` (`:56`) and `num` (`:62`) are
module-private; the file exports only `sellPnlToCsv` and `sellPnlFilename`. **Verified
during this review.** So "reuses" means "copy-paste", and the formula-lead guard is an
injection control: `sellPnlCsv.ts:46-54` documents that a symbol reaches the CSV as
attacker-chosen free text. Two copies of a security guard is the drift this design exists to
prevent.

**Fix:** name the extraction — export `text`/`num`, or move them to a shared
`frontend/src/utils/csvCells.ts` both builders import — and add that file to the target list.

## Nits

*   **N1** — `ReportResponse` repeats `strategy` and `threshold_days` already in
    `ReportContext`, and `AssetTotals` carries the per-asset pricing dates. Three homes for
    two facts, which is the objection the design raises when rejecting Alternative D. Pick one
    owner per field.
*   **N2** — Goal 5 says the response enumerates *every* missing symbol and pair; the error
    contract caps the list with an omitted count. Contradictory. Given the shrink bounds the
    asset count, drop the cap.
*   **N3** — Target file 16 offers "add a string-preserving path **or** leave this file
    untouched". The design already decided report money bypasses `normalize*`. Delete the row
    and pin the invariant in `taxReportCsv.test.ts`; an "or do nothing" row invites a second
    normalizer.
*   **N4** — `AssetTotals.split_applied` reuses a field name whose existing meaning is
    deliberately narrow — scoped to lots a specific sell consumed (`analytics.rs:232-238`).
    A per-asset field cannot mean the same thing. Rename with an explicit definition, or drop
    it.
*   **N5** — Route, PDF and subset rationales are restated in four places: Non-Goals, Decided
    during drafting, Reconciliation, and Alternatives. Consolidating would cut roughly 100
    lines and remove the risk that a revision updates one copy and not the other — which is
    how B1 happened.

## Verified sound

Prerequisites P1, P2 and the core of P3 are real live defects, each independently confirmed:
the FX symbol form and ignored date (`currency_service.rs:235`, `:220-222`, `:257`) with no
range parameters at `:236` and a per-transaction caller at `tax_engine.rs:403`; the missing
unique constraint (`lib.rs:170-176`) with `INSERT OR REPLACE` at `:397` and `:490`; the
count-then-insert race at `currency_service.rs:46` and `:59`.

Engine claims all verified: `replay` private (`:390`), breakdowns discarded after summing
(`:569`), `latest_date` from `sorted_txs.last()` (`:550-551`), split branch rescaling every
lot (`:517-531`), `sell_breakdown` replaying full history (`:619`), `TaxLot.original_qty`
and `remaining_qty` both the post-decrement `lot.qty` (`:587-588`) with no buy-transaction
id. The Holdings-table consequence is real: `PortfolioDetail.tsx:322-323` renders two
columns fed from one value at `:333-334`. The "three changes, not one parameter" analysis is
the most valuable content in the document.

Existing-figure claims verified: `Access::Viewer` (`analytics.rs:30`),
`body = serde_json::Value` (`:17`), error erasure via `.map_err(|_| Status::InternalServerError)`,
`normalizeTaxSummary` coercing through `Number()` (`decimal.ts:59-84`), the
`SellPnlModal.tsx:283` sentence, no `@media print` rule anywhere in `frontend/src`, the hard
counts at `route_coverage_test.rs:140`/`:262`/`:276`, the `auth_test.rs:41-49` special case,
the `isolation_test.rs:195` path list, the 200 ms throttle (`currency_service.rs:14`), and
`k8s/app-deployment.yaml` at `replicas: 1` / 256Mi.

Correct reuse, no reinvention: `decimal_json` (`schemas.rs:5`), `downloadCsv` owning the BOM
(`download.ts:20-25`), `Access::Viewer` and `require_asset_in` (`access.rs:26`, `:141`), the
`Unavailable` responder precedent (`auth/routes.rs:33-52`), `formatters.ts`, and the hermetic
seeding pattern.

Rejections not to disturb: no server PDF; extend `tax-summary` rather than add a route;
server-owned totals; one engine function with two selections; no TypeScript lot matching;
deferred column metadata.

Slice 1 is not double-precision despite appearances: predecessor Decision 7 permits the print
view to round for display, and full precision is a CSV requirement.

## Deletable outright without losing the stated goal

| Item | Cost of keeping |
| :-- | :-- |
| `report_guard.rs` + managed state + 429 variant + guard tests + 429 UI state | New shared mutable state with a permanent-block failure mode, guarding an unmeasured cost on the cheaper of two paths |
| `check_print_styles.sh` + CI wiring | A green gate that can pass while printing is broken |
| P3's split-detection clause | Unimplementable without re-adding the forbidden fetch |
| `decimal.ts` target row | Invites a second normalizer |
| `ReportResponse.strategy` / `threshold_days` duplicates | Two owners for one fact |
| Missing-list cap | Contradicts Goal 5 |

Net: roughly one new backend module, one CI script, one CI step, one status code, one UI
state and two payload fields, with no loss against Goals 1–7.
