# Stage 9 — Implementation Readiness Test

**Target:** `docs/superpowers/specs/2026-11-09-portfolio-statement-print.md` (624 lines, commit `71990a7`)
**Standard:** `implementation-readiness-standard.md` (+ `feature-spec-standard.md` for the style lens)
**Range:** `FeatureRange = 1..End` — the document has no separate implementation-plan section, so the whole spec is the range.
**Reviewers (3 concurrent):** `egm-implementation-readiness`, `egm-codebase-details`, `egm-style`. Raw output is in the run log; this is the deduplicated synthesis.

**Verdict: not implementable on a first attempt.** Stage 8's fix wave landed and was confirmed by two independent reviewers — `:has()` scoping is valid against the real DOM, the prop-fixture test is supported by the harness, the stale-payload precedent is real, the pricing-layer inheritance framing is not stale. What remains is a different gap than Stages 5–8 found: the document specifies *behaviour* well and *interfaces* poorly. A fresh engineer would have to invent the component's props, the nav label, the print palette's literal values, the table markup, and the smoke's PDF options.

## Tally

| Severity | Count | Raised by 2+ reviewers |
| :-- | :-- | :-- |
| Blocker | 5 | 4 |
| Major | 16 | 7 |
| Nit | ~25 | 4 |

---

## Blockers

### RB1 — The component's prop signature is undefined, and the spec forbids the prop `data-at` needs

*Readiness B1 + Style M2. Independently raised.*

The spec says only "Takes the summary as a prop, takes no date parameter" (Target Files 1). The required copy needs four inputs and the payload supplies one of them cleanly:

- `run-context` needs the **portfolio name** — the tax-summary response carries `strategy`, `threshold_days`, `currency` (`analytics.rs:98-100`) and no name. `TaxSummary` (`types.ts:89-95`) has no name field and Target Files 3 adds only `strategy?`/`threshold_days?`. **The two new fields are provably insufficient.**
- The header's base currency comes from the portfolio object (per *Money*), a second non-payload input.
- `data-at` is captured in `fetchPortfolioData` — a third input the spec never says reaches the component. And "takes no date parameter" forbids exactly the prop that would make it available and testable.

Every one of these choices is duplicated at the App call site and in the layer-1 fixture, so a wrong guess is a rewrite.

**Fix:** state the signature verbatim, e.g. `interface Props { taxSummary: TaxSummary; portfolio: Portfolio; fetchedAt: Date }`, name the App-side state feeding it (`taxSummaryPortfolioId`, `taxFetchedAt`, both set at `App.tsx:114` beside `setTaxSummary`), and restate "takes no date parameter" as "takes no *user-selected* date".

### RB2 — The headline figures and the holdings table have no hook, so four specified assertions cannot be written

*Readiness B2.*

The copy table is the document's own stated contract for testability — "an assertion that 'the rounding line is present' is not an assertion until the string or the hook is fixed". That rule is applied to seven strings and to none of the figures:

- Layer 1 must assert "all three headline figures appear" — no hook exists, so the only selector is a text search for a formatted string, which is the golden's job and separately forbidden from being derived.
- Layer 2 must assert computed `color` "on one headline figure" — no selector exists.
- Print rule 5 requires "the headline block never splits" — no class or element is named, so the CSS declaration cannot be written.
- The holdings table is a column *set* with no header strings, no class, no per-column renderer (`PortfolioDetail.tsx:277` uses `toLocaleString(undefined,{maximumFractionDigits:6})` for quantity; `:292-294` uses `formatPercent` for ROI — neither is named here), and **no test at all**. The nine-column table Decided item 1 fought for ships unasserted.

**Fix:** add `data-testid` rows for `figure-realized`, `figure-unrealized`, `figure-value`, and `holdings` on the table; give the exact `<th>` strings and the renderer per column; name the headline block's class and use it in rule 5; pin the golden's currency and expected literal (fixture `currency: 'EUR'`, `total_realized_pnl: 1234.565`, expected `€1,234.57`).

### RB3 — The nav item's visible label is never given, and it is the smoke's only entry point

*Readiness B3 + Codebase M4 + Style M3. Three reviewers.*

The spec fixes the tab id (`statement`) but not the nav text. Every existing item has a multi-word label unrelated to its id — `Holdings & Tax lots`, `Risk & Correlation` (`App.tsx:386-397`) — and the smoke's first step is "the nav item whose accessible name is the statement label", which is a placeholder, not a selector. Alternative G's rejection removes every other entry point. The icon is also unpicked. "Accessible name" additionally misdescribes the mechanism: the items are `<li className="nav-item" onClick>` with no role, and the working precedent is `page.getByText('Transaction Ledger').click()` (`sell-pnl.smoke.mjs:107`).

