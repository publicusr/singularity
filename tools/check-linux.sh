#!/bin/sh
# Type-check the whole workspace for Linux from a Mac, so macOS work can't break the Linux build (D49).
# Once: rustup target add x86_64-unknown-linux-gnu && brew install zig cargo-zigbuild openssl@3
# (OpenSSL's headers are only read: nothing is linked.)
set -e
cd "$(dirname "$0")/.."
X86_64_UNKNOWN_LINUX_GNU_OPENSSL_DIR="${X86_64_UNKNOWN_LINUX_GNU_OPENSSL_DIR:-$(brew --prefix openssl@3)}" \
  exec cargo-zigbuild check --workspace --all-targets --target x86_64-unknown-linux-gnu "$@"
