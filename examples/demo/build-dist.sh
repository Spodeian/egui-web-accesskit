#!/usr/bin/env bash
# Builds the public demo into dist/ for a given public path (default "/").
#
# Steps: trunk build (cargo + wasm-bindgen), explicit wasm-opt -Oz with the browser feature
# baseline, then minification of index.html (not the JS: see below). Every step fails loudly; nothing is
# skipped silently.
#
# Tool versions are pinned in .github/workflows/ci.yml. Required on PATH: trunk 0.22.2,
# wasm-opt (Binaryen 132), and npx for the pinned html-minifier-terser.
set -euo pipefail

PUBLIC_URL="${1:-/}"
cd "$(dirname "$0")"

trunk build --release --public-url "$PUBLIC_URL"

WASM_OPT_FLAGS=(
  -Oz
  --enable-bulk-memory
  --enable-mutable-globals
  --enable-sign-ext
  --enable-nontrapping-float-to-int
  --enable-reference-types
  --strip-debug
  --strip-producers
)
for wasm in dist/*.wasm; do
  before=$(stat -c%s "$wasm")
  wasm-opt "${WASM_OPT_FLAGS[@]}" "$wasm" -o "$wasm"
  after=$(stat -c%s "$wasm")
  echo "wasm-opt $wasm: $before -> $after bytes"
done

# The JS glue is not minified: trunk writes a subresource-integrity hash for it into index.html,
# and rewriting the file would break the hash (the browser then refuses to run the script). The
# gain would be a few KB gzipped.

npx --yes html-minifier-terser@7.2.0 \
  --collapse-whitespace --remove-comments --remove-redundant-attributes \
  --remove-script-type-attributes --use-short-doctype --minify-css true --minify-js true \
  -o dist/index.html dist/index.html

echo "Built demo for public path '$PUBLIC_URL' in $(pwd)/dist"
