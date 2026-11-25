#!/bin/sh
# Type-check the whole workspace for Linux, x86_64 and aarch64 (D73), from a Mac, so macOS work can't break the Linux
# build (D49).
# Once: rustup target add x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu && brew install zig cargo-zigbuild openssl@3
# (OpenSSL's headers are only read and ALSA is only named: nothing is linked.)
set -e
cd "$(dirname "$0")/.."
# alsa-sys (cpal, for the librespot Engine) asks pkg-config for ALSA; a stand-in .pc answers (its bindings are
# pre-generated, so no headers are needed for a check)
pc="target/check-linux-pkgconfig"
mkdir -p "$pc"
printf 'Name: alsa\nDescription: stand-in for a type-check\nVersion: 1.2.12\nLibs: -lasound\nCflags:\n' > "$pc/alsa.pc"
for target in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
  lower=$(echo "$target" | tr - _)
  upper=$(echo "$lower" | tr '[:lower:]' '[:upper:]')
  # <TARGET>_OPENSSL_DIR, when set, wins over Homebrew's openssl@3
  eval "openssl=\${${upper}_OPENSSL_DIR:-}"
  [ -n "$openssl" ] || openssl=$(brew --prefix openssl@3)
  echo "check-linux: $target" >&2
  env PKG_CONFIG_ALLOW_CROSS=1 "PKG_CONFIG_PATH_$lower=$PWD/$pc" "PKG_CONFIG_LIBDIR_$lower=$PWD/$pc" \
    "${upper}_OPENSSL_DIR=$openssl" \
    cargo-zigbuild check --workspace --all-targets --target "$target" "$@"
done
