#!/usr/bin/env bash
# Pre-flight audit for migration 0006. Run this BEFORE the deploy that carries it.
#
# Why it exists: `sqlx::migrate!` runs at startup (`main.rs`), so a migration that raises
# means the app does not boot — exit 1, crashloop, and the reason is in a container log
# rather than in front of whoever is deploying. `0006_asset_symbol_unique.sql` raises on
# purpose, in one situation:
#
#   two rows in one portfolio whose symbol is the same once upper-cased — the same asset
#   stored twice, or stored twice under two spellings.
#
# That is exactly what has been possible forever, because asset identity was a
# SELECT-then-INSERT with no constraint underneath it, and a case-variant symbol is invisible
# to that lookup. So the audit is not checking for damage that should be absent; it is
# finding out whether the live database already has any, while the answer is still "fix these
# rows" rather than "the site is down".
#
# It also reports two things it does NOT fix, so the operator is not surprised after the
# fact: symbols stored in a spelling the write path never produces (0006 deliberately does
# not rewrite them — upper-casing an asset symbol would silently detach its price history,
# see step (2) of the migration), and ISINs held twice in one portfolio, which 0006 does not
# constrain at all.
#
# Exit codes, in the order a caller should read them:
#   0  clean — 0006 will apply
#   1  colliding rows found; the detail is printed
#   2  could not run: no DATABASE_URL, no psql, or the query failed
#
# Prints what it inspected. A gate that silently inspects nothing is worse than no gate.

set -uo pipefail

if [ -z "${DATABASE_URL:-}" ]; then
  echo "AUDIT FAILED — DATABASE_URL is not set. It is the database the deploy will migrate."
  exit 2
fi

if ! command -v psql >/dev/null 2>&1; then
  echo "AUDIT FAILED — psql is not on PATH."
  exit 2
fi

q() { psql "$DATABASE_URL" -Atq -v ON_ERROR_STOP=1 -c "$1" 2>&1; }

echo "auditing asset symbols in $DATABASE_URL"

if [ "$(q "select count(*) from information_schema.tables
           where table_schema='public' and table_name='assets'")" != "1" ]; then
  echo "AUDIT FAILED — no 'assets' table in $DATABASE_URL; this is not a migrated prism database"
  exit 2
fi

problems=0

# --- 1. rows that would collapse onto one index key ------------------------------
# Same predicate as the migration's own audit, grouped on `upper(symbol)` — the key the
# index will use — so a group printed here is exactly a future unique violation and the two
# cannot disagree about what a collision is.
collisions=$(q "select format('portfolio_id=%s symbol=%s rows=%s ids=[%s] spellings=[%s]',
                       portfolio_id, key, n, ids, spellings)
                  from (
                        select portfolio_id,
                               upper(symbol) as key,
                               count(*) as n,
                               string_agg(distinct id::text, ', ' order by id::text) as ids,
                               string_agg(distinct quote_literal(symbol), ', '
                                 order by quote_literal(symbol)) as spellings
                          from assets
                         group by portfolio_id, upper(symbol)
                       ) g
                 where n > 1
                 order by portfolio_id, key")
if [ -n "$collisions" ]; then
  problems=1
  echo "FOUND — same symbol twice within one portfolio (these would collapse onto one key):"
  echo "$collisions" | sed 's/^/    /'
  echo "    0006 will refuse to run. Decide which row is real and merge or delete the other"
  echo "    by hand, including its transactions. A split asset is a split cost basis."
else
  echo "ok   no portfolio holds one symbol twice"
fi

# --- 2. symbols in a spelling the write path never produces ----------------------
# Not a failure, and 0006 does not rewrite them — see step (2) of the migration, which
# explains why upper-casing an asset symbol would silently detach its price history.
# Reported because a nonzero count here is the interesting case: those rows arrived by
# restore or by hand, and until this migration lands they are invisible to the dedupe
# lookup. The lookup change makes them visible; this count says how many there are.
lower=$(q "select count(*) from assets where symbol is distinct from upper(symbol)")
case "$lower" in
  ''|*[!0-9]*)
    echo "AUDIT FAILED — could not count non-uppercase symbols: $lower"
    exit 2 ;;
esac
if [ "$lower" != "0" ]; then
  echo "NOTE — $lower row(s) stored in a spelling the write path never produces. 0006 leaves"
  echo "       them as they are (upper-casing would detach their price history), but they"
  echo "       become visible to the dedupe check, so a later import of that symbol is"
  echo "       refused against them. Clean them up by hand if they are wrong."
else
  echo "ok   every symbol is already upper-case"
fi

# --- 3. isin, reported and not fixed ---------------------------------------------
# The same gap exists for ISIN — `services::assets::insert` checks it per portfolio in code,
# with no constraint behind it — and 0006 does not close it, so this says so rather than
# letting a clean exit read as "asset identity is constrained".
isins=$(q "select format('portfolio_id=%s isin=%s rows=%s', portfolio_id, key, n)
             from (
                   select portfolio_id, upper(isin) as key, count(*) as n
                     from assets
                    where isin is not null and btrim(isin) <> ''
                    group by portfolio_id, upper(isin)
                  ) g
            where n > 1
            order by portfolio_id, key")
if [ -n "$isins" ]; then
  echo "NOTE — same ISIN twice within one portfolio; 0006 does NOT constrain isin, so these"
  echo "       will still apply, and they are still a split cost basis:"
  echo "$isins" | sed 's/^/    /'
else
  echo "ok   no portfolio holds one ISIN twice (unconstrained — see the note in this script)"
fi

if [ "$problems" -ne 0 ]; then
  echo "AUDIT FAILED — fix the rows above, then re-run. Do not disable the index instead."
  exit 1
fi

echo "AUDIT CLEAN — 0006 will apply to $DATABASE_URL"
exit 0
