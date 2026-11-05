#!/usr/bin/env bash
# Headless browser smoke for the sell P&L breakdown.
#
#   ./e2e/run.sh            # from frontend/  (or: npm run e2e:sell-pnl)
#
# Starts the Rust API against a throwaway SQLite file, starts Vite, drives a real
# Chromium through the ledger -> breakdown flow, then tears everything down. The
# throwaway DATABASE_URL means this never reads or writes your real portfolio.db.
#
# Needs: a chromium-compatible browser, node with playwright-core installed
# (npm install in frontend/), and at least one font available to fontconfig.
# On NixOS, run it inside:  nix-shell -p chromium -p liberation_ttf --run './e2e/run.sh'
#
# Env overrides: CHROMIUM, FONT_DIR, BASE_URL, API_URL, SMOKE_SHOT=<path for a screenshot>

set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1
REPO_ROOT="$(cd .. && pwd)"
WORK="$(mktemp -d)"
BACKEND_PID=""
FRONTEND_PID=""

cleanup() {
  [ -n "$FRONTEND_PID" ] && kill "$FRONTEND_PID" 2>/dev/null
  [ -n "$BACKEND_PID" ] && kill "$BACKEND_PID" 2>/dev/null
  wait 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT INT TERM
die() { echo "ERROR: $1" >&2; exit 2; }

# --- prerequisites -----------------------------------------------------------
[ -n "${CHROMIUM:-}" ] || CHROMIUM="$(command -v chromium 2>/dev/null || command -v chromium-browser 2>/dev/null || command -v google-chrome 2>/dev/null || true)"
[ -n "$CHROMIUM" ] || die "no chromium binary found. Set CHROMIUM=/path/to/chromium (on NixOS: nix-shell -p chromium -p liberation_ttf --run './e2e/run.sh')"
export CHROMIUM
[ -x "$CHROMIUM" ] || die "CHROMIUM=$CHROMIUM is not executable"
node --input-type=commonjs -e "require.resolve('playwright-core')" 2>/dev/null || die "playwright-core is not installed. Run: npm install"

# --- fonts -------------------------------------------------------------------
# A browser with no fonts renders zero-width text and the page looks fine except
# that every glyph is missing. Only synthesize a fontconfig file when the host
# really has none.
has_dir() { for d in "$@"; do [ -d "$d" ] && return 0; done; return 1; }
if [ -z "${FONTCONFIG_FILE:-}" ] && ! has_dir /run/current-system/sw/share/fonts /usr/share/fonts; then
  found=()
  for d in "${FONT_DIR:-/nonexistent}" /nix/store/*-liberation-*/share/fonts /nix/store/*-dejavu-*/share/fonts; do
    [ -d "$d" ] && found+=("$d")
  done
  [ ${#found[@]} -gt 0 ] || die "no fonts found for fontconfig. Set FONT_DIR=/path/to/fonts (on NixOS: nix-shell -p liberation_ttf --run './e2e/run.sh')"
  {
    echo '<?xml version="1.0"?>'
    echo '<!DOCTYPE fontconfig SYSTEM "fonts.dtd">'
    echo '<fontconfig>'
    for d in "${found[@]}"; do echo "  <dir>$d</dir>"; done
    echo "  <cachedir>$WORK/fc-cache</cachedir>"
    echo '</fontconfig>'
  } > "$WORK/fonts.conf"
  export FONTCONFIG_FILE="$WORK/fonts.conf"
  echo "fonts  ${found[0]}"
fi

# --- servers -----------------------------------------------------------------
command -v curl >/dev/null || die "curl is required"
DB="$WORK/e2e.db"
export DATABASE_URL="sqlite:$DB?mode=rwc"
export BASE_URL="${BASE_URL:-http://localhost:5173}"
export API_URL="${API_URL:-http://127.0.0.1:8000/api}"

wait_for() { # url, label, tries
  for _ in $(seq 1 "$3"); do
    curl -s -o /dev/null -m 2 "$1" && return 0
    sleep 1
  done
  return 1
}

echo "backend  cargo run (DATABASE_URL=$DATABASE_URL)"
(cd "$REPO_ROOT/backend_rust" && exec cargo run) > "$WORK/backend.log" 2>&1 &
BACKEND_PID=$!
wait_for "http://127.0.0.1:8000/" backend 180 || { tail -20 "$WORK/backend.log"; die "backend did not come up"; }

echo "frontend  vite dev"
(npx vite --port 5173) > "$WORK/vite.log" 2>&1 &
FRONTEND_PID=$!
wait_for "$BASE_URL/" frontend 60 || { tail -20 "$WORK/vite.log"; die "vite did not come up"; }

# --- run ---------------------------------------------------------------------
node e2e/sell-pnl.smoke.mjs
STATUS=$?

[ "$STATUS" -ne 0 ] && [ -n "${SMOKE_SHOT:-}" ] && echo "screenshot: $SMOKE_SHOT" >&2
exit $STATUS
