#!/usr/bin/env bash

if command -v cargo &> /dev/null; then
    cargo fmt --all
    cargo clippy
    [[ "$1" == "t" ]] && cargo test
fi

if command -v prettier &> /dev/null; then
    prettier --write src/web/index.js
fi

exit 0

