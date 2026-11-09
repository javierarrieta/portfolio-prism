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

**Focus trap — reversed after implementation.** The row below originally refused focus management for
all modals together. The branch then shipped `role="dialog"` + `aria-modal="true"` + Escape on
`SellPnlModal` anyway, which turns that refusal into a live defect: `aria-modal` asserts to
assistive tech that the background is inert, and without a trap a sighted keyboard user Tabs
straight through it — the attribute becomes a claim the component does not keep. Kept the ~30-line
per-component trap so the attribute tells the truth. This is explicitly **not** a shared
focus-management system: the five other modals still declare no dialog semantics at all, so the
original "all three together" follow-up stands for them. Focus restoration to the trigger was
considered in the same pass and dropped — no precedent in this codebase, and it is not what makes
the `aria-modal` claim honest.

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
| Focus trap, focus restoration, scroll lock, `aria-modal` | *Partly superseded — see **Focus trap** above.* The remaining refusal stands for the five modals that declare no dialog semantics: scroll lock and focus restoration are still out. |
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
- The split-sync refusal above was re-tested on a live build rather than re-argued, and holds.
  Measured on a fresh database: first `GET /transactions/<id>/pnl` after a cold throttle costs
  ~350 ms and writes a `split_sync` row; the same call warm costs ~5 ms. `split_sync` is written
  only on success, so a host without egress re-pays the full retry ladder (3 attempts, 1 s + 2 s
  backoff, ~3 s floor, up to ~30 s at the 10 s request timeout) on every click, forever. Adding
  the call also made `cargo test` reach Yahoo Finance — the `sell_pnl` suite went 1.33 s → 5.67 s —
  which is the hermeticity `analytics_test.rs` already guards against by pre-seeding `split_sync`.

---

## Open items

1. **FX conversion path has no backend test.** Every fixture is USD-into-USD, so `get_rate` is never
   reached and the one path that can hit the network and 500 is unexercised. Recorded in the plan's
   Task 4 rather than left silent. Do not describe that path as verified.
   **Discharged 2026-11-09** by `backend_rust/tests/fx_pricing_test.rs` on branch
   `fix/pricing-layer-truthfulness`: a past-dated multi-currency sell is asserted against a
   hand-computed figure with the provider unreachable, and the conversion is shown to
   discriminate dated from undated rates. See
   `docs/release-notes/2026-11-09-pricing-layer-truthfulness.md`.
2. **Optional:** replace the hand-written `decimal_opt_json` with `rust_decimal::serde::str_option`
   behind `serde-with-str`. Steps are in the plan's Task 1.

**Closed after this record was first written:** the split footnote is now a server-computed
`split_applied` field on the response rather than something the browser infers from whatever
transaction list it holds. One field added; a prop, a derived flag in `App` and a rerender test
removed. Net deletion, and the warning can no longer silently fail to fire because the client's
copy of the transaction list is one fetch stale.
