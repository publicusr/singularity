#!/bin/sh
# Start the Singularity Daemon at login on macOS (a LaunchAgent), or stop doing so.
#   packaging/launchd/install.sh              install (after `cargo install --locked --path crates/daemon`)
#   packaging/launchd/install.sh --uninstall  remove
# SINGULARITYD=<path> picks another binary; LABEL=<label> another job name (tests).
set -e
here="$(cd "$(dirname "$0")" && pwd)"
label="${LABEL:-io.github.publicusr.singularityd}"
plist="$HOME/Library/LaunchAgents/$label.plist"
domain="gui/$(id -u)"
if [ "$1" = "--uninstall" ]; then
  launchctl bootout "$domain/$label" 2>/dev/null || true
  rm -f "$plist"
  echo "removed $label"
  exit 0
fi
bin="${SINGULARITYD:-$HOME/.cargo/bin/singularityd}"
[ -x "$bin" ] || { echo "no singularityd at $bin (cargo install --locked --path crates/daemon)" >&2; exit 1; }
log="${XDG_STATE_HOME:-$HOME/.local/state}/singularity/daemon.log"
mkdir -p "$(dirname "$log")" "$HOME/Library/LaunchAgents"
# owner-only (it names the account); launchd keeps the mode of a file that's already there
touch "$log" && chmod 600 "$log"
tmp="$(getconf DARWIN_USER_TEMP_DIR)"
sed -e "s|@BIN@|$bin|" -e "s|@LOG@|$log|" -e "s|@TMPDIR@|$tmp|" -e "s|io.github.publicusr.singularityd|$label|" "$here/singularityd.plist" > "$plist"
plutil -lint -s "$plist"
# load it fresh (a running Daemon started by a Client keeps the socket; this one then exits as "already running"
# and starts at the next login)
launchctl bootout "$domain/$label" 2>/dev/null || true
launchctl bootstrap "$domain" "$plist"
echo "installed $label: starts at login; log in $log"
