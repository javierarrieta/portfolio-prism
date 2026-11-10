# Statement design — decision records moved out of the spec

Stage 8 (intermediate zoom out) found that roughly 180 of the statement spec's 718 lines were
decision records rather than requirements: numeric-precision forensics, harness forensics, and
correction history. The zoom-out standard puts those in the review record, which is already this
repo's convention for exactly this (`2026-11-05-sell-pnl-breakdown.md`).

This file holds what was moved out of
`docs/superpowers/specs/2026-11-09-portfolio-statement-print.md` so the reasoning is not lost. The
spec keeps the conclusion; this file keeps the derivation.

---

## 1. Where money precision is actually lost, and why the strings are not preserved

Moved from spec *Money: a display artifact, declared*.

`normalizeTaxSummary` (`frontend/src/utils/decimal.ts:65-67`) runs every money field through
`num()`, which is `Number(v)`, and `TaxSummary` types them as `number`
(`frontend/src/types.ts:89-95`). The statement renders from that state, so the full-precision
decimal string never reaches the component — it is gone before any statement code runs.
`formatCurrency` then calls `toNumber` again (`frontend/src/utils/formatters.ts:4`), a second
conversion of a value that is already a double.

Two consequences the spec states as conclusions:

- "The payload carries full precision" describes the wire, not something the page holds. The
  page's rounding declaration is a statement about the engine's output, not about a value the
  browser could recover.
- Preserving decimal strings for the statement alone would mean a second payload shape and a drift
  duty, and the strings would still have to survive `formatCurrency`, which coerces its argument
  whatever it is given: `formatCurrency("1234.565")` and `formatCurrency(1234.565)` are
  byte-identical. This is the reasoning behind Alternative C1.

## 2. The double-precision residual — measured where it was, and not measured where it wasn't

Moved from spec *Money*, "The residual, bounded rather than disclaimed". Stage 8 M6 found the
bound was evidenced from the wrong field, so the spec no longer states a numeric bound.

What was claimed in an earlier revision: *a printed figure may differ from the exact decimal
rounding of the payload by at most one unit of the last displayed digit*, pinned by a golden whose
payload exceeds 15 significant digits.

Why that was withdrawn:

- The evidence cited was `docs/superpowers/reviews/2026-11-05-sell-pnl-breakdown.md:22`, which
  measures `roi` at **28 fractional digits**. `roi` is a ratio, not a money field.
- The same record measures the money fields on the same fixture at two decimals:
  `realized_pnl` `"2000.00"`, `proceeds` `"10400.00"`, `cost_basis` `"8400.00"`.
- The FX multiplication named as the source of long decimals is the path the pricing-layer spec
  says currently produces no figure at all for foreign-currency assets
  (`2026-11-09-pricing-layer-truthfulness.md:24-27`).

So the >15-significant-digit shape was neither measured nor currently reachable, and the bound was
unmeasured on the fields it bounded. What remains in the spec is the reachable case: a value with a
third decimal place (`1234.565`), which display rounding moves regardless of any double-precision
question.

If someone later measures a money field exceeding 15 significant digits on real data, this is the
note to revisit, and the bound is worth restating.

## 3. Why the print smoke is a local step — the CI forensics

Moved from spec *Verification layers* layer 2. **This text is destined for GitHub issue #45** and
for `frontend/e2e/README.md`, which already has a "Why it is not in CI" section at `:73-77` that
will otherwise drift from the spec.

- CI runs `ubuntu-latest`, where `npx playwright install chromium` is one line and
  `emulateMedia`/`page.pdf()` need no display server. So a browser job is possible; keeping the
  smoke local is a choice, not an impossibility.
- `frontend/e2e/run.sh:3-16` is not browser-only. It compiles and starts the Rust API against a
  throwaway SQLite file and starts Vite, so a CI job costs a full cargo build and needs seeded
  data.
- A `continue-on-error` job nobody reads would read as a pass. The real alternative is a blocking
  browser job that also runs the existing sell-P&L smoke, which is #45's scope.
