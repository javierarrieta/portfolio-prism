#!/usr/bin/env bash
# Pre-flight audit for migration 0002. Run this BEFORE the deploy that carries it.
#
# Why it exists: `sqlx::migrate!` runs at startup (`main.rs`), so a migration that
# raises means the app does not boot — exit 1, crashloop, and the reason is in a
# container log rather than in front of whoever is deploying. `0002_transaction_type_check.sql`
# raises on purpose in two situations, and both are about data this script can see in
# advance:
#
#   1. a `type` outside BUY|SELL|SPLIT, which the migration refuses to rewrite because
#      rewriting it would invent tax data;
#   2. two `SPLIT` rows on one asset and UTC day whose only difference is case or
#      whitespace, which normalization would collapse onto the unique partial index
#      `uq_split_asset_date` — rows that have been escaping split dedupe precisely
#      because that index's predicate is case-sensitive.
#
# The migration will catch both. This script's whole value is catching them while the
# answer is "fix these rows" instead of "the site is down".
#
# Exit codes, in the order a caller should read them:
#   0  clean — 0002 will apply
#   1  out-of-set types and/or colliding SPLIT rows found; the detail is printed
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

echo "auditing transaction types in $DATABASE_URL"

# Does the table exist at all? A database that has not run 0001 is not an audit failure
# of this kind, but answering "clean" would be a lie, so it exits 2 with the reason.
if [ "$(q "select count(*) from information_schema.tables
           where table_schema='public' and table_name='transactions'")" != "1" ]; then
  echo "AUDIT FAILED — no 'transactions' table in $DATABASE_URL; this is not a migrated prism database"
  exit 2
fi

problems=0

# --- 1. types outside the closed set --------------------------------------------
# Same predicate as the migration's own check, so the two cannot disagree about what
# "out of set" means. `quote_literal` so a value that is only whitespace or an empty
# string is visible rather than printed as nothing.
out_of_set=$(q "select quote_literal(type) || ' x' || count(*)::text
                  from transactions
                 where upper(btrim(type)) not in ('BUY','SELL','SPLIT')
                 group by type
                 order by type")
if [ -n "$out_of_set" ]; then
  problems=1
  echo "FOUND — types outside BUY|SELL|SPLIT (value x row-count):"
  echo "$out_of_set" | sed 's/^/    /'
  echo "    0002 will refuse to run. Reclassify or delete these rows first;"
  echo "    rewriting them would invent tax data."
else
  echo "ok   no type outside BUY|SELL|SPLIT"
fi

# --- 2. SPLIT rows that normalization would collapse ----------------------------
# Grouped on the same expression `uq_split_asset_date` keys on, so a group here is
# exactly a future unique violation. Case-insensitive match on the type, because that
# is what the normalization does to them.
split_dups=$(q "select format('asset_id=%s utc_day=%s rows=%s spellings=[%s]',
                       asset_id, day, n, spellings)
                  from (
                        select asset_id,
                               (date at time zone 'UTC')::date as day,
                               count(*) as n,
                               string_agg(distinct quote_literal(type), ', '
                                 order by quote_literal(type)) as spellings
                          from transactions
                         where upper(btrim(type)) = 'SPLIT'
                         group by asset_id, (date at time zone 'UTC')::date
                       ) g
                 where n > 1
                 order by asset_id, day")
if [ -n "$split_dups" ]; then
  problems=1
  echo "FOUND — SPLIT rows that differ only by case or whitespace on one asset and UTC day:"
  echo "$split_dups" | sed 's/^/    /'
  echo "    These have been escaping split dedupe: uq_split_asset_date's predicate is"
  echo "    \`type = 'SPLIT'\` and is case-sensitive. Normalization would collapse them"
  echo "    onto that unique index, so 0002 will refuse. Decide which row is real."
else
  echo "ok   no SPLIT rows collide once case is normalized"
fi

if [ "$problems" = "1" ]; then
  echo "AUDIT FAILED — 0002 would abort on this database. Fix the rows above, then re-run."
  exit 1
fi

echo "AUDIT PASSED — 0002 will apply cleanly to $DATABASE_URL"
exit 0
