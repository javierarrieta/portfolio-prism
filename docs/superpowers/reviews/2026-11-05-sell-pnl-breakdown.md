# Review record — Sell Transaction P&L Breakdown

**Date:** 2026-11-05
**Reviewed:** `docs/superpowers/specs/2026-11-05-sell-pnl-breakdown-design.md` and
`docs/superpowers/plans/2026-11-05-sell-pnl-breakdown.md` as of `f46b44f`
**Findings applied in:** `805f13d` (blockers and majors) and `864cf68` (plan altitude, measured corrections)
**Method:** five-stage review, 8 reviewer passes, consolidated to 8 blockers / 22 majors / ~55 nits, 8 findings rejected as over-reach

This file records *decisions and rejections*. It deliberately does not restate the findings: every
accepted finding's reasoning was written into the document it governs, next to the rule it
justifies. If a rule here seems arbitrary, look in the spec or plan section it belongs to — the
"why" is inline.

---

## Decisions that needed an author

**Decimal scale — full precision, no quantisation.** No field is rounded or normalised; totals are
defined as the sum of their parts, so the cross-endpoint invariant is exact rather than
tolerance-based. Tests compare parsed `Decimal`s, never serialized strings. Measured on the
plan's own FIFO fixture: `realized_pnl` is `"2000.00"`, `proceeds` `"10400.00"`, `cost_basis`
`"8400.00"`, the empty sum `"0"`, and `roi` carries 28 fractional digits. Three different
plausible string forms had been predicted for that fixture and only one is what the code does —
which is the argument for the decision, stated once so nobody reintroduces a string assertion.

**Split-sync ordering — documented and pinned, not removed.** `/tax-summary` inserts `SPLIT` rows
(throttled per symbol per 30 days, errors discarded); the new endpoint is read-only and agrees
with it only because the UI fetches tax-summary before the ledger. Making the new endpoint run the
sync was considered and rejected: it turns a GET into a writer and puts a network fetch on a click
the user expects to be instant. The ordering is pinned by a route test rather than left implicit.

---

## Rejected as over-reach — do not re-raise without new evidence

| Proposal | Why it was refused |
|---|---|
| `fx_degraded` flag; serve asset-currency figures when FX fails | Invents a "these numbers are in a currency you did not ask for" state on a tax screen. Turns a loud 500 into a silently wrong figure. |
| Per-field `round_dp` quantisation | Actively harmful: `Σ rounded lots ≠ unrounded total` breaks the invariant that holds by construction today. |
| History-version / row-count echo the client compares against | New cross-request protocol and invalidation semantics for a single-user local SQLite app. Documented the ordering instead. |
| FX rate dedup inside one replay; p50/p99 targets; max-transactions limit | No measured bottleneck. Discharged by one sentence stating the scale assumption (see spec *Risks*). |
| Input validation matrix for `strategy` / `threshold_days` (400s, ranges) | The frontend owns those query strings and the engine's FIFO fallback is documented. The real defect was echoing unvalidated input as fact; one normalisation fixes it. |
| `term` as a serde enum | `schemas.rs` has no serde enums; `Option<String>` with a documented value set matches the file and yields equivalent OpenAPI. |
| Focus trap, focus restoration, scroll lock, `aria-modal` | The two existing modals in `PortfolioDetail.tsx` have none of this. Adding a focus-management system to one feature is either a lie in the spec or an unrelated refactor. Follow-up for all three together. |
| Ownership scoping / id-enumeration threat model | No auth anywhere in this API and `DELETE /api/transactions/<id>` already takes a bare id. Trust model stated in one sentence. |

---

## Verified against the toolchain, not derived

Recorded because each of these overturned a claim made during the review.

- Baseline on the untouched tree: 73 unit + 49 integration tests pass. The plan's "14 `tax_engine`
  / 8 `db_types`" gates are accurate.
- `Σ per-lot pnl == sell total` compares equal at full precision.
- `Option<Decimal>` with a custom serializer derives `utoipa::ToSchema` cleanly on 4.2.3 — no
  `value_type` workaround needed.
- CI (`.github/workflows/ci.yml`) runs `cargo test` and nothing else. There is no clippy step, so
  the plan's lint gates are stricter than CI on purpose.
- `rust_decimal` has no `serde::option_str`; the module is `serde::str_option` and it needs the
  `serde-with-str` feature, which this project does not enable.

---

## Open items

1. **FX conversion path has no backend test.** Every fixture is USD-into-USD, so `get_rate` is never
   reached and the one path that can hit the network and 500 is unexercised. Recorded in the plan's
   Task 4 rather than left silent. Do not describe that path as verified.
2. **Optional:** replace the hand-written `decimal_opt_json` with `rust_decimal::serde::str_option`
   behind `serde-with-str`. Steps are in the plan's Task 1.

**Closed after this record was first written:** the split footnote is now a server-computed
`split_applied` field on the response rather than something the browser infers from whatever
transaction list it holds. One field added; a prop, a derived flag in `App` and a rerender test
removed. Net deletion, and the warning can no longer silently fail to fire because the client's
copy of the transaction list is one fetch stale.