- **The harness does not currently start.** `run.sh` sets no `AUTH_MODE` and there is no
  `backend_rust/.env` (`run.sh:74-76` exports only `DATABASE_URL`, `BASE_URL`, `API_URL`). With
  `AUTH_MODE` unset the server asks for OIDC (`backend_rust/src/auth/mod.rs:85`) and bails naming
  four missing variables (`:132-145`), so `wait_for backend` burns its whole 420-second budget and
  the smoke dies with "backend did not come up". The last commit to touch `run.sh` predates the
  auth work.
- The fix is one line and has a precedent in the repo: `start.sh:21` already exports
  `AUTH_MODE=disabled` with the dev-only rationale, and `AGENTS.md` documents it. `AUTH_MODE=disabled`
  is compiled out of release builds (`auth/mod.rs:97-101`), so this is safe only because `run.sh`
  builds debug.
- **No figure should be pinned through `historical_prices`.** Stage 7 added price-pinning to the
  smoke; Stage 8 removed it. `grep -rn historical_prices backend_rust/src/api_routes/` returns
  nothing — the only writers are internal cache writes after a successful provider fetch
  (`backend_rust/src/services/currency_service.rs:397`, `:490`), so pinning would require writing
  SQLite out-of-band, against the harness's own rule (`frontend/e2e/README.md:10-11`, "seeds its
  own fixture through the REST API"). It also couples this slice's smoke to the schema the
  pricing-layer change is actively revising (its P2 is the `historical_prices` dedupe + unique
  index + real upsert, `2026-11-09-pricing-layer-truthfulness.md:184`). And it buys nothing: the
  smoke asserts no figure, so a zero-price run cannot fail it. If deterministic prices are ever
  wanted, the pricing-layer work owns `historical_prices` uniqueness and already needs seeded
  closes.

## 4. Correction record: the echoed strategy

Moved from spec *Decided during drafting* item 3, which Stage 7 already called a correction record.

An earlier draft claimed the `tax-summary` payload does not carry `strategy` or `threshold_days`,
and built a hedging label and a risk row on that claim. It was wrong: `analytics.rs:95-100` returns
`strategy`, `threshold_days` and `currency`, and the route passes the query values straight to
`calculate_lots`, so the echoed value is the value used. The only gap was that
`normalizeTaxSummary` dropped two of the three fields.

A second, smaller correction from the same area: this route **defaults but does not normalize**
strategy (`analytics.rs:59`, `strategy.unwrap_or_else(|| "FIFO".to_string())`), while the sibling
sell-P&L route normalizes it with the comment *"so the modal can never name a matching method the
engine did not use"* (`analytics.rs:216-222`). Normalizing this route is backend work and belongs
to the parent design's Target File 6, which already edits this handler — it was recorded there
rather than guarded against in the frontend, because the app cannot send a strategy outside
`FIFO`/`LIFO`/`HYBRID` (`App.tsx:39` initialises to `'FIFO'`; the only setter is the three-option
select at `PortfolioDetail.tsx:179-181`).

## 5. Alternative B's scope note, in full

Moved from spec *Alternative B*. The spec keeps the rejection; the nuance lives here.

What Alternative B rejects is a grep over a **stylesheet** — a check that a `.css` file mentions
some selectors, which passes on a page that prints white-on-white.

A different check was considered and is **not** rejected by that argument: a grep over
`PortfolioStatement.tsx` for a hardcoded color literal, which fails precisely when the "no
hardcoded colors" convention breaks. The repo already runs a grep gate in that shape, with the
silent-no-match failure mode solved, in
`backend_rust/scripts/check_access_gate.sh`. It is out of this slice only because the convention it
protects is one file wide and the smoke's text-color assertion catches the same breakage from the
user's side. Note the limit Stage 8 made explicit: a text-color assertion cannot see a hardcoded
**background**, so the color-literal grep is the only check that would. That residual is accepted
in the spec's Risks table rather than gated.
