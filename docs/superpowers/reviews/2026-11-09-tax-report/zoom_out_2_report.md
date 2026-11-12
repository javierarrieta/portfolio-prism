# Stage 8 — Intermediate Zoom Out

*   **Target:** `docs/superpowers/specs/2026-11-09-tax-report-and-statement-design.md`
*   **Reviewer:** `egm-zoom-out`, fresh context, read stages 5–7 reports first
*   **Date:** 2026-11-09
*   **Question asked:** not "what is wrong" but "does this survive what was found, and is it still one feature"

## Verdict

The design survives — the engine analysis, the route choice, the server-owned-totals decision
and the realized/unrealized cut line are sound and were independently confirmed by three prior
rounds. But it is no longer one feature. It is three features and one orphaned defect held
together by a sentence in the Overview ("they are not two features"), and the accumulated
findings land on the two halves that were never load-bearing for the owner's actual question.
**Targeted repair will not fix this; a split will.** The dated-FX work should become its own
design document that this one depends on, the print/statement half should be deferred to its own
short design, and the cost guard, the asset subset and P3 should be deleted outright.

## 1. The cost story, and whether the guard survives its fix

The critic's framing is slightly too strong; the underlying defect is real and is a sequencing
bug inside file 1. The design *does* list a change reaching the per-transaction FX path — file
1's third bullet ("a per-currency-pair range fetch with local date lookup, so a dated report
issues one close per asset and one FX range per pair rather than one call per transaction date")
and the Pricing paragraph. What it never says is the mechanism that makes that true:
`get_rate`'s cache is keyed `(from, to, date)` (`currency_service.rs:125`) and the URL carries no
range (`:236`), so "one FX range per pair" requires the cache to stop being keyed by date and
become a per-pair series that a date lookup reads.

As written, file 1 bullets 1 and 3 are separable, and the separable reading is worse than today:
an implementer adds `period1`/`period2` to `get_rate`, keeps the per-date cache key, and adds a
separate range helper used only by the valuation path. Result: every distinct transaction date
now triggers a **multi-year range fetch** instead of today's cheap default-range call — ~200 range
fetches on the *undated* screen, which the guard explicitly declines to touch. **These two bullets
are one change and must be stated as one.**

**Given that fix, the guard collapses:**

*   After a per-pair series cache, the work set is structurally `#assets closes + #pairs ranges`
    ≈ 22 calls for the flagship portfolio ≈ 4.4 s at the 200 ms floor (`:14`). The budget
    duplicates a bound the shrink already enforces — stage 5 M1a stands and gets stronger.
*   The in-flight slot bounds concurrency, not cost — and the design used *exactly that argument*
    to retire the `Editor` rule ("an `Editor` can issue the same expensive request as often a
    `Viewer` can"). The Access paragraph refutes the guard in the same document.
*   The guard is per-process state resting on `replicas: 1` being a manifest value, not an
    invariant (verified `k8s/app-deployment.yaml:9`), with a permanently-blocked-user failure mode
    the design itself concedes.
*   The real hazard is elapsed time, not call count. Verified: `k8s/ingress.yaml` carries **no
    annotations at all**, so the ingress-nginx 60 s default applies; reqwest times out at 10 s
    (`:134`, `:173`) with `MAX_RETRIES = 3` and 1 s/2 s backoff ≈ 33 s worst case per unreachable
    host. Two unreachable symbols blow the cutoff and the browser gets an HTML gateway error,
    never the typed 503. A counter cannot bound that.

**Delete:** file 3 (`report_guard.rs`) entirely, file 2's guard-registration sentence, the 429
clause in the Error contract, file 6's slot acquisition, file 9's guard cases, file 18's 429
sentence, and the first paragraph of Reconciliation — which exists only to amend the RFC non-goal
to admit the guard. **Replace with two sentences:** the ingress deadline is a stated constraint,
the resolved work set is bounded by a local constant in the pricing resolver failing with the
existing 503 variant, and the handler stops attempting further assets once elapsed time
approaches the cutoff, returning what it collected. No shared state, no `Drop`, no new status
code, no new UI state — and the elapsed-deadline sentence fixes critic M-b, which the guard did not.

## 2. The asset subset

Confirmed unfixable as drawn: `tax-summary` already returns per-asset summaries plus a portfolio
rollup (`analytics.rs:57-97`) and the payload copies that shape, so a client-side subset filter's
only legal-looking move — summing the selected `AssetTotals` — is the forbidden operation, and
printing the rollup above a filtered row set puts a headline describing a different asset set
than the rows beneath it on the one artifact meant for an adviser.

**Cut the subset.** What goes with it: **Alternative G** (14 lines existing solely to justify
keeping it), the `require_asset_in`/IDOR discussion — the only security content in a document with
no Risks section, so cutting the subset removes the row rather than leaving it unmitigated — file
19's client-side filter clause, and stage 6 Nit 16 with it. The alternative repair (a server
`asset_ids` parameter) would add a query parameter, an access surface the grep gate cannot see, and
`isolation_test` cases: it grows the design to save a checkbox.

