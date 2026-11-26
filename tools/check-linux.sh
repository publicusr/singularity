#!/bin/sh
# Type-check the whole workspace for Linux, x86_64 and aarch64 (D73), from a Mac, so macOS work can't break the Linux
# build (D49).
# Once: rustup target add x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu && brew install zig cargo-zigbuild openssl@3
# (OpenSSL's headers are only read and ALSA is only named: nothing is linked.)
set -e
cd "$(dirname "$0")/.."
targets="x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu"

# <TARGET>_OPENSSL_DIR, when set, wins over Homebrew's openssl@3
openssl_dir() {
  upper=$(echo "$1" | tr - _ | tr '[:lower:]' '[:upper:]')
  eval "echo \"\${${upper}_OPENSSL_DIR:-}\""
}

# Preflight: one clear line for each missing piece, before any tool fails in its own words
missing=0
need() {
  echo "check-linux: $1 (see the \"Once:\" line at the top of tools/check-linux.sh)" >&2
  missing=1
}
command -v zig >/dev/null 2>&1 || need "zig is missing"
command -v cargo-zigbuild >/dev/null 2>&1 || need "cargo-zigbuild is missing"
installed=$(rustup target list --installed 2>/dev/null || true)
for target in $targets; do
  echo "$installed" | grep -qx "$target" || need "the Rust target $target isn't installed"
done
brew_openssl=""
for target in $targets; do
  [ -z "$(openssl_dir "$target")" ] || continue
  if ! command -v brew >/dev/null 2>&1; then
    need "brew is missing (it provides openssl@3)"
  else
    brew_openssl=$(brew --prefix openssl@3 2>/dev/null || true)
    if [ -z "$brew_openssl" ] || [ ! -d "$brew_openssl/include/openssl" ]; then
      need "Homebrew's openssl@3 isn't installed"
    fi
  fi
  break
done
[ "$missing" = 0 ] || exit 1

# alsa-sys (cpal, for the librespot Engine) asks pkg-config for ALSA; a stand-in .pc answers (its bindings are
# pre-generated, so no headers are needed for a check)
pc="target/check-linux-pkgconfig"
mkdir -p "$pc"
printf 'Name: alsa\nDescription: stand-in for a type-check\nVersion: 1.2.12\nLibs: -lasound\nCflags:\n' > "$pc/alsa.pc"
for target in $targets; do
  lower=$(echo "$target" | tr - _)
  upper=$(echo "$lower" | tr '[:lower:]' '[:upper:]')
  openssl=$(openssl_dir "$target")
  [ -n "$openssl" ] || openssl=$brew_openssl
  echo "check-linux: $target" >&2
  env PKG_CONFIG_ALLOW_CROSS=1 "PKG_CONFIG_PATH_$lower=$PWD/$pc" "PKG_CONFIG_LIBDIR_$lower=$PWD/$pc" \
    "${upper}_OPENSSL_DIR=$openssl" \
    cargo-zigbuild check --workspace --all-targets --target "$target" "$@"
done
