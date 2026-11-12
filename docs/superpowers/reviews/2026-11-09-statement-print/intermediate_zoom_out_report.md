# Stage 8 — Intermediate Zoom Out Review

**Target:** `docs/superpowers/specs/2026-11-09-portfolio-statement-print.md` (718 lines, commit `d880f21`)
**Standard:** `zoom-out-standard.md`
**Reviewer:** `egm-zoom-out` (Skeptical Architect), fresh context
**Focus:** what the Comprehension (Stage 6) and Critic (Stage 7) stages added or changed, judged at high altitude
**GoldfishContext:** parent tax-report design, pricing-layer spec, `AGENTS.md`, prior review record, and the Stage 5/6/7 reports for this document

Controller re-verified every code claim below against the working tree. All confirmed.

---

## Verdict

The high-level architecture is sound and survived three fix waves without being corrupted: a fifth
tab over already-fetched state, a variable-override print stylesheet, no backend change, and a
display-artifact declaration instead of a fake-exactness mechanism. Every Stage 5–7 mitigation that
changed *what* the slice does was the right call and is now internally consistent.

What went wrong is narrower and specific. The fix waves added **three pieces of machinery that no
requirement consumes** — a dead `[data-no-print]` rule, a frontend assertion for an input the app
cannot send, and a smoke step that writes to a table no API route writes — plus one mechanism
(`body[data-print-scope]`) that is justified in substance but implemented in JS where the platform
already provides it in CSS. Separately, roughly 180 of 718 lines have dropped from feature-spec
altitude to implementation-plan altitude, and a large part of that drop was *caused* by review
findings demanding literal strings and literal assertions.

The root problem is still what is being solved. The drift is in altitude and in two verification
mechanisms, not in the product.

---

## Blockers

### B1 — The smoke's price-pinning has no write path, no consumer, and creates a coupling the document elsewhere denies

**Scope:** *Verification layers* layer 2, "no figure is asserted" (lines 417–422); *Sequencing* row 3
(line 466); *Target Files* 6 (lines 531–537).

This was added in the Stage 7 fix wave and it does not work.

- **No write path.** The read side is real — `get_price` returns a same-day `historical_prices` row
  before touching the network (`currency_service.rs:267-281`). The write side does not exist
  through the app: `grep -rn historical_prices backend_rust/src/api_routes/` returns nothing. The
  only writers are internal cache writes after a successful provider fetch
  (`currency_service.rs:397`, `:490`). So the smoke must write SQLite out-of-band, which the
  document never says, and which contradicts the harness's own stated principle
  (`frontend/e2e/README.md:10-11`: *"seeds its own fixture through the REST API"*). No prerequisite
  exists for it either: `run.sh:39-46` checks chromium, node, `playwright-core`, fonts and curl,
  and `frontend/package.json:31-49` declares no sqlite driver.
- **It buys nothing.** The same bullet says **no figure is asserted**, so a zero-price run cannot
  fail the smoke. The pinning mitigates a failure mode the assertions already tolerate.
- **It does not remove network dependence anyway.** Opening the statement runs
  `fetchPortfolioData`, which also calls `/performance` (`App.tsx:104`) →
  `get_historical_prices` (`stats_engine.rs:139`), and tax-summary runs `sync_splits_for_asset` per
  asset (`analytics.rs:63-65`). Two pinned closes stop none of that.
- **It creates a coupling the document denies.** Goal 5 calls the statement *"the one slice of this
  programme not blocked on the pricing layer"*, while the pricing layer's P2 is precisely the
  `historical_prices` dedupe + unique index + real upsert
  (`2026-11-09-pricing-layer-truthfulness.md:184`). A direct insert into that table makes this
  slice's smoke depend on a schema its own dependency is actively changing.

**Fix:** delete the pinning (419–422) and the Sequencing cell's *"reads prices from
`historical_prices` rows it inserts"* (466). Keep the honest sentence: the smoke asserts structure,
visibility and page count only; figures depend on the provider and are therefore not asserted. If
determinism is ever wanted it belongs to the pricing layer, which already needs seeded closes.

