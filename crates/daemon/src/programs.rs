//! The programs the Daemon runs (#77, audit S3/S10), found at fixed system locations instead of on `PATH`, and
//! started without the secrets the Daemon read from its own environment.
//!
//! `PATH` belongs to whoever started the Daemon (a shell with the user's `~/bin` first, a wrapper script, an
//! editor's terminal): `pkill -x spotify` or `tar -xzf` must not run a program from there. The directories below
//! come before anything a user installed; on macOS Homebrew's are searched last, for the tools a stock Mac doesn't
//! ship (a pkg-installed `pulseaudio` puts `pactl`/`parec` there).

use std::path::{Path, PathBuf};

/// The one variable the Daemon reads a secret from (`soloist.rs::api_key`). Children have no use for it (S10).
pub const SECRET_ENV: &str = "SOLOIST_API_KEY";

/// A program the Daemon runs, or one the Client starts for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    /// looks a process up by name (the Spotify app's)
    Pgrep,
    /// ends it ("Restart the player")
    Pkill,
    /// unpacks Soloist's build
    Tar,
    /// the Secret Service client behind the Linux keyring
    SecretTool,
    /// PipeWire's control tool and recorder (the audio tap, and the health check's output name)
    Pactl,
    Parec,
    /// macOS: starts the Spotify app hidden (`open -gj -a Spotify`)
    Open,
    /// the Spotify app's own binary (Linux; on macOS the app is a bundle in /Applications)
    Spotify,
    /// macOS: reads the app's `Info.plist` and this Mac's name
    Defaults,
    Scutil,
    /// Linux: this computer's name, when `/etc/hostname` doesn't say it
    Hostname,
    /// Linux: the Client's browser opener for the Spotify login page
    XdgOpen,
}

impl Tool {
    /// Where programs are looked for, in order. `/bin` is a symlink to `/usr/bin` on most systems but not all.
    pub const DIRS: &'static [&'static str] = if cfg!(target_os = "macos") { &["/usr/bin", "/bin", "/opt/homebrew/bin", "/usr/local/bin"] } else { &["/usr/bin", "/bin", "/usr/local/bin"] };

    /// The tools this OS's Daemon uses; the health check names the ones it can't find.
    pub const REQUIRED: &'static [Tool] = if cfg!(target_os = "macos") { &[Tool::Pgrep, Tool::Pkill, Tool::Tar] } else { &[Tool::Pgrep, Tool::Pkill, Tool::Tar, Tool::SecretTool, Tool::Pactl, Tool::Parec] };

    pub fn name(self) -> &'static str {
        match self {
            Tool::Pgrep => "pgrep",
            Tool::Pkill => "pkill",
            Tool::Tar => "tar",
            Tool::SecretTool => "secret-tool",
            Tool::Pactl => "pactl",
            Tool::Parec => "parec",
            Tool::Open => "open",
            Tool::Spotify => "spotify",
            Tool::Defaults => "defaults",
            Tool::Scutil => "scutil",
            Tool::Hostname => "hostname",
            Tool::XdgOpen => "xdg-open",
        }
    }

    /// Where this tool is looked for. Most are in [`Tool::DIRS`]; `scutil` is in `/usr/sbin`, and a Linux Spotify is
    /// also installed by snap (still a fixed, root-owned location, not a user's `PATH`).
    pub fn dirs(self) -> &'static [&'static str] {
        match self {
            Tool::Scutil => &["/usr/sbin"],
            Tool::Spotify if !cfg!(target_os = "macos") => LINUX_SPOTIFY_DIRS,
            _ => Self::DIRS,
        }
    }

    /// The first fixed location that has an executable of this name.
    pub fn find(self) -> Option<PathBuf> {
        self.find_in(self.dirs())
    }

    /// [`Tool::find`] in the given directories (tests pass a scratch one).
    pub fn find_in(self, dirs: &[&str]) -> Option<PathBuf> {
        dirs.iter().map(|d| Path::new(d).join(self.name())).find(|p| is_executable(p))
    }

    /// The program's path, or an error that names it and where it was looked for.
    pub fn path(self) -> anyhow::Result<PathBuf> {
        self.path_in(self.dirs())
    }

    /// [`Tool::path`] with an explicit list (tests pass an empty scratch directory).
    pub fn path_in(self, dirs: &[&str]) -> anyhow::Result<PathBuf> {
        self.find_in(dirs).ok_or_else(|| anyhow::anyhow!("`{}` is missing (looked in {})", self.name(), dirs.join(", ")))
    }
}

