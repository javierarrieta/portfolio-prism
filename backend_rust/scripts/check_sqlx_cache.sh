#!/usr/bin/env bash
# Prepare-cache freshness gate: `.sqlx/` must describe the schema the migrations
# actually build.
#
# The committed prepare cache is what lets `cargo check` compile 94 `sqlx::query!`
# macros with no database in reach. That is also its failure mode: the cache is a
# snapshot of a schema, and a stale snapshot compiles happily while lying about the
# columns. `cargo build` would stay green against a schema nobody runs, and the whole
# point of compile-time checked queries — the `BIGINT`-into-`i32` class this port
# exists to kill — is quietly undone. Spec §8.
#
# Two steps, in this order, and the order is load-bearing:
#
#   1. `sqlx migrate run`. `cargo sqlx prepare --check` describes the **live**
#      database, not `migrations/`. Point it at a database that is one migration
#      behind and every cached query still matches the schema the server has, so the
#      gate passes vacuously — on exactly the drift it exists to catch. Migrating
#      first is what makes step 2 compare the cache against the schema head.
#   2. `cargo sqlx prepare --check -- --all-targets`. `--all-targets` because a
#      third of the macros live in `#[cfg(test)]` code (`src/test_db.rs` is exempt,
#      but `src/access.rs`, `src/engines/*` and `src/services/currency_service.rs`
#      are not), and a gate that only checked `src/` would let the test half of the
#      cache rot.
#
# It also prints the `macro-exempt` residue — the count of `// macro-exempt:` markers
# in `src/`, i.e. the queries that are still runtime SQL on purpose. The point of
# printing it is that the number has to be *changed on purpose*: the residue is meant
# to reach zero deliberately rather than by accident, and an unreported count drifts
# upward one careless conversion at a time. `tests/sqlx_cache_test.rs` pins the number,
# so adding an exemption is an edit someone has to justify in a test.
#
# One direction this gate does NOT close, stated because the rest of the header claims
# the cache is honest: `cargo sqlx prepare --check` only *warns* when `.sqlx` holds
# entries that nothing regenerates any more (`potentially unused queries found in
# .sqlx`), and the warning does not change its exit status. So the cache can accumulate
# dead entries and the gate stays green. That is upstream `sqlx-cli` behaviour, not a
# choice here — `prepare_check` (`sqlx-cli-0.9.0/src/prepare.rs:111-147`) bails on a
# missing or differing entry and merely `println!`s the extra-entry case. The script
# surfaces the warning as a `NOTE:` line so it is not silently swallowed, but does not
# fail on it: pruning a cache entry is a judgement call, and a gate that fails on
# cosmetics gets deleted. To prune, delete `.sqlx/` and re-run
# `cargo sqlx prepare -- --all-targets`.
#
# What a real failure looks like, recorded verbatim from the two mutations at the
# bottom of this comment.
#
# (A) Schema drift — `ALTER TABLE assets ADD COLUMN drift_note TEXT` on the live
# database, nothing regenerated. Nine macros read `assets` with `SELECT *` into
# `models::Asset`, so the drift is a compile error and the gate names every call
# site and the SQL itself:
#
#   GATE FAILED — .sqlx does not describe the schema at head.
#         Checking backend_rust v0.1.0 (.../backend_rust)
#     error[E0560]: struct `Asset` has no field named `drift_note`
#        --> src/api_routes/portfolios.rs:135:18
#         |
#     135 |     let assets = sqlx::query_as!(DbAsset, "SELECT * FROM assets WHERE portfolio_id = $1", portfolio_id)
#         |                  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ `Asset` does not have this field
#       ...
#     error: could not compile `backend_rust` (lib) due to 9 previous errors
#     error: `cargo check` failed with status: exit status: 101
#
# (B) A stale cache with no compile error — one `.sqlx/query-*.json` edited to claim
# `portfolios.base_currency` is `Int8`. The code still compiles against the live
# database, which is exactly the case that would otherwise pass silently, and the
# only thing left to compare against is the cache:
#
#   GATE FAILED — .sqlx does not describe the schema at head.
#     error: prepare check failed: one or more query files differ (query-486ac57139015fcd22dccf11253aba3d6b2cc93732193c5843440737d5103e67.json); you should re-run sqlx prepare
#     drifted query (query-486ac57139015fcd22dccf11253aba3d6b2cc93732193c5843440737d5103e67.json):
#       "SELECT id, name, description, currency, base_currency FROM portfolios WHERE id = $1"
#
# `sqlx-cli`'s `bail!` names only the cache *file*, so the script prints the `query`
# field of every named file too — a sha256 nobody can read is not a failure message.
# Regenerate with:
#
#   DATABASE_URL=... cargo sqlx prepare -- --all-targets
#
# and commit `.sqlx/` in the same commit as the migration that made it stale.
#
# The two mutations that prove this gate can fail, and how each was reverted:
#   (A) `ALTER TABLE assets ADD COLUMN drift_note TEXT`, reverted with
#       `ALTER TABLE assets DROP COLUMN drift_note`.
#   (B) edit `describe.columns[4].type_info` in one `.sqlx/query-*.json`, reverted by
#       restoring the file.
#
# `sqlx-cli` is not part of the toolchain: install it with
#   cargo install sqlx-cli --no-default-features --features postgres --locked

