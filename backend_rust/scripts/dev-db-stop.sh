#!/usr/bin/env bash
set -euo pipefail
DATA="${PRISM_PG_DIR:-${XDG_RUNTIME_DIR:-/tmp}/prism-pg}"
pg_ctl -D "$DATA" -m fast -w stop
