#!/usr/bin/env bash


cargo fmt --all
cargo check
[[ "$1" == "t" ]] && cargo test

exit 0

