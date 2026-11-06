#!/bin/sh
set -e

npm ci

rustup target add wasm32-unknown-unknown

echo "=== WASM-PACK VERSION ==="
wasm-pack --version

echo "=== WASM-PACK LOCATION ==="
which wasm-pack

echo "=== WASM-PACK HELP ==="
wasm-pack build --help

node -e "fetch('https://rustwasm.github.io/wasm-pack/installer/init.sh').then(r=>r.text()).then(console.log)" | sh