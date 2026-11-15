#!/usr/bin/env bash
# Pre-cutover orphan audit — run this against the **SQLite** file, before the export.
#
# STOP CONDITION. The four counts in the orphan section below are each either 0 or a
# stop condition, and the script exits non-zero when any of them is non-zero, so the
# runbook only has to read the exit status. That is the whole point of the exit code: a
# report the operator has to read line by line will be read line by line exactly once,
# and a number that has to be *noticed* is a number that will be missed. Spec §5.1; this
# is the stop condition the cutover runbook branches on. The row counts printed after
# the orphan section are context for the before-and-after comparison, never a stop.
#
# Exit codes, because the runbook branches on them and they mean different things:
#   0  no orphans. Export, and cut over.
#   1  orphans found. STOP. Repair the SQLite file, re-run, proceed only on 0.
#   2  the audit could not run — no argument, no sqlite3, an unreadable file, a file that
#      is not a database, or a database that is not the schema this app writes. Also a
#      stop, but a different one: the operator pointed the script at the wrong thing,
#      which is not the same as the data being bad, and must not be fixed by
#      "cleaning" data.
#
# Why SQLite and not Postgres: the orphans are in the source, and only here. The bundle
# is nested — `build_in` reads portfolios by id, then `assets WHERE portfolio_id = $1`,
# then `transactions WHERE asset_id = $1` (`src/backup/export.rs:131,233,244`) — so a
# row whose parent is gone is under no parent, is never read, and is never mentioned.
# Nothing in the export path can report it, because from the export's point of view
# those rows do not exist. And Postgres will refuse to accept them, so the loss is
# silent at both ends: not exported, not rejected. This script is the only thing that
# sees them, and it has to run *before* the export, while the rows are still there to
# find. After the cutover there is nothing left to audit — the orphans will simply be
# absent, and indistinguishable from never having existed.
#
# The four relationships are the four foreign keys `migrations/0001_init.sql` declares.
# Two of them (`assets.portfolio_id`, `transactions.asset_id`) were already enforced
# under SQLite, because sqlx-sqlite sets `foreign_keys=ON` per connection by default;
# the two that are new at cutover are `portfolios.owner_id` — SQLite added the column
# with a bare `ALTER TABLE`, which cannot carry a foreign key — and `split_sync.asset_id`,
# whose SQLite table declared no foreign key at all. Expect findings on those two.
# `tests/foreign_keys_test.rs` pins what Postgres does with all four once they arrive.
#
# `portfolios with no owner` is printed but is NOT part of the exit code, and that is
# deliberate. A NULL `owner_id` is not a dangling reference; it is a row no user can
# see, and `ensure_bootstrap_owner`/`backfill_ownership` already has an explicit rule
# for it at startup — hand it to the placeholder owner, or refuse to start. It is
# printed because it belongs to the same class of "the export will not carry this", and
# it is not counted because making it an orphan would give exit 1 two different
# meanings, and the runbook would have to unbundle them again by hand.
#
# Usage: bash backend_rust/scripts/pre_cutover_audit.sh /path/to/portfolio.db
# Set SQLITE3=/path/to/sqlite3 to use a build other than the one on PATH.

set -uo pipefail

# The usage error is exit 2, not the shell's default 1 for `${1?}`: exit 1 means
# exactly one thing here — orphans found — and a runbook that branches on the code must
# not read a typo in the command line as a data problem.
if [ $# -lt 1 ] || [ -z "$1" ]; then
  printf 'usage: pre_cutover_audit.sh /path/to/portfolio.db\n' >&2
  exit 2
fi
DB="$1"
SQLITE3="${SQLITE3:-sqlite3}"
SHOW=20

die() { printf 'AUDIT ERROR: %s\n' "$1" >&2; exit 2; }

command -v "$SQLITE3" >/dev/null 2>&1 \
  || die "sqlite3 is not on PATH, and this audit cannot be skipped. Install it (apt install sqlite3, brew install sqlite) or set SQLITE3=/path/to/sqlite3. The audit runs against the SQLite file because that is where the orphans are; there is no Postgres query that can find rows Postgres will not accept."
[ -e "$DB" ] || die "$DB does not exist. Point this at the SQLite file the running app uses — in development that is portfolio.db in the repository root, in production the file mounted at /app/data/portfolio.db — not at the Postgres database."
[ -f "$DB" ] || die "$DB is not a regular file."
[ -r "$DB" ] || die "$DB is not readable by $(id -un). Run the audit as the user that can read it."

# One integer answer on stdout, or a non-zero return after saying why on stderr.
#
# `-cmd '.bail on'` makes sqlite3 exit non-zero on the first error instead of printing
# to stderr and carrying on; without it a broken query would print an empty count and
# the audit would report "0 orphans" about a database it never actually read.
#
# `scalar` cannot exit the script by itself: it is always called in a command
# substitution, and `exit` there only leaves the subshell. So it returns 1 and every
# call site has to turn that into `exit 2` in the *current* shell. A call site that
# forgets does not fail quietly into a zero — the substitution yields the empty string
# and the `[ "$n" -gt 0 ]`/`case` guards refuse it — but the honest rule is: never call
# `scalar` without `|| exit 2` on the assignment.
scalar() {
  local out
  # `.timeout 5000`: the sqlite3 CLI has no busy timeout by default, so a read that
  # lands inside a writer's EXCLUSIVE window on the live rollback-journal file fails
  # immediately with SQLITE_BUSY and the stop condition would report "could not run".
  # The app itself runs with a 5 s busy_timeout (src/backup/export.rs).
  if ! out=$("$SQLITE3" -cmd '.bail on' -cmd '.timeout 5000' "$DB" "$1" 2>&1); then
    printf 'AUDIT ERROR: query failed against %s: %s\n' "$DB" "$out" >&2
    return 1
  fi
  case "$out" in
    '' | *[!0-9]*)
      printf 'AUDIT ERROR: expected one integer from %s, got: %s (query: %s)\n' "$DB" "$out" "$1" >&2
      return 1
      ;;
  esac
  printf '%s\n' "$out"
}