`AssetTotals` stays; the statement's per-asset table needs it regardless. **Cost:** the owner
cannot print a statement for three funds instead of the whole portfolio. Given Alternative G's own
words ("a tax report is definitionally portfolio-wide") and the predecessor's note that nobody
files or signs the statement, that is cosmetic.

## 3. Goal 7 restated — and slice 0 needs restructuring, not reordering

Restating Goal 7 as "every change to existing figures is named and ships with its own note" is
necessary and not sufficient. The sequencing has an unstated hard dependency and two misplaced
items:

*   **P2 must precede P1.** P1's range fetch writes the fetched series into `historical_prices`;
    without `UNIQUE(symbol, date)` that write appends ~250 rows per symbol per fetch. Shipping P1
    first makes the duplication the report depends on *worse*. Slice 0's cell reads "P1, P2, P3"
    with no order.
*   **P1 must ship with the per-pair series cache**, so slice 0 is really two ordered changes: P2,
    then P1 + series + symbology + cache versioning + `sell_pnl_test.rs` + `SellPnlModal.tsx`
    footnote + release note.
*   **P4 has no consumer until slice 4.** The error contract is its only reason to exist; putting
    it in slice 0 forces the undated-path behaviour change early with nothing to justify it. Move
    it to slice 4 and state there that the undated path keeps today's permissive behaviour — which
    also answers critic M-u and stage 6 Major 9 for free.
*   Slice 0's `Network? no` is false and `Risk: low` grades the highest-blast-radius item as low.
    **Both cells become honest the moment slice 0 stops existing** — which is what the split does.

## 4. P3 is not in scope, and this feature should explicitly not wait for it

Verified: `main.rs:63` is a plain `.launch()` with no Rocket tasks; `k8s/` contains deployment,
service, configmap, ingress, pv, pvc, namespace and secret only — no CronJob, no Job. "Move it off
the read path" has no destination without new background machinery that both this design's
Non-Goals and the RFC's concurrency exclusion rule out. Both readings break an invariant (C3).

Two facts the earlier rounds did not use:

*   `sync_splits_for_asset` is throttled to once per symbol per 30 days and returns `Ok(0)` on a
    `split_sync` hit (`currency_service.rs:541-560`). The read-path write is a rare event, not a
    per-request fan-out. The cost urgency P3 was promoted on does not survive that.
*   **The repo already decided this question once.** The standing record's *Split-sync ordering —
    documented and pinned, not removed* entry says making the new endpoint run the sync "was
    considered and rejected: it turns a GET into a writer", and the read-only endpoint's agreement
    with `tax-summary` is "pinned by a route test". P3 silently reverses a standing decision — the
    same pattern as the validation-matrix reversal — and unlike that one, it reverses it with no
    new evidence and no destination.

**Judgement:** P3 is a separate investigation (non-transactional count-then-insert, no unique
constraint on `transactions`, no WAL/`busy_timeout`, no worker to move the write to). The cheap
rule, following existing precedent: the dated path does not sync splits, exactly as
`/transactions/<id>/pnl` already does not; the run context states that splits are as recorded; the
retroactive-split hazard is named as a known limitation with a pointer to the separate
investigation. That deletes stage 5 B2, removes the dated/undated split-disagreement objection (it
becomes the pinned, already-accepted shape), and dissolves critic M-c's SQLITE_BUSY-has-no-error-class
problem, because the dated path then performs only the price write the predecessor explicitly
permitted ("no `SPLIT` inserts, not no writes").

**Cost:** the race stays live (two concurrent first-in-30-days requests for one symbol can double a
split) and a late-recorded split can still move an already-filed quarter. Both are today's
behaviour, unchanged by shipping this feature.

## 5. Access and strict validation, with their reasons removed

