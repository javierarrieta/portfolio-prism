#!/usr/bin/env bash
# The migration is the schema. This pins the column types the rest of the codebase
# is written against, so 0001_init.sql cannot drift silently into something the
# queries disagree with. Shell rather than a Rust test because it has to run
# during the engine switch, before sqlx's postgres feature exists.
#
# Run it against a database that has the migrations applied:
#   nix-shell -p postgresql_18 --run 'bash scripts/dev-db.sh'
#   sqlx migrate run --no-dotenv --database-url "$DATABASE_URL" --source migrations
#   DATABASE_URL="$DATABASE_URL" bash scripts/check_migrations.sh
set -euo pipefail
: "${DATABASE_URL:?run scripts/dev-db.sh and export DATABASE_URL}"

q() { psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -tAc "$1"; }

want() { # table column type
  local t
  t=$(q "select data_type from information_schema.columns
          where table_schema='public' and table_name='$1' and column_name='$2'") \
    || { echo "FAIL could not read the type of $1.$2 from $DATABASE_URL"; exit 1; }
  [ "$t" = "$3" ] || { echo "FAIL $1.$2 is '$t', expected '$3'"; exit 1; }
  echo "ok   $1.$2 = $3"
}

# Identity is checked apart from the type, because information_schema reports it as a
# default rather than a type. It matters: identity refuses a supplied id, which is the
# SQL expression of the bundle rule that ids are never trusted from the file.
want_identity() { # table column
  local i
  i=$(q "select is_identity from information_schema.columns
          where table_schema='public' and table_name='$1' and column_name='$2'") \
    || { echo "FAIL could not read the identity of $1.$2 from $DATABASE_URL"; exit 1; }
  [ "$i" = "YES" ] \
    || { echo "FAIL $1.$2 must be GENERATED ALWAYS AS IDENTITY, is_identity='$i'"; exit 1; }
  echo "ok   $1.$2 is identity"
}

want_pk() { # table expected-primary-key
  local d
  d=$(q "select coalesce(pg_get_constraintdef(oid), '') from pg_constraint
          where conrelid = 'public.$1'::regclass and contype = 'p'") \
    || { echo "FAIL could not read the primary key of $1 from $DATABASE_URL"; exit 1; }
  [ "$d" = "$2" ] || { echo "FAIL $1 primary key is '$d', expected '$2'"; exit 1; }
  echo "ok   $1 primary key = $2"
}

# The split dedupe is an index, not a query. `apply_split_events` inserts with
# `ON CONFLICT DO NOTHING` and has no read-based check of its own, so a schema that
# drops `uq_split_asset_date` silently reintroduces #50 — two concurrent syncs doubling
# a split adjustment and corrupting cost basis. A gate that checked only tables and
# column types would have passed that schema, which makes it complicit in the bug it
# exists to prevent, so the index itself is asserted: present, unique, partial on SPLIT,
# and keyed on the UTC calendar day.
want_split_dedupe_index() {
  local row uniq partial pred def
  row=$(q "select concat_ws('|', i.indisunique, (i.indpred is not null),
                             coalesce(pg_get_expr(i.indpred, i.indrelid), ''),
                             pg_get_indexdef(i.indexrelid))
            from pg_class t
            join pg_namespace n on n.oid = t.relnamespace
            join pg_index i on i.indrelid = t.oid
            join pg_class x on x.oid = i.indexrelid
            where n.nspname = 'public' and t.relname = 'transactions'
              and x.relname = 'uq_split_asset_date'") \
    || { echo "FAIL could not read uq_split_asset_date from $DATABASE_URL"; exit 1; }
  [ -n "$row" ] \
    || { echo "FAIL transactions.uq_split_asset_date is missing; apply_split_events dedupes by conflict and has no read-based check (#50)"; exit 1; }
  IFS='|' read -r uniq partial pred def <<<"$row"
  [ "$uniq" = "t" ] || { echo "FAIL uq_split_asset_date must be UNIQUE, it is not"; exit 1; }
  [ "$partial" = "t" ] \
    || { echo "FAIL uq_split_asset_date must be partial (WHERE type = 'SPLIT'); a full index would constrain every transaction type"; exit 1; }
  case "$pred" in
    *"type = 'SPLIT'"*) ;;
    *) echo "FAIL uq_split_asset_date predicate is '$pred', expected type = 'SPLIT'"; exit 1 ;;
  esac
  case "$def" in
    *"AT TIME ZONE 'UTC'"*::date*) ;;
    *) echo "FAIL uq_split_asset_date must key on the UTC calendar day, (date AT TIME ZONE 'UTC')::date; definition is: $def"; exit 1 ;;
  esac
  case "$def" in
    *"(asset_id,"*) ;;
    *) echo "FAIL uq_split_asset_date must lead with asset_id; definition is: $def"; exit 1 ;;
  esac
  echo "ok   transactions.uq_split_asset_date = unique, partial on SPLIT, keyed on (asset_id, UTC day)"
}

want portfolios         id            bigint
want transactions       id            bigint
want transactions       date          "timestamp with time zone"
want transactions       price         text
want historical_prices  date          date
want historical_prices  close_price   text
want users              is_active     boolean

want_identity users        id
want_identity portfolios   id
want_identity assets       id
want_identity transactions id

# The two keys a query is written against rather than inferred: `split_sync` is upserted
# and deleted by asset_id alone, and price restore upserts on (symbol, date).
want_pk split_sync         "PRIMARY KEY (asset_id)"
want_pk historical_prices  "PRIMARY KEY (symbol, date)"

want_split_dedupe_index

echo "schema shape ok"