Also closed here: there is **no tab union to extend**. `DEFAULT_TAB` is a bare `const` (`App.tsx:31`) and `useState(DEFAULT_TAB)` infers `string` (`:36`), so the nav-side work is the `<li>` and the branch — which makes the missing label the entire specification for that `<li>`.

**Fix:** one line in *Surface* plus a copy-table row: the fifth item's literal label and icon, and layer 2 navigates with `page.getByText('<label>')`.

### RB4 — "The rounding note is unconditional" contradicts Goal 7, the copy table, and the asserted negative list

*Style B1.*

Four places say the rounding note is **not** printed on the empty branch (copy table row, *Empty portfolio*, Goal 7, layer-1 negative list). One normative rule says "**The rounding note is unconditional.**" The rule's intended predicate is "not computed per currency"; as written it governs the whole document. An implementer reading the rules list ships a rounding declaration under "No holdings in this portfolio" — the exact artifact Goal 7 exists to prevent.

This is the document's recurring failure mode exactly: a rule stated normatively, then re-derived stronger somewhere else.

**Fix:** retitle to the actual predicate — "**The rounding note is not conditional on the currency.** On any branch that prints figures, it prints for every currency…" — and do not use "unconditional" while the empty branch exists.

### RB5 — The null-payload state is unspecified, and the pricing-layer justification for it is refuted by the code

*Style B2 + Codebase M7 + Readiness M8. Three reviewers. Controller-verified.*

The header says foreign-currency assets are "already **mispriced** on the dashboard today". The chain produces something else:

`currency_service.rs:234` builds the broken `EUR=USD` pair → `get_rate` returns `Err` → `replay` propagates with `?` (`tax_engine.rs:403`) → `calculate_lots` propagates (`:551`) → **`analytics.rs:86` maps it to `Status::InternalServerError`**. So `GET /tax-summary` returns **500** for a foreign-currency portfolio. `App.tsx:112` throws, `setTaxSummary` never runs, and the mandated guard renders **nothing** — while the error banner that would explain it lives inside `.header-row` and print rule 2 hides it.

So the inherited behaviour for the named case is not "mispriced", it is "no statement, and no visible reason". Two consequences are unspecified:

1. **A third state exists** — `taxSummary === null` — alongside populated and `assets: []`. No copy, no hook, no test. Goal 7 covers `assets: []`, not `null`.
2. **`:has()` goes inert.** Both hide rules are gated on `:has([data-print="statement"])`. If the statement renders nothing, the selector does not match, and Ctrl-P on the statement tab prints the **full dashboard including the sidebar** — precisely the outcome `:has()` was introduced to prevent on the other three tabs.
3. The smoke's fixture must be same-currency or the harness 500s before any print assertion runs. The spec never says so.

**Fix:** (a) header → "foreign-currency portfolios already fail the `tax-summary` request outright today (`currency_service.rs:234`, `analytics.rs:86`), so nothing this slice prints can be worse than the dashboard's current behaviour"; mirror in Risks. (b) State the null branch explicitly — render heading + `empty-note`, or accept the unstyled print and record it under *Other tabs* — and add a test case. (c) State in layer 2 that the seeded holdings are denominated in the portfolio's base currency.

---

## Majors

### M1 — The print palette has no values
*Readiness M1.* Step 4 names six variables to override and never says what they become. The implementer chooses six literals, and then "computed `color` … is dark" has no expected value — "is dark" is not executable; `getComputedStyle` returns an `rgb()` string that must be compared to something. **Fix:** give the six declarations verbatim and make the assertion literal, e.g. `expect(color).toBe('rgb(17, 17, 17)')`.

### M2 — Four of the seven "literal text" rows are templates and no clock is frozen
*Readiness M2 + Codebase M5 + Style M2. Three reviewers.* Layer 1 requires every hook "present with its specified text", but `generated-at`, `data-at`, `empty-note` and `run-context` are templates. "The app's existing date idiom (`Dashboard.tsx:114`)" is `toLocaleDateString('en-US', {month:'short',day:'numeric',year:'numeric'})` — never reproduced — and **"same idiom to the minute" has no referent**: no time-formatting idiom exists anywhere in the repo. **Fix:** write both formatters verbatim; require `vi.useFakeTimers(); vi.setSystemTime(...)`; cite `test/setup.ts:3`, which pins `process.env.TZ = 'UTC'` and is what makes the assertion deterministic at all.

