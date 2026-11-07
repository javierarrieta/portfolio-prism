#!/usr/bin/env bash
# Automated OIDC checks that need no browser.
#
# WHAT THIS PROVES, AND WHAT IT CANNOT
# It proves the backend can talk to a real provider: discovery resolves, the
# authorization request is well-formed, the failure paths answer the way the
# frontend expects, and unauthenticated `/api` is denied. It cannot prove a login
# completes — that needs a browser and a human typing a password. The script ends by
# printing the manual steps, including the logout click-through, which is the one
# open question about the provider (see docs/auth-testing.md §5).
#
# WHY IT IS A SHELL SCRIPT AND NOT A cargo TEST
# Every check here needs a live provider and a live server. A test that skips itself
# when the environment is absent is a test that reports success while doing nothing,
# which is the failure mode this repository has already been bitten by twice.
#
# USAGE
#   OIDC_ISSUER=https://auth.example.com/application/o/portfolio-prism/ \
#   OIDC_CLIENT_ID=portfolio-prism \
#   OIDC_CLIENT_SECRET=... \
#   OIDC_REDIRECT_URI=http://localhost:8000/auth/callback \
#   ./scripts/auth_smoke.sh
#
# OIDC_CLIENT_SECRET may be any non-empty string for the automated checks: nothing
# here completes a code exchange, so the secret is never sent. It is required because
# the backend refuses to start without it, which is the point.
set -uo pipefail

ISSUER="${OIDC_ISSUER:-}"
CLIENT_ID="${OIDC_CLIENT_ID:-portfolio-prism}"
CLIENT_SECRET="${OIDC_CLIENT_SECRET:-smoke-test-not-a-real-secret}"
REDIRECT_URI="${OIDC_REDIRECT_URI:-http://localhost:8000/auth/callback}"
PORT="${PORT:-8000}"
BASE="http://localhost:${PORT}"

PASS=0; FAIL=0
ok()   { echo "  PASS  $1"; PASS=$((PASS + 1)); }
bad()  { echo "  FAIL  $1"; FAIL=$((FAIL + 1)); }
need() { command -v "$1" >/dev/null || { echo "FAIL: $1 not on PATH"; exit 1; } }

need curl; need python3

if [ -z "$ISSUER" ]; then
  echo "OIDC_ISSUER is required. Example:"
  echo "  OIDC_ISSUER=https://auth.titan.arrieta.eu/application/o/portfolio-prism/ ./scripts/auth_smoke.sh"
  exit 2
fi

# Trailing slash matters: authentik's per-provider issuer is
# `.../application/o/<slug>/` and the discovery document's `issuer` field carries the
# slash. A configured value that disagrees with the document by one character is a
# token-validation failure that looks like a bad key.
case "$ISSUER" in */) : ;; *) echo "NOTE: OIDC_ISSUER has no trailing slash; authentik's does. Continuing."; ;; esac

echo "== 1. provider discovery =="
DISC_URL="${ISSUER%/}/.well-known/openid-configuration"
DISC=$(curl -sS --max-time 20 "$DISC_URL" 2>/dev/null)
if [ -z "$DISC" ]; then
  bad "no response from $DISC_URL"
  echo "        The provider must exist before the app can be pointed at it."
  echo "        authentik returns 404 for a provider that has not been created yet."
  exit 1
fi
echo "$DISC" | python3 -c '
import json,sys
d=json.load(sys.stdin)
want=sys.argv[1].rstrip("/")
got=d.get("issuer","").rstrip("/")
print("        issuer   :", d.get("issuer"))
print("        authorize:", d.get("authorization_endpoint"))
print("        token    :", d.get("token_endpoint"))
print("        end_ses. :", d.get("end_session_endpoint","(none)"))
sys.exit(0 if want==got else 1)
' "$ISSUER" \
  && ok "discovery reachable and its issuer matches OIDC_ISSUER" \
  || bad "discovery issuer does not match OIDC_ISSUER (trailing slash or wrong slug)"

AUTH_ENDPOINT=$(echo "$DISC" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("authorization_endpoint",""))')
END_SESSION=$(echo "$DISC" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("end_session_endpoint") or "")')
if [ -n "$END_SESSION" ]; then
  ok "provider advertises an end_session_endpoint (logout can reach it)"
else
  bad "provider advertises no end_session_endpoint — logout will be local-only"
fi

echo "== 2. start the backend in oidc mode =="
DB=$(mktemp /tmp/authsmoke.XXXXXX.db)
export AUTH_MODE=oidc OIDC_ISSUER="$ISSUER" OIDC_CLIENT_ID="$CLIENT_ID" \
       OIDC_CLIENT_SECRET="$CLIENT_SECRET" OIDC_REDIRECT_URI="$REDIRECT_URI" \
       DATABASE_URL="sqlite://$DB?mode=rwc" \
       ROCKET_SECRET_KEY="$(head -c 32 /dev/urandom | base64)" \
       ALLOWED_ORIGINS="$BASE" ROCKET_PORT="$PORT" ROCKET_ADDRESS=127.0.0.1
cargo run --quiet 2>/tmp/authsmoke.log &
SERVER=$!
trap 'kill $SERVER 2>/dev/null; rm -f "$DB" /tmp/authsmoke.log' EXIT

for _ in $(seq 1 60); do
  curl -s --max-time 2 "$BASE/" >/dev/null 2>&1 && break
  sleep 1
done
curl -s --max-time 2 "$BASE/" >/dev/null 2>&1 \
  && ok "backend started in oidc mode" \
  || { bad "backend did not come up"; tail -20 /tmp/authsmoke.log; exit 1; }

