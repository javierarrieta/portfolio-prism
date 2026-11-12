# Consolidated Review Report — Portfolio Statement (Printable)

**Document:** `docs/superpowers/specs/2026-11-09-portfolio-statement-print.md`
**Reviewed at:** commit `bd0390d` (first draft) through `cec16ab`
**Review mode:** Interactive, all rubrics — Stages 5 → 6 → 7 → 8 → 9, check-in after each
**Standard:** `feature-spec-standard.md` (Stages 6–7), `zoom-out-standard.md` (Stages 5, 8), `implementation-readiness-standard.md` (Stage 9)

| Stage | Report | Findings | Disposition |
| :-- | :-- | :-- | :-- |
| 5 — Initial Zoom Out | `zoom_out_report.md` | 2 B / 7 M / 7 N | **applied** — `a43418c`, `b2a5b5b`, `49bd706`; one re-audit used |
| 6 — Comprehension Test | `comprehension_report.md` | 1 B / 12 M / 5 N | **applied** — `0fddeff` |
| 7 — Critic Review (design + structural + style) | `critic_report.md` | 8 B / ~30 M | **applied** — `d880f21` |
| 8 — Intermediate Zoom Out | `intermediate_zoom_out_report.md` | 1 B / 7 M / 8 N | **applied** — `71990a7` |
| 9 — Implementation Readiness | `implementation_readiness_report.md` | 5 B / 16 M / ~25 N | **OPEN — not applied** |