### M3 — "No horizontal overflow" is vacuous as specified, and nothing makes nine columns fit 162 mm
*Readiness M3 + Style M7.* `index.css:47` sets `body { overflow-x: hidden }`, which propagates to the viewport — so `document.documentElement.scrollWidth <= clientWidth` **cannot fail**; it silently clips the right-hand columns off the paper instead. Meanwhile `.data-table td` carries `padding: 16px` (`index.css:245`), so nine columns cost 288 px of padding against an A4 content width of ≈612 px, before any digits. Separately, `emulateMedia` does not change the viewport (the harness runs 1400×1000), so Goal 2's "at A4 width" is asserted at desktop width. **Fix:** measure on `[data-print="statement"]` not the document, and note why the document-level check is meaningless; state print typography for the table; specify `page.pdf({ format: 'A4', preferCSSPageSize: true })` or set the viewport before the check.

### M4 — The printed table's row separators are a hardcoded color none of the five rules reaches
*Readiness M4 + Codebase M3. Two reviewers.* `.data-table td { border-bottom: 1px solid rgba(255,255,255,0.03) }` (`index.css:247`) is not a variable, so step 4's `--border-color` override does not reach it. `.data-table th`'s border **is** `var(--border-color)` (`:238`), so the header rule inverts and the body rules do not — a half-inverted table. White at 3% alpha on white paper means no visible row separation. The spec's non-variable inventory names only `.badge-crypto` and the hover rule. **Fix:** add the explicit print override and add `index.css:247` to the inventory.

### M5 — Two variables the spec says "invert" are not in its own override list
*Codebase M2.* Step 4 enumerates exactly six variables. Then the spec claims `.badge-crypto`'s colour "does invert" — that variable is `--color-secondary` (`index.css:335`), not in the list — and the Risks row says `var(--color-success)` "inverts but is not one of the six variables step 4 lists", which is self-contradictory. **Fix:** either add them to step 4, or state that the statement renders asset type as plain text and not `.badge` markup, and correct the "inverts" wording.

### M6 — `decimal.ts:65-67` is cited twice and is not the line either claim is about
*Codebase M1.* Controller-verified: `:63` is `currency: str(r,'currency') || 'USD'`; `:64-66` are the three money fields; `:67` starts the assets map; per-asset money is `:71-77`. Target Files 3 says "`:65-67` keeps `currency` only" — the currency line is `:63`, and it is the line the change edits. **Fix:** `decimal.ts:63` for the currency-only claim; `:64-66` and `:71-77` for the `num()` coercion. Note this citation was introduced by the Stage 7 fix wave.

### M7 — Two files the change must modify are absent from Target Files
*Codebase M6 + Style M14. Two reviewers.* `docs/auth-testing.md` (three separate statements commit to adding the print preview to its walkthrough) and `frontend/e2e/README.md` (its "Running it" block documents only `npm run e2e:sell-pnl`, and its "Why it is not in CI" section is the exact text the spec says the harness reasoning is destined for). Under the standard's file-boundary clause both must be Modify entries. Style adds that §5 is the *auth* walkthrough; the print preview's natural home is §8, "What this still does not prove" (`:347`) — and two of the three statements are written in the present perfect, asserting a state that does not exist yet.

### M8 — `page.pdf()` options and the page-count method are unspecified
*Readiness M5.* Playwright defaults to Letter with `preferCSSPageSize: false`, so as written the assertion measures Letter, not the A4 rule 1 ships. And "one page" needs either a PDF parse (no PDF dependency exists) or a byte-level regex on `/Type /Page`. One of these choices may add a dependency, which the file-boundary rule normally forbids. **Fix:** state the call and the counting method, or downgrade to "non-empty" and move the page count to the manual check.

### M9 — The smoke cannot produce the run context the spec says it will
*Readiness M6 + Style M8. Two reviewers.* "The fixture is two holdings in one portfolio with the strategy passed explicitly" — strategy is not a seeding parameter. It is a query parameter the **browser** sends from its own state (`App.tsx:39`, `:110`), set only by the select on the Holdings tab. The copied seeding block never touches it. An implementer who seeds with `strategy=HYBRID` will assert a `(threshold 30d)` segment the page will never print. **Fix:** either assert `· FIFO` with no threshold segment, or add the `getByLabel('Tax accounting method').selectOption('HYBRID')` step (precedent: `sell-pnl.smoke.mjs:189-193`) and the expected string.