---

## Majors

### M1 — The print scope is a JS lifecycle for a predicate CSS already expresses

**Scope:** *Palette inversion* ¶295–300 and rule 2 (305–334); *Target Files* 1 (482), 2 (489).

Scoping the hide rules is correct — Stage 7 B1 was a real regression, and the mechanism is needed:
`.main-content`'s children are the header row (`App.tsx:458`) plus each tab's root, and nothing
else carries the attribute. The *chosen* mechanism is the heavier of the two available: a
`useEffect` mutating `document.body` on mount and unmount, in a codebase that touches
`document.body` exactly once and only to append a download anchor (`utils/download.ts:33`). CSS
`:has()` expresses the identical condition with no JS, no cleanup obligation, and no attribute that
can outlive the component:

```css
.app-container:has([data-print="statement"]) > *:not(.main-content) { display: none }
.main-content:has([data-print="statement"]) > *:not([data-print="statement"]) { display: none }
```

Both keep the child-combinator preconditions the document already documents. `:has()` is baseline
(Chrome 105 / Safari 15.4 / Firefox 121); the repo sets no browserslist override (verified: no
`browserslist` key in `frontend/package.json`). Neither layer 1 (jsdom applies no print CSS) nor
the non-gated smoke can test the JS attribute, so that half is unverified either way.

**Fix:** use `:has()`; delete *"Sets `body[data-print-scope="statement"]` on mount and clears it on
unmount"* (482) and the matching clause in Target Files 2. If the body attribute is kept, add one
sentence saying why `:has()` was rejected — right now the alternative is not considered.

### M2 — `[data-no-print]` is a specified interface with no user

**Scope:** rule 6 (346–347); mentions at 329 and 480.

Stage 7 C3 found the rule simultaneously used and unused. The fix kept the rule and added the
admission: *"Nothing uses it today; it is specified so a later addition can opt out without
restructuring the tree."* That is the form this standard rejects — an interface built for a
requirement nobody has, now costing three places in the spec and one permanent CSS rule. The only
known opt-out case (the Print control) is already handled by the sibling-position rule, which the
document says twice.

**Fix:** delete rule 6 and both mentions. Re-add it in the change that first needs it.

### M3 — The echoed-strategy caveat puts a backend fix's work in the frontend's spec, and tests an unreachable input

**Scope:** *What "splits" and the strategy line mean*, third bullet (270–280); layer-1 assertion
(383–384).

The underlying defect is real and verified: this route passes the query value through un-normalized
(`analytics.rs:59`, `strategy.unwrap_or_else(|| "FIFO".to_string())`) while the sibling route
normalizes it with the comment *"so the modal can never name a matching method the engine did not
use"* (`analytics.rs:216-222`). But the document concludes the fix belongs to the parent design
(278–279) — and then adds a *frontend* guard anyway: layer 1 asserts a strategy outside
`FIFO`/`LIFO`/`HYBRID` is not printed verbatim. The app cannot send such a value: strategy state is
initialised to `'FIFO'` (`App.tsx:39`) and only ever set from a three-option select
(`PortfolioDetail.tsx:179-181`). The assertion covers an unreachable input, in the wrong layer, for
a defect the parent owns.

**Fix:** delete the layer-1 assertion (383–384); compress 270–280 to what the page can claim plus a
pointer. Add the normalization as an explicit line in the parent design's Target File 6, which
already edits this handler.

### M4 — `taxSummaryFor` is wider than the root problem needs, and offers an option the repo already rejected

**Scope:** *Target Files* 4, "Stale payload — the implementable form" (506–513).

The root problem is correct and important: a printed page with portfolio B's numbers under portfolio
A's heading is the one unacceptable failure. But reshaping `taxSummary` into `{ id, data }` touches
state consumed by three tabs (`App.tsx:479` Dashboard, `:483` guard, `:486` PortfolioDetail), while
Target File 4's declared scope is "the fifth nav item and the tab branch".

