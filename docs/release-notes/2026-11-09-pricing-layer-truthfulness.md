# Pricing layer: FX conversion resolves, and is actually dated

Branch `fix/pricing-layer-truthfulness`.

## What a user will notice

**Foreign-currency assets on the tax endpoints used to fail outright.** The FX symbol was
built as `EUR=USD`; Yahoo's FX symbol is `EURUSD=X`. The provider returned no result, the
conversion returned an error, and `GET /api/portfolios/<id>/tax-summary` answered 500 for
any asset whose currency differed from the portfolio base. It now answers with figures.

**Every foreign-currency realized and unrealized number moves.** Cost basis, proceeds,
gain/loss, market value and unrealized P&L for a foreign-currency asset are all converted
with a different rate than before — the one dated on or before each transaction, rather
than whatever the last fetch happened to return. Single-currency portfolios are unchanged:
the same-currency short-circuit returns 1 and no number on that path moves.

**The sell-P&L footnote now names both conversion dates.** Cost basis is converted at the
rate recorded on or before each buy date; proceeds at the rate recorded on or before the
sell date. The old footnote claimed a single "latest available rate" for both legs, which
was never what the engine did.

## Behaviour changes worth knowing about

- **A close is chosen as of a date.** `get_rate` and `get_price` take the last close on or
  before the requested date, within a 30-day lookback, and return the date actually used.
  A gap longer than 30 days is an error naming the symbol, the date and the bound — it is
  never silently zero.
- **`historical_prices` is unique per `(symbol, date)`.** Existing duplicates are collapsed
  at boot, keeping the row with the greatest `rowid` — the most recently written, since the
  table has no write timestamp. Reads have no `ORDER BY`, so before this the "close on date
  X" was an arbitrary row among N.
- **A stored FX window only answers while it is fresh.** The database answers while its
  newest row is within 4 days of the requested date; otherwise the provider is asked for
  the tail and the rows are merged. `historical_prices` records no per-symbol coverage, so
  this is a proxy for "the window reaches this date"; a real coverage record is the
  follow-up. Inside that bound, a date with no stored close can still be priced from a
  close up to 30 days older.
- **A provider outage no longer discards the stored window.** When the provider fails, the
  performance series falls back to the stored closes instead of dropping them, so the
  series is priced from the last known figures rather than empty. Two limits are
  unchanged and named honestly: a window older than 4 days is no longer treated as an
  answer for today, and `stats_engine` has no carry-forward, so a date with no stored
  close in the window still prices at zero.
- **The market-value leg converts at the valuation date.** Unrealized P&L marks today's
  price at today's exchange rate. Realized legs keep their own transaction dates; the two
  cannot share one lookup, and the code says so at the call site.
- **A legacy database is migrated at boot.** The decimal column migration, the duplicate
  collapse and the unique index all run inside `init_db`, in that order, and a boot against
  a database that already holds duplicates is covered by a test.

## Not changed here

- `stats_engine` still applies one FX rate to every day of a performance series, and a
  failed conversion there still converts at 1:1.
- Split synchronisation still runs inside read paths (#50) and is still throttled per
  symbol (#38).
- Nine pre-existing tests still make live Yahoo calls (#37). The tests added on this
  branch are hermetic; the suite as a whole is not.
