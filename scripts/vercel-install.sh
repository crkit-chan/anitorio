#!/bin/sh
set -e

npm ci
rustup target add wasm32-unknown-unknown
node -e "fetch('https://rustwasm.github.io/wasm-pack/installer/init.sh').then(r=>r.text()).then(console.log)" | sh