The repo already has this exact pattern and already rejected the alternative the doc leaves open.
`App.tsx:68` is `useState<{ txId: number; portfolioId: number } | null>(null)` with the render guard
at `:518`, and the comment at `:66-67` states why it is *not* cleared from an effect: *"five places
change `selectedId`, and a reset that has to be remembered in each one is a reset that gets
forgotten."* The doc's *"or an equivalent clear-on-switch"* (512) re-opens a settled decision.

**Fix:** name the precedent (`App.tsx:68`, guard at `:518`), delete the clear-on-switch fork, and
prefer the minimal form — record the id beside `setTaxSummary` and guard only the statement branch,
leaving the two existing consumers untouched and the change inside the stated scope.

### M5 — The pricing-layer dependency is stated two contradictory ways

**Scope:** header (lines 6–10) vs *Goal 5* (~110).

The header says the statement *"Depends on"* the pricing layer because *"a statement that converts at
the wrong date is worse than no statement"*; Goal 5 says it is *"the one slice of this programme not
blocked on the pricing layer"*. Both cannot guide a sequencing decision. The truth is less dramatic:
for same-currency assets there is no FX at all, and for foreign-currency assets the pricing defect is
already live on the dashboard today (`2026-11-09-pricing-layer-truthfulness.md:24-27`:
*"Foreign-currency assets cannot be priced at all"*). The statement **inherits a pre-existing
defect** rather than depending on a fix — which is the claim that actually justifies shipping first.

**Fix:** restate the header as *"inherits the pricing layer's known FX defect; not blocked on it"*
and drop the correctness framing.

### M6 — The >15-significant-digit bound and its bespoke golden are evidenced from a ratio, not from money

**Scope:** *Money*, "The residual, bounded rather than disclaimed" (220–227); layer-1 golden
(371–378).

The bound is justified by citing the review record's measurement — but the value measured at 28
fractional digits is `roi` (`reviews/2026-11-05-sell-pnl-breakdown.md:22`), **a ratio, not a money
field**. The same record measures the money fields on the same fixture at two decimals:
`realized_pnl` `"2000.00"`, `proceeds` `"10400.00"`, `cost_basis` `"8400.00"` (`:22`). And the FX
multiplication named as the source of long decimals is the path the pricing-layer doc says currently
produces no figure at all for foreign-currency assets. So the document pins a golden for a money
shape that is neither measured nor currently reachable, and states a quantitative bound it has not
measured on the fields it bounds.

**Fix:** keep the on-page declaration and the 3-decimal golden (`1234.565`), which is real and
reachable. Drop the numeric bound and the >15-significant-digit fixture, or move both to the review
record as an unmeasured caveat.

### M7 — Altitude: roughly a quarter of the document is implementation-plan material

**Scope:** lines 203–235, 270–280, 389–399, 417–443, 454–467, 521–537, 577–586, 613–617, 702–718.

Concrete count of the weeds now on the page: 4 literal CSS rule bodies (302, 311–312, 341, 346);
6 verbatim copy strings plus a leading-space rule (159–165); two per-test assertion lists and a
negative list (379–387); an MSW registration-order explanation with an exact fixture id (389–399);
a shell env-export with its placement in a specific file (442, 536); an npm/nix command line
(703–711); two golden numeric literals (371–374); ~15 hand-counted backend line citations.

Some of this is legitimate — the copy table is a user-visible contract, and the argument that an
assertion needs a fixed string is correct. What is not legitimate for a feature spec is the harness
forensics (the 420-second `wait_for` budget, the cargo-build cost argument, the OIDC bail diagnosis
at 434–443) and the numeric-precision forensics (203–219). Those are decision records, not
requirements.

**Fix:** see Scope judgement. Target roughly 480–520 lines.

---

## Nits

- `data-at` invents a second date idiom. The app already has one: `Dashboard.tsx:114` renders
  "Data as of {client date}" from the same payload. And "ISO 8601 to the minute" is a new format in
  a codebase that uses `toLocaleDateString`/`toLocaleString` (`Dashboard.tsx:114`,
  `TransactionLedger.tsx:50`, `SellPnlModal.tsx:254`).
