#!/usr/bin/env bash
# Access-control gate for the API handlers.
#
# Three static rules, all satisfiable on the intended end state. A gate that fails
# on correct code gets weakened or deleted in the next PR that lands, which is the
# failure mode this script is written to avoid.
#
#   Rule A — every handler goes through the chokepoint: its file imports from
#            `crate::access` and calls one of the require functions. One handler
#            that genuinely resolves no caller-supplied id may carry the marker
#            `// access-gate: global-config (no caller-supplied id)` on the line
#            directly above its route attribute. That waives the require half for
#            that handler alone — never for an unmarked neighbour — and it still
#            owes Rule C an `AuthedUser` and Rule B its ordering. The marker is
#            printed whenever it is honoured, so it cannot be added quietly.
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
#   * Rule A's marker is honoured only as the whole line above the route attribute,
#     so it cannot be smuggled into a string literal or a doc comment; a marker in
#     the wrong place fails the gate rather than passing quietly.
#   * Rule A's pass test is still file-level: one `require*` call anywhere in a file
#     satisfies it for every handler in that file. The marker is per handler; the
#     pass branch is not. Rule B is what pins the ordering per handler.
#   * The marker's claim is cross-checked against the route path only, and by shape
#     rather than spelling: any path parameter at all contradicts it, because the
#     claim is "no caller-supplied id", not "no id-named id". A handler that takes an
#     id from a query string, a form field, or a JSON body is invisible to this,
#     exactly as it is to Rule B — the isolation matrix is the real gate.
#   * That check reads the attribute's text, so it fails closed on shape, not
#     meaning: a marked handler with a harmless parameter such as `<tab>` fails, and
#     `#[route(..., data = "<body>")]` fails because the data generic also uses angle
#     brackets. Failing closed is the right way to be wrong about a security gate.
#   * The attribute is read to its closing `)]`, at most nine lines, so a wrapped
#     attribute is still seen. An attribute wrapping further than that hides its
#     parameters, which is the same limit Rule B accepts with per-line SQL matching.
#
# The script prints what it inspected. A gate that silently inspects nothing is worse
# than no gate.

set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

HANDLER_DIR="src/api_routes"
# Files that legitimately never touch portfolios/assets/transactions.
EXEMPT=( "lookup.rs" )

# The whole-line comment a handler carries when it resolves no caller-supplied id
# and so has nothing to send to the chokepoint. Honoured per handler, never per
# file. Kept as one string so the script has a single spelling of it.
MARKER='// access-gate: global-config (no caller-supplied id)'

fail=0
rules_b_hits=0

is_exempt() {
  local name="$1"
  for e in "${EXEMPT[@]}"; do
    [ "$name" = "$e" ] && return 0
  done
  return 1
}

# One line per route handler in $1: "<attribute line>\t<1 if the marker is
# honoured>\t<1 if the marker contradicts the route>\t<fn name>". The marker is
# honoured only when it is the whole previous line, so it cannot hide inside a
# string or a doc comment, and it is refused outright when the route takes any path
# parameter — the marker's whole claim is that the caller supplied nothing to check.
handlers_in() {
  awk -v marker="$MARKER" '
    { line[NR] = $0 }
    END {
      for (i = 1; i <= NR; i++) {
        if (line[i] !~ /^#\[(get|post|put|delete|patch|head|options|route)\(/) continue
        prev = (i > 1 ? line[i - 1] : "")
        sub(/^[ \t]+/, "", prev); sub(/[ \t]+$/, "", prev)
        marked = (prev == marker) ? 1 : 0
        # Any path parameter contradicts the claim: `<portfolio>` is as much
        # caller-supplied as `<portfolio_id>`, and matching the spelling `id` is what
        # let `<key>` and `<target>` through. The attribute may wrap, so read on to
        # its closing `)]` instead of testing only the line that opened it.
        attr = ""
        for (k = i; k <= NR && k <= i + 8; k++) {
          attr = attr " " tolower(line[k])
          if (line[k] ~ /\)\]/) break
        }
        contradict = (marked && attr ~ /</) ? 1 : 0
        if (contradict) marked = 0
        fn = "(unknown)"
        for (j = i; j <= NR && j <= i + 40; j++) {
          if (match(line[j], /fn [A-Za-z_0-9]+/)) { fn = substr(line[j], RSTART + 3, RLENGTH - 3); break }
        }
        printf "%d\t%d\t%d\t%s\n", i, marked, contradict, fn
      }
    }
  ' "$1"
}

mapfile -t FILES < <(find "$HANDLER_DIR" -name '*.rs' | sort)
if [ "${#FILES[@]}" -eq 0 ]; then
  echo "GATE FAILED: no handler files found under $HANDLER_DIR"
  exit 1
fi

# ---------------------------------------------------------------------------
# Rule A — every handler goes through the chokepoint, or names why it has none
# ---------------------------------------------------------------------------
echo "Rule A: every handler goes through the access chokepoint, or carries the marker saying why it has nothing to check"
files_checked=0
for file in "${FILES[@]}"; do
  name="$(basename "$file")"
  if is_exempt "$name"; then
    echo "  exempt   $name (touches none of portfolios/assets/transactions)"
    continue
  fi
  files_checked=$((files_checked + 1))

  handlers="$(handlers_in "$file")"
  total=0
  unmarked=0
  contradicted=0
  # Waived handlers are printed whether or not the file also needs the chokepoint,
  # so a marker always shows up in the gate's output and in a diff review.
  while IFS=$'\t' read -r hline hmarked hcontra hfn; do
    [ -z "$hline" ] && continue
    total=$((total + 1))
    if [ "$hmarked" = "1" ]; then
      echo "  waived   $name:$hline $hfn ($MARKER — no caller-supplied id; Rules B and C still apply)"
    elif [ "$hcontra" = "1" ]; then
      # The marker says "no caller-supplied id" and the route path says otherwise.
      # Refused even in a file that calls the chokepoint elsewhere: a false claim is
      # the thing to surface, not something a neighbour's `require` can paper over.
      echo "  MARKER-CONTRADICTS $name:$hline $hfn (the route takes a path parameter, so the marker's claim is false)"
      contradicted=$((contradicted + 1))
      fail=1
    else
      unmarked=$((unmarked + 1))
    fi
  done <<< "$handlers"

  if grep -qE 'use crate::access::' "$file" && grep -qE '(^|[^A-Za-z_])require(_asset(_in)?|_transaction)?\(' "$file"; then
    echo "  ok       $name"
  elif [ "$total" -gt 0 ] && [ "$unmarked" -eq 0 ] && [ "$contradicted" -eq 0 ]; then
    echo "  ok       $name (every handler carries the global-config marker: none resolves a caller-supplied id)"
  else
    # Name each handler that neither calls the chokepoint nor carries the marker.
    # A message that only named the file would hide which handler regressed, and a
    # marker on one handler must never cover an unmarked neighbour.
    while IFS=$'\t' read -r hline hmarked hcontra hfn; do
      [ -z "$hline" ] && continue
      [ "$hmarked" = "1" ] && continue
      [ "$hcontra" = "1" ] && continue   # already reported above
      echo "  NO-CHOKEPOINT $name:$hline $hfn calls no require* and carries no access-gate marker"
      fail=1
    done <<< "$handlers"
    if [ "$total" -eq 0 ]; then
      echo "  MISSING  $name neither imports crate::access nor calls a require function, and holds no route handler to inspect"
      fail=1
    fi
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
