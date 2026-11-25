//! App folders (D50, D89): where Singularity keeps its files, decided here and nowhere else. The XDG rule on both OSes:
//! `$XDG_CONFIG_HOME/singularity` else `~/.config/singularity`, and the same for data (`~/.local/share`), state
//! (`~/.local/state`) and cache (`~/.cache`); the Downloads folder for exports; the socket's folder.
//!
//! With `HOME` unset (and the `XDG_*` folder that would need it unset too) there's nowhere to put them: the Client and
//! the Daemon stop at start with [`NoHome`]'s message (Q93), never a temp folder. The socket's folder keeps its own
//! rule, which never needs `HOME`.
//!
//! The rule reads the environment through a lookup, so tests pass a made-up one; [`AppFolders::from_env`] is this
//! process's. Read on every call: a test that sets an `XDG_*` variable is seen at once.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Singularity's own folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppFolders {
    /// `$XDG_CONFIG_HOME/singularity`: config.toml, keys.toml, the secrets file without a keyring
    pub config: PathBuf,
    /// `$XDG_DATA_HOME/singularity`: the listening log, backups, Soloist, the Catalogue's login
    pub data: PathBuf,
    /// `$XDG_STATE_HOME/singularity`: the Daemon's log, recent searches
    pub state: PathBuf,
    /// `$XDG_CACHE_HOME/singularity`: covers, the Catalogue's queries
    pub cache: PathBuf,
    /// `$HOME`, if set
    home: Option<PathBuf>,
    /// `$XDG_CONFIG_HOME` (or `~/.config`): where `user-dirs.dirs` is
    config_home: PathBuf,
    /// `$XDG_DOWNLOAD_DIR`, if set
    download_dir: Option<String>,
}

/// `HOME` isn't set, and an `XDG_*` folder that would stand in for it isn't either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoHome;

impl std::fmt::Display for NoHome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HOME isn't set, so there's nowhere to keep settings and data. Set HOME (or all of XDG_CONFIG_HOME, XDG_DATA_HOME, XDG_STATE_HOME and XDG_CACHE_HOME) and start again.")
    }
}

impl std::error::Error for NoHome {}

/// `$<var>`, else `$HOME/<under_home>`.
fn base(env: &impl Fn(&str) -> Option<OsString>, home: Option<&Path>, var: &str, under_home: &str) -> Result<PathBuf, NoHome> {
    env(var).map(PathBuf::from).or_else(|| home.map(|h| h.join(under_home))).ok_or(NoHome)
}

impl AppFolders {
    /// This process's folders, from its environment.
    pub fn from_env() -> Result<AppFolders, NoHome> {
        AppFolders::from_lookup(|k| std::env::var_os(k))
    }

    /// This process's folders, for code that runs after the start: the Client and the Daemon have already stopped
    /// when they can't be named (`from_env` at start), so only a tool or test run without `HOME` panics here, with
    /// [`NoHome`]'s message.
    pub fn this() -> AppFolders {
        AppFolders::from_env().unwrap_or_else(|e| panic!("{e}"))
    }

    /// The folders an environment names (`env` looks a variable up). A variable that is set counts, even empty.
    pub fn from_lookup(env: impl Fn(&str) -> Option<OsString>) -> Result<AppFolders, NoHome> {
        let home = env("HOME").map(PathBuf::from);
        let h = home.as_deref();
        let config_home = base(&env, h, "XDG_CONFIG_HOME", ".config")?;
        Ok(AppFolders {
            config: config_home.join("singularity"),
            data: base(&env, h, "XDG_DATA_HOME", ".local/share")?.join("singularity"),
            state: base(&env, h, "XDG_STATE_HOME", ".local/state")?.join("singularity"),
            cache: base(&env, h, "XDG_CACHE_HOME", ".cache")?.join("singularity"),
            download_dir: env("XDG_DOWNLOAD_DIR").map(|v| v.to_string_lossy().into_owned()),
            config_home,
            home,
        })
    }

    /// The Downloads folder, for exports: `$XDG_DOWNLOAD_DIR`, else `user-dirs.dirs`'s, else `~/Downloads` (or home if
    /// that's missing). `$HOME` in either is expanded. Reads the disk, so it's asked when needed.
    pub fn downloads(&self) -> Result<PathBuf, NoHome> {
        let expand = |v: &str| -> Result<PathBuf, NoHome> {
            let v = v.trim_matches('"');
            if !v.contains("$HOME") {
                return Ok(PathBuf::from(v));
            }
            let home = self.home.as_ref().ok_or(NoHome)?;
            Ok(PathBuf::from(v.replace("$HOME", &home.to_string_lossy())))
        };
        if let Some(v) = &self.download_dir {
            return expand(v);
        }
        if let Ok(text) = std::fs::read_to_string(self.config_home.join("user-dirs.dirs"))
            && let Some(v) = text.lines().find_map(|l| l.trim().strip_prefix("XDG_DOWNLOAD_DIR="))
        {
            return expand(v);
        }
        let home = self.home.clone().ok_or(NoHome)?;
        Ok(Some(home.join("Downloads")).filter(|d| d.is_dir()).unwrap_or(home))
    }
}

/// The XDG config folder other programs read too (a terminal's config): `$XDG_CONFIG_HOME`, else `~/.config`; `None`
/// without either.
pub fn xdg_config_home() -> Option<PathBuf> {
    base(&|k: &str| std::env::var_os(k), std::env::var_os("HOME").map(PathBuf::from).as_deref(), "XDG_CONFIG_HOME", ".config").ok()
}

/// The socket's folder, by its own rule: `$XDG_RUNTIME_DIR/singularity`, else `<temp>/singularity-<uid>/singularity`
/// (macOS has no `XDG_RUNTIME_DIR`; its temp folder is per user). Never needs `HOME`.
pub fn socket_dir() -> PathBuf {
    socket_dir_from(|k| std::env::var_os(k), &std::env::temp_dir())
}

/// [`socket_dir`] for an environment and a temp folder.
pub fn socket_dir_from(env: impl Fn(&str) -> Option<OsString>, temp: &Path) -> PathBuf {
    let base = env("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| temp.join(format!("singularity-{}", uid(&env))));
    base.join("singularity")
}

/// `$UID`, else the Host's user id. A Mac's folder has always been `singularity-user` (this read `/proc`, which macOS
/// lacks): renaming it would strand a running Daemon on the old path, so it keeps that name.
fn uid(env: &impl Fn(&str) -> Option<OsString>) -> String {
    use crate::host;
    env("UID").and_then(|v| v.into_string().ok()).unwrap_or_else(|| if host::MACOS { "user".into() } else { host::uid().to_string() })
}

#[cfg(test)]
mod tests;