- The `data-print="…"` hook namespace duplicates the repo's existing test-hook convention,
  `data-testid`, which both test layers already use (`SellPnlModal.tsx:132,209`;
  `SellPnlModal.test.tsx:36`; `sell-pnl.smoke.mjs` `getByTestId('strategy-note')`). Keep `data-print`
  for the one CSS-load-bearing attribute; use `data-testid` for the six copy hooks.
- The statement fixture is pinned to "id 3" (392–394, 527–530), but `handlers.ts:60-63` already
  returns `id: 3` for a POST-created portfolio, so id 3 is not free in the mock's fiction — a future
  portfolio-creation test lands on the statement fixture.
- The `AUTH_MODE=disabled` fix is presented as a discovery; the repo already has the line and the
  rationale at `start.sh:21` and `AGENTS.md:19-21`. Cite the precedent instead of re-deriving the
  OIDC bail. (`run.sh` genuinely lacks it — verified, `run.sh:74-76` exports only
  `DATABASE_URL`/`BASE_URL`/`API_URL`; and there is no `backend_rust/.env`.)
- There is no shared e2e seeding helper to reuse — `seed()` is inline at `sell-pnl.smoke.mjs:36-59`.
  The design says it will seed "as `sell-pnl.smoke.mjs:37-57` already does", i.e. copy it. Since
  `run.sh` is being parameterised anyway, extract `e2e/seed.mjs` and let #45 inherit it, or state
  that duplication is accepted.
- *Sequencing* (454–467) duplicates the parent design's slice table
  (`2026-11-09-tax-report-and-statement-design.md:329-340`); one of the two should own it.
- Citation drift continues: the doc cites `.header-row` at `App.tsx:459`; it is `App.tsx:458`.
  Stage 7's remedy (cite by symbol) was not applied here.
- Carried from Stage 7 and still unfixed: the parent design's line 89 still says *"the statement
  prints the same run context in its header block"*, which this document's Non-Goals (129–131)
  refuses. The one-line correction belongs in the parent, not here.

---

## Scope judgement

The slice boundary is right — pure frontend over an existing payload, sequenced first, and the parent
design confirms it owns nothing of this
(`2026-11-09-tax-report-and-statement-design.md:332, 337-340, 816-824`). The problem is content
placement inside and around it.

**Keep here (unchanged):** 17–135 (Overview, Problem, Goals, Non-Goals), 138–186 (surface, copy
table, empty portfolio), 282–362 (palette inversion, minus rule 6), 469–543 (Target Files, trimmed),
545–559 (Risks), 560–576 (Decided 1–2), 587–701 (Alternatives A–G).

**Move to the parent design:**

- 270–280, the echoed-strategy caveat, and the normalization work itself → an explicit bullet in the
  parent's Target File 6, which already edits this handler. Delete the layer-1 assertion at 383–384.
- 129–131's refusal of per-asset price-date provenance is correct here; the parent's line 89 must be
  corrected there.

**Move to the pricing-layer doc, or delete:**

- 417–422 and the Sequencing cell at 466 (price pinning). If determinism is genuinely wanted, the
  pricing layer owns `historical_prices` uniqueness and already needs seeded closes. Otherwise it
  goes nowhere.
- Header lines 6–10: restate the dependency as "inherits a pre-existing defect", per M5.

**Move to issue #45 (and `frontend/e2e/README.md`, which already has a "Why it is not in CI" section
at `:73-77` that will otherwise drift):**

- 424–443: the CI-cost argument, the cargo-build/NixOS detail, the OIDC-bail forensics. Keep one
  sentence here: local step, not a gate; needs `AUTH_MODE=disabled`, which lands in `run.sh` with
  this slice.

**Move to the review record** (`docs/superpowers/reviews/2026-11-09-statement-print/`), which is the
repo's existing convention for exactly this (`2026-11-05-sell-pnl-breakdown.md`):

