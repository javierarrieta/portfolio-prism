# Stage 7 — Critic Review

*   **Target:** `docs/superpowers/specs/2026-11-09-portfolio-statement-print.md` (644 lines, whole document)
*   **Reviewers:** `egm-design`, `egm-structural`, `egm-style` (concurrent, 3 at once)
*   **Standard:** `feature-spec-standard.md` for design and structural; style reviewer on its own remit
*   **GoldfishContext:** pricing-layer spec, parent tax-report design, `AGENTS.md`
*   **Raw counts:** design 1 blocker / 8 majors / 5 nits · structural 4 / 12 / 15 · style 3 / 11 / 17

Every Blocker-level claim below was re-derived from the tree by the controller, not accepted on
the reviewer's word. The reviewers were told not to re-report the two known weak points
(document length, and the `run.sh` `AUTH_MODE` gap); they complied, and still found a blocker
that makes the printed page blank on three of the app's four tabs.

**The pattern across all three reports:** this document has been patched in place through two
review rounds, and every patch restated a rule somewhere it was already stated. Six of the
findings are the same rule worded two incompatible ways in two sections. Structural counted it:
the empty-portfolio currency rule appears **5 times**, the rounding rule **6 times**, the
grep-gate rejection **4 times**. That is not a length problem, it is the mechanism by which the
contradictions below became possible.

---

## Convergence — findings raised by two or more reviewers independently

| # | Finding | Raised by | Severity |
| :-- | :-- | :-- | :-- |
| C1 | Rule 2 blanks every non-statement tab; the document claims the opposite | design B1 | **Blocker** |
| C2 | Money section restates the "2 dp for fiat" claim Goal 3 explicitly rejects | design M4, structural M1 | Major |
| C3 | `data-no-print` is simultaneously used and unused | design M9, structural M2 | Major |
| C4 | Smoke asserts "dark text on a light background" — the thing Goal 6 disclaims; `page.pdf()` defaults `printBackground: false` | design M7, structural B4 | **Blocker** |
| C5 | Sequencing cell says "reaches network: no" while the slice ships a smoke that contacts a real backend | design M2, structural B1 | **Blocker** |
| C6 | Mandated on-page copy asserts a tax-report CSV that does not exist | style B2, structural M7 | **Blocker** |
| C7 | Layer-1 hook-presence assertions are unsatisfiable against the empty-portfolio branch | structural B3, design N13 | **Blocker** |
| C8 | Generation date has no hook, no copy row, no assertion, and no stated clock | structural M3, design M3 | Major |
| C9 | The MSW branch rationale names a test relationship that does not exist | design M8, structural M11 | Major |
| C10 | Narrative leakage and honesty self-commentary are still pervasive after the last compression pass | style M1, M2 (18 + 12 instances) | Major |

---

## Blockers

### B1 — Rule 2 makes every non-statement tab print a blank page, and the document says the opposite

`egm-design`. Rule 2 is `.main-content > *:not([data-print="statement"]) { display: none }`. On
the dashboard, analytics or ledger tab nothing in `.main-content` carries that attribute, so
every direct child is hidden and the page is empty. The document asserts the opposite in *Scoping
the inversion*: "Ctrl-P from the dashboard after this ships produces a light-on-dark page with
the sidebar hidden and the dashboard printed — better than today's white-on-white."

Both statements cannot be true. The Problem section names "Ctrl-P on the dashboard" as the
owner's **current** workflow, so this slice regresses the one behaviour named as the baseline —
into a blank A4 sheet — with no risk row and no mitigation. It also voids the residual analysis
below it: `.badge-crypto` cannot "print badly" because nothing prints.

**Fix:** the document already names the fix — scope both hide rules under a body attribute the
statement tab sets (`body[data-print-scope="statement"]`). One attribute, other tabs print as
today, blank-page failure removed. If declined, correct the scoping paragraph to say "prints a
blank page", add the risk, and drop the two background residuals as moot.

*Controller note: this is the strongest finding of the review. It came from a rule I introduced
in Stage 5 to replace an enumerated hide list, and the inclusive form that made it drift-proof
also made it over-reaching. The body-attribute scoping was written down in the same paragraph as
the escape hatch and never applied to the rules above it.*

### B2 — The smoke's central assertion is unachievable in the harness as written

*design M7 + structural B4.* Layer 2 says "assert the statement is visible — non-zero box, dark
computed text color **on a light background**". Goal 6's entire premise is that background
graphics are **off**; `page.emulateMedia({ media: 'print' })` does not enable background
painting and `page.pdf()` defaults `printBackground: false`. So the check either gets dropped in
implementation or passes for a reason the document never named. The checkable half is computed
`color` on the statement root and one headline figure.

**Fix:** assert computed text color only. If the background overrides are to be checked at all,
that is a second capture with `printBackground: true`, named as a different check.

