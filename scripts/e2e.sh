#!/usr/bin/env bash
# Runs the E2E conversion test over real packs.
# Usage: scripts/e2e.sh [fixtures-dir]   (or set PACKCONV_FIXTURES)
set -euo pipefail

cd "$(dirname "$0")/.."
export PACKCONV_FIXTURES="${1:-${PACKCONV_FIXTURES:?pass a fixtures directory or set PACKCONV_FIXTURES}}"
nice -n 19 cargo test -p packconv-core --test e2e -- --nocapture