# The schema this app writes. Every one of these is created unconditionally at startup
# by `init_db` (`src/lib.rs` at the merge base), so a file missing any of them was never
# opened by the Rust backend — a Python-era file, a truncated copy, or the wrong file
# entirely. Auditing the wrong file and printing "0 orphans" is the failure mode this
# script most needs to avoid, so it is an error rather than a zero.
for table in portfolios assets transactions users split_sync; do
  n=$(scalar "select count(*) from sqlite_master where type = 'table' and name = '$table'") || exit 2
  [ "$n" = 1 ] || die "$DB has no '$table' table, so it is not a portfolio-prism SQLite database and this audit cannot answer anything about it."
done
n=$(scalar "select count(*) from pragma_table_info('portfolios') where name = 'owner_id'") || exit 2
[ "$n" = 1 ] || die "$DB's portfolios table has no owner_id column, so it predates the ownership work and has never been opened by a backend that added it. The CURRENT backend cannot add it: it is Postgres-only and will not open a SQLite file at all. Run the PRE-CUTOVER image — the last build that still speaks SQLite — against a COPY of this file once; that adds the column and backfills owners. Then audit the copy. Never do this to the original: the backfill writes."

found=0

# check <label> <count sql> <detail sql>
#
# The detail query is the remediation list, printed only when the count is non-zero and
# capped at $SHOW rows, because the operator needs the first twenty ids to start from,
# not a thousand-line dump that gets paged past. The count is always the true count.
#
# `check` runs in the current shell — it is called as a plain command, never in a
# pipeline or a substitution — which is what lets `found` survive it and `exit 2` work.
check() {
  local label="$1" count_sql="$2" detail_sql="$3" n
  n=$(scalar "$count_sql") || exit 2
  printf '%-28s: %s\n' "$label" "$n"
  [ "$n" -gt 0 ] || return 0
  found=1
  if [ "$n" -gt "$SHOW" ]; then
    printf '    %s total, first %s shown\n' "$n" "$SHOW"
  fi
  if ! "$SQLITE3" -cmd '.bail on' -cmd '.timeout 5000' "$DB" "$detail_sql limit $SHOW" 2>&1 | sed 's/^/    /'; then
    # Orphans are already known to exist here, so this is a stop for the same reason the
    # count is: exit 1, not 2. Exit 2 means "the audit could not run", and the runbook is
    # allowed to read that as a clean file that merely failed to be inspected.
    printf 'AUDIT ERROR: the detail query for %s failed against %s (the count above is still the real one)\n' "$label" "$DB" >&2
    exit 1
  fi
}

printf 'orphan audit of %s\n\n' "$DB"

check 'orphan transactions' \
  "select count(*) from transactions t left join assets a on a.id = t.asset_id where a.id is null" \
  "select 'transaction ' || t.id || ' points at asset ' || t.asset_id
     from transactions t left join assets a on a.id = t.asset_id
    where a.id is null order by t.id"

check 'orphan assets' \
  "select count(*) from assets a left join portfolios p on p.id = a.portfolio_id where p.id is null" \
  "select 'asset ' || a.id || ' (' || a.symbol || ') points at portfolio ' || a.portfolio_id
     from assets a left join portfolios p on p.id = a.portfolio_id
    where p.id is null order by a.id"

check 'portfolios, owner missing' \
  "select count(*) from portfolios p left join users u on u.id = p.owner_id where p.owner_id is not null and u.id is null" \
  "select 'portfolio ' || p.id || ' (' || p.name || ') names user ' || p.owner_id
     from portfolios p left join users u on u.id = p.owner_id
    where p.owner_id is not null and u.id is null order by p.id"

check 'orphan split_sync rows' \
  "select count(*) from split_sync s left join assets a on a.id = s.asset_id where a.id is null" \
  "select 'split_sync row for asset ' || s.asset_id || ' has no asset'
     from split_sync s left join assets a on a.id = s.asset_id
    where a.id is null order by s.asset_id"

# Informational for the exit code — see the header — but not harmless: an unowned
# portfolio is exactly as unexportable as an orphan, and the message at the bottom says so.
n=$(scalar 'select count(*) from portfolios where owner_id is null') || exit 2
unowned="$n"
printf '%-28s: %s   (informational, not a stop condition)\n' 'portfolios, no owner' "$n"

printf '\nrow counts, for the before-and-after comparison in the runbook\n'
for table in users portfolios assets transactions historical_prices split_sync; do
  n=$(scalar "select count(*) from $table") || exit 2
  printf '%-28s: %s\n' "$table" "$n"
done

printf '\n'
if [ "$found" -ne 0 ]; then
  printf 'STOP. Orphans found. The export will not carry these rows and the restore will\nnot reject them, so nobody downstream will ever learn they were missed. Repair\nthem in SQLite — reparent each row, or confirm it is meant to be gone and delete it\n— then re-run this script and proceed only on exit 0.\n'
  exit 1
fi
if [ "$unowned" -ne 0 ]; then
  printf "No orphans, but %s portfolio(s) have no owner and are still not exportable.\nThe export carries only portfolios the caller is authorised for, and an unowned row is\nnobody's (src/access.rs denies it to every user). Assign them an owner in SQLite\nfirst - the startup backfill runs on the Postgres target, after the export that never\ncarried the row.\n" "$unowned"
else
  printf 'No orphans. Safe to export.\n'
fi
