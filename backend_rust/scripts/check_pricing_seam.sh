#!/usr/bin/env bash
# Pricing-seam gate (#37): the test suite must not be able to reach the live
# pricing provider.
#
# One rule. Nothing outside `src/main.rs` constructs a `CurrencyService` with the
# default provider URL. Tests build one through `offline_pricing_service()`, which
# points at the local stand-in in `src/test_provider.rs`.
#
# Why static rather than a grep over `cargo test` output. The first version of this
# gate searched test output for the provider's hostname, and it was wrong quietly:
# the service logs a URL only when a request *fails*, so a test that reached Yahoo
# and got a price printed nothing and the gate passed. Verified by experiment — a
# throwaway test that fetched a real AAPL price passed the gate. A check blind to the
# success path is not a check. This one reads the source, where the choice is made.
#
# Prints what it inspected. A gate that silently inspects nothing is worse than no
# gate — same rule as `check_access_gate.sh`.

set -uo pipefail
cd "$(dirname "$0")/.."

ALLOWED="src/main.rs"

checked=$(grep -rl "CurrencyService" src tests --include='*.rs' | wc -l | tr -d ' ')
echo "files mentioning CurrencyService: $checked"
if [ "$checked" = "0" ]; then
  echo "GATE FAILED — inspected nothing; the source layout changed under this script"
  exit 1
fi

matches=$(grep -rn "CurrencyService::new()" src tests --include='*.rs' \
  | grep -v "^${ALLOWED}:" \
  | grep -vE ':[0-9]+: *(//|///|//!)')

if [ -z "$matches" ]; then
  echo "live-provider constructions outside ${ALLOWED}: 0"
  echo "GATE PASSED"
  exit 0
fi

echo "GATE FAILED — live-provider construction outside ${ALLOWED}:"
echo "$matches"
echo
echo "Use offline_pricing_service() (tests/common/mod.rs, or crate::offline_pricing_service()"
echo "for unit tests inside src/) so the provider is the local stand-in."
exit 1
