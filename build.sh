#!/usr/bin/env bash
# Builds both transports into public/, which is what GitHub Pages serves.
set -euo pipefail

WASM_BINDGEN_VERSION="0.2.127"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

rustup target add wasm32-unknown-unknown

if ! command -v wasm-bindgen >/dev/null || \
   [[ "$(wasm-bindgen --version)" != "wasm-bindgen ${WASM_BINDGEN_VERSION}" ]]; then
  # The CLI has to match the wasm-bindgen crate version in Cargo.lock exactly.
  cargo install wasm-bindgen-cli --version "${WASM_BINDGEN_VERSION}" --locked
fi

# One crate, two builds: the iroh transport carries its own networking stack,
# the WebRTC one gets its wire from JS and so is a fraction of the size.
build_wasm() {
  local features="$1" out="$2" name="$3"

  cargo build --release --target wasm32-unknown-unknown \
    --manifest-path "${root}/wasm/Cargo.toml" \
    --no-default-features --features "${features}"

  wasm-bindgen \
    --target web \
    --no-typescript \
    --out-dir "${root}/public/${out}" \
    --out-name "${name}" \
    "${root}/wasm/target/wasm32-unknown-unknown/release/poker_wasm.wasm"

  # wasm-opt takes roughly a quarter off the module, but it rewrites the binary
  # after wasm-bindgen has already generated JS against it, and it has a history
  # of breaking the externref table that glue depends on. It is off unless asked
  # for, so what ships is what the browser tests ran against.
  if [[ "${WASM_OPT:-0}" == "1" ]]; then
    if ! command -v wasm-opt >/dev/null; then
      echo "WASM_OPT=1 but wasm-opt is not installed" >&2
      exit 1
    fi
    wasm-opt -Os "${root}/public/${out}/${name}_bg.wasm" -o "${root}/public/${out}/${name}_bg.wasm"
  fi

  echo "  ${out}/${name}_bg.wasm  $(du -h "${root}/public/${out}/${name}_bg.wasm" | cut -f1)"
}

echo "building wasm:"
build_wasm iroh-transport pkg poker
build_wasm webrtc-transport pkg-webrtc room

# Trystero is split across several npm packages, and the browser loads plain
# static files, so its imports are resolved ahead of time into public/vendor/.
echo "bundling signaling:"
npm ci --no-audit --no-fund --prefer-offline 2>/dev/null || npm install --no-audit --no-fund
npm run --silent vendor

# A module that cannot grow its externref table throws the moment it loads, and
# nothing before this point would have noticed.
echo "checking wasm:"
node "${root}/scripts/check-wasm.mjs" \
  "${root}/public/pkg/poker_bg.wasm" \
  "${root}/public/pkg-webrtc/room_bg.wasm"