Also in this directory: `decision_records.md` — derivations deliberately moved out of the spec at Stage 8 (precision forensics, e2e harness forensics, correction history, Alternative B's scope note).

---

## Where the document ended up

First draft 330 lines → peak 718 after three fix waves → **624** after the Stage 8 altitude cut. Roughly 15 blockers, 75 majors and 50 nits were raised across five stages; 12 blockers and about 56 majors were fixed.

The architecture never changed and survived every stage: a fifth tab over already-fetched state, a variable-override print stylesheet scoped to that tab, no backend change, and a display-artifact declaration instead of a fake-exactness mechanism.

### Decisions the review changed

| Original position | Final position | Stage |
| :-- | :-- | :-- |
| String-preserving decimal normalizer | Deleted — the statement is a **display artifact**; `normalizeTaxSummary` already crossed the boundary | 5 |
| Payload doesn't echo `strategy`/`threshold_days` | False — it echoes all three; the gap was the normalizer dropping two | 5 |
| Enumerated hide list | Inclusive `:not()` form, then scoped with `:has()` | 5 → 8 |
| Smoke asserts dark text on light background | Text colour only — `printBackground: false` makes the background assertion unsatisfiable | 7 |
| Smoke pins prices via `historical_prices` | Deleted — no API route writes that table, and no figure is asserted | 7 → 8 |
| ≤15-significant-digit carve-out + bespoke golden | Deleted — evidenced from `roi`, a ratio, not a money field | 7 → 8 |
| `[data-no-print]` opt-out rule | Deleted — a specified interface with no user | 7 → 8 |
| `body[data-print-scope]` set from JS | CSS `:has()` — no lifecycle, no cleanup obligation | 8 |
| MSW handler branch, fixture pinned to id 3 | Prop fixture, per the repo's existing convention; `handlers.ts` untouched | 8 |
| `taxSummaryFor: {id, data}` state reshape | Parallel id, following the `App.tsx:68` precedent that already rejects clear-on-switch | 8 |
| "Depends on: pricing layer" | "Inherits a pre-existing defect; not blocked on it" | 8 |
| Precision and harness forensics inline | Moved to `decision_records.md` | 8 |

---

## Blockers — all five closed after conclusion

**This document is not implementation-ready.** Stage 9's verdict was that a fresh engineer could not produce the correct change on a first attempt without inventing interfaces. Four blockers remain:

1. **RB1 — the component's prop signature was undefined**, and "takes no date parameter" forbade the prop `data-at` needs. The two new optional type fields are provably insufficient: `run-context` needs the portfolio name, which the payload does not carry. **Closed:** new spec section *The component's interface* — `{ taxSummary, portfolio, fetchedAt }`, with App-side `taxMeta: { portfolioId, fetchedAt }` as the parallel record set beside `setTaxSummary`.
2. **RB2 — no hook on any headline figure or the holdings table.** Four specified assertions could not be written; print rule 5's "headline block" had no class to name; the nine-column table Decided item 1 argued for shipped with no test. **Closed:** new section *Structural hooks and the holdings table* — `figure-value` / `figure-realized` / `figure-unrealized` / `holdings` / `.statement-headline`, nine columns with a named renderer each, and the golden pinned to `€1,234.57`.
3. **RB3 — the nav item's visible label existed nowhere in the document**, and it is the smoke's only entry point. **Closed:** `Printable Statement` with a `Printer` icon; the `<li>` is written out in *Surface*, and the smoke uses `page.getByText('Printable Statement')` — the working idiom, since these are `<li onClick>` with no accessible name.
4. **RB4 — "The rounding note is unconditional"** contradicted four places that say it is not printed on the empty branch. **Closed:** rewritten to "not conditional on the currency", with the empty-branch exclusion stated in the same bullet.

**RB5 is closed as an accepted gap** (owner decision, after this report). A foreign-currency portfolio returns **500** (`analytics.rs:86`), not a mispriced figure, so `taxSummary` stays `null`, the guard renders nothing, and the explanatory banner is hidden by print rule 2. The spec now states this in *When there is no payload at all — an accepted gap*: in that state the tab behaves like the other three tabs, which are already out of scope, and the pricing-layer change removes the cause. Two consequences are recorded rather than left to be discovered — the hidden banner, and the smoke's same-currency fixture constraint.

Sixteen majors cluster into three kinds: values the implementer must choose (print palette hexes, date formats, `page.pdf()` options, per-column renderers), assertions that cannot execute as written (document-level overflow check defeated by `body { overflow-x: hidden }`; the smoke cannot set strategy through seeding), and two files missing from the file boundary (`docs/auth-testing.md`, `frontend/e2e/README.md`).

### Three defects the fix waves introduced

Recorded because the pattern matters more than the instances: each was a correct fix to a real finding, applied without re-deriving the claim around it.

- Stage 7 introduced the `decimal.ts:65-67` citation, which is the wrong line (currency is `:63`).
- Stage 8 introduced "foreign-currency assets are already mispriced on the dashboard today", which the code refutes — they fail outright.
- Stage 8's `:has()` scoping, correct in itself, created the inert-selector hole in RB5.

### The two structural patterns

**Restatement produced contradiction.** Stage 8 counted six sections restating the rounding rule and five restating the empty-portfolio currency rule; Stage 9 counted thirteen rules restated somewhere. RB4 and M12 are both products of that mechanism, as is the whole Stage 7 blocker set.

**Review-demanded machinery accumulated.** Stage 7 asked for determinism, a defensive guard, a bounded residual, an opt-out hatch and an implementable stale-state rule. Each demand produced a mechanism; by Stage 8, four of the five were unreachable, untestable, or owned by a different document. Stage 8 deleted them.

---

## What is sound and should not be revisited

Verified independently by two Stage 9 reviewers against the working tree:

- `:has()` matches the real DOM. `.app-container` → `<main class="main-content">` are parent/child; the auth alert, sidebar and modal are direct children of `.app-container` so rule 1's enumeration is complete; the tab branches sit in a JSX fragment, so a bare component root genuinely can be a direct child of `.main-content`.
- The prop-fixture test is supported by the harness (`vite.config.js:26-29`, `test/setup.ts:3` pins `TZ=UTC`), and `handlers.ts:121` is genuinely untouched by this design.
- The rounding arithmetic is right: the nearest double to `1234.565` sits above the midpoint, so `Intl` gives `1,234.57`; JPY gives `¥1,235`.
- No cross-branch contamination — nothing cited exists only on `fix/pricing-layer-truthfulness`, and `currency_service.rs:234` still builds the broken pair here, so the inheritance framing is accurate.
- Slice atomicity holds. Nothing waits on the parent design; the only coupling is that both edit `types.ts` and `App.tsx` — a merge conflict, not a dependency.
- Of 90 citations re-derived at Stage 9, 79 were correct.

---

## Consequences for what comes next

1. ~~Do not write Doc C's implementation plan from this document as it stands.~~ **Done.** RB1–RB4 are closed in the spec, so the plan no longer inherits them. The remaining majors are citation drift and restatement cleanup; none blocks a plan.
2. ~~RB5 needs a decision.~~ **Done.** The owner chose "accept the gap and say so"; the spec now states it as such.
3. **Two edits already landed in the parent design** as a result of Stage 8: its Goal 4 no longer claims the statement prints the same run context, and its Target File 6 now carries strategy normalization for the undated path.
4. **`decision_records.md` §3 is destined for GitHub issue #45** and `frontend/e2e/README.md`. Not yet posted — that needs a network call.
5. **Doc B (the tax report) is unchanged by this review** and remains the next design work: its design is settled except for four open values (single-parameter dates, the CSV summable-column convention, the elapsed-deadline constant, the missing-item cap).
