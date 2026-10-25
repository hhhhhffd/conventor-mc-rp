#!/usr/bin/env bash
# Publishes the wasm build as a _k0t1k launcher plugin (GitHub Release with a signed plugin.json).
# Usage: scripts/release-plugin.sh
# Env: K0T1K_PLUGIN_KEY  Ed25519 private key (default ~/.config/k0t1k/plugin-signing-key.pem)
set -euo pipefail

REPO="hhhhhffd/conventor-mc-rp"
PLUGIN_ID="packconv"
API=1
MIN_LAUNCHER="0.1.0"
# Raw Ed25519 public key embedded in the launcher (base64).
PUBLIC_KEY_B64="b37Axim0D7VgDNWVIgvlnpdn2nVOMSLM/jIFbRFc/4k="
KEY="${K0T1K_PLUGIN_KEY:-$HOME/.config/k0t1k/plugin-signing-key.pem}"

die() { echo "error: $*" >&2; exit 1; }

cd "$(dirname "$0")/.."

version="$(awk '/^\[workspace\.package\]/{s=1; next} /^\[/{s=0} s && /^version[[:space:]]*=/{gsub(/[" ]/, "", $3); print $3; exit}' Cargo.toml)"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "cannot read workspace version from Cargo.toml (got '$version')"
tag="v$version"

[[ -f "$KEY" ]] || die "signing key not found: $KEY"
[[ -z "$(git status --porcelain)" ]] || die "working tree is not clean"
git rev-parse --abbrev-ref '@{u}' >/dev/null 2>&1 || die "current branch has no upstream"
git fetch --quiet --tags || die "git fetch failed"
head_sha="$(git rev-parse HEAD)"
git merge-base --is-ancestor "$head_sha" '@{u}' || die "HEAD is not pushed to its upstream"
[[ -z "$(git tag -l "$tag")" ]] || die "tag $tag already exists locally"
[[ -z "$(git ls-remote --tags origin "refs/tags/$tag")" ]] || die "tag $tag already exists on origin"
if gh release view "$tag" --repo "$REPO" >/dev/null 2>&1; then
  die "release $tag already exists"
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

cargo build --release --target wasm32-unknown-unknown -p packconv-wasm
wasm-bindgen --target web --out-dir "$work/pkg" \
  "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/release/packconv_wasm.wasm"

files=(packconv_wasm.js packconv_wasm_bg.wasm)
for f in "${files[@]}"; do
  [[ -f "$work/pkg/$f" ]] || die "build did not produce $f"
done
js_sha="$(sha256sum "$work/pkg/packconv_wasm.js" | cut -d' ' -f1)"
wasm_sha="$(sha256sum "$work/pkg/packconv_wasm_bg.wasm" | cut -d' ' -f1)"

# Written once; the launcher verifies these exact bytes.
cat > "$work/pkg/plugin.json" <<JSON
{
  "id": "$PLUGIN_ID",
  "version": "$version",
  "api": $API,
  "minLauncher": "$MIN_LAUNCHER",
  "files": {
    "packconv_wasm.js": "$js_sha",
    "packconv_wasm_bg.wasm": "$wasm_sha"
  }
}
JSON

{ printf '_k0t1k-plugin\n'; cat "$work/pkg/plugin.json"; } > "$work/msg"
openssl pkeyutl -sign -rawin -inkey "$KEY" -in "$work/msg" -out "$work/sig.bin"
base64 -w0 "$work/sig.bin" > "$work/pkg/plugin.json.sig"

# Verify against the launcher's public key: SPKI DER prefix for Ed25519 + raw key.
{ printf '\x30\x2a\x30\x05\x06\x03\x2b\x65\x70\x03\x21\x00'; printf '%s' "$PUBLIC_KEY_B64" | base64 -d; } > "$work/pub.der"
openssl pkey -pubin -inform DER -in "$work/pub.der" -out "$work/pub.pem"
openssl pkeyutl -verify -rawin -pubin -inkey "$work/pub.pem" -in "$work/msg" -sigfile "$work/sig.bin" >/dev/null \
  || die "signature does not verify against the launcher public key (wrong signing key?)"

gh release create "$tag" --repo "$REPO" --target "$head_sha" \
  --title "packconv $version" --notes "packconv launcher plugin $version (API $API)" \
  "$work/pkg/plugin.json" "$work/pkg/plugin.json.sig" \
  "$work/pkg/packconv_wasm.js" "$work/pkg/packconv_wasm_bg.wasm"
echo "released $tag"
