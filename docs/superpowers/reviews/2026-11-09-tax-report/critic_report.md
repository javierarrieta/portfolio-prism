# Stage 7 — Critic Review (three concurrent reviewers)

*   **Target:** `docs/superpowers/specs/2026-11-09-tax-report-and-statement-design.md`, FeatureRange 1–414
*   **Reviewers:** `egm-design`, `egm-structural`, `egm-style` — run concurrently, design and structural applying `feature-spec-standard.md`, style auditing language only
*   **Date:** 2026-11-09
*   **Consolidated by:** orchestrator. Findings appearing in more than one report are marked **[x2]** / **[x3]**.

## Net assessment

`egm-design`: the architecture's core shape — one dated replay, server-owned totals, two thin
renderers — is sound, and the Alternatives section is unusually strong. What is not sound is
the cost and operational story. Three goals are stated absolutely and are falsified as
written (Goals 4, 5, 6), and Goal 7 will be falsified by its own slice 0.

`egm-structural`: the Markdown is clean — no skipped heading levels, all seven alternatives
carry pros/cons/reason, fences closed. The problems are placement and duplication: roughly
18% of the FeatureRange is restatement, and the same rationale exists in two to four copies,
which is the mechanism behind the stale-diagram defect found in stage 5.

`egm-style`: 14 blockers, 15 majors, 25 nits, mostly one-line rewrites. Several are outright
contradictions between sections rather than style.

## Blockers

### C1 — The statement's asset subset cannot coexist with server-owned totals (`egm-design` 1)

The payload carries per-asset totals and a portfolio rollup, and renderers are forbidden from
summing. A client-side subset filter therefore has no legal way to produce headline figures:
summing the selected assets' per-asset totals is exactly the forbidden arithmetic (and, per
Alternative C's own rejection, a double), while printing the portfolio rollup above a filtered
row list puts a headline describing a different asset set than the rows beneath it — on the
one artifact meant to be handed to an adviser. Goal 6 pins the two headline figures to payload
totals, and none of I1–I5 can catch the mismatch because they are all portfolio-wide.

Decide one: make the subset a server parameter on the JSON payload (Alternative G shows it then
needs `require_asset_in`, which the grep gate cannot see, so `isolation_test.rs` must), drop
the subset, or state that a subset view prints rows only and suppresses both headline figures.

### C2 — The cost shrink does not reach the dominant outbound consumer; slice 3 is not network-free for multi-currency **[x2]** (`egm-design` 2; corroborated by `egm-style` M1)

Verified: `tax_engine.rs:403` calls `get_rate(asset_ccy, base_ccy, tx.date)` for **every
transaction** in `replay`; `currency_service.rs:220-264` issues one network call per uncached
`(pair, date)` and its URL carries no range. Realized P&L is converted at the transaction-date
rate, so realized is **not** quote-free for any asset whose currency differs from the portfolio
base — the diagram's "realized uses recorded tx prices, no quote" is false for exactly the
portfolios that motivate dated FX.

The claim that "FX is fetched once per currency pair as a range … which keeps a 20-asset
multi-currency portfolio from issuing hundreds of requests" is true only of the *valuation* FX
at `end`. The engine section lists three changes, none touching the per-transaction FX path,
and plan file 3 puts the budget counter in the pricing layer, which cannot see the engine's
call site. Consequences: the flagship portfolio still issues ~200 lookups cold; the outbound
budget either 503s that portfolio or is set so high it bounds nothing; slice 3's "Network? no"
is false for multi-currency.

Fix: add a fourth engine change (`replay_at` takes a pre-resolved rate table, or `get_rate`
becomes a per-pair range fetch cached as a series), state it in P1, and restate sequencing as
"no network for single-currency portfolios".

### C3 — P3 has no destination, and both readings break something the design claims **[x3]** (`egm-design` 5; `egm-structural` 1; `egm-style` B12; also stage 6 Blocker 1)

"Moving it off the read path is a prerequisite" names no destination, and none is available:
`main.rs:63` is a plain `.launch()` with no Rocket tasks, and `k8s/` has no CronJob or Job.

