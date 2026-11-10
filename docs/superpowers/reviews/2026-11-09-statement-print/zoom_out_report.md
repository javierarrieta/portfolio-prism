# Stage 5 — Initial Zoom Out Review

*   **Target:** `docs/superpowers/specs/2026-11-09-portfolio-statement-print.md` (330 lines, whole document)
*   **Reviewers:** `egm-zoom-out` (Skeptical Architect), EGM Inventive Catalyst (persona file, dispatched via general subagent)
*   **GoldfishContext:** `2026-11-09-pricing-layer-truthfulness.md`, `2026-11-09-tax-report-and-statement-design.md`, `AGENTS.md`
*   **Preset:** Feature spec only (Stages 5-8). No `## Detailed Implementation Plan` header exists, so `FeatureRange = 1..End`, `DetailedRange = N/A`.

Both reviewers were told to verify the document's factual claims against the tree rather than
trust them. Both did, and both independently landed on the same Blocker, which is the strongest
signal in this report. The controller re-verified every Blocker-level claim directly; those
checks are marked below.

---

## Blockers

### B1 — The money-fidelity mechanism cannot deliver Goal 3. `formatCurrency` throws the string away one layer below the boundary it was preserved to protect.

*Raised by both reviewers independently.*

**Verified:** `frontend/src/utils/formatters.ts:3-4` is
`formatCurrency = (value: string | number | null | undefined, currencyCode) => { const v = toNumber(value);`
and `toNumber` is `Number(v)` (`decimal.ts:20-23`).

The document identifies the double conversion correctly, then protects the wrong boundary. It
mandates a string-preserving normalizer (file 3) and simultaneously mandates that every money
value be "the payload's decimal string rendered **through `formatters.ts`**" (Goal 3). Those
two requirements cancel: `formatCurrency("1234.565")` and `formatCurrency(1234.565)` produce
byte-identical output. `normalizeTaxSummaryStrings` changes nothing observable on the printed
page.

Two consequences the document does not carry:

