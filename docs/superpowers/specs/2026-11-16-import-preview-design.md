# Backup Import Preview Design

- **Status:** approved design, not yet implemented
- **Date:** 2026-11-16
- **Builds on:** `2026-11-13-backup-export-restore-design.md` (the bundle format, `preview`, `restore`, `capabilities`)
- **Supersedes nothing:** the bundle format is unchanged. No `schema_version` bump, no `docs/backup-schema` diff.

## Overview

The restore preview reports counts and nothing else: `Will insert 2 portfolios, 4 assets,
41 transactions.` That is not enough to answer the one question a preview exists for — *is this
the file I think it is?* Two portfolios look identical at count level if a symbol is missing from
one of them, because the count stays plausible. And in `replace` mode the same bare assertion is
made about the destructive half: `Will delete 2 portfolios…` describes live data the caller
cannot inspect.

This design makes both sides of the preview name what they contain, adds a post-restore readback
so the panel can show what actually landed, and moves price history out of the user flow
rhetorically, where it always was operationally.

Success looks like: opening a bundle in the panel and being able to confirm, without importing,
which portfolios it holds, which securities each holds, what date range the transactions cover,
and what is odd about it — and then, after the import, seeing the same shape read back from the
database.

## Requirements

1. The preview identifies the file: per portfolio, name, currency, exact asset and transaction
   counts, date span, and its securities with per-security counts and spans.
2. The preview describes what `replace` will delete with the same detail, read from the live
   database rather than asserted from counts.
3. Exact totals are always exact. Display is bounded; counting never is.
4. Anomalies in the bundle are surfaced, not silently accepted, and surfaced without requiring
   interaction — and they never block a restore.
5. After a successful restore the panel shows what the database now holds, in the same shape.
6. Price history is presented as an operator flow, not as something the panel is one checkbox
   short of.
7. The plan response stays small enough to cross a proxy that has already refused one oversized
   body once.

## Non-goals

- Applying price history from the panel. See *Decisions*, first entry.
- Paging, search, or a "show all" control over the capped detail.
- Transaction-level listing in the preview.
- Expected-versus-found reconciliation after restore.
- Async or resumable import for large bundles.
- Any change to the bundle format, its validation, or its versioning.

## Decisions and rationale

**Price restore is an operator flow, so the panel stops implying otherwise.** `apply_prices` and
`capabilities.allow_price_restore` stay exactly as they are — the parameter, the server flag, and
the 422 when asked for while disabled. What changes is the panel's copy: today it says price rows
"will be left alone — this panel does not restore price history", which reads as a limitation
about to be fixed. It is not a limitation; it is a different job. Price history is shared market
data, not caller data, and the panel is a caller-data flow. The conflation exists because the
restore route was already there, not because it belongs.

**Flags are informational.** A flagged row never blocks a restore and never unlocks a checkbox.
The existing typed `REPLACE` confirmation remains the only gate on the destructive mode. A
checkbox per flag kind would train the operator to tick boxes unread — which is worse than the
flag it was meant to make actionable — and the app is not entitled to decide on its own that a
given row is a defect. Zero-price transactions are the proof case: they are legitimate output of
a stock-dividend flow (see `docs/roadmap/stock-dividend-transactions.md`), so a flag that blocked
on them would be wrong on real data.

**Caps bound display, never counts.** 25 portfolios shown, 50 assets shown per portfolio, with
the exact totals always present and an explicit "showing 25 of 1,204". No paging: the useful
answer — how many, and are the names what I expected — is already on screen, and a second code
path for the remainder is ceremony.

**`Plan.delete` and `Plan.insert` change type.** They become `PlanSide` rather than gaining a
sibling field, because a side that carries its own counts is one concept and a `delete` plus a
`delete_detail` pair is two that can disagree. This is a breaking change to the preview response,
and it is contained: the published OpenAPI declares that body as untyped `serde_json::Value`, so
nothing generated breaks, and the only consumer is `BackupPanel`'s `asPlan` guard.

**The readback is the delete-side query, exposed.** `GET /api/backup/owned` is not a second
implementation of "describe what this user owns"; it is the function `preview` already needs for
its delete side, given a route. One query path, three call sites.

**`RestoreOutcome` gains `deleted`.** A plan is a statement about a moment. The database can move
between preview and restore — another tab, another device, a scheduled import — and `replace`
deletes what is there at execution time, not what was there at preview time. Reporting what was
actually deleted keeps the result honest when the plan has gone stale.

## Interface contract

