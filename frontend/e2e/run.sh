#!/usr/bin/env bash
# Headless browser smoke harness.
#
#   ./e2e/run.sh [target]   # default sell-pnl.smoke.mjs; npm run e2e:{sell-pnl,statement}
#
# Starts the Rust API against a throwaway SQLite file, starts Vite, drives a real
# Chromium through whichever flow the target drives, then tears everything down. The
# throwaway DATABASE_URL means this never reads or writes your real portfolio.db.
#
# Needs: a chromium-compatible browser, node with playwright-core installed
# (npm install in frontend/), and at least one font available to fontconfig.
# On NixOS, run it inside:  nix-shell -p chromium -p liberation_ttf --run './e2e/run.sh'
#
# Env overrides: CHROMIUM, FONT_DIR, BASE_URL, API_URL, SMOKE_SHOT=<path for a screenshot>,
# SMOKE_LOG_DIR=<directory to copy backend.log and vite.log into before they are deleted>

set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1
# ${1:-sell-pnl.smoke.mjs}, not $1: this script runs under `set -uo pipefail` above, so a bare $1
# aborts the script when it is called with no argument -- which is how npm run e2e:sell-pnl calls it.
TARGET="${1:-sell-pnl.smoke.mjs}"
REPO_ROOT="$(cd .. && pwd)"
WORK="$(mktemp -d)"
BACKEND_PID=""
BACKEND_PGID=""
FRONTEND_PID=""
FRONTEND_PGID=""

cleanup() {
  # Same reasoning as the backend: npx spawns vite, which spawns esbuild, and a
  # plain kill of the wrapper leaves the dev server holding :5173.
  [ -n "$FRONTEND_PGID" ] && kill -- -"$FRONTEND_PGID" 2>/dev/null
  [ -n "$FRONTEND_PID" ] && kill "$FRONTEND_PID" 2>/dev/null
  # `cargo run` forks the compiled binary, so killing the cargo PID leaves the
  # binary alive and bound to :8000 -- verified: after `kill $BACKEND_PID` the port
  # still answered 200. Kill the whole process group instead of pattern-matching a
  # name, which would also hit a backend the operator runs in another shell.
  [ -n "$BACKEND_PGID" ] && kill -- -"$BACKEND_PGID" 2>/dev/null
  wait 2>/dev/null
  # backend.log and vite.log live in $WORK and $WORK is deleted on the next line, so without this
  # the only trace of a red run is the 20-line tail printed to stdout. SMOKE_LOG_DIR copies them
  # out first, which is what makes a CI failure debuggable somewhere you cannot reproduce it.
  # Copied on success too: "it looked like a green run until it wasn't" is exactly the case where
  # you want the log that was already deleted. Best-effort -- a missing or read-only target must
  # not turn a passing smoke into a failing one.
  if [ -n "${SMOKE_LOG_DIR:-}" ]; then
    if mkdir -p "$SMOKE_LOG_DIR" 2>/dev/null; then
      cp "$WORK/backend.log" "$SMOKE_LOG_DIR/backend.log" 2>/dev/null
      cp "$WORK/vite.log" "$SMOKE_LOG_DIR/vite.log" 2>/dev/null
      echo "logs  $SMOKE_LOG_DIR"
    fi
  fi
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
# Without this the server defaults to OIDC, refuses to launch, and `wait_for backend`
# burns its whole 420-second budget -- which is why this harness stopped starting. Safe
# only because run.sh builds debug: `disabled` is compiled out of release builds
# (backend_rust/src/auth/mod.rs:96-101). start.sh:21 does exactly the same.
export AUTH_MODE=disabled
export BASE_URL="${BASE_URL:-http://localhost:5173}"
export API_URL="${API_URL:-http://127.0.0.1:8000/api}"

wait_for() { # url, label, tries
  for _ in $(seq 1 "$3"); do
    curl -s -o /dev/null -m 2 "$1" && return 0
    sleep 1
  done
  return 1
}

# Fail fast rather than driving someone else's server: this smoke seeds a throwaway
# database and needs to own both ports.
curl -s -o /dev/null -m 2 "http://127.0.0.1:8000/" && die "something already answers on :8000 -- stop it first"
curl -s -o /dev/null -m 2 "$BASE_URL/" && die "something already answers on $BASE_URL -- stop it first"

echo "backend  cargo run (DATABASE_URL=$DATABASE_URL)"
# setsid puts cargo and the binary it spawns in their own process group so cleanup
# can take both down with one signal.
(cd "$REPO_ROOT/backend_rust" && exec setsid cargo run) > "$WORK/backend.log" 2>&1 &
BACKEND_PGID=$!
BACKEND_PID=$BACKEND_PGID
# Cold `cargo run` on a fresh target/ has been measured well past three minutes for
# this crate; a shorter budget fails the smoke for a reason that has nothing to do
# with the feature under test.
wait_for "http://127.0.0.1:8000/" backend 420 || { tail -20 "$WORK/backend.log"; die "backend did not come up"; }

echo "frontend  vite dev"
# --strictPort: vite silently increments the port when 5173 is busy, and BASE_URL
# would then point at whatever else is listening there.
# setsid for the same reason as the backend -- see cleanup().
(exec setsid npx vite --port 5173 --strictPort) > "$WORK/vite.log" 2>&1 &
FRONTEND_PGID=$!
FRONTEND_PID=$FRONTEND_PGID
wait_for "$BASE_URL/" frontend 60 || { tail -20 "$WORK/vite.log"; die "vite did not come up"; }

# --- run ---------------------------------------------------------------------
node "e2e/${TARGET}"
STATUS=$?

[ "$STATUS" -ne 0 ] && [ -n "${SMOKE_SHOT:-}" ] && echo "screenshot: $SMOKE_SHOT" >&2
exit $STATUS
