//! The Spotify app on macOS: a bundle in /Applications or ~/Applications, started hidden with `open -gj`.

use crate::app_engine::App;
use crate::programs::{Tool, command};
use std::path::PathBuf;

/// The name the app's process runs under.
pub const PROCESS: &str = "Spotify";

/// Where the app was looked for, for the message when it isn't there.
pub const SEARCHED: &str = "looked in /Applications and ~/Applications";

/// The app, if it's installed.
pub fn installed() -> Option<App> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let path = [PathBuf::from("/Applications/Spotify.app"), home.join("Applications/Spotify.app")].into_iter().find(|p| p.join("Contents/Info.plist").exists())?;
    let plist = path.join("Contents/Info.plist");
    let version = Tool::Defaults.find().and_then(|d| output(&d, &["read", &plist.to_string_lossy(), "CFBundleShortVersionString"])).unwrap_or_default();
    Some(App { path, version })
}

/// Start the app without showing it.
pub fn start() -> std::io::Result<()> {
    // -g: don't bring it to the foreground; -j: launch it hidden
    let open = Tool::Open.find().ok_or_else(|| std::io::Error::other("`open` is missing (looked in /usr/bin)"))?;
    let s = command(open).args(["-gj", "-a", "Spotify"]).status()?;
    if s.success() { Ok(()) } else { Err(std::io::Error::other(format!("`open -gj -a Spotify` failed ({s})"))) }
}

/// A command's trimmed output, if it worked.
fn output(exe: &std::path::Path, args: &[&str]) -> Option<String> {
    let o = command(exe).args(args).stderr(std::process::Stdio::null()).output().ok()?;
    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (o.status.success() && !s.is_empty()).then_some(s)
}
