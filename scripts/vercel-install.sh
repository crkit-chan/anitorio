#!/bin/sh
set -e

npm ci

node -e "fetch('https://sh.rustup.rs').then(r=>r.text()).then(console.log)" | sh -s -- -y --profile minimal --default-toolchain stable --target wasm32-unknown-unknown

. "$HOME/.cargo/env"

node -e "fetch('https://rustwasm.github.io/wasm-pack/installer/init.sh').then(r=>r.text()).then(console.log)" | sh