### M10 — The Print control's placement is verified only by a human
*Readiness M7.* If the control is placed *inside* the statement root rather than beside it, the statement stays visible, page count is unaffected, and layer 1 cannot see it because it renders the component in isolation. The one structural requirement of this slice is checked only by the manual preview. **Fix:** add to layer 2, under print emulation, a count of `button, input, select, svg` inside `[data-print="statement"]` equal to zero.

### M11 — "Six strings" counts seven, and the Goal attribution undercounts
*Style M1.* Controller-verified: the table has seven rows. And "Goals 1, 3, 4 and 7 each assert one" is wrong twice — Goal 4 asserts three hooks, and `data-at` is asserted by no Goal at all.

### M12 — The currency rule's "stated once here" claim is false, and Goal 4 contradicts it
*Style M4.* The paragraph says "Stated once here; Goal 4, Goal 7 and Target Files 4 all point at this paragraph". Verified: Goal 4 contains no pointer and says the base currency prints "**as the server echoed them**" — the opposite of the rule, and false on the empty branch where nothing is echoed. Goal 7 never mentions currency. Target Files 4 restates the rule in full before pointing. Alternative D adds a third currency source. **Fix:** make Goal 4 read "portfolio name and base currency from the portfolio object; strategy and `threshold_days` as echoed (see *Money*)", delete the restatement in Target Files 4, and delete the sentence announcing non-redundancy.

### M13 — The smoke is "the gate" in one section and "not a gate" in two others
*Style M5.* Alternative B: "The Playwright smoke is the gate". Layer 2: "**Local step, not a gate.**" Target Files 6: "**Not wired into CI**". "The gate" is the word Alternative B's rejection turns on. **Fix:** "the check", consistently.

### M14 — Goal 6's second half has no verification hook in any layer
*Style M6.* Goal 6 is two requirements: the inversion must not *depend* on background graphics, **and** the background overrides must be present for a reader who enabled them. Layer 2 is text-colour-only by design; layer 3's checklist says "dark text on white", which is the graphics-**off** case. The second half is checked by nothing, while the document is careful elsewhere to name what each layer cannot cover. **Fix:** add "and again with background graphics enabled — background must be white" to the manual checklist.

### M15 — The spec proves the printed rounding sentence is not true of the value the page holds, and keeps it verbatim
*Style M9.* *Money* establishes that `normalizeTaxSummary` converts every money field with `Number(v)` before any statement code runs, and that the declaration "is a statement about the engine's output, not about a value the browser could recover". The page nonetheless tells an adviser its figures "are rounded for display from full-precision calculations" — a required string. **Fix:** either change the copy to "Figures are rounded for display." or state in one line that the fuller wording is a deliberate accepted imprecision.

### M16 — Restatement inventory: thirteen rules stated normatively and re-derived elsewhere
*Style M11 + M12.* Currency source; empty-branch field absence; money rounded once at display; `AUTH_MODE` export and its precedent; no CI browser job; the unwrapped-direct-child precondition; smoke asserts no figure; third renderer; the manual checklist (near-verbatim duplicate in *Verification commands*); `normalizeTaxSummary` coercion; `@page` cannot be scoped; the golden pins the third decimal; the CSV cross-check. Plus three sentences in which the document praises its own non-redundancy — two of which are false on inspection. **Fix:** one normative statement per rule, pointers elsewhere, in the style already used correctly at Target Files 2 and *Sequencing*. Delete the three self-congratulatory sentences.

---

## Nits (consolidated)

**Citation drift — the codebase reviewer re-derived all 90 citations. 79 correct, 1 wrong (M6), 10 off-by-one-to-three:**

| Spec says | Reality |
| :-- | :-- |
| `decimal.ts:65-67` keeps currency only | currency is `:63` (M6) |
| `App.tsx:111` populates `taxSummary` | setter is `:114` (cited correctly twice elsewhere in the same doc) |
| `Dashboard.tsx:40-45` renders the headline figures | render is `:66-99`; `:40-45` is the derivation |
| `PortfolioDetail.test.tsx:25-63` | fixtures are `:27-55` |
| `Dashboard.test.tsx:38-42` | factory is `:22-42` |
| `sell-pnl.smoke.mjs:36-59` | `seed()` is `:37-62` |
| `sellPnlCsv.ts:61-66` | helper is `:62-66` |
| `App.tsx:386-398` nav | `:386-397`; `:398` is `</ul>` |
| `index.css:3-31` palette | `:root` is `:3-33` |
| `index.css:93-98` grid | `:93-97` |
| `.data-table tr:hover td` at `:252` | selector at `:251` |
| `SellPnlModal.tsx:132` background | `:131` |
| `formatters.ts:19-22` catch | `:19-21` |
| `analytics.rs:45-51` / `:59` | `:46-53` / `:60` (readiness reviewer) |