```rust
pub struct PlanSide {
    pub counts: Counts,               // exact: portfolios / assets / transactions
    pub shown: Vec<PlanPortfolio>,    // capped at 25
    pub more_portfolios: usize,
    pub transactions_in_shown: usize, // so the UI can say "these 25 hold 4,000 of 9,120"
}

pub struct PlanPortfolio {
    pub name: String,
    pub currency: String,
    pub assets: usize,                // exact for this portfolio
    pub transactions: usize,          // exact for this portfolio
    pub shown: Vec<PlanAsset>,        // capped at 50
    pub more_assets: usize,
    pub asset_flags: BTreeMap<String, usize>, // aggregated over ALL assets, not just `shown`
    pub flags: Vec<String>,           // stable codes; unknown codes render, they do not throw
}

pub struct PlanAsset {
    pub symbol: String,
    pub name: String,
    pub currency: String,
    pub transactions: usize,
    pub first_date: Option<String>,   // None when there are no transactions
    pub last_date: Option<String>,
    pub zero_price: usize,            // a count, not a bool: one row and thirty differ
    pub flags: Vec<String>,
}
```

`asset_flags` is a map because the header needs counts per flag kind, and it is computed by the
producer rather than derived in the browser for the same reason the counts are: `shown` is capped,
so a frontend aggregation would silently under-report a flag sitting on asset 51. A header that
says `zero-price ×2` when the portfolio holds three is a preview that lies, which is the failure
this whole design exists to close.

`Plan` keeps its field names; two change type:

```rust
pub struct Plan {
    pub schema_version: i64,
    pub mode: String,
    pub delete: PlanSide,             // was Counts
    pub insert: PlanSide,             // was Counts
    pub prices: usize,
    pub warnings: Vec<String>,
}
```

New route:

| | |
| --- | --- |
| `GET /api/backup/owned` | `PlanSide` for everything the caller owns right now |

It takes `AuthedUser`, which is what Rule C of `scripts/check_access_gate.sh` checks for. It calls
no `require*` function, and that is correct rather than a gap: `require*` answers "may this caller
see portfolio N", and this route takes no portfolio id — it scopes by `owner_id`, exactly as
`preview`'s delete side does. `preview` and `restore` are in the same position. What does bind it
is `tests/route_coverage_test.rs`: a mounted route missing from `PROTECTED_ROUTES` fails that
test, and that inventory is the gate that actually catches a new route.

`RestoreOutcome` gains `pub deleted: Counts`.

## Backend changes

Two producers, one shape:

- **Insert side** — a pure function of the parsed bundle, no database. Walks every portfolio and
  asset to compute exact counts and flags, and materialises `shown` only for the first 25
  portfolios and their first 50 assets.
- **Delete side and readback** — `owned_detail(pool, user_id) -> PlanSide`. Two queries,
  deliberately:
  1. one aggregate query, one row per owned portfolio, carrying asset count, transaction count,
     date span and zero-price count. Cheap at any size, and the source of every exact total.
  2. one detail query, for the selected portfolios only, bounded by the cap.

  Exact totals never depend on how many rows the second query returned. Both queries filter by
  `owner_id` and by nothing else, which is the same scoping argument `owned_counts` already makes.

  `asset_flags` comes from a grouped aggregate over every asset the caller owns, not from the
  bounded detail query — same rule as the counts, arrived at independently.

### Flags

Computed identically on both sides, so one renderer handles both.

| code | level | fires when |
| --- | --- | --- |
| `no_assets` | portfolio | zero assets |
| `no_transactions` | asset | asset present, no transactions |
| `zero_price` | asset | any transaction on the asset whose `price` text matches the zero rule below — neither producer filters by transaction type, and a zero-price `SPLIT` reaches neither input anyway: bundle validation and the ordinary write path both refuse one |
| `future_dated` | asset | a transaction dated after today, **in UTC** — otherwise the same portfolio is future-dated in one timezone and not in another |
| `currency_mismatch` | asset | asset currency differs from its portfolio's |
| `duplicate_symbol` | portfolio | present when at least one symbol appears more than once in the portfolio. It is a **presence**, not a count: three copies of one symbol is one duplicated symbol, and two symbols each copied twice is still reported the same way the other producer reports it. The SQL side counts distinct duplicated symbols only to decide whether that number is above zero |

Two rules bind both producers, and they are the ones a later refactor is most likely to
break quietly:

- **Zero is a text match, not a cast.** `price::numeric = 0` would make one unparseable
  price row fail every preview for that user, because Postgres does not promise to
  evaluate a guard before a cast. `0`, `0.0` and `0.0000` are all zero; a string compare
  against `"0.0"` catches one of the three.
- **A code means one thing on both sides.** `asset_flags` is filled by a full walk on the
  bundle side and by a grouped aggregate on the database side, and the round-trip test
  compares the maps, not just the counts.

