#!/usr/bin/env bash


cargo fmt --all
cargo clippy
[[ "$1" == "t" ]] && cargo test

exit 0

