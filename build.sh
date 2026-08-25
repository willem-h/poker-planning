#!/usr/bin/env bash
# Builds the wasm module into public/pkg/, which is what GitHub Pages serves.
set -euo pipefail

WASM_BINDGEN_VERSION="0.2.127"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

rustup target add wasm32-unknown-unknown

if ! command -v wasm-bindgen >/dev/null || \
   [[ "$(wasm-bindgen --version)" != "wasm-bindgen ${WASM_BINDGEN_VERSION}" ]]; then
  # The CLI has to match the wasm-bindgen crate version in Cargo.lock exactly.
  cargo install wasm-bindgen-cli --version "${WASM_BINDGEN_VERSION}" --locked
fi

cargo build --release --target wasm32-unknown-unknown --manifest-path "${root}/wasm/Cargo.toml"

wasm-bindgen \
  --target web \
  --no-typescript \
  --out-dir "${root}/public/pkg" \
  --out-name poker \
  "${root}/wasm/target/wasm32-unknown-unknown/release/poker_wasm.wasm"

# wasm-opt roughly halves the module; skipped when binaryen isn't installed.
if command -v wasm-opt >/dev/null; then
  wasm-opt -Os "${root}/public/pkg/poker_bg.wasm" -o "${root}/public/pkg/poker_bg.wasm"
fi

echo "built $(du -h "${root}/public/pkg/poker_bg.wasm" | cut -f1) -> public/pkg/"