`negative_quantity` is deliberately absent. Deciding it honestly needs the split-aware holdings
arithmetic that lives in the tax engine; a naive `SUM(quantity) < 0` raises false alarms on any
portfolio containing a split, and a flag that cries wolf is worse than no flag. If it is wanted,
it goes through the engine, not through a preview query.

Flag codes are stable strings. The frontend renders an unrecognised code rather than throwing, so
adding a flag later is a backend-only change.

## Frontend changes

New component `frontend/src/components/BackupPlanDetail.tsx`, exporting `PlanSideView({ title,
side })`. Three call sites: insert side, delete side, post-restore readback. `BackupPanel.tsx` is
688 lines and stops growing.

- **Collapse by default above 5 assets**, using a native `<details>` with no JS state. 25 × 50 is
  1,250 rows, and a wall of rows is noise rather than verification. The threshold is a constant in
  the component, not a contract.
- **A collapsed portfolio must still show everything that matters.** The header line carries name,
  currency, exact counts, portfolio flags, *and an aggregate of its assets' flags* —
  `zero-price ×3`, `future-dated ×1`. Without this, a bundle at the cap can park an anomaly at
  position 12 of 25 behind a click, and a flag reachable only by expanding is a flag that was not
  raised. Nothing is visible only after interaction.
- **Known limitation of `<details>`:** browser find-in-page does not expand a closed block, so ⌘F
  will not find a symbol inside a collapsed portfolio. Accepted because the header line already
  answers the question find-in-page is usually being used for — *is this portfolio here* — and the
  alternative is hand-rolled expand state for a marginal gain.
- **`asPlan` gets stricter, not looser.** It already refuses to render a plan it does not
  recognise instead of half-rendering one; the nested arrays get the same treatment.
- **Copy.** The prices line becomes: *"This bundle carries N price rows. Price history is shared
  market data and is restored by an operator, not from this panel."*
- **Readback.** After a successful restore the panel calls `GET /api/backup/owned` and renders it
  under "Your portfolios now". A failed readback shows the counts from `RestoreOutcome` plus one
  line saying the readback failed; a failed readback must never look like a failed restore.

## Failure modes

| case | behaviour |
| --- | --- |
| preview rejects the file | unchanged: 422 with the reason list, no plan rendered |
| `GET /api/backup/owned` fails | result renders from `RestoreOutcome`, plus "could not read back your data" |
| plan shape unexpected | `asPlan` returns null → existing "unexpected response" path, never a partial render |
| bundle over `max_bytes` | existing pre-flight check in the panel, unchanged |
| database changed after preview | `deleted` reports what was really deleted; readback shows what is there |
| plan response size | bounded by the caps; gzipped, since `application/json` is in `gzip_types` |

The plan now carries portfolio and asset names. Before this ships, confirm no logging path
serialises a plan or a restore outcome; if one does, that is fixed in the same change.

## Testing

**Backend, unit.** `detail_from_bundle`: 26 portfolios → 25 shown and `more_portfolios: 1`; 51
assets → 50 shown and `more_assets: 1`; every flag fires and fails to fire on the right input;
`zero_price` counts rather than flags; date spans correct including the no-transactions case.
Plus the invariant the design rests on, asserted directly rather than incidentally: **counts stay
exact when the display is truncated.** A cap that quietly started affecting totals would produce
a preview that lies, and no other test would notice. The same assertion covers `asset_flags`: a
flag on asset 51 must still be counted on a portfolio showing 50.

**Backend, integration.** Round-trip equality: build a bundle, restore it, then `owned_detail`
must equal the preview's insert side. This is the claim that the readback describes the reality
the preview promised, and it is the test that fails if the two producers drift.

**Routes.** `GET /api/backup/owned` answers 401 without a session, and with two users in the
database returns only the caller's rows.

**Gate.** `check_access_gate.sh` passes with the new route present, and `tests/access_gate_test.rs`
gains a case proving it fails when a route skips `require*`. A gate is only as good as the
mutation that proves it can fail.

**Frontend.** `PlanSideView`: cap wording, flags, empty portfolio, collapsed state, and — the case
that the collapse rule would otherwise lose — a collapsed portfolio surfacing its assets'
aggregated flags on the header line. `asPlan`: accepts the new shape, rejects malformed ones.
Restore: readback rendered, and readback failure degrading correctly. The new prices copy.

**Housekeeping.** Regenerate `docs/openapi/openapi.json`. `cargo test`,
`bash scripts/check_access_gate.sh`, `cargo clippy --all-targets`, `npm run lint`.

## Rollout

One PR, backend and frontend together, as the original backup work did. Then `v0.1.0-M3`, then the
k8s-titan bump with both containers on the same release tag.

## Out of scope

Price apply in the panel, paging or show-all, transaction-level listing, `negative_quantity`,
expected-versus-found reconciliation, async or resumable import, and any change to the bundle
format.
