#!/usr/bin/env bash
# Portability gate for the backup module.
#
# The bundle format is engine-neutral by discipline, not by abstraction: the
# engine-selection work is a separate spec, and this script is what keeps its
# share of the job a mechanical change to `src/backup/` rather than a rewrite.
# A gate that fails on correct code gets deleted in the next PR that lands, so
# the banned set is limited to constructs with no cross-engine spelling. `?`
# placeholders are allowed on purpose: SQLite's syntax today, and what an
# Any-style pool rewrites for Postgres.
#
# The rule list is the spec's *Portability discipline* section, plus the three
# `INSERT OR …` variants the spec's list omits. Two rules, both static:
#   Rule 1 — no banned construct appears in SQL the module can send. Matching is
#     case-insensitive, because SQLite keywords are: `PRAGMA`, `pragma` and
#     `Pragma` are one construct, and the module's house style is upper-case SQL
#     keywords, so a case-sensitive match would miss the spelling the code
#     actually writes.
#   Rule 2 — every `INSERT INTO` names its columns. A bare `INSERT INTO t
#     VALUES (...)` silently changes meaning when a column is added, and the
#     column list must be the thing that follows the table name.
#
# Known limits, stated rather than hidden (same style as `check_access_gate.sh`):
#   * The rules are about SQL the module sends to the engine, not about English.
#     A line whose first non-blank characters are `//`, `//!` or `*` is a comment
#     line and is skipped — the precedent `check_pricing_seam.sh` already sets. A
#     banned token in prose no longer trips this; the rule is about statements,
#     not English. `src/backup/restore.rs` says `PRAGMA foreign_keys` in a comment
#     to explain a schema fact, and that comment is not a portability hazard.
#   * The skip is per whole line, so it is not a hiding place: a banned token on a
#     line that also holds code (`let q = "PRAGMA x"; // note`) still trips Rule 1.
#   * A block comment is only skipped where its body lines start with `*`. One
#     whose body is flush left trips Rule 1. That fails closed, the harmless
#     direction to be wrong.
#   * Rule 1 matches literal text, so a statement assembled at runtime with
#     `format!` from a banned keyword split across pieces is invisible to it — the
#     same limit `check_access_gate.sh` accepts for `format!`-built queries.
#   * Both rules scan `*.rs` only. SQL kept in a `.sql` file and pulled in with
#     `include_str!` would be invisible to both.
#   * Rule 2 reads the statement text joined across lines, because the module's
#     SQL lives in Rust string literals that wrap mid-statement. Matching per line
#     would refuse correct code (the `(` is on the next line) and miss a bare
#     `VALUES` whose table name is on the line above. The join is whole-file, so a
#     column list may wrap as far as it likes; a quoted table name (`INSERT INTO
#     "assets"`) is not recognised and fails closed rather than passing quietly.
#   * `find` matches any path named `*.rs`. One that is not a readable regular
#     file — a directory, a dangling link, a file the test user cannot open — fails
#     the gate twice over: the `-f` test refuses it, and the scanner ends every
#     file with a sentinel carrying its count, so a file `awk` never read yields no
#     sentinel and fails too. Either way it cannot contribute zero statements and
#     let the gate print `passed`.
#
# The script prints what it inspected. A gate that silently inspects nothing is
# worse than no gate.

set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

DIR="src/backup"
if [ ! -d "$DIR" ]; then
  echo "GATE FAILED: $DIR does not exist"
  exit 1
fi

# Matched case-insensitively, so each construct appears once. `pragma_` is kept
# apart from `PRAGMA` because the spec names `pragma_table_info` as its own item.
BANNED=(
  "PRAGMA"
  "pragma_"
  "INSERT OR REPLACE"
  "INSERT OR IGNORE"
  "INSERT OR ROLLBACK"
  "INSERT OR ABORT"
  "INSERT OR FAIL"
  "AUTOINCREMENT"
  "last_insert_rowid"
  "strftime"
  "julianday"
  "datetime("
  "IFNULL"
  "GROUP_CONCAT"
)