1.  **Goal 3's equality is false by construction, and not because of doubles.** The tax report
    CSV is written at full precision by standing repo decision
    (`docs/superpowers/reviews/2026-11-05-sell-pnl-breakdown.md:18`, "full precision, no
    quantisation"; per-field rounding called "actively harmful" at `:50`), and the engine
    multiplies by the FX rate with no `set_scale` anywhere (`tax_engine.rs:418,422,433`).
    `formatCurrency` quantises to 2 dp via `Intl`. So a verbatim `1234.56500000000000000000`
    is never the same decimal as a 2-dp rendering, string normalizer or not. The cross-check
    the document schedules at its Verification section will fail on exactly the EUR/FX
    portfolio the A4 risk row says is the target.
2.  **The guard test cannot fail.** The Inventive Catalyst ran 20,000 random 2-dp and 20,000
    random 3-dp values up to 10^7 through `Number()` then `Intl.NumberFormat('en-US',
    {style:'currency'})` against exact decimal half-up rounding and found **zero mismatches**.
    The double only bites near 2^53. So the document's own central test — "the two normalizers
    agree to the cent" — is the same failure mode the document invokes to delete Alternative B:
    a check that passes while the thing it checks is broken.

**Correction to the finding that motivated this document.** The claim reported to the owner was
"the frontend throws away the decimals before anything renders them, so a statement can differ
from the CSV by a cent." The *mechanism* was misdiagnosed. The double is not what bites at
realistic magnitudes; **2-dp display quantisation against a full-precision CSV is**. The defect
is real, the named cause is wrong, and the fix the document chose treats the wrong cause.

**Resolution — the document must pick one:**

*   **(a) Declare the statement a display artifact.** Quantised once at render, by design,
    stated on the page ("figures rounded to the cent; the CSV carries full precision").
    Delete `normalizeTaxSummaryStrings`, the string-typed mirror types, the second App state,
    and the agree-to-the-cent test. Restate Goal 3 as "same payload value, quantised once at
    display." Deletes target file 3 and half of files 4 and 5.
*   **(b) Require exactness.** Then the honest fix is one string-preserving money formatter in
    `formatters.ts` — one function, reused, which is also what Alternative C gestures at — and
    the statement reads the payload's own strings through it. One function replaces a mirror
    type, a second normalizer, a second state slot, and a drift test. Liability: a
    hand-written decimal rounder must be right about scale, sign, `-0` and exponent input, and
    if it is a branch inside `formatCurrency` it changes what existing screens print for >2-dp
    values, which collides with "moves no existing figure." Scope it to the statement. The test
    becomes a golden — "the printed text equals this exact string" — not a cross-normalizer
    equality.

### B2 — The document states three times that the payload does not echo `strategy` or `threshold_days`. It echoes both.

**Verified:** `backend_rust/src/api_routes/analytics.rs:97-99` returns
`"strategy": strategy_val, "threshold_days": threshold_val, "currency": portfolio.base_currency`.
The route is a literal pass-through to `calculate_lots`, so the sent value *is* the value used.

This is a false statement of fact in the document, and three things rest on it: Decided item 3
(a decision, a labelling requirement, and a retirement condition for something already true),
the risk row "The strategy line asserts more than the payload confirms", and Goal 4's hedge
that the strategy must be labelled "as the strategy in use, because the payload does not
confirm them". All three are void. Printing a hedged label for a server-confirmed fact
undersells the truth, which is the opposite of the intent.

The only real gap is that `normalizeTaxSummary` drops those two fields (`decimal.ts:64-68`
keeps `currency` only) — one field-pick, not a decision. There is existing wording to reuse at
`SellPnlModal.tsx:210` (`realized · FIFO (threshold 30d)`).

---

## Majors

### M1 — The competing surface is missing from Alternatives: a print mode over the view that already renders this payload

*Raised by both reviewers.*

`PortfolioDetail.tsx:229-350` already renders a holdings table from `taxSummary.assets` —
symbol, type, shares, avg cost, price, market value, realized P&L, unrealized P&L, ROI — which
is Decided item 1's row set plus an Actions column and expanders. `Dashboard.tsx:39-44` already
renders the three headline figures from the same payload. `PortfolioStatement.tsx` is therefore
a **third renderer of one payload**, and the document half-admits it: "the page without the
print stylesheet is a dashboard with a different heading."

The document's central anxiety is two representations of one payload drifting — that is why the
agree-to-the-cent test exists — and it never applies that argument one layer up, where it cuts
hardest. The two tables will drift in column set, in currency source (the statement takes
currency from the portfolio object; `PortfolioDetail.tsx:260` already does
`portfolio?.currency || 'USD'`), and in sign and rounding conventions.

The counter-argument is real and should be recorded: the existing table is exactly what Goal 2
forbids — buttons, an Actions column, inline `var(--color-success)` colors
(`PortfolioDetail.tsx:287-295`) — so a print mode over it cannot satisfy Goal 2 or Goal 3
without the same work. A dedicated leaf component is defensible. What the document must do is
**record that it chose a second renderer over a print mode**, and say what keeps the two tables
honest with each other.

### M2 — The hide list is an allowlist, and it already misses an element

**Verified:** the auth-failed alert at `App.tsx:359-376` is `position: fixed`, full-width,
`background: var(--danger-color, #b3261e)`, `color: #fff` — outside both `.sidebar` and
`.header-row`, and a fixed-position element repeats on every printed page.

Since the statement root already carries `data-print="statement"`, invert the polarity: hide
the shell, show only the statement subtree. One rule, drift-proof, and it satisfies Goals 2 and
6 more strongly than an enumerated list. The document's argument for naming selectors (so a CI
assertion cannot grep a selector nobody wrote) is about honesty of the assertion, not
minimality of the rule; the inclusive rule serves both, and the smoke then asserts *the
statement is visible* rather than *a hand-maintained list is complete*.

### M3 — Verification layer 2: the two reviewers disagree, and the disagreement is the finding

*Zoom-Out:* cut it. No CI browser job exists, issue #45 is open, the slice ships unverified by
design, so layer 2 buys a new smoke file, a change to the only working smoke harness
(`run.sh:112`), a new npm script, and a Decided item to bound its own page-count claim — in
exchange for a check no gate runs, duplicating the manual preview already scheduled in
`auth-testing.md` §5. Move the named `emulateMedia`/computed-color assertions into issue #45 so
they land with the job that will execute them.