### B3 — The Sequencing table's own definition makes its own cell false

*design M2 + structural B1.* The column is defined as "a test contacting a real backend or the
Yahoo provider". Slice 1 ships the smoke, which starts the Rust API and seeds through it. The
cell says `no`. Design adds a consequence structural did not: the smoke's `market_value` and
unrealized P&L come from a **live provider** (`currency_service.rs:267-299` is network-first with
a DB fallback returning `Decimal::ZERO` when unreachable), so the smoke is not reproducible and
prints zeros when Yahoo is down — the Goal 7 failure mode, in the test meant to catch failures.

**Fix:** split the cell per artifact (component test: no; local smoke: yes, not a gate), and pin
the fixture's closes by inserting `historical_prices` rows into the throwaway SQLite file
`run.sh` already owns — `get_price` reads a same-day row before touching the network
(`currency_service.rs:269-279`). Otherwise state that the smoke asserts structure and visibility
only and never a figure.

### B4 — Mandated on-page copy asserts an artifact that does not exist

*style B2 + structural M7.* The `rounding-note` string reads "The tax report CSV **carries** the
unrounded values." The document says three times that the tax report does not exist yet. An
adviser will ask for a file the owner cannot produce — a provenance defect of exactly the class
Goal 4 exists to prevent. Structural also caught the tense collision: the Problem section says
page and CSV "are guaranteed to differ" while Goal 3 says they are equal once rounded.

**Fix:** make the printed sentence self-contained — "Figures are rounded for display from
full-precision calculations." — and add the CSV sentence when the tax report ships.

### B5 — Layer 1's hook assertions contradict the empty branch

*structural B3 + design N13.* Layer 1 asserts the `rounding-note`, `splits-note` and
`label-realized` hooks "are each present", and separately that an `assets: []` fixture renders
`empty-note` **alone**. Unsatisfiable together. The document never says whether the rounding or
splits notes print on the empty branch — and the default MSW fixture already returns
`assets: []`, so the empty branch is the first path an implementer hits. Design's version of the
same point: a page reading "No holdings in this portfolio on {date}" followed by "Split-adjusted
history is as recorded on {date}" is nonsense.

**Fix:** scope hook-presence to the populated fixture; add an explicit negative list for the
empty fixture.

### B6 — The document's own count contradicts its normative table *(style B1)*

"Three strings are requirements… Goals 1, 3 and 7 each assert one." The table has **five** rows;
`splits-note` and `run-context` are asserted by Goal 4, which the sentence omits. This is the
section that declares which strings are normative; under-counting by two invites an implementer
to treat two of them as optional.

### B7 — "Conditional in one case" followed by "always" *(style B3)*

"The rounding note is **conditional in one case**: a base currency whose `Intl` rendering is
already exact… **still prints it**… Simpler to always print it." The paragraph announces a
condition and removes it. The rule is unconditional; the paragraph is a rejected idea presented
as a rule.

### B8 — The smoke's verification command is assigned to two owners *(structural B2)*

Layer 2 argues at length that the harness cannot start without `AUTH_MODE=disabled`, then says
the fix belongs to #45. Target File 6 nevertheless puts `run.sh` and `package.json` **inside this
slice**, and *Verification* lists `npm run e2e:statement` as a step. One file, two owners, and
the slice's own acceptance step dies on the day it is written. Either the one-line export lands
here, in a file this slice already edits, or the command is struck and marked "authored here,
first executed under #45".

---

## Majors, grouped

### Money and precision

* **M1 — the money section reintroduces the claim Goal 3 kills** (C2). Goal 3 spends three
  clauses rejecting "to the cent" *because of JPY at 0 dp*; two sections later the money section
  says "to 2 dp for a fiat base currency". An implementer who reads the Technical Plan and not
  the Goals writes a 2-dp assumption into the golden. Point at Goal 3 instead of restating it.