fail=0
checked=0
statements=0

# The comment-line skip (`//`, `//!`, `///` and block-comment continuations) is
# the `grep -vE` on the second line of each loop: prose is not SQL.
for pattern in "${BANNED[@]}"; do
  hits=$(grep -rini --include='*.rs' -F "$pattern" "$DIR" \
    | grep -vE ':[0-9]+:[[:space:]]*(//|\*)' || true)
  if [ -n "$hits" ]; then
    echo "GATE FAILED: '$pattern' has no cross-engine spelling:"
    echo "$hits"
    fail=1
  fi
done

# One line per `INSERT INTO` in $1: "<line>\t<verdict>", where the verdict is `ok`
# or the reason the statement is refused, and then a final
# "-\tscanned\t<count>" sentinel. No sentinel means the scanner never finished the
# file, which is a failure rather than an empty file.
#
# The file is joined into one string before matching so a statement that wraps
# across Rust string-literal lines is read as the one statement it is; `start[i]`
# maps a position in the joined text back to the line to report. Backslashes go
# first: a trailing `\` is Rust's line continuation, not part of the SQL.
insert_statements() {
  awk '
    {
      t = $0
      sub(/^[ \t]+/, "", t)
      line[NR] = (t ~ /^(\/\/|\*)/) ? "" : $0
      gsub(/\\/, "", line[NR])
    }
    END {
      joined = ""
      for (i = 1; i <= NR; i++) { start[i] = length(joined) + 1; joined = joined line[i] " " }
      up = toupper(joined)
      pos = 1
      n = 0
      while ((k = index(substr(up, pos), "INSERT INTO")) > 0) {
        at = pos + k - 1
        ln = 1
        for (i = 1; i <= NR; i++) { if (start[i] <= at) ln = i; else break }
        rest = substr(up, at + 11)
        sub(/^[ \t]+/, "", rest)
        n++
        if (match(rest, /^[A-Z_][A-Z0-9_.]*/) == 0) {
          printf "%d\tno-table-name\n", ln
        } else {
          after = substr(rest, RSTART + RLENGTH)
          sub(/^[ \t]+/, "", after)
          if (substr(after, 1, 1) == "(") printf "%d\tok\n", ln
          else printf "%d\tno-column-list\n", ln
        }
        pos = at + 11
      }
      printf "-\tscanned\t%d\n", n
    }
  ' "$1"
}

while IFS= read -r file; do
  checked=$((checked + 1))
  if [ ! -f "$file" ]; then
    # A directory or a dangling link named `*.rs`. `awk` warns and skips such a
    # path while still reporting success, so this has to be said out loud.
    echo "GATE FAILED: $file matches *.rs but is not a readable regular file"
    fail=1
    continue
  fi
  found=-1
  while IFS=$'\t' read -r lineno verdict count; do
    [ -z "$lineno" ] && continue
    if [ "$verdict" = "scanned" ]; then found=$count; continue; fi
    case "$verdict" in
      ok) ;;
      no-table-name)
        echo "GATE FAILED: $file:$lineno INSERT INTO is not followed by a table name"
        fail=1
        ;;
      *)
        echo "GATE FAILED: $file:$lineno INSERT without an explicit column list"
        sed -n "${lineno}s/^[[:space:]]*/  /p" "$file"
        fail=1
        ;;
    esac
  done < <(insert_statements "$file")
  if [ "$found" -lt 0 ]; then
    echo "GATE FAILED: $file: the INSERT scanner produced no verdict for this file"
    fail=1
  else
    echo "  inspected $file: $found INSERT statement(s)"
    statements=$((statements + found))
  fi
done < <(find "$DIR" -name '*.rs' | sort)

if [ "$checked" -eq 0 ]; then
  echo "GATE FAILED: no files inspected under $DIR"
  exit 1
fi

echo "INSERT statements inspected: $statements"

if [ "$fail" -ne 0 ]; then
  echo "Portability gate failed. See the spec's *Portability discipline* section."
  exit 1
fi
echo "Portability gate passed: $checked file(s) inspected under $DIR"