*Inventive Catalyst:* "ships unverified by design" is a local-workstation decision applied to
CI, which runs `ubuntu-latest`, where `npx playwright install chromium` is the ordinary
one-liner and `page.emulateMedia`/`page.pdf()` need no display server. The remedy the document
names — `nix-shell -p chromium -p liberation_ttf` — is the author's machine, not CI. Land the
smoke as a **non-blocking** CI job now (`continue-on-error: true`), flipped to blocking when #45
lands.

*Counter to the Catalyst:* `e2e/run.sh:3-16` is not browser-only — it compiles and starts the
Rust API against a throwaway SQLite file and starts Vite — so the job costs a cargo build and
needs seeded data, and a job allowed to fail is decoration unless someone reads it.

**Owner decision required.** Either way, the sentence "ships unverified by design" needs to be
re-examined: it was inherited from the parent design's constraint set and it is weaker than it
looks once the runner is checked.

### M4 — The adviser cannot log in, and the document never says so

Every `/api` route goes through `AuthedUser` plus a `require*` call, and `apiFetch.ts:25`
bounces an unauthenticated caller to `/auth/login`. An adviser has no account. So the artifact
necessarily leaves the app, and the document fixes on "a browser-printed PDF of the SPA"
without examining the cheaper exit: a copy-to-clipboard plain-text block — run context, two
headline figures, holdings rows — pasted into an email body. No print stylesheet, no palette
inversion, no `@page`, no A4/Letter question, no page-count question, no CI gap, works from a
phone, and the only variant that survives an adviser replying inline. The app already ships text
artifacts (`utils/download.ts`, `utils/sellPnlCsv.ts`).

Objection: a text block cannot carry a 40-row table legibly and is not something an adviser
files; Decided item 1 is an explicit owner decision that the artifact is a document. The middle
is to ship the copy block *with* the print view (~20 lines, no CSS), or to record that it was
considered and rejected. Leaving the adviser's access situation unaddressed anywhere in the
document is not an option.

### M5 — Alternative A rejects a server-generated *PDF*; the unexamined option is the same route serving server-rendered HTML

The parent design already commits to `?format=csv` on `tax-summary`, "served where the exact
decimals live", on the grounds that a total which must equal `tax-summary` to the decimal should
not be re-rendered through a double on its way to a file. That argument applies verbatim to the
statement. `?format=html` (or `?format=print`) on that same route — server-rendered HTML with a
small inline light stylesheet — dissolves B1, the linkability half of M6, M4, and the entire
Palette Inversion section at once, and gives a real URL the owner can bookmark and mail.

The objection is strong and probably wins: a templating dependency or hand-built HTML in Rust,
a second rendering technology in a repo with none today, the backend change Goal 5 refuses,
blocked on the parent design's route work and therefore on the pricing layer — destroying the
statement's one independence property — and it cannot reuse `formatters.ts`, so currency
grouping drifts from the rest of the app. **Reject it, but write it down.** Alternative A as
written rejects only the PDF variant, and a reader will otherwise conclude server rendering was
considered and rejected when it was not.

### M6 — "No router, therefore no URL" is a false dilemma: the app already reads and rewrites its own query string

`App.tsx:58` already does `new URLSearchParams(window.location.search).get('auth_error')`, and
`App.tsx:347-353` builds a new `URLSearchParams` and calls `history.replaceState` to strip it.
A query parameter is an existing convention in the exact file the document cites.
`?view=statement` read at mount to initialise `activeTab` delivers re-open, bookmark and link
with no router and no dependency, and makes the smoke reproducible
(`page.goto('/?view=statement')` instead of a click path).

Objection: it is not a real deep link — `selectedId` is state and is not in the URL either, so a
bookmarked `?view=statement` reopens on the default portfolio. It buys half of what it promises
and adds a second navigation mechanism. The honest resolution is to pick one: state plainly that
the statement is not linkable and delete the "no link to re-open a printed page" complaint as a
non-problem, or spend the query parameter and say what it carries.

### M7 — Goal 5's "no backend change" is a sequencing constraint presented as an architectural principle

The parent design already rewrites this handler's response — it replaces
`Json<serde_json::Value>` (which is why the committed OpenAPI body is empty) with a typed struct
and adds `from`/`to`. So "no new field on this endpoint" is not a standing boundary; it is "not
in the same change as the pricing layer." Echoing `strategy` and `threshold_days` is early
plumbing for the run context the parent design will carry anyway, and it retires B2's subject
matter properly. Counter: any response-shape change lands in `docs/openapi/openapi.json`, which
CI diffs byte-for-byte, so it cannot be a quiet two-field change, and the statement is the one
slice in the programme not blocked on the pricing layer. The refusal is defensible; it should
read as a sequencing choice, not a principle.