set -euo pipefail
cd "$(dirname "$0")/.."

: "${DATABASE_URL:?run backend_rust/scripts/dev-db.sh and export DATABASE_URL}"

if ! command -v sqlx >/dev/null 2>&1 || ! command -v cargo-sqlx >/dev/null 2>&1; then
  echo "GATE FAILED — sqlx-cli not on PATH."
  echo "  install it with: cargo install sqlx-cli --no-default-features --features postgres --locked"
  echo "  (a gate that skips because its tool is missing is a gate that always passes)"
  exit 1
fi

echo "database: ${DATABASE_URL%%\?*}"
echo "migrations: $(ls migrations/*.sql | wc -l | tr -d ' ') file(s) on disk"
# The residue. Counted from the marker lines themselves, anchored so the module doc in
# `src/test_db.rs` that *describes* the marker does not count as one: `//!` never
# matches `^ *// macro-exempt:`.
residue=$( { grep -rn '^[[:space:]]*// macro-exempt:' src --include='*.rs' 2>/dev/null || true; } \
  | wc -l | tr -d ' ')
echo "macro-exempt runtime queries in src/: ${residue:-0}"

# Step 1. Without this the comparison below is against whatever schema happens to be
# attached, and a cache generated against an older schema passes.
sqlx migrate run --no-dotenv

# Step 2. `--check` re-describes every macro against the live database and compares
# the result to `.sqlx/`; it forces the macros to re-run rather than trusting a warm
# target directory.
if out=$(cargo sqlx prepare --check -- --all-targets 2>&1); then
  echo "cached queries: $(ls .sqlx/query-*.json | wc -l | tr -d ' ')"
  # `prepare --check` exits 0 for this; see the header for why the gate does too.
  if printf '%s\n' "$out" | grep -q 'potentially unused queries'; then
    echo "NOTE: sqlx-cli warned and still exited 0 — .sqlx holds entries nothing regenerates."
    printf '%s\n' "$out" | grep 'potentially unused queries' | sed 's/^/      /'
    echo "      Prune by deleting .sqlx/ and re-running: cargo sqlx prepare -- --all-targets"
  fi
  echo "GATE PASSED"
  exit 0
fi

echo "GATE FAILED — .sqlx does not describe the schema at head."
printf '%s\n' "$out" | sed 's/^/  /'

# `sqlx-cli` names the cache file, not the SQL. Print the SQL: the point of the gate
# is to tell someone *which* query moved, and a sha256 does not.
for name in $(printf '%s\n' "$out" \
  | grep -oE 'query-[0-9a-f]+\.json' | sort -u); do
  if [ -f ".sqlx/$name" ]; then
    echo "  drifted query ($name):"
    grep -m1 '"query":' ".sqlx/$name" | sed 's/^ *"query": */    /; s/,$//'
  fi
done

exit 1