/// Linux: the system locations a Spotify install uses (a `.deb`/`.rpm` puts it in `/usr/bin`, snap in `/snap/bin`).
const LINUX_SPOTIFY_DIRS: &[&str] = &["/usr/bin", "/bin", "/usr/local/bin", "/snap/bin"];

/// The engine-specific tool the health view should name when it's missing: the app's own launcher.
pub fn app_tool() -> Tool {
    if cfg!(target_os = "macos") { Tool::Open } else { Tool::Spotify }
}

/// A child process that never inherits the Daemon's secret (S10: the app, `pactl` and the rest have no use for it).
pub fn command(prog: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    let mut c = std::process::Command::new(prog);
    c.env_remove(SECRET_ENV);
    c
}

/// [`command`] for the daemon's async spawns (Soloist, the VM's helpers, `tar`).
pub fn tokio_command(prog: impl AsRef<std::ffi::OsStr>) -> tokio::process::Command {
    let mut c = tokio::process::Command::new(prog);
    c.env_remove(SECRET_ENV);
    c
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_locations_win_over_anything_else() {
        let scratch = tempfile::tempdir().unwrap();
        let fake = scratch.path().join("pgrep");
        std::fs::write(&fake, "#!/bin/sh\nexit 0\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let dirs = [scratch.path().to_str().unwrap()];
        assert_eq!(Tool::Pgrep.find_in(&dirs).as_deref(), Some(fake.as_path()), "a scratch directory is usable when it is searched");
        // the real lookup never looks there: only `Tool::DIRS` is consulted
        if let Some(found) = Tool::Pgrep.find() {
            assert_ne!(found, fake, "PATH and a scratch directory are ignored");
            assert!(Tool::DIRS.iter().any(|d| found == Path::new(d).join("pgrep")), "{} is not a fixed location", found.display());
        }
    }

    #[test]
    fn a_missing_tool_is_named_with_where_it_was_looked_for() {
        let empty = tempfile::tempdir().unwrap();
        let dirs = [empty.path().to_str().unwrap()];
        assert!(Tool::Parec.find_in(&dirs).is_none());
        let err = Tool::Parec.path_in(&dirs).unwrap_err().to_string();
        assert!(err.contains("parec") && err.contains(dirs[0]), "{err}");
        assert!(!is_executable(&empty.path().join("nope")));
    }

    /// S3: the rest of the programs are fixed locations too, and none of them is a bare name.
    #[test]
    fn the_new_programs_have_fixed_locations() {
        for t in [Tool::Open, Tool::Spotify, Tool::Defaults, Tool::Scutil, Tool::Hostname, Tool::XdgOpen] {
            assert!(t.dirs().iter().all(|d| d.starts_with('/')), "{} is looked up in {:?}", t.name(), t.dirs());
            assert!(t.find().is_none_or(|p| p.is_absolute()));
        }
        if cfg!(target_os = "macos") {
            assert_eq!(Tool::Scutil.dirs(), ["/usr/sbin"]);
        } else {
            assert!(Tool::Spotify.dirs().contains(&"/snap/bin"));
        }
    }

    /// S10: a child starts without the secret its parent may carry.
    #[test]
    fn a_child_does_not_inherit_the_secret() {
        let c = command("/bin/sh");
        let env = c.get_envs().find(|(k, _)| *k == SECRET_ENV);
        assert_eq!(env, Some((std::ffi::OsStr::new(SECRET_ENV), None)), "the secret is removed from the child's environment");
        let c = tokio_command("/bin/sh");
        assert!(c.as_std().get_envs().any(|(k, v)| k == SECRET_ENV && v.is_none()));
    }
}