**Access: keep the decision, delete the paragraph.** Verified `access.rs:37-57` — `resolve` returns
`Owner` or `None`; `Viewer`/`Editor` are unattainable and there is no membership table. The route
already requires `Viewer` (`analytics.rs:29`), so the decision is a no-op that costs nothing. What
survives is one sentence: access is `Viewer`, dated or not; the grep gate cannot see a
parameter-dependent rule, so `isolation_test` covers the dated variant. Delete the
accountant/adviser argument and file 9's Viewer-collaborator case, which cannot be written against
this model.

**Strict validation: keep it on the dated path only.** With the fictional precedent removed, the
surviving argument is the design's own — a typo'd strategy in a tax file is a defect, not a
default — and that applies to a brand-new contract with no back-compat obligation. On the undated
path, strict rejection contradicts "absent parameters mean today's behaviour", contradicts Goal 7,
has no target file, and silently reverses a standing over-reach rejection. Scoping to the dated
path also shrinks file 10 and removes the `/pnl` route question entirely. **Cost:** the app keeps
two rules on one axis for the legacy screen — the status quo the standing record already accepted.
If the owner still wants it on both paths, that is a legitimate call, but it must cite the record
it reverses and amend Goal 7.

## 6. Provenance placement — not cosmetic; it is the drift mechanism

Verified: `docs/superpowers/reviews/2026-11-05-sell-pnl-breakdown.md:9-12` states the convention's
division of labour — the record holds decisions and rejections and deliberately does not restate
findings, *because the why lives inline next to the rule it justifies*. This feature has one file
doing both jobs and no review record at all. That is the mechanism behind three reviewed defects:
the stale diagram, the Access paragraph arguing against a rule the same section retired, and the
Reconciliation paragraph defending a guard that stage 5 showed guards nothing. Rationale carried in
three to four copies will drift; the copies are the bug.

Fix, respecting the existing convention: create `docs/superpowers/reviews/2026-11-09-tax-report.md`
in the existing shape and move into it the retraction narratives, the RFC-amendment record (which
disappears with the guard), the two standing-rejection reversals, and the toolchain-verification
claims. In the spec, keep the rule plus the one or two sentences of reason that justify it in
place. If P1/P2 leave for their own document, `### Prerequisites` collapses to a two-line
dependency pointer and critic M-h resolves itself without a new table.

## Scope judgement: three features and an orphan

| Strand | What it is | Load-bearing for the quarterly question? |
| :-- | :-- | :-- |
| **A. Dated reporting** | `replay_at`, dated `tax-summary`, typed payload, CSV, `ReportControls` | Yes — this is the feature |
| **B. Pricing-layer truthfulness** | P1 dated FX + symbology + per-pair series cache + cache versioning, P2 uniqueness, P4 honest result types | No — a live bug fix worth shipping if the reports never exist |
| **C. Print subsystem** | net-new print CSS, statement component, print e2e, CI grep, manual browser check | No — and its first half needs no backend at all, per the design's own table |
| **D. P3** | split writes on a read path | No — pre-existing, unowned, no destination |

The Overview's thesis is true of A and one of its renderers. It is being used to hold B and C
inside one review unit, and that is what let a 689-line document accumulate four prerequisite
defects, a stateful guard, a CI grep and a browser runtime while describing itself as "one dated-
calculation change plus two renderers".

**Doc A** (`currency_service.rs`, `lib.rs`, `sell_pnl_test.rs`, `SellPnlModal.tsx`, release note, FX
backend test): ~5 files, no new route, no new frontend surface, order P2 → P1+series → P4.
**Doc B** (this one, repaired): ~14 files.

Reframing dated FX as a dependency rather than an in-document prerequisite fixes at once: the
"outside this feature's scope but block it" contradiction, Goal 7's exclusivity, the disappearance
of slice 0 and its two false cells, the P2-before-P1 ordering, and the standing record's open item
1 ("FX conversion path has no backend test") — which Doc A discharges as a side effect.

**The 25-file plan is not the right cut — it is simultaneously too big and incomplete.** Seven
files leave (1, 2, 11, 24 → Doc A; 3, 16, 22 → deleted), two are missing and must be added if the
print and CSV halves stay (`frontend/package.json` for the `e2e:statement` script file 23 requires;
a shared CSV-cell module if the formula-lead guard is to be reused rather than copied), and two of
the design's own decisions have no owning file at all (P3, strict validation of the `/pnl` route).
A file list that cannot name where two of its decisions land is mis-cut, not under-populated.