*   **Reading A** — remove the sync from `tax-summary`: changes existing figures, falsifying
    Goal 7, and breaks the ordering the standing record pinned ("the new endpoint agrees with
    tax-summary only because the UI fetches tax-summary before the ledger").
*   **Reading B** — skip the sync only when `from`/`to` are present: makes the dated report and
    the undated screen disagree on split state, the exact drift Alternative B was rejected for,
    and is a parameter-dependent behaviour on one route — the same shape this draft already
    rejected for access levels.

Separately, the detection clause is unimplementable as written: with the sync off the read path,
nothing in the payload or `ErrorBody` can carry "unrecorded split" — `AssetTotals` has
`split_applied` (a *recorded* split) and `ErrorBody` lists only `missing_prices` and
`missing_rates`. Name the mechanism and field, or delete the clause.

### C4 — `P4` is referenced in four places and defined in none of the in-scope ones **[x3]** (`egm-structural` 1; `egm-style` B11; stage 5 M2)

The Prerequisites section is titled and enumerated P1–P3. Line 296 says "This choice carries a
fourth prerequisite", line 313 refers to "the P4 result types", and the label first appears at
line 432 in the out-of-scope half. The slice-0 cell reads `P1, P2, P3`, so slice 0 silently
under-scopes by one reviewed change. Add a fourth bullet and fix the slice-0 cell.

### C5 — Goal 5 promises "every", the error contract caps the list **[x2]** (`egm-style` B5; `egm-design` 13)

> "enumerates every missing symbol and every missing currency pair in one response" vs.
> "The missing-item list is capped, with a count of what was omitted."

Both are stated as requirements and no implementer can satisfy both. A budget-exhausted request
never attempted most assets either. Restate Goal 5 as "as many as the cap and budget allow,
plus how many were omitted".

### C6 — Goal 7's exclusivity is falsified by its own slice 0 **[x2]** (`egm-style` B6; `egm-design` 8)

> "The only change to numbers on existing endpoints is the dated-FX fix …"

P2 changes which row "the close on date X" resolves to wherever duplicate rows diverge — Yahoo
revises closes around corporate actions, and today's read has no `ORDER BY` or tie-break. P3
moves figures under either reading of C3. P4 stops failed lookups returning zero. Restate as
"every change to existing figures is named and ships with its own note", enumerating P1–P4.

### C7 — Goal 1's equality is stated for any window; Alternative E states it only for inception-to-date (`egm-style` B7)

`tax-summary` has no window, so "for any `[start, end]` … equals that asset's `realized_pnl`
from `tax-summary`" is literally false for a mid-year window. An implementer will write the
test from Goal 1 and it will fail. Split into two sentences: windowed sum equals
`realized_in_window`; inception-to-`end` sum equals `tax-summary`'s `realized_pnl`.

### C8 — Five names for one date, and three for the pricing date **[x2]** (`egm-style` B3, B4; corroborated by stage 6 Major 10 and Nit 17)

The same concept is `to`, `end`, `as_of`, `as-of date` and `valuation_date`; the start is `from`
or `start`. `price_date`, `price_date_used` and "pricing date actually used" name one value, and
Goal 4 omits the FX rate date entirely. Style calls this the single highest misread risk in the
document and supplies a canonical-name table:

| Concept | Canonical | Remove |
| :-- | :-- | :-- |
| Window end | `to` (query), `end` (prose/engine) | `as_of`, `as-of date` |
| Window start | `from` (query), `start` (prose) | — |
| FX valuation date | `valuation_date` | "the FX date" |
| Per-asset close date | `price_date_used` | `price_date` |
| Per-pair FX date | `fx_rate_date_used` | "the date each price was taken from" |
| Open-lot gain | `unrealized_gain_loss` | `latent_gain_loss`, `latent_roi` |
| Sold-lot P&L column | `lot_pnl` (from `LotMatch.pnl`) | "per-lot P&L", "the payload's realized total" |
| Pricing failure handling | "collected pricing failures" | "fail-collect rule", "collected-failure rule" |

### C9 — Other style blockers, each a one-line fix

*   **BOM ownership stated two ways.** The renderers section says the builder follows the
    existing export's "UTF-8 BOM"; `downloadCsv` owns the BOM and file 15 tests for no BOM in
    builder output. As written an implementer adds one and corrupts the header row.
*   **`latent_gain_loss` vs `unrealized`** — an implementer cannot tell whether it is the same
    quantity as `unrealized_at_end` or a third figure; Goal 2's equality is stated against
    `unrealized_pnl`.
*   **"each carrying only lot-level values" is false for `UNMATCHED`**, which carries a sell id
    and a sell-level quantity.
*   **The engine section says the pricing rule forbids nearby-day substitution; the pricing rule
    permits it.** Verified as a self-contradiction: the rule explicitly allows last-close-on-or-
    before within a lookback. The real objection — the FX date is derived from transaction dates
    rather than the requested date — is lost.
*   **The three defects are "outside this feature's scope" and also its prerequisites** (slice 0
    of this feature's own table). "Outside scope but blocks it" tells an implementer they may
    skip them.
*   **"No partial file is produced" is not server-testable** — the server produces no file; the
    CSV is assembled in the browser. Rewrite as a frontend obligation.
*   **`lot_pnl` is never named in the FeatureRange**, yet file 15 tests that it sums to "the
    payload's realized total", which has two candidate fields plus a rollup.

## Majors

### M-a — Dated FX has no persistence, so the hermetic test pattern cannot cover it (`egm-design` 3)

There is no FX table: `lib.rs:170-185` creates only `historical_prices` (keyed by symbol) and
`split_sync`. Rates live only in a process `HashMap` cache, lost on restart. The test plan seeds
the two seedable tables and has no mechanism for a past FX rate, so a dated multi-currency test
either reaches Yahoo — the regression the standing record measured at 1.33 s → 5.67 s — or skips
the FX path, which the standing record forbids describing as verified. Fix: land P1's range
fetch on `historical_prices` with `symbol = 'EURUSD=X'`, giving a seedable table, cross-restart
caching and one read path.

### M-b — Fail-collect cannot complete inside the only client-visible deadline (`egm-design` 4)

Verified: `k8s/ingress.yaml` carries no `proxy-read-timeout` (ingress-nginx default 60 s); the
reqwest client times out at 10 s with `MAX_RETRIES = 3` and 1 s/2 s backoff — ≈33 s worst case
per unreachable host. Goal 5 requires attempting every asset, so two unreachable symbols ≈66 s:
the browser receives a gateway timeout with an HTML body and never the typed 503, while the pod
keeps working and still holds the per-user slot, so the next click returns 429 "a report is
already running". The budget counts calls, not elapsed time, and the Non-Goal rules out per-call
timeout knobs. State the ingress deadline as a constraint, derive the budget from it, add a
handler-level elapsed deadline, and specify frontend behaviour for a non-JSON gateway failure
and slot release on client disconnect.

### M-c — The report path still writes, and the error taxonomy has no class for it (`egm-design` 6)

The concepts doc stated the rule as "no `SPLIT` inserts, not no writes"; the design keeps the
price write (P2 makes it a real upsert) and never says so. SQLite has no WAL and no
`busy_timeout`, so a concurrent writer surfaces `SQLITE_BUSY`. The contract has three classes
(422 / 503 / 429) and P4 adds only `NoData` vs `ProviderUnavailable`. A busy write on a report
request has no home and will land in whichever bucket an implementer picks, with a `retryable`
flag that may not agree with the status — the one property the contract exists to guarantee.

### M-d — The Access-level rationale is fictional **[x2]** (`egm-design` 9; corroborated by stage 5 M1)

Verified: `access.rs:37-57` — `resolve` returns `Access::Owner` for the owner and `None` for
everyone else; `Viewer` and `Editor` are unattainable today and there is no sharing table. The
stated reason, that an `Editor` rule "locked the read-only collaborator, which is the accountant
or tax adviser case that motivates sharing a portfolio at all", argues from a feature that does
not exist, and plan file 9 promises a test that a `Viewer` collaborator can request a dated
report, which cannot be written against this model. The outcome is fine and costs nothing — the
route already requires `Viewer`, so the decision is a no-op — but a fictional rationale will be
cited as precedent. Keep `Viewer`, replace the rationale with the real one, drop the collaborator
test until sharing exists.

### M-e — Strict validation silently reverses a recorded over-reach rejection **[x2]** (`egm-design` 10; stage 5 M3)

The standing record refuses an "Input validation matrix for `strategy` / `threshold_days`" and
instructs: do not re-raise without new evidence. The design adopts strict validation on both
paths, never cites that record, and its only new evidence — that the single-sell route already
normalizes strictly — was found false in stage 5. Either cite the record with real new evidence,
or scope strict validation to the dated path, which also keeps Goal 7 intact.

### M-f — No section owns risk or non-functional requirements (`egm-structural` 2)

Neither `Risks & Mitigations` (required by the standard "if applicable") nor `Non-Functional
Requirements` (template) exists, and the content is scattered: the IDOR warning inside
Alternative G, the outbound cost bound inside Pricing resolution, rollout and backward
compatibility inside Goal 7 and P1. Recommend a short `## Risks & Non-Functional Requirements`
H2 with four rows — existing-figure movement, outbound cost bound, IDOR/access posture, print
fidelity — each pointing at the mitigating section rather than re-arguing.

### M-g — Provenance sections are subordinated to the wrong parent (`egm-structural` 3)

`Decided during drafting` and `Reconciliation with the predecessor` are 72 lines — about 17% of
the FeatureRange — filed as H3 peers of "The engine change" and "The payload". Neither is
architecture; both are drafting provenance, and the repo already has a home for that material:
the standing review record's own contract is *"This file records decisions and rejections. It
deliberately does not restate the findings."* The design has inverted that, keeping the decision,
the argument and the retraction narrative inline. Either dissolve both into a review record, or
promote to H2 and rename "Decided during drafting" — a temporal heading that is false to every
future reader.

### M-h — Prerequisites duplicate The Problem 1:1 (`egm-structural` 4)

The Problem enumerates the same three defects that Prerequisites re-lists as P1–P3. The
predecessor treats them as a top-level `## Prerequisites`. Recommend one H2 after Non-Goals with
columns `defect | where | user-visible effect | fix | slice | tests that move`, reducing The
Problem to two sentences. This deletes ~25 lines and gives P4 a home.

### M-i — The same rationale is carried in two to four copies (`egm-structural` 5; `egm-style` M14)

Ownership map, with the copy to keep: server PDF cost → Alternative A; totals must be
server-side → The payload plus Alternative C; prefix replay yields both figures → The engine
change; outbound cost bound → Pricing resolution; fail-collect contract → Error contract; route
choice → Alternative B owns the rationale, Route owns the decision; the six accepted additions →
Reconciliation as a 6-row index table.

### M-j — `## Open Questions` omission is defensible but the discharge is invisible, and the count is wrong (`egm-structural` 6)

The predecessor's `## Still Open` lists **five** items and all five are answered here, but
`Decided during drafting` asserts "Four items were open", and nothing lets a reader verify every
open question was closed. Restore the section with `*None open.*` and five one-line pointers.

### M-k — The Sequencing table's `Risk` column misleads **[x2]** (`egm-structural` 7; `egm-style` M8)

The column mixes magnitudes with scopes: `low`, `zero backend`, `contained`, `moderate`,
**the risky piece**. Worse, slice 0 is graded `low` while P1 — the highest-blast-radius item,
since it moves realized figures for every foreign-currency asset — sits in it. `Network?` is
also undefined: slice 0 changes outbound HTTP code yet reads "no", so the column must mean "does
verification reach the network", which is nowhere stated. Split into `Blast radius` and
`New-code risk`, or drop the column.

### M-l — Prose where a table is the honest form (`egm-structural` 8)

Date parameter semantics states nine validation conditions in a 16-line paragraph; the invariant
a reader must check is *coverage* — that every malformed input has a defined outcome — which
prose cannot be checked against. Same for the three-status error contract and its stated
`retryable` invariant. Note: the standing record refused a validation matrix as over-reach; this
review is not re-litigating that, only observing that the document has since adopted strict
validation, so the matrix is now the design.

### M-m — Alternative D is a deferral filed under rejections (`egm-structural` 9)

Its own text says "deferred rather than rejected on merit". The template's
`### Future Improvements` is omitted. As written the Alternatives Rule is technically satisfied
but the reader is told a rejection happened when it did not.

### M-n — Uncited assertions about existing code (`egm-style` M1)

Nine claims about code carry no `file:line` while the document cites code elsewhere, including
the load-bearing premise for "no engine change" ("the engine already documents that a sell's
matches depend only on transactions at or before it"), "There is no `@media print` rule anywhere
in the frontend today" (a global negative with no grep evidence), "near five requests per
second" (imprecise and uncited), and "the shared formatter" (never named).

### M-o — `must` / `forbidden` / `never` with no named test (`egm-style` M2)

Six normative statements have no enforcement named, including "Renderers are forbidden from
summing", "`sync_splits_for_asset` must not run inside a report request", and "a `retryable`
flag that agrees with the status" (unfalsifiable as written — rewrite as
"`retryable: false` for 422, `true` for 503 and 429"). A negative `threshold_days` case is
required in prose but absent from file 8's list.

### M-p — Numbers required and never stated (`egm-style` M7; stage 5 M1's budget)

The bounded lookback, the outbound-request budget, the missing-item cap, the FX cache bound, and
the definition of "per user" (suggest: the `sub` claim) are all named and never valued. The lookback
length *is* the pricing rule and appears nowhere.

### M-q — 422 carries two meanings with no body rule (`egm-style` M9)

Validation failures and no-data failures share 422, and it is not stated whether validation uses
`ErrorBody` — which has no field for a validation message — or Rocket's status text.

### M-r — Goal 4 outruns its evidence **[x2]** (`egm-style` M10; `egm-design` 7)

"Same inputs, same numbers, regardless of when the report is regenerated" is falsified by the
app's own read-path writes: a split recorded after the fact changes in-window cost basis, and a
partially filled price range can gain a closer-dated row between runs, moving "the last close on
or before `end`". The run context records the date used but not the source — the predecessor's
Decision 8 required a `price_source` flag and the design dropped it. Qualify to "same inputs and
same recorded pricing inputs" and restore the source disclosure.

### M-s — The Access/cost/CSV alternatives gap: server-generated CSV is never evaluated (`egm-design` 11)

Alternative A rejects server-side *PDF* and the design generalises that into "the backend returns
JSON only", but the artifact that must be exact is the CSV. Every argument for server-owned
numbers — strings end to end, renderers forbidden from summing, the `normalize*` bypass, "a total
derived in TypeScript is a double" — is an argument for emitting the CSV where the decimals live.
A `format=csv` variant on the same route preserves Alternative B's no-new-route property, is
byte-reproducible (Goal 4), `curl`-able, immune to `normalize*`, and removes the float-summed
frontend test. Record it with its real cost rather than leaving it unevaluated.

### M-t — The CSV's summable-columns disclosure contradicts the convention it claims to follow (`egm-design` 12)

`sellPnlCsv.ts:13-16` adopts the opposite convention deliberately: caveats "ride along as
columns rather than a leading `#` row, which breaks naive" parsers. A leading comment row also
breaks the app's own `detectColumns` import path if the owner ever re-uploads the file. Pick one
and correct the "follows the existing conventions" sentence.

### M-u — P4's effect on the undated path is stated as a side effect, never specified (stage 6 Major 9, carried)

The document never says what the undated path does after P4 — whether an unobtainable price now
returns 422 or 503, whether collected-failure applies there, or which tests move. This is a
user-visible change on a screen the plan claims is untouched.

## Nits (selected)

*   **503 + `retryable: true` for budget exhaustion is a deterministic failure wearing a transient
    label** (`egm-design` 14). Same inputs, same budget, same result; retrying into a 5 req/s
    throttle makes it worse.
*   **The guard silently depends on `replicas: 1`** (`egm-design` 15). That is a manifest value,
    not an invariant, and a rolling update with default maxSurge runs two pods for a window — two
    slots per user and two SQLite writers with no WAL.
*   **The frontend CSV test will assert the totalling property in floats** (`egm-design` 16).
    `sellPnlCsv.test.ts:184-194` uses `toBeCloseTo(…, 9)` precisely because the payload went
    through `Number()`. With strings preserved the same assertion is either still float,
    contradicting the exactness prose, or needs a decimal library not in the dependency list.
*   **Goal 3's arithmetic** (`egm-design` 17; `egm-style` M6): "four common cases" against five
    presets, and "no calendar component" alongside two `<input type="date">` fields, which render
    native pickers in every target browser.
*   **Sentence fragment in Alternative A** — no verb (`egm-style` M15).
*   **"schemaless" vs "empty schema"** describing the same defect two ways (`egm-style` M13).
*   **Structural nits:** the Overview ends with the Technical Plan's thesis; "landed first and
    separately" puts a sequencing instruction in a heading; the predecessor reference is a bare
    filename rather than a link; no `### Additional Resources` despite ~25 file references.
*   **Style nits:** ~25 one-line rewrites — editorial transitions, decorative adverbs, aphorisms
    that repeat the preceding sentence, "honesty" used as a technical noun, passive constructions
    that hide the actor in the Reconciliation section. Full list in the style report.

## Cross-half disagreements recorded by structural (out of scope, noted)

The Verification Plan asserts "invariants I1 through I5" and DIP item 8 cites "I1, I2 and I4 from
the predecessor RFC", but the FeatureRange never names I1–I5 — its acceptance criteria are
Goal 1…Goal 7. The two halves use different identifier systems for the same invariants. DIP item
22 creates `frontend/scripts/check_print_styles.sh` while the Verification Plan runs
`bash scripts/check_print_styles.sh`.
