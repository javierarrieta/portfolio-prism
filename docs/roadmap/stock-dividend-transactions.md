# Stock dividends have no transaction type

Status: **not started.** Recorded 2026-11-15 while the reason is fresh.

## Why this exists as a document

A real database import surfaced a batch of `BUY` rows with `price = 0.0`. They are not missing
data and they are not junk: they are **shares received as a stock dividend**, acquired for no cash.
An automated review of that import flagged them as incomplete records. They are the correct value,
and the application has no other way to express it.

`KNOWN_TRANSACTION_TYPES` is `["BUY", "SELL", "SPLIT"]` (`backend_rust/src/backup/mod.rs:221`).
There is no type for shares received for free.

## What the zero-cost encoding does to the numbers

A zero-cost lot is not a bug; it is the arithmetic the encoding implies.

- `tax_engine.rs:852` guards the ratio: `latent_roi` is `0` when `lot_cost` is zero, because there
  is nothing to divide by.
- `latent_gain_loss` for such a lot is the **full market value**, and selling it books **full
  proceeds** as realized gain.

Whether zero is the basis you want to carry is the open question below, not a defect.

## The trap, demonstrated

The ordinary write path does **not** restrict `transaction.type` to the known set — only `SPLIT` is
special-cased (`api_routes/transactions.rs:240,245`). The backup boundary **does**. So the two ends
of the product disagree, and the disagreement is one-way: you can write it, you cannot read it back.

Proven against a local `v0.1.0-M1` on Postgres 18.7, using a throwaway database and a synthetic
type name:

```norun
POST /api/portfolios/1/assets/1/transactions  {"type":"DIVIDEND","quantity":"10","price":"0",…}
  → 200   {"id":39,"type":"DIVIDEND",…}
GET  /api/backup/export?portfolio_ids=1
  → 200   the exported bundle carries "type": "DIVIDEND"
POST /api/backup/preview?mode=copy   (that bundle)
  → 422   {"path":"portfolios[0].assets[0].transactions[N].type",
           "reason":"\"DIVIDEND\" is not one of [\"BUY\", \"SELL\", \"SPLIT\"]"}
```

Anyone who invents a type to express a stock dividend gets a working application and a backup that
can never be restored. That is the failure mode this document exists to prevent, and it is live
today rather than hypothetical.

## Why `SPLIT` is not the answer

A split re-rates shares already held; a stock dividend issues new shares. `SPLIT` also requires a
positive quantity **and** a positive price, is limited to one per asset per UTC calendar day
(`uq_split_asset_date`), and has its fee zeroed by the write path. None of that describes a free
share.

## The decision the product has to make, and this doc will not make

Whether a share received for free carries zero basis or an assigned non-zero basis is a tax
question for the owner, in their jurisdiction, with their adviser — not a developer's choice. In
several jurisdictions a stock dividend is treated as a cash dividend that was then reinvested,
which gives the new shares a non-zero acquisition value rather than zero. The application currently
cannot record anything but zero.

Whatever is chosen has to be **recorded as data at the time**, not inferred later from a zero price
— because "free because it was a dividend" and "free because nobody filled the field in" are
indistinguishable in the current schema, and only one of them is correct.

## What a fix touches

- A new member of `KNOWN_TRANSACTION_TYPES`, and the write-path validation that should have enforced
  that set all along. **That validation gap is worth closing on its own**, with or without this
  feature: it is a data-integrity hole that silently poisons backups.
- The tax engine's cost-basis handling for the new type.
- **Backup schema `v2`.** Adding an enum member changes what `collect()` accepts, and the policy in
  `docs/backup-schema/README.md` is explicit that this requires `v1.json` → `v2.json` plus an
  upgrader, with `v1` accepted forever after. This is the expensive part, and the main reason to
  write the change down before someone "just adds" a type to the array.
- Frontend: the transaction form, the statement and the tax report display.
- Existing data. Rows already recorded as `BUY` at `0` would have to be decided on: whether they
  become the new type *is* the tax question above, so it cannot be answered inside a migration file.

## Do not do this

Do not "clean up" zero-price `BUY` rows as junk. Where they came from real events, they are the
correct and only available representation of those events.
