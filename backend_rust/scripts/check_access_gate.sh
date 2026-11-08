#!/usr/bin/env bash
# Access-control gate for the API handlers.
#
# Three static rules, all satisfiable on the intended end state. A gate that fails
# on correct code gets weakened or deleted in the next PR that lands, which is the
# failure mode this script is written to avoid.
#
#   Rule A — every handler file goes through the chokepoint: it imports from
#            `crate::access` and calls one of the require functions.
#   Rule B — every handler whose body resolves a row by `WHERE id` calls
#            `require*` *before* that query, not merely somewhere inside itself.
#   Rule C — every route handler takes the caller (`AuthedUser`).
#
# These are heuristics and stay heuristics. Known limits, stated rather than hidden:
#   * Rule B ends a function at a `}` in column 0, so a `}` inside a block comment
#     or a multi-line string closes the block early.
#   * A query assembled with `format!` is invisible to all three rules.
#   * `analytics.rs`'s `WHERE asset_id IN (SELECT id FROM assets WHERE portfolio_id
#     = ?)` slips past Rule B — the subquery's `WHERE portfolio_id` is not `WHERE id`.
#     That handler calls `require(..., Viewer)` first, and it is the isolation matrix
#     (`tests/isolation_test.rs`), not this script, that is the real gate.
#   * The require functions are imported (`use crate::access::require`), so the
#     patterns match the bare call name as well as the qualified path.
#   * Rule C inspects the signature only (up to the first `->`), so a body comment
#     naming `AuthedUser` no longer satisfies it.
#
# The script prints what it inspected. A gate that silently inspects nothing is worse
# than no gate.

set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

HANDLER_DIR="src/api_routes"
# Files that legitimately never touch portfolios/assets/transactions.
EXEMPT=( "lookup.rs" )

fail=0
rules_b_hits=0

is_exempt() {
  local name="$1"
  for e in "${EXEMPT[@]}"; do
    [ "$name" = "$e" ] && return 0
  done
  return 1
}

mapfile -t FILES < <(find "$HANDLER_DIR" -name '*.rs' | sort)
if [ "${#FILES[@]}" -eq 0 ]; then
  echo "GATE FAILED: no handler files found under $HANDLER_DIR"
  exit 1
fi

# ---------------------------------------------------------------------------
# Rule A — every handler file goes through the chokepoint
# ---------------------------------------------------------------------------
echo "Rule A: every handler file calls the access chokepoint"
files_checked=0
for file in "${FILES[@]}"; do
  name="$(basename "$file")"
  if is_exempt "$name"; then
    echo "  exempt   $name (touches none of portfolios/assets/transactions)"
    continue
  fi
  files_checked=$((files_checked + 1))
  if grep -qE 'use crate::access::' "$file" && grep -qE '(^|[^A-Za-z_])require(_asset(_in)?|_transaction)?\(' "$file"; then
    echo "  ok       $name"
  else
    echo "  MISSING  $name neither imports crate::access nor calls a require function"
    fail=1
  fi
done

# ---------------------------------------------------------------------------
# Rules B and C — per handler function
# ---------------------------------------------------------------------------
echo
echo "Rule B: an id-resolving query in a handler implies a require* call"
echo "Rule C: every route handler takes AuthedUser"

handlers=0
cleared=0
for file in "${FILES[@]}"; do
  # awk walks the file once per handler attribute: from the attribute line to the
  # first `}` at column 0, recording whether the body queries by id, whether it
  # calls require*, and whether the signature names AuthedUser.
  out="$(awk '
    # One pass per file. A handler runs from its route attribute to the first `}` in
    # column 0. `FNR` is used rather than a hand-rolled counter so the reported line
    # is the real one even after the first handler in a file; `options` and the
    # generic `route` attribute are recognised so a handler written with either is
    # not silently invisible to all three rules.
    /^#\[(get|post|put|delete|patch|head|options|route)\(/ {
      if (infn) { printf "MALFORMED line %d: route attribute inside a function body\n", FNR }
      infn = 1; req = 0; idq = 0; authed = 0; sig = ""; start = FNR
      reqline = 0; idline = 0; sigdone = 0
    }
    infn {
      # The signature is everything up to the first `->`. Rule C is about the
      # parameter list: matching `AuthedUser` anywhere in the block would let a
      # comment naming the type satisfy the rule.
      if (!sigdone) {
        sig = sig " " $0
        if ($0 ~ /->/) sigdone = 1
      }
      if ($0 ~ /(^|[^A-Za-z_])require(_asset(_in)?|_transaction)?\(/ && reqline == 0) { req = 1; reqline = FNR }
      if ($0 ~ /(FROM|UPDATE|DELETE FROM)[ \t]+(portfolios|assets|transactions)[ \t].*WHERE[ \t]+id/ && idline == 0) { idq = 1; idline = FNR }
      if (sig ~ /AuthedUser/) authed = 1
      if ($0 ~ /^\}/) {
        printf "%d\t%d\t%d\t%d\t%d\t%d\t%s\n", start, idq, req, authed, idline, reqline, sig
        infn = 0
      }
    }
    END { if (infn) printf "MALFORMED: file ends inside a handler body started at line %d\n", start }
  ' "$file")"

  while IFS=$'\t' read -r line idq req authed idline reqline sig; do
    [ -z "$line" ] && continue
    case "$line" in MALFORMED*) echo "  MALFORMED $file: $line"; fail=1; continue ;; esac
    handlers=$((handlers + 1))
    fn="$(printf '%s' "$sig" | grep -oE 'fn [a-z_]+' | head -1 | awk '{print $2}')"
    [ -z "$fn" ] && fn="(line $line)"

    ok=1
    # Ordering matters. A `require*` that runs *after* the id-resolving query has
    # already read the row, so it guards nothing; presence alone would pass it.
    if [ "$idq" = "1" ] && [ "$req" = "0" ]; then
      echo "  UNGUARDED $file:$line $fn queries a row by id with no require call"
      rules_b_hits=$((rules_b_hits + 1))
      fail=1
      ok=0
    elif [ "$idq" = "1" ] && [ "$reqline" -gt "$idline" ]; then
      echo "  LATE-GUARD $file:$line $fn calls require at line $reqline, after the id query at line $idline"
      rules_b_hits=$((rules_b_hits + 1))
      fail=1
      ok=0
    fi
    if [ "$authed" = "0" ]; then
      echo "  NO-CALLER $file:$line $fn does not take AuthedUser"
      fail=1
      ok=0
    fi
    [ "$ok" = "1" ] && cleared=$((cleared + 1))
  done <<< "$out"
done

echo
echo "handlers inspected: $handlers, cleared: $cleared, files checked: $files_checked"
if [ "$handlers" -eq 0 ]; then
  echo "GATE FAILED: inspected zero handlers — the parser matched nothing"
  exit 1
fi

if [ "$fail" -ne 0 ]; then
  echo "GATE FAILED"
  exit 1
fi
echo "GATE PASSED"
