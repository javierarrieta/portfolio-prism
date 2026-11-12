# Stage 6 — Comprehension Test

*   **Target:** `docs/superpowers/specs/2026-11-09-portfolio-statement-print.md` (lines 1-end)
*   **Reviewer:** `egm-comprehension` (naive fresh-reader persona)
*   **Mode:** REVIEW, no document standard, **no GoldfishContext** — per the skill, Stage 6
    reads the document alone, because the test is self-containment.
*   **Result:** 1 blocker, 12 majors, 5 nits.

The paraphrase below is the reviewer's, reproduced verbatim. The author compares it with the
intent; a wrong paraphrase is itself a finding. This one is accurate, which is the useful
outcome — the document's *shape* survives a cold read. What fails is that three of its
correctness claims are stated inconsistently enough that a cold reader has to go read the Rust
handler to work out which statement was meant.

---

## What this proposes

A new **fifth tab** (`statement`) in the existing React SPA that renders a printable,
one-portfolio statement: cumulative realized P&L since inception, unrealized P&L as of today,
portfolio value, plus one row per holding, a provenance block (portfolio name, base currency,
the server-echoed `strategy`/`threshold_days`, generation date, a splits caveat) and an
explicit on-page line declaring that the printed figures are rounded from the payload's full
precision. It takes **no date**, makes **no backend change**, and produces **no server-side
PDF** — it re-renders the `tax-summary` payload the app already fetches, and the user prints to
PDF from the browser. Most of the engineering is a new `frontend/src/print.css` that inverts the
dark palette by overriding CSS variables (without depending on the browser's "background
graphics" setting) and hides the app shell with two allowlist-style selectors. Verification is
a Vitest structural + golden test, a Playwright smoke that is explicitly a **local step, not a
CI gate** (pending issue #45), and a manual print preview added to `docs/auth-testing.md` §5.

## How the existing system works, per this document

The app is a Vite React SPA with **no router**: tabs are `useState(DEFAULT_TAB)` in `App.tsx`
with four `<li className="nav-item">` entries, and a query string is already read and rewritten
for auth purposes. Data comes from `/api/portfolios/<id>/tax-summary`, which requires an
authenticated user (so an adviser cannot reach it), and returns full-precision decimal strings
plus echoed `strategy`, `threshold_days` and `currency` — except in the empty-portfolio branch,
which returns zeros and none of the three. The frontend passes that payload through
`normalizeTaxSummary`, which coerces values to numbers and keeps only `currency`, and renders
money via `formatters.ts` (2 dp fiat, up to 8 dp for `BTC`/`ETH`), so displayed figures are
quantised at display and are guaranteed to differ from the full-precision CSV. Styling is
variable-driven dark glassmorphism in `index.css`; there is no `@media print` rule anywhere in
`frontend/src/`, CI has no browser job, and CI byte-diffs the committed OpenAPI document.

*(Reviewer spot-checked the load-bearing citations against source: `analytics.rs:45-51` and
`:95-100`, `formatters.ts:4,10-11`, `decimal.ts:20-23,63`, the `handlers.ts` tax-summary mock
returning `total_realized_pnl: 0` as a number, `App.tsx` auth alert / sidebar / nav /
`header-row` / `activeTab === 'portfolios' && taxSummary` guard, `e2e/run.sh` hardcoding
`node e2e/sell-pnl.smoke.mjs`, `ci.yml` OpenAPI byte-diff, `auth-testing.md` §5 being the
browser walkthrough. All resolve as described.)*

---

## Findings

### 1. Blocker — the currency source is stated three different ways

Location: "Money: a display artifact" → *What the payload must supply*; Target Files §4; Risks
row 4 ("takes currency from the payload, not from `portfolio?.currency || 'USD'`").

The Money section says the statement "reads `strategy`, `threshold_days` and **`currency`** from
the payload"; Target Files §4 says "Base currency for the header comes from the portfolio
object, **not** from the payload"; Alternative D's risk row says the statement "takes currency
from the payload". As a fresh reader I cannot tell which value formats the money and which
prints in the header, and the two can disagree (empty portfolio: payload has no `currency`,
normalizer defaults `USD`, portfolio object says `EUR`). The resolution is actually available —
`analytics.rs:99` returns `portfolio.base_currency`, so the two agree whenever money exists to
format — but the document never says that, so the reader has to go read the Rust handler to
discharge an apparent contradiction in the document's central correctness claim. One sentence
("money formats with the payload `currency`; the header prints the portfolio object's base
currency; they can only differ when there are no holdings, and the empty branch prints no money")
closes it.

### 2. Major — the `[data-no-print]` rule is referenced but never specified, and its stated consumer contradicts the plan

Location: "Palette inversion", step 2 final paragraph; "Surface: a fifth tab"; Target Files §1.

The stylesheet is said to do "five things, in this order", and none of the five is a
`[data-no-print] { display: none }` rule — yet step 2 says "`[data-no-print]` is still
honoured". Meanwhile the Print control is defined as being **outside** the printed subtree,
which means rule 2 (`.main-content > *:not([data-print="statement"])`) already hides it;
`[data-no-print]` is only needed for things opting out from **inside** the printed subtree. The
parenthetical "for the Print control and anything else that opts out from inside the printed
subtree" conflates two different DOM locations. An implementer cannot tell whether a sixth rule
exists or what it targets.

### 3. Major — an unstated DOM precondition for the hide rules: the statement root must be a direct child of `.main-content`

Location: "Palette inversion", step 2.

Both rules use the child combinator `>`. If the statement is wrapped in any container (a
`<section className="statement">` inside a layout div, which is the natural thing to write),
`.main-content > *:not([data-print="statement"])` hides the wrapper and the printed page comes
out blank — a failure mode the smoke would catch only because it asserts visibility, and only if
someone runs it locally (finding 9). The document names the selectors "on purpose" but never
states the two structural requirements that make them work: the statement root carries
`data-print="statement"` **and** is a direct child of `.main-content`, and the class names
`.app-container`/`.main-content` are load-bearing.

### 4. Major — the three headline figures are never mapped to payload fields, so Goal 5 ("no backend change") is not verifiable from the document

Location: Goal 1, Goal 3, "What the payload must supply", Target Files §1.

The run-context fields are named with precision (`strategy`, `threshold_days`, `currency`,
`analytics.rs:98-100`), but "cumulative realized P&L", "unrealized P&L" and "portfolio value"
are never given as payload keys. The only field name that appears anywhere is
`total_realized_pnl`, and that in a discussion of a mock being wrong. Since the document's
headline claim is that the payload already carries everything needed, the three keys
(`total_portfolio_value`, `total_realized_pnl`, `total_unrealized_pnl`) should be listed next to
the run-context ones; the same list would also settle finding 1 and make the "two added fields"
count checkable.

### 5. Major — "the standing caveat that splits are as recorded at that moment" is undefined

Location: Goal 4.

Three unknowns in one clause: what "splits" are (stock splits? split transactions? the code
auto-detects and persists them inside the tax-summary handler, but the document never says so),
who "records" them, and what "standing" refers to — it implies caveat text that exists somewhere
else (the parent design?) and is not quoted. Goal 4 is a requirement that specific copy appear
on a printed page, and the copy is the one thing an implementer cannot derive from this
document. Same class of gap as the rounding line and the "cumulative since inception" label: see
finding 6.

### 6. Major — three on-page strings are mandated as requirements, but no text and no hook is given, while the tests assert their presence

Location: Goal 1 ("cumulative since inception" is on the page, not implied), Goal 3 ("prints a
visible line saying its figures are rounded"), Verification layer 1 ("the rounding line is
present"; "all three headline figures appear").

The document is emphatic that assertions must be specific rather than left to implementation
time (that is its stated reason for naming selectors and smoke assertions), yet the only
user-visible copy in the slice is left unwritten. "The rounding line is present" is not an
assertion until either the literal string or a stable hook (e.g.
`data-print="rounding-note"`) is specified. This is the one place where the document's own
standard for specificity is not met.

### 7. Major — the empty-portfolio behavior is specified only in a Risks row

Location: Risks, last row ("the empty case prints an explicit 'no holdings' line rather than
three zeros").

A user-visible branch appears nowhere in the Overview, Goals, Technical Plan, Target Files §1
scope, or Verification layer 1. It also half-contradicts Goal 4, which says the run-context line
is *omitted* on that branch but says nothing about a "no holdings" line. An implementer working
from the Technical Plan will not ship it, and no shipped test covers it.

### 8. Major — the document gives two different reasons for the same decision (smoke is not a CI gate)

Location: Risks row 1 ("The reason is a **workstation fact**, not a CI law") vs Verification
layer 2 ("The reason is a **choice**, not an impossibility … the job costs a full cargo build
and needs seeded data").

Verification layer 2 explicitly corrects the "workstation fact" framing and then replaces it
with a CI-cost framing; the Risks table still carries the corrected-away version, and the
Verification section separately says the *local* run needs a chromium binary and fonts. A
reader cannot tell which reason the author actually holds. This matters more than usual here
because "we are being honest about the gap" is the document's recurring rhetorical claim.

### 9. Major — the local smoke's prerequisites are unstated, including the auth path the document itself identifies as the core obstacle

Location: Verification layer 2; Verification (`npm run e2e:statement`).

The Problem section's argument that the artifact must leave the app rests on "Every `/api` route
takes `AuthedUser` plus a `require*` call, and `apiFetch.ts:22-26` sends an unauthenticated
caller to `/auth/login`". The proposed smoke drives a real browser at a real backend started by
`run.sh` — and the document never says how that browser passes the auth guard, nor how the
"two-row fixture" portfolio is created (seeded via REST? via the UI? which strategy parameter,
since the printed run context depends on it). The existing sell-P&L smoke seeds its own fixture
through the REST API, so the answer exists in code, but the document's self-containment standard
is that such prerequisites be stated, especially the one it uses as a premise three sections
earlier.

### 10. Major — "the parent design" is an ambiguous referent, and two cross-document dependencies are unresolvable from this file

Location: header (`Depends on` vs `Related`); Non-Goals ("Decided in the parent design"); Goal 5
("the parent design already rewrites this handler's response"); Alternative E (a direct
quotation, "served where the exact decimals live"); Verification cross-check ("its own date
semantics are still open in the parent design"); Risks row 3 (`reviews/2026-11-05-…:18`).

The header names two documents: the pricing-layer spec as **Depends on** and the tax-report spec
as **Related**. "The parent design" is then used six times and plainly means the tax-report
spec, but nothing says so — a first read plausibly maps "parent" onto the declared dependency.
Consequences: Alternative E's rejection rests on a **quoted** commitment (`?format=csv`) with no
path or anchor; the CSV full-precision claim cites an elided path (`reviews/2026-11-05-…:18`)
that cannot be opened; and the cross-check is deferred to a document whose open question is
described only as "still open". Also, `Origin` cites a **directory**
(`docs/superpowers/reviews/2026-11-09-tax-report/`) and an undefined "stage 8 scope judgement".

### 11. Major — project-specific strategy vocabulary is used as a printing rule without definition: `HYBRID`, `FIFO`, `threshold_days`

Location: Goal 4; Decided item 3.

`FIFO` is a general cost-basis method, but `HYBRID` is this project's term and is never
explained — the reader is not told what it hybridizes or why a day threshold is meaningful only
for it, yet Goal 4 makes a user-visible decision ("prints only for `HYBRID`") depend on exactly
that. `threshold_days` is likewise never defined (what is measured against the threshold?), and
its meaning is what the printed line "realized · FIFO (threshold 30d)" is supposed to convey to
an adviser. A one-line definition plus a pointer to where the strategy enum lives would carry
Goal 4 from "trust me" to "checkable".

### 12. Major — Goal 6's guarantee is not visibly delivered by the plan as written

Location: Goal 6 vs "Palette inversion" step 4 vs The Problem.

The Problem states "browsers do not print background colors unless the user turns on background
graphics". Step 4 then says the inversion overrides `--bg-dark`, `--bg-card`, `--border-color` —
i.e. backgrounds and borders, the very properties that may not be painted. Goal 6 asserts the
result "must not rely on background graphics being ticked", and the sentence "Text becomes dark
because the stylesheet says so" gestures at the reason, but the plan never states the mechanism:
legibility is carried entirely by the **text color** override, and the background overrides are
belt-and-braces for users who do enable background graphics. A fresh reader is left with "you
set the background to white, but you also said backgrounds don't print — so which is it?" One
sentence, and step 4 should say which of the six variables are load-bearing and which are
cosmetic.

### 13. Major — "to the cent for a fiat base currency" is false for zero-decimal currencies, by the document's own argument

Location: Goal 3; "Money: a display artifact".

Goal 3 rejects the phrase "rounded to the cent" as "false on a BTC page" and then substitutes
"to the cent for a **fiat** base currency". `formatCurrency` uses `Intl.NumberFormat` with the
currency code, so JPY, KRW, CHF-style zero-decimal codes render at 0 dp — the same defect, one
category over. The precise claim is "whatever `formatters.ts`/`Intl` renders for that code",
which the document already makes correctly for crypto.

### 14. Nit — undefined project/process terms and uncited project facts

Locations and terms: `Author: … with EGM review support` and `Origin: … EGM review … stage 8
scope judgement` (EGM and the stage numbering are never expanded); "issue #45" (no tracker/repo
link, cited four times and load-bearing for the verification story); "the one slice of **this
programme**" (the programme is never enumerated); "the app runs in a **256 Mi single-replica
pod**" (Alternative A's decisive con, no citation, unlike nearly every other factual claim in
the document); "the owner's portfolios are **EUR-denominated**" as mitigation for A4-on-Letter
(EUR denomination does not imply A4 paper); "the committed OpenAPI body is empty" (Goal 5 —
which body? the reader must open `analytics.rs` to parse the sentence);
"`backend_rust/scripts/check_access_gate.sh` … with its silent-no-match failure mode solved"
(Alternative B's precedent — neither "the access gate" nor the failure mode is explained, so the
analogy cannot be evaluated); "the **splits**" (finding 5); "`price_date_used`" (Non-Goals — a
field from the pricing-layer design, used to justify a non-goal, undefined here).

### 15. Nit — mock terminology and line numbers are inconsistent

Location: Verification layer 1 ("the mock at `handlers.ts:121` is **branched**, not duplicated")
vs Target Files §5 ("the mock at `handlers.ts:122`").

Off-by-one for the same handler. More substantively, "branched, not duplicated" is unexplained:
branched on what — portfolio id, query string, strategy? — and "the handler-ordering comment at
`handlers.ts:130-131` still applies" points at a code comment the document does not paraphrase,
so a reader cannot tell whether the statement mock must be registered above or below the
existing one.

### 16. Nit — the Sequencing table's columns are undefined jargon, and one appears contradicted by layer 1

Location: Sequencing table ("Reaches network in tests? **no**"; "Moves an existing figure? no").

Layer 1 uses `test/mocks/handlers.ts`, i.e. an intercepted network layer, so "reaches network"
needs to mean "reaches a real backend" rather than "performs a fetch". "Moves an existing figure"
is used as a term of art throughout (Goal 5, Target Files §3) without a definition; it is
guessable but it is the phrase the whole "no regression" claim rests on.

### 17. Nit — the non-variable background list includes an element the hide rules already remove

Location: "Scoping the inversion" — "Some backgrounds are not variables and will not flip:
`.sidebar` is `rgba(10, 15, 28, 0.95)`".

`.sidebar` is hidden by rule 1, so it cannot print badly regardless of whether its background
flips. Mixing it into the list with genuinely visible cases (`.badge-crypto`,
`.data-table tr:hover td`) makes the residual-risk set look larger and less specific than it is.

### 18. Nit — "a refresh in flight cannot print zeros" rests on unstated state behavior

Location: Target Files §4.

The proposed guard mirrors `activeTab === 'portfolios' && taxSummary`, which only helps if
`taxSummary` is cleared when a fetch starts. If a failed or in-flight fetch leaves the previous
portfolio's payload in state, the printed page can carry the wrong portfolio's numbers under the
right portfolio name — the one failure mode a printed artifact cannot survive quietly. Worth one
sentence stating what happens to `taxSummary` on fetch start and on error (the Risks table
covers "empty or failing" only for the empty case).