echo "== 3. unauthenticated data access =="
for spec in "GET /api/portfolios/" "POST /api/portfolios/" "DELETE /api/assets/1" "PATCH /api/portfolios/1"; do
  m=${spec% *}; p=${spec#* }
  code=$(curl -s --max-time 10 -o /tmp/smoke_body -w "%{http_code}" -X "$m" "$BASE$p")
  body=$(cat /tmp/smoke_body)
  if [ "$code" = "401" ] && echo "$body" | grep -q '"error":"unauthenticated"'; then
    ok "$m $p -> 401 unauthenticated"
  else
    bad "$m $p -> ${code} (expected 401 unauthenticated)"
  fi
done

echo "== 4. public surface =="
curl -s -o /dev/null -w "%{http_code}" "$BASE/" | grep -q 200 \
  && ok "GET / -> 200 (probes and the SPA entry stay reachable)" \
  || bad "GET / is not 200"
curl -s -o /dev/null -w "%{http_code}" "$BASE/api-docs/openapi.json" | grep -q 200 \
  && ok "GET /api-docs/openapi.json -> 200" \
  || bad "the OpenAPI document is not reachable"

echo "== 5. the authorization request =="
LOC=$(curl -s -o /dev/null -D - --max-time 20 "$BASE/auth/login" | tr -d '\r')
if echo "$LOC" | grep -qi '^location:'; then
  ok "/auth/login redirects"
  LOCATION=$(echo "$LOC" | grep -i '^location:' | cut -d' ' -f2-)
  case "$LOCATION" in
    "$AUTH_ENDPOINT"*) ok "redirect target is the provider's authorization endpoint" ;;
    *) bad "redirect target is not the authorization endpoint: $LOCATION" ;;
  esac
  for param in client_id="$CLIENT_ID" code_challenge= code_challenge_method=S256 state= nonce= redirect_uri=; do
    echo "$LOCATION" | grep -q "${param%%=*}" \
      && ok "authorize URL carries ${param%%=*}" \
      || bad "authorize URL is missing ${param%%=*}"
  done
  echo "$LOC" | grep -qi 'set-cookie: pp_oauth_state' \
    && ok "state cookie set on the way out" \
    || bad "no pp_oauth_state cookie"
else
  bad "/auth/login did not redirect — provider unreachable? see /tmp/authsmoke.log"
fi

echo "== 6. does the provider accept this client at all? =="
# Worth its own check because the failure is invisible from the app's side: /auth/login
# succeeds, the browser is handed a perfectly well-formed authorize URL, and the
# provider answers with an error page the app never sees. The usual cause is that the
# provider's SLUG is what appears in the issuer URL, while its CLIENT ID is a separate
# field that authentik fills with a random string unless you overwrite it.
if [ -n "${LOCATION:-}" ]; then
  AR=$(curl -s -o /tmp/smoke_authorize -w "%{http_code}" --max-time 25 "$LOCATION")
  if grep -qiE 'client (identifier|ID).*(missing|invalid)|invalid_client|unauthorized_client' /tmp/smoke_authorize; then
    bad "the provider rejected client_id=${CLIENT_ID} — open the provider in authentik and read its Client ID field; it is not necessarily the slug"
  elif [ "$AR" = "302" ] || [ "$AR" = "200" ]; then
    ok "the provider accepted client_id and the redirect URI (http $AR)"
  else
    bad "provider answered http $AR for the authorize request — check the redirect URI registration"
    head -c 300 /tmp/smoke_authorize; echo
  fi
else
  bad "no authorize URL to test (check 5 failed)"
fi

echo "== 7. the failure path (the only provider path testable without a browser) =="
# A browser's Accept header, not curl's `*/*`. `WantsHtml` branches on it, and `*/*`
# means "not a browser" — asserting the browser shape with `*/*` asserts the opposite
# of what it looks like.
H=$(curl -s -o /dev/null -D - -H "Accept: text/html,application/xhtml+xml" \
     --max-time 20 "$BASE/auth/callback?code=bogus&state=bogus" | tr -d '\r')
echo "$H" | grep -qi 'location: */?auth_error=auth_failed' \
  && ok "browser callback failure -> redirect to /?auth_error=auth_failed" \
  || { bad "browser callback failure not redirected as expected"; echo "$H" | head -5; }
B=$(curl -s -o - -w "|%{http_code}" -H "Accept: application/json" --max-time 20 \
      "$BASE/auth/callback?code=bogus&state=bogus")
case "$B" in *'"error":"authentication_failed"'*"|401") ok "JSON callback failure -> 401 authentication_failed" ;;
              *) bad "JSON callback failure not handled: $B" ;; esac

echo
echo "automated: $PASS passed, $FAIL failed"
echo
echo "--------------------------------------------------------------------------"
echo "The rest needs a browser. Open:"
echo "  $BASE/auth/login"
echo "Sign in, then confirm you land on the app and that:"
echo "  $BASE/auth/me          shows your sub and email"
echo "  $BASE/api/portfolios/  returns [] and not 401"
echo
if [ -n "$END_SESSION" ]; then
  echo "Then the logout question (§5 of docs/auth-testing.md):"
  echo "  1. $BASE/auth/logout"
  echo "  2. note whether the provider shows a confirmation page, and what it says"
  echo "  3. open a NEW tab to $BASE/auth/login"
  echo "     straight back into the app  -> the provider session survived; M13 is real"
  echo "     a credential prompt         -> the provider ended it; M13 closes as-is"
fi
echo "--------------------------------------------------------------------------"
[ "$FAIL" -eq 0 ] || exit 1