* **M2 — the ≤15-significant-digit scoping withdraws the claim it is attached to, and mislocates
  the conversion** (design M5, the deepest finding in the review). Two problems. First, the
  bullet argues the engine's products *do* exceed double precision — the review record it cites
  measures `roi` at **28 fractional digits** (`reviews/2026-11-05-sell-pnl-breakdown.md:22`) —
  then scopes every money claim to ≤15 significant digits, i.e. it excludes precisely the inputs
  the statement prints. That is a disclaimer, not a mitigation, and it contradicts Goal 3's
  headline. Second, and factually: the document places the double conversion at
  `formatCurrency` → `toNumber`, but the statement renders from `taxSummary` state, which is
  `normalizeTaxSummary` output, and that normalizer already converts every money field through
  `num()` (`decimal.ts:63-70`) with `TaxSummary` typed as `number` (`types.ts:89-95`). **The
  full-precision string never reaches the component at all.** C1's conclusion survives; its
  stated mechanism does not, and Goal 3's "the payload carries full precision, so the page says
  its figures are rounded from that precision" describes a wire property, not something the page
  holds.
  **Fix:** state the conversion site correctly; replace the carve-out with a bounded declared
  defect ("a printed figure may differ from the exact decimal rounding of the payload by at most
  one unit in the last displayed digit") and add one golden case whose payload exceeds 15
  significant digits, so the divergence is pinned rather than disclaimed.
* **M3 — `formatCurrency` has a third path the exhaustive claim omits** (style N7). Any crypto
  code outside `BTC`/`ETH` throws in `Intl` and falls to the `catch` at `formatters.ts:19-22`,
  rendering exactly 2 dp. Verified.

### Provenance

* **M4 — the page prints a generation date but no data date** (C8). The statement renders cached
  state and issues no request on open; `fetchPortfolioData` runs only on portfolio/strategy
  change or reload (`App.tsx:143-150`). A page printed the morning after a session opened
  yesterday carries today's date over yesterday's prices. Worse, `splits-note` asserts a
  **database fact** using a client render date, while the document itself says splits are "as
  recorded when the request ran". The payload carries no date field (`analytics.rs:93-101`), so
  `{date}` is necessarily a client clock and the document never says so. For an artifact whose
  thesis is saying what it does not know, this is a bigger provenance hole than the rounding
  declaration that got its own Goal. Structural adds the mechanical half: no hook, no copy row,
  no assertion, and `{portfolio name}` / `{base currency}` / `{date}` interpolations have no
  stated source or format.
* **M5 — the strategy the statement prints is unvalidated on this route, unlike its sibling**
  (design M6). `analytics.rs:59` is `strategy.unwrap_or_else(|| "FIFO".to_string())` — no
  normalization — and the engine's match falls through `_ =>` to FIFO for any unrecognized
  string. So `?strategy=average_cost` prints "average_cost" while the engine silently used FIFO.
  **The repo already fixed this exact hazard one route over**: `analytics.rs:218-222` normalizes
  with the comment *"so the modal can never name a matching method the engine did not use."*
  Verified. The run-context line is the artifact's only method claim and it rests on the one
  echo that is not normalized.

### Verification integrity

* **M6 — the MSW branch rationale is wrong** (C9). The document says the branch keeps the
  statement fixture and the sell-P&L fixture apart. The sell-P&L fixture is on a different route
  (`/api/transactions/:id/pnl`) and never touches tax-summary; the existing tax-summary consumers
  in tests pass the payload as a **prop**, not through MSW. The tests genuinely at risk are the
  four that `render(<App />)` — `App.test.tsx`, `AppLedger.test.tsx`, `AppSignOut.test.tsx`,
  `AppAuthError.test.tsx` — all driving portfolio **id 1**, whose `Dashboard` is fed by the
  tax-summary payload. Pin the statement fixture to an unused id and say so.
* **M7 — layer 2 asserts a computed style on an element that will not exist** (structural M12).
  The auth alert renders from the `auth_error=auth_failed` query param; under the document's own
  `AUTH_MODE=disabled` prerequisite there is no auth failure and no element. The same paragraph
  argues that enumerating hidden selectors re-states a rule that cannot miss — then enumerates
  two.
* **M8 — "no hardcoded colors" is required and unverified** (structural M9). The stated
  mitigation is the smoke's text-color assertion, but the cited failure mode is a hardcoded
  **background** (`SellPnlModal.tsx:132`), which a text-color check cannot see. Alternative B's
  own scope note says the color-literal grep is the check worth adding. Either ship it or record
  the residual honestly.
* **M9 — assertions with no requirement behind them** (structural M8). Layer 1 forbids `select`
  and `svg`; Goal 2 says "no buttons, no inputs, no charts" and Target File 1 says
  `button`/`input` only. Layer 2 asserts no horizontal overflow, stated nowhere else. Per the
  standard a verification assertion *is* a requirement; unstated ones get deleted as
  over-specified.

### Structure and implementability

* **M10 — Target File 3 breaks `npm run typecheck`, a listed verification command** (structural
  M10). `types.ts:89-95` declares every field required; `PortfolioDetail.test.tsx` and
  `Dashboard.test.tsx` construct full `TaxSummary` literals. Adding two required fields breaks
  three files that Target Files does not list. Verified. Make them optional
  (`strategy?: string; threshold_days?: number`) or list the files.
* **M11 — the stale-summary mitigation is not implementable as scoped** (design N14). "Render
  only when the summary belongs to `selectedId`" requires a state-shape change;
  `normalizeTaxSummary` output carries no portfolio id. Target File 4 authorizes only "the fifth
  nav item and the tab branch". The alternative — clearing `setTaxSummary` at fetch start — also
  changes the sibling `portfolios` tab, which is outside the "moves no existing figure" framing.
  Pick one and say which.
* **M12 — two sections named "Verification"** (structural M4), one H3 holding the layers, one H2
  holding commands, neither pointing at the other. Goal 3's "the golden under Verification" is
  ambiguous as a result.
* **M13 — `## Target Files` skips a heading level and item 7 breaks the section's template**
  (structural M5). H2 → H4 with no H3; item 7 is prose duplicating Alternative B and is not a
  target file.
* **M14 — the asserted copy contains an HTML entity** (structural M6): `Realized P&amp;L …` in
  the cell a test must match against DOM text.
* **M15 — narrative leakage is still pervasive** (C10). Style counted **18** instances of
  review-history narration ("an earlier draft claimed", "a review found it", "recorded because",
  "the correction that") and **12** of honesty self-commentary, including "honest/honestly" used
  six times as a term of approval, plus **6** verbatim-duplicated aphorisms ("unverified into
  verified and therefore fine" appears twice; "cannot miss" twice; "teaches people to ignore
  gates/smoke" twice). The last compression pass removed some of this and left the rest. The
  single largest cut is Decided item 3: nine lines of correction history for a decision whose
  operational content is one sentence.

### Citation drift — the document's own standard, unmet

Design re-derived every `file:line` and found the document violates its own rule that a citation
must point at something real. Controller confirmed each:

| Document says | Reality |
| :-- | :-- |
| `analytics.rs:99` returns `currency` (×2) | `:100`. `:99` is `threshold_days` |
| `analytics.rs:137-140` — splits synced by the tax-summary handler | That is `get_portfolio_performance` (starts `:113`). In tax-summary it is **`:63-65`** |
| `tax_engine.rs:396` — engine supports FIFO/LIFO/HYBRID | `:396` is the `hybrid_threshold_days` parameter; the match is `:441-463` |
| `App.tsx:385-396` nav items · `:359-376` auth alert · `:458` header-row | `:386-398` · `:360-378` · `:459` |
| `SellPnlModal.tsx:132` inline background · `Dashboard.tsx:40-45` headline | `:131` · `:38-44` |

**Fix:** cite by symbol or selector where the line will drift anyway (`the "currency" key in the
tax-summary response`), and re-derive the rest at final pass. Three review rounds have now each
found off-by-one drift, which means hand-counted line numbers are the wrong instrument.

---

## Nits (consolidated)

Style: "the the" duplicated word; "a decimal rounder right about scale" dropped words; "the easy
third" with no first or second; "sits beside nothing" unclear; "belt-and-braces"; three
unexplained portfolio sizes (40-row, 40-row, 50 holdings); the `run-context` cell hides a leading
space inside backticks; the splits copy is written verbatim in two places; British/American mix
(labelled/parameterised vs normalizeTaxSummary); "half-cent value" is USD jargon in a
EUR-denominated project and is wrong for JPY and BTC; "poor man's deep link".

Structural: broken-by-convention pointers to bold paragraphs rather than headings ("see the
scoping note below", "the six rules above"); Goals mix outcomes with mechanisms (5 and 6 are
constraints); "ships unverified by design" overclaims since layer 1 does verify structure and the
golden; the run-context string drifts from its own cited precedent (both quotes include a
`realized ·` segment the hook copy lacks); the smoke never says how it reaches the statement tab;
the parent design's line 89 still claims the statement prints per-asset price-date provenance,
which this document's Non-Goals refuses; the manual check "match the dashboard" is weaker than
the golden because `Dashboard.tsx:39-41` falls back to `metrics.*` on falsy; Goal 2 states A4 as
unqualified while step 1 concedes `@page` is global; Alternative F attributes a rejection to an
"owner decision" with no link, which the standard forbids; Decided item 3 is a correction record
mislabelled as a decision and has no inbound referent.

Design: Goal 5's OpenAPI justification is wrong twice — the parent is unshipped and cannot cause
anything in the committed spec, and the committed spec does have a 200 body entry, with an empty
schema (`docs/openapi/openapi.json:326-334`); `.data-table tr:hover td` cannot fire in print, and
`.badge-crypto`'s **color** is variable-driven, so only its background is hardcoded.

---

## Net

B1 alone is a shipped-behaviour regression on three of four tabs, and it came from the Stage 5 fix
that was supposed to be drift-proof. B2/B3/B4/B5 are all the same failure from different angles:
**the document specifies checks that cannot be executed as written**, which is the exact defect it
invokes to reject the grep gate in Alternative B. M2 says the money story I rewrote twice still
has the wrong mechanism in it.

The accretion is no longer deniable. Six sections restate the rounding rule and five restate the
empty-portfolio currency rule, and that is what produced C2, C3, C6 and M15. Stage 8 should be
expected to recommend splitting or cutting, and the structural finding that the document now
contradicts the parent design's line 89 belongs in whichever document gets corrected.
