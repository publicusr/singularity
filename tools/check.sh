#!/bin/sh
# What CI runs (.github/workflows/ci.yml), before a push: tests, clippy, cargo deny. The MSRV check needs Rust 1.89
# installed and runs only if it is (rustup toolchain install 1.89). Extra arguments go to cargo test.
set -e
cd "$(dirname "$0")/.."
# every test binary runs even when one fails, so one red doesn't hide another crate's tests
cargo test --workspace --locked --no-fail-fast "$@"
cargo clippy --workspace --all-targets --locked -- -D warnings
if command -v cargo-deny >/dev/null; then cargo deny check; else echo "cargo-deny isn't installed: cargo install --locked cargo-deny" >&2; exit 1; fi
if rustup toolchain list 2>/dev/null | grep -q '^1\.89'; then cargo +1.89 check --workspace --all-targets --locked; fi
echo "check: all green"
