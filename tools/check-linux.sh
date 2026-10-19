#!/bin/sh
# Type-check the whole workspace for Linux from a Mac, so macOS work can't break the Linux build (D49).
# Once: rustup target add x86_64-unknown-linux-gnu && brew install zig cargo-zigbuild openssl@3
# (OpenSSL's headers are only read and ALSA is only named: nothing is linked.)
set -e
cd "$(dirname "$0")/.."
# alsa-sys (cpal, for the librespot Engine) asks pkg-config for ALSA; a stand-in .pc answers (its bindings are
# pre-generated, so no headers are needed for a check)
pc="target/check-linux-pkgconfig"
mkdir -p "$pc"
printf 'Name: alsa\nDescription: stand-in for a type-check\nVersion: 1.2.12\nLibs: -lasound\nCflags:\n' > "$pc/alsa.pc"
PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_PATH_x86_64_unknown_linux_gnu="$PWD/$pc" PKG_CONFIG_LIBDIR_x86_64_unknown_linux_gnu="$PWD/$pc" \
X86_64_UNKNOWN_LINUX_GNU_OPENSSL_DIR="${X86_64_UNKNOWN_LINUX_GNU_OPENSSL_DIR:-$(brew --prefix openssl@3)}" \
  exec cargo-zigbuild check --workspace --all-targets --target x86_64-unknown-linux-gnu "$@"
