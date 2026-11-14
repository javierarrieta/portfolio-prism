#!/usr/bin/env bash
# Local PostgreSQL for development and tests. Postgres-only since the engine
# switch: there is no SQLite path to fall back to.
#
# nix-shell rather than a container because rootless podman cannot create a
# usable user namespace in this workspace (spec §9.2: the kernel refuses a
# multi-range uid_map even with the setuid helper installed). nixpkgs'
# postgresql_18 is 18.7; production is 18.6. Same major, and nothing in the
# schema depends on the difference.
set -euo pipefail
PORT="${PRISM_PG_PORT:-55432}"
DATA="${PRISM_PG_DIR:-${XDG_RUNTIME_DIR:-/tmp}/prism-pg}"

if [ -s "$DATA/PG_VERSION" ] && pg_ctl -D "$DATA" status >/dev/null 2>&1; then
  echo "already running on 127.0.0.1:$PORT (data: $DATA)"
else
  command -v initdb >/dev/null || { echo "initdb not on PATH — run inside: nix-shell -p postgresql_18 --run 'bash scripts/dev-db.sh'"; exit 1; }
  [ -s "$DATA/PG_VERSION" ] || { mkdir -p "$DATA"; initdb -D "$DATA" -U postgres -A trust --no-sync; }
  mkdir -p "$DATA/sock"
  pg_ctl -D "$DATA" -o "-p $PORT -k $DATA/sock -c listen_addresses=127.0.0.1" \
         -l "$DATA/log" -w start
fi

psql "postgres://postgres@127.0.0.1:$PORT/postgres" -qc "alter role postgres createdb"
psql "postgres://postgres@127.0.0.1:$PORT/postgres" -tAc \
  "select 1 from pg_database where datname='prism'" | grep -q 1 \
  || psql "postgres://postgres@127.0.0.1:$PORT/postgres" -qc "create database prism"

echo "DATABASE_URL=postgres://postgres@127.0.0.1:$PORT/prism"
