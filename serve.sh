#!/usr/bin/env bash
# Serves public/ for local development. Opening index.html from disk will not
# work: ES modules and wasm need a real http origin.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/public"
echo "http://localhost:${1:-8080}"
exec python3 -m http.server "${1:-8080}"
