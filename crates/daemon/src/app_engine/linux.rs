//! The Spotify app on Linux: a `spotify` binary in a fixed system folder (a package's, or snap's), started minimized.

use crate::app_engine::App;
use crate::programs::{Tool, command};

/// The name the app's process runs under.
pub const PROCESS: &str = "spotify";

/// Where the app was looked for, for the message when it isn't there.
pub const SEARCHED: &str = "looked in /usr/bin, /bin, /usr/local/bin and /snap/bin";

/// The app, if it's installed.
pub fn installed() -> Option<App> {
    // a fixed location, never `PATH` (S3): a planted `spotify` would run with the Daemon's privileges
    let path = Tool::Spotify.find()?;
    Some(App { path, version: String::new() })
}

/// Start the app without showing it.
pub fn start() -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    let null = || std::process::Stdio::null();
    let spotify = Tool::Spotify.find().ok_or_else(|| std::io::Error::other(format!("the Spotify app is missing ({SEARCHED})")))?;
    // its own process group, so it outlives the Daemon like the desktop app it is
    command(spotify).arg("--minimized").stdin(null()).stdout(null()).stderr(null()).process_group(0).spawn().map(|_| ())
}
