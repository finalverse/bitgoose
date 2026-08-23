#!/usr/bin/env bash
# Everything CI checks, in the order CI checks it.
#
# This exists because a whole session's work went in with CI red and nobody
# noticed: `cargo clippy --workspace --all-targets` passes locally while CI
# fails, for two reasons that are easy to forget and impossible to see.
#
#   * CI sets `RUSTFLAGS=-D warnings`, so an unused import is an *error* there
#     and a warning here. Four leftovers from a refactor sat in bg-web for
#     eleven releases.
#   * CI runs `cargo fmt --all --check`, which nothing local runs at all.
#
# Run this before pushing. Same commands, same environment, so a green run
# here is a green run there.
set -euo pipefail
export RUSTFLAGS="${RUSTFLAGS:--D warnings}"

step() { printf '\n\033[1;33m==> %s\033[0m\n' "$1"; }

step "cargo fmt --all --check"
cargo fmt --all --check

step "cargo clippy --workspace --all-targets"
cargo clippy --workspace --all-targets

# bg-web is the trap: its default feature set is the WASM one, so the server
# half is not built by a plain workspace clippy and its warnings never appear.
step "cargo clippy -p bg-web --features ssr"
cargo clippy -p bg-web --no-default-features --features ssr --all-targets

step "cargo test --workspace"
cargo test --workspace -- --test-threads=1

step "cargo test -p bg-web --features ssr"
cargo test -p bg-web --features ssr -- --test-threads=1

step "wasm boundary: bg-core must build for wasm32"
cargo build -p bg-core --target wasm32-unknown-unknown

printf '\n\033[1;32mall green — this is what CI runs\033[0m\n'