**Bare filenames lacking repo-relative paths:** `analytics.rs`, `tax_engine.rs`, `db_types.rs`, `ci.yml`, `run.sh`, `start.sh`, `apiFetch.ts`, `test/mocks/handlers.ts`. Unambiguous but not allowlist-grade.

**Other nits:** `data-print="statement"` described as load-bearing "rather than in tests" while layer 1 asserts it; the splits copy says "when this page was loaded" after the rule explains that is a client-render moment distinct from the request; empty-note quoted two ways in one section; "minus its Actions column" is minus two (there is also an expander column, `PortfolioDetail.tsx:265-272`); "Two terms" followed by three bullets; `decision_records.md` §2 cites a spec heading the Stage 8 rewrite deleted, §3 cites `README.md:73-77` (actual `:68-72`) and `:10-11` (actual `:18-19`), and the header says the spec is 718 lines; §5 gives two reasons for dropping the color-literal grep, one of which the spec's own Risks row refutes; synonym drift ("the review record" vs `decision_records.md`; "the CSV" vs "the exact file next door" vs "the filed artifact"); Goal 3 says the CSV cross-check "**should**" equal while the parent says the two "**must**" agree; Goal 5 says "no backend change" while Target Files 6 changes the backend's launch environment — scope it to `backend_rust/` sources and `openapi.json`; "the smoke" and "the printed subtree" used before definition; uncalibrated claims ("low-effort", "the largest provenance hole", "the kind of work that turns a working pod into an OOMKill", "works on any device", "which is the harder one"); narrative leakage at spec:29, :43, :193, :504, :511.

---

## What was verified as sound

Recorded so the fix wave does not disturb it:

- **`:has()` works against the real DOM.** `.app-container` (`App.tsx:358`) and `<main class="main-content">` (`:456`) are parent/child; the auth alert (`:359-377`), sidebar (`:379`) and New-Portfolio modal (`:527`) are direct children of `.app-container`, so rule 1's enumeration is complete; the tab branches sit in a JSX fragment (`:469`) which renders no DOM node, so a bare component root **can** be a direct child of `.main-content` and the Print control can be its sibling. Rule 3 is load-bearing, not cosmetic: `display:none` on the sidebar does not free the `260px` grid track.
- **The prop-fixture test is supported.** `vite.config.js:26-29` (`globals`, `jsdom`, `setupFiles`); `test/setup.ts:3` pins `TZ=UTC`; both cited precedents are prop fixtures; `handlers.ts:121` is genuinely untouched by this design.
- **`run.sh` needs exactly the change described**, plus one trap the spec does not mention: `run.sh:16` is `set -uo pipefail`, so a bare `$1` aborts with no argument and breaks the existing `npm run e2e:sell-pnl`. Use `TARGET="${1:-sell-pnl}"`.
- **No cross-branch contamination.** Nothing cited exists only on `fix/pricing-layer-truthfulness`; `currency_service.rs:234` still builds the broken pair here, so the inheritance framing is accurate.
- **The rounding arithmetic is right.** The nearest double to `1234.565` sits above the midpoint, so `Intl` gives `1,234.57` at 2 dp; JPY gives `¥1,235`; BTC/ETH ≤8 dp; the catch path is 2 dp.
- **Slice atomicity holds.** Nothing waits on the parent design; the only coupling is that both edit `types.ts` and `App.tsx` — a merge conflict, not a dependency.

---

## Pattern across Stage 9

Stages 5–8 found contradictions, unreachable machinery and citation drift. Stage 9 finds a **specification gap of a different kind**: the document describes what the printed page must *say* and *look like* in detail, and what its parts *are* only in prose. Five of the sixteen majors and three of the five blockers reduce to "the implementer must invent an identifier" — a prop name, a nav label, a CSS class, a hex value, a date format.

The second pattern is that the Stage 8 fix wave, while landing cleanly, **introduced three new defects**: the `decimal.ts:65-67` citation (M6), the "mispriced" claim that the code refutes (RB5), and the `:has()` inert-when-unrendered hole that the same scoping created (RB5). Each was a correct fix to a real finding, applied without re-deriving the surrounding claim.