---

## Nits

*   **N1 — the drift guard tests the wrong property.** Both normalizers read the same object, so
    "agree to the cent on one fixture" is near-tautological. The real drift is a field present
    in one shape and absent from the other: assert key-set equality.
*   **N2 — the existing mock contradicts the payload contract.** `handlers.ts:122` returns
    `total_realized_pnl: 0` as a JSON **number**; the backend always sends strings
    (`analytics.rs:95-97`, `db_types.rs:31`). Branching the handler is not enough — the fixture
    must carry decimal strings or the string path is tested against a shape the server never
    emits.
*   **N3 — `frontend/src/styles/print.css` invents a one-file directory.** Existing stylesheets
    are flat (`src/index.css`, plus `src/App.css`, which is imported nowhere — dead file,
    separate cleanup). Use `src/print.css`.
*   **N4 — the cascade argument is about source order, not file boundaries.** A `@media print`
    block appended to `index.css` wins just as reliably, and puts the print palette next to the
    palette it inverts. The separate file is fine for organization; do not claim the split is
    what makes the cascade work.
*   **N5 — the print stylesheet is global; the document never says what Ctrl-P on the other four
    tabs produces afterwards.** `@page` has no scope. And "most of the app flips at once" is true
    for text and false for several backgrounds that are not variables — verified:
    `.sidebar { background: rgba(10, 15, 28, 0.95) }` (`index.css:101`), `.badge-crypto`
    (`:334`), `.data-table tr:hover td` (`:252`). Either state the intended behaviour for a print
    started from another tab, or scope the inversion under a body attribute the statement sets.
*   **N6 — null/loading state is unspecified.** `taxSummary` is null until the fetch resolves and
    is not re-set on failure; the sibling tab guards with `activeTab === 'portfolios' &&
    taxSummary` (`App.tsx:474`). Printing mid-refresh is the same bad output as the empty case
    with friendlier clothing.
*   **N7 — Alternative B's rejection is over-generalized.** The rejected gate greps a
    *stylesheet*; but the mitigation for the hardcoded-color risk is a convention, and
    conventions are what static gates enforce. A grep over `PortfolioStatement.tsx` for a color
    literal fails precisely when the invariant breaks, and the repo already runs this shape of
    gate with its silent-no-match failure mode solved (`check_access_gate.sh`). It is a different
    check from Alternative B and is rejected only by association.

---

## Net effect if B1(a), B2, M1, M3-cut and M2 are taken

Target files 3 and 6 disappear, file 4 loses its second state, Decided items 2 and 3 disappear,
two risk rows disappear, and the document shrinks to what it actually is: a print stylesheet, a
print affordance, and one honest decision about whether the printed page is a display artifact
or an exact one.

## What survived review unchanged

*   **Palette inversion via `:root` variable override.** The Catalyst checked the premise rather
    than accepting it — `body` reads `var(--bg-dark)`/`var(--text-primary)` (`index.css:41-43`),
    `.glass-card` reads `var(--bg-card)` (`:78`), `.data-table th` reads `var(--text-secondary)`
    (`:240`), and inline styles do use variables (`SellPnlModal.tsx:206`). The inventive
    alternative — render the statement into an isolated light-theme iframe so there is nothing to
    invert — is a real pattern but pays iframe stylesheet injection, an unusual React pattern and
    a changed jsdom surface to solve a problem the existing CSS architecture already mostly
    solves.
*   **The claims verified as stated:** no `@media print` anywhere in `frontend/src/`;
    `normalizeTaxSummary` runs every money field through `Number()`; the empty-portfolio branch
    returns no `currency` key (`analytics.rs:45-51`); `e2e/run.sh:112` hardcodes one smoke
    target; `ci.yml` has no browser job; the `App.tsx` and `handlers.ts` line references;
    `docs/auth-testing.md` §5 exists.
*   **Rejecting server-generated PDF** (Alternative A) — sound as far as it goes; see M5 for the
    variant it does not cover.
*   **Not adding a router** — one page does not justify it.
