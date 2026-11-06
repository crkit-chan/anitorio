#!/bin/sh
set -e

wasm-pack build --target web "$@"
node scripts/write-build-info.mjs "$@"