//! The programs the Daemon runs (#77), found at fixed system locations instead of on `PATH`.
//!
//! `PATH` belongs to whoever started the Daemon (a shell with the user's `~/bin` first, a wrapper script, an
//! editor's terminal): `pkill -x spotify` or `tar -xzf` must not run a program from there. The directories below
//! come before anything a user installed; on macOS Homebrew's are searched last, for the tools a stock Mac doesn't
//! ship (a pkg-installed `pulseaudio` puts `pactl`/`parec` there).

use std::path::{Path, PathBuf};

/// A program the Daemon runs.
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
        }
    }

    /// The first fixed location that has an executable of this name.
    pub fn find(self) -> Option<PathBuf> {
        self.find_in(Self::DIRS)
    }

    /// [`Tool::find`] in the given directories (tests pass a scratch one).
    pub fn find_in(self, dirs: &[&str]) -> Option<PathBuf> {
        dirs.iter().map(|d| Path::new(d).join(self.name())).find(|p| is_executable(p))
    }

    /// The program's path, or an error that names it and where it was looked for.
    pub fn path(self) -> anyhow::Result<PathBuf> {
        self.path_in(Self::DIRS)
    }

    fn path_in(self, dirs: &[&str]) -> anyhow::Result<PathBuf> {
        self.find_in(dirs).ok_or_else(|| anyhow::anyhow!("`{}` is missing (looked in {})", self.name(), dirs.join(", ")))
    }
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
}
