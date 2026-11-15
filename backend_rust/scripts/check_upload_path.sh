#!/usr/bin/env bash
# Upload-path gate: the proxy in front of the API must never be narrower than the
# API's own bundle cap, and the panel must refuse an oversized file locally.
#
# Two rules, both born of a real failure. A bundle restore posts the whole file as
# the request body. `MAX_BUNDLE_BYTES` in `src/backup/mod.rs` is the cap the API
# advertises and enforces, and the route opens the body with that limit per-route
# rather than through a global `Limits` entry. The frontend container's nginx
# carried no `client_max_body_size`, so it applied its 1 MiB default and answered
# 413 with an HTML page before the backend saw a byte. The app's own cap was 8 MiB,
# so the app could export a bundle its own UI could not re-import, and the panel
# could only report "answered 413 without a reason this app could read" — which was
# true, and useless.
#
# Rule 1 is the pairing: nginx must allow at least `MAX_BUNDLE_BYTES`. Raising the
# Rust constant alone re-breaks this silently, and nothing in the Rust tree would
# notice. Rule 2 keeps the panel's local refusal in place, so an oversized file gets
# a sentence naming the file and its size instead of whatever a gateway answers with.
#
# Prints what it inspected. A gate that silently inspects nothing is worse than no
# gate — same rule as `check_access_gate.sh`.

set -uo pipefail
cd "$(dirname "$0")/.."
ROOT="$(cd .. && pwd)"

RUST_SRC="$ROOT/backend_rust/src/backup/mod.rs"
NGINX="$ROOT/frontend/nginx.conf"
PANEL="$ROOT/frontend/src/components/BackupPanel.tsx"

for f in "$RUST_SRC" "$NGINX" "$PANEL"; do
  if [ ! -f "$f" ]; then
    echo "GATE FAILED — ${f#$ROOT/} is missing; the layout changed under this script"
    exit 1
  fi
done

fail() { echo "GATE FAILED — $*"; exit 1; }

# --- the cap the API advertises -------------------------------------------------
expr=$(sed -n 's/^pub const MAX_BUNDLE_BYTES: *usize *= *\(.*\);$/\1/p' "$RUST_SRC" | head -1)
[ -n "$expr" ] || fail "no \`pub const MAX_BUNDLE_BYTES: usize = ...\` in backend_rust/src/backup/mod.rs"
case "$expr" in
  *[!0-9\ *]*) fail "MAX_BUNDLE_BYTES expression is not arithmetic: \`$expr\`" ;;
esac
max_bytes=$((expr))
[ "$max_bytes" -gt 0 ] || fail "MAX_BUNDLE_BYTES evaluates to $max_bytes"

# --- the cap the proxy allows ---------------------------------------------------
# nginx sizes are a number with an optional k/m/g suffix, and a bare number is bytes.
nginx_bytes() {
  local v
  v=$(echo "$1" | tr -d ' ;' | tr 'KMGT' 'kmgt')
  case "$v" in
    *k) echo $(( ${v%k} * 1024 )) ;;
    *m) echo $(( ${v%m} * 1024 * 1024 )) ;;
    *g) echo $(( ${v%g} * 1024 * 1024 * 1024 )) ;;
    *)  echo "$v" ;;
  esac
}

# Where the directive must live to reach /api: inside `location /api`, or at server
# level above every location. A directive parked in some other location block would
# satisfy a file-wide grep while doing nothing for the upload path.
api_start=$(grep -n 'location /api' "$NGINX" | head -1 | cut -d: -f1)
[ -n "$api_start" ] || fail "no \`location /api\` block in frontend/nginx.conf"
api_end=$(awk -v s="$api_start" 'NR>s && /^[[:space:]]*}[[:space:]]*$/ {print NR; exit}' "$NGINX")
[ -n "$api_end" ] || fail "the \`location /api\` block in frontend/nginx.conf never closes"
server_level=$(sed -n "1,$((api_start - 1))p" "$NGINX")
api_block=$(sed -n "${api_start},${api_end}p" "$NGINX")

directive=$(printf '%s\n%s\n' "$server_level" "$api_block" \
  | grep -o 'client_max_body_size[[:space:]]*[^;]*' | head -1 | sed 's/client_max_body_size[[:space:]]*//')

echo "MAX_BUNDLE_BYTES: $max_bytes bytes ($expr)"
if [ -z "$directive" ]; then
  fail "no \`client_max_body_size\` at server level or inside \`location /api\` in frontend/nginx.conf." \
"nginx then applies its 1 MiB default and refuses a bundle the API would have accepted."
fi
allowed=$(nginx_bytes "$directive")
echo "client_max_body_size: $directive = $allowed bytes (in scope for /api)"

if [ "$allowed" = "0" ]; then
  echo "client_max_body_size 0 means unlimited: the proxy imposes no ceiling at all."
  echo "That is a choice, not a mistake — but say it on purpose, in a comment."
elif [ "$allowed" -lt "$max_bytes" ]; then
  fail "the proxy allows $allowed bytes but the API advertises $max_bytes." \
"A bundle between those sizes exports fine and cannot be re-imported through the UI."
fi

# --- the panel refuses locally --------------------------------------------------
# Definition plus one call per write path (preview and restore). Fewer than three
# means a call site was dropped, and the refusal would be back to a gateway's guess.
calls=$(grep -c 'oversizeReason' "$PANEL")
echo "oversizeReason mentions in BackupPanel.tsx: $calls"
[ "$calls" -ge 3 ] || fail "BackupPanel.tsx no longer refuses an oversized bundle before sending it ($calls mentions, expected at least 3: the definition and both write paths)."

echo "UPLOAD PATH GATE PASSED"