- 203–219 (where precision is actually lost, why not preserve strings), 220–227 (the bound),
  Alternative C's C1 forensics (622–632), Decided item 3 (577–586, already called a correction
  record by Stage 7), Alternative B's scope note (613–617).

**Out entirely:**

- Rule 6 `[data-no-print]` (346–347) and its mentions (329, 480).
- The clear-on-switch fork (511–512).
- The >15-significant-digit golden and the numeric bound (220–227, part of 371–378).
- The MSW branch and id pinning (389–399, and the `handlers.ts` half of Target File 5) — see Reuse
  findings; replace with a prop fixture.

---

## Reuse findings

Searched `frontend/src/`, `frontend/e2e/`, `backend_rust/src/`.

**Reinvented:**

- **The MSW handler branch is not needed.** Both existing tax-summary consumers are tested with a
  **prop fixture**, not through MSW: `PortfolioDetail.test.tsx:25-47` (`mockTaxSummary`), `:49-55`
  (`emptyTaxSummary`), rendered as a prop at `:58-63`; `Dashboard.test.tsx:38-42`. And the repo's
  existing convention for per-test payload variation is a runtime override —
  `server.use(http.get('/api/portfolios/1/transactions', …))` (`AppLedger.test.tsx:18-20`,
  `SellPnlModal.test.tsx:116`) — not branching the shared handler. Either existing mechanism replaces
  the entire id-3 branch, the decimal-string retrofit of the shared mock, and the MSW
  registration-order paragraph.
- **The stale-payload pattern already exists**: `App.tsx:68` (`{ txId, portfolioId }`) with the render
  guard at `:518` and the rationale comment at `:66-67`. Cite it and drop the clear-on-switch fork
  the comment already rejects.
- **A client-clock provenance line already exists**: `Dashboard.tsx:114`, "Data as of {today}",
  rendered from the same payload. The two-clock design is tightening an existing on-screen
  convention, not inventing provenance from nothing.
- **`AUTH_MODE=disabled` already exists as a documented convention**: `start.sh:21` with the dev-only
  rationale, and `AGENTS.md:19-21`.

**Searched and found nothing — the design is not reinventing these:**

- No print stylesheet or print-scope mechanism anywhere: `@media print|print-scope|data-print|data-no-print`
  → no matches in `frontend/`.
- No body-attribute or class-toggle precedent: `document.body|setAttribute|removeAttribute|classList`
  in `frontend/src` → one hit, `download.ts:33`.
- No shared e2e seeding helper: `frontend/e2e/` contains only `README.md`, `run.sh`,
  `sell-pnl.smoke.mjs`.
- No API route writes `historical_prices` (B1).

**Verified as stated** (load-bearing claims re-derived by the reviewer and confirmed by the
controller): `analytics.rs:45-51`, `:63-65`, `:95-100`, `:216-222`; `tax_engine.rs:645-648`;
`formatters.ts:4,10-11,19-22`; `decimal.ts:59-66`; `types.ts:89-95` plus the literals at
`PortfolioDetail.test.tsx:25-55,192-195` / `Dashboard.test.tsx:38-42`; `handlers.ts:121-122`;
`App.tsx:36,39,111,114,359-377,379,386-398,456,458,483`; `main.tsx:3`; `index.css:7,22,78,95,251,333-335`;
`openapi.json:326-334`; `ci.yml:28-37`; `auth/mod.rs:85,96-101,136-145`; `currency_service.rs:267-281`;
`sellPnlCsv.ts:61-66`; `auth-testing.md` §5 at `:212`; `reviews/2026-11-05-sell-pnl-breakdown.md:22`.

---

## Tally

| Severity | Count |
| :-- | :-- |
| Blocker | 1 |
| Major | 7 |
| Nit | 8 |

**Pattern across the findings:** every blocker and five of seven majors are machinery the review
process itself added. Stage 7 demanded determinism, a defensive guard, a bounded residual, an
opt-out hatch, and an implementable stale-state rule; each demand produced a mechanism, and each
mechanism is now either unreachable, untestable, or owned by a different document. This is the same
accretion mechanism Stage 7 named for prose, now visible in the design itself.
