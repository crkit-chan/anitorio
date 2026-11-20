#!/bin/sh
set -e

npm ci
rustup toolchain install nightly-2025-11-15 --component rust-src --component rust-analyzer --target wasm32-unknown-unknown
node -e "fetch('https://rustwasm.github.io/wasm-pack/installer/init.sh').then(r=>r.text()).then(console.log)" | sh