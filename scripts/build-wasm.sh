#!/usr/bin/env bash
# Builds the wasm bindings and generates the JS package in ./pkg.
# Usage: scripts/build-wasm.sh [web|bundler|nodejs]
set -euo pipefail

target="${1:-web}"
case "$target" in
  web | bundler | nodejs) ;;
  *) echo "usage: $0 [web|bundler|nodejs]" >&2; exit 2 ;;
esac

cd "$(dirname "$0")/.."
nice -n 19 cargo build --release --target wasm32-unknown-unknown -p packconv-wasm
nice -n 19 wasm-bindgen --target "$target" --out-dir pkg \
  "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/release/packconv_wasm.wasm"
