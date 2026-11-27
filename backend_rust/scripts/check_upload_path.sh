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

# --- Rule 3: the MCP door is reachable, and the proxy is not narrower -----------
# Same failure class as Rule 1, found in review rather than in production: the `/mcp`
# route shipped, every test passed, and nothing in the deployed topology could reach it,
# because `location /mcp` did not exist and the SPA fallback answered the POST with
# index.html. A route that no proxy forwards is a feature that does not exist, so this
# rule fails when the block is missing — it is not allowed to skip.
MOUNT="$ROOT/backend_rust/src/agent/mount.rs"
[ -f "$MOUNT" ] || fail "backend_rust/src/agent/mount.rs is missing; the layout changed under this script"

mcp_start=$(grep -n 'location /mcp' "$NGINX" | head -1 | cut -d: -f1)
if [ -z "$mcp_start" ]; then
  fail "no \`location /mcp\` block in frontend/nginx.conf."
"Rocket serves /mcp (agent/mount.rs) but nginx has no block for it, so \`location /\`"
"answers POST /mcp with index.html and the agent surface is unreachable in the deployed"
"topology while every Rust test still passes."
fi
mcp_end=$(awk -v s="$mcp_start" 'NR>s && /^[[:space:]]*}[[:space:]]*$/ {print NR; exit}' "$NGINX")
[ -n "$mcp_end" ] || fail "the \`location /mcp\` block in frontend/nginx.conf never closes"

if ! grep -q 'proxy_pass' <(sed -n "${mcp_start},${mcp_end}p" "$NGINX"); then
  fail "\`location /mcp\` exists but has no proxy_pass — it is not forwarded to the backend."
fi

mcp_expr=$(sed -n 's/^const MAX_BODY_BYTES: *usize *= *\(.*\);$/\1/p' "$MOUNT" | head -1)
[ -n "$mcp_expr" ] || fail "no \`const MAX_BODY_BYTES: usize = ...\` in backend_rust/src/agent/mount.rs"
case "$mcp_expr" in
  *[!0-9\ *]*) fail "MAX_BODY_BYTES expression is not arithmetic: \`$mcp_expr\`" ;;
esac
mcp_bytes=$((mcp_expr))

directive=$(sed -n "${mcp_start},${mcp_end}p" "$NGINX" \
  | grep -o 'client_max_body_size[[:space:]]*[^;]*' | head -1 | sed 's/client_max_body_size[[:space:]]*//')
if [ -z "$directive" ]; then
  directive=$(printf '%s\n' "$server_level" | grep -o 'client_max_body_size[[:space:]]*[^;]*' | head -1 | sed 's/client_max_body_size[[:space:]]*//')
  [ -n "$directive" ] || fail "\`location /mcp\` sets no client_max_body_size and neither does server level,"
"so nginx applies its 1 MiB default: a batch at the backend's own ${mcp_bytes}-byte cap is"
"refused by the proxy with an HTML page instead of a protocol-shaped 413."
fi
mcp_allowed=$(nginx_bytes "$directive")
echo "MAX_BODY_BYTES (/mcp): $mcp_bytes bytes ($mcp_expr)"
echo "client_max_body_size (/mcp): $directive = $mcp_allowed bytes"
if [ "$mcp_allowed" != "0" ] && [ "$mcp_allowed" -lt "$mcp_bytes" ]; then
  fail "the proxy allows $mcp_allowed bytes for /mcp but the route accepts $mcp_bytes."
"A batch between those sizes is refused by nginx before the handler can answer in the"
"protocol's own shape."
fi

echo "UPLOAD PATH GATE PASSED"