## The smallest version that delivers the owner's goal

The goal as stated: a quarterly answer — what I sold in the window, at what basis, with what gain;
what I still held at the window's end and what it was worth — as a file the owner can re-crunch.
That is **Doc A + slices 2, 3, 4 minus the guard**: `replay_at`, dated `tax-summary` with
both-or-neither dates, typed payload, collected-failure error contract with an elapsed deadline,
CSV builder and tests, `ReportControls`. One route, no new route, no new shared state, no new CI
job, no new browser runtime, no new deployment machinery.

**Cut the statement and everything print-shaped** — files 19, 20, 21, 23 plus `package.json`,
Goal 6, Alternative A, the Print line of the Verification Plan, and the manual browser-print item
that `docs/auth-testing.md` does not list. Slice 1 is pure frontend over today's payload and
depends on none of the dated machinery, so it is a good follow-up design and a poor reason to hold
the CSV hostage.

**What cutting costs:** one release cycle without "a page to hand to someone", and no past-dated
printable statement. Everything the tax answer needs is in the CSV, including both headline figures
and the full run context. Note what the cut removes on the reviewers' side too: stage 6 Majors 3,
7, 8 and 16, stage 5 M5, and the undefined-selector / unrunnable-script / unstated-page-count
cluster all live in that slice. It is the part of the design no reviewer could verify.

**One thing to re-open before slice 3, not ratify:** server-generated CSV via `?format=csv` on the
same route (critic M-s). Every argument the design makes for server-owned numbers — strings end to
end, renderers forbidden from summing, the `normalize*` bypass, "a total derived in TypeScript is a
double" — is an argument for emitting the file where the decimals live. It preserves Alternative B's
no-new-route property, makes Goal 4 byte-reproducible and `curl`-testable, and deletes the browser
CSV builder, its float-summed test, the `decimal.ts` question, the BOM-ownership contradiction, the
copied formula-lead guard, and the `#`-header-versus-columns contradiction with
`sellPnlCsv.ts:13-16`. That is roughly two files replaced by one and six reviewed contradictions
dissolved. It is cheaper than the guard it would displace and it is the single highest-leverage
simplification left on the table.

## What to delete, concretely

| Delete | Lines / files | Why it is safe |
| :-- | :-- | :-- |
| `report_guard.rs` + managed state + 429 + guard tests + 429 UI state + RFC-amendment paragraph | file 3, file 2 tail, Error-contract 429 clause, Reconciliation ¶1, file 9 guard cases, file 18 tail | Work set is structurally bounded after the series cache; the slot bounds concurrency, which the Access paragraph already refused as a cost argument; the real hazard is elapsed time, which a counter cannot bound |
| Asset subset (statement) + Alternative G + IDOR discussion | Alternative G, file 19 clause | Only consumer of the forbidden client-side sum; cosmetic loss |
| P3 as a prerequisite + split-detection clause | P3 bullet, detection clause | No destination exists; standing record already decided "pinned, not removed"; dated path simply does not sync, per the `/pnl` precedent |
| Strict validation on the undated path | Date-semantics scope | Restores Goal 7 and the Route sentence; removes the missing target file |
| Access rationale + collaborator test | Access ¶, file 9 clause | Decision is a no-op; rationale cites a sharing feature that does not exist |
| `decimal.ts` row | file 16 | Design already decided report money bypasses `normalize*` |
| `check_print_styles.sh` + CI wiring | file 22 | A green gate that can pass while printing is broken; the Playwright smoke is the honest check |
| Print/statement slice (defer, not reject) | files 19, 20, 21, 23, Goal 6, Alternative A | Slice 1 needs no backend; belongs in its own short design where selectors and page count can be named |
| P1, P2, P4, work-set shrink, cache versioning → Doc A | files 1, 2, 11, 24 | Independent user-visible bug fix with its own release note and blast radius |
| Provenance narratives → new review record | Decided-during-drafting retractions, Reconciliation | Matches the existing convention's stated division of labour |

**Do not trim:** the engine analysis (three changes, not one parameter), one engine function with
two selections, server-owned totals, Alternative B's reasoning, the C/E/F rejections,
no-server-PDF, the hermetic seeding requirement, `decimal_json` reuse, and the realized/unrealized
cut line. The Alternatives section is unusually strong; the trimming target is provenance, not
alternatives.
