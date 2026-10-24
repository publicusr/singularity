//! Secrets (spec D24, #5, #64): the system keyring, with an owner-only file fallback that warns. Items:
//! `service=singularity account=<name>` (`soloist`, `spotify`).
//!
//! - Linux: Secret Service via `secret-tool` (KWallet, GNOME Keyring, KeePassXC…)
//! - macOS: the login Keychain via `/usr/bin/security`. Items it creates trust that (Apple-signed) tool, so a rebuilt
//!   `singularityd` reads them without a Keychain prompt. Values go over stdin (`security -i`, hex), never on a
//!   command line.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Where a secret ended up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stored {
    Keyring,
    /// the keyring wasn't available: an owner-only file (0600) instead
    File(PathBuf),
}

/// What this OS calls its keyring, for messages.
pub const KEYRING: &str = if cfg!(target_os = "macos") { "Keychain" } else { "keyring" };

fn fallback_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config")).join("singularity/secrets")
}

/// `SINGULARITY_NO_KEYRING=1` skips the keyring (tests, or a deliberate file-only setup).
fn keyring() -> bool {
    std::env::var_os("SINGULARITY_NO_KEYRING").is_none()
}

/// Is the keyring in use? (Then its client tool is one the health check names when missing.)
pub fn keyring_enabled() -> bool {
    keyring()
}

/// The keyring's copy of a secret, if there is one.
pub fn in_keyring(account: &str) -> Option<String> {
    let o = keyring().then(|| store::lookup(account)?.stderr(Stdio::null()).output().ok()).flatten()?;
    let v = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (o.status.success() && !v.is_empty()).then_some(v)
}

/// Read a secret: keyring first, then the fallback file.
pub fn get(account: &str) -> Option<String> {
    if let Some(v) = in_keyring(account) {
        return Some(v);
    }
    let v = std::fs::read_to_string(fallback_dir().join(account)).ok()?.trim().to_string();
    (!v.is_empty()).then_some(v)
}

/// Store a secret in the keyring, or (if there is none) in an owner-only file.
pub fn set(account: &str, value: &str) -> std::io::Result<Stored> {
    if keyring() && store::store(account, value) && in_keyring(account).as_deref() == Some(value) {
        let _ = std::fs::remove_file(fallback_dir().join(account));
        return Ok(Stored::Keyring);
    }
    write_private(&fallback_dir().join(account), value)?;
    Ok(Stored::File(fallback_dir().join(account)))
}

/// Remove a secret everywhere.
pub fn clear(account: &str) {
    if keyring() && let Some(mut c) = store::remove(account) {
        let _ = c.stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    let _ = std::fs::remove_file(fallback_dir().join(account));
}

/// Run `cmd` with `input` on its stdin; true if it took it and succeeded.
fn feed(mut cmd: Command, input: &str) -> bool {
    let Ok(mut c) = cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else { return false };
    let wrote = c.stdin.take().map(|mut s| s.write_all(input.as_bytes()).is_ok()).unwrap_or(false);
    wrote && c.wait().is_ok_and(|s| s.success())
}

#[cfg(not(target_os = "macos"))]
mod store {
    use std::process::Command;

    pub fn lookup(account: &str) -> Option<Command> {
        let mut c = Command::new(crate::programs::Tool::SecretTool.find()?);
        c.args(["lookup", "service", "singularity", "account", account]);
        Some(c)
    }

    /// The value goes over stdin, never on a command line.
    pub fn store(account: &str, value: &str) -> bool {
        let Some(mut c) = crate::programs::Tool::SecretTool.find().map(Command::new) else { return false };
        c.args(["store", "--label", &format!("Singularity: {account}"), "service", "singularity", "account", account]);
        super::feed(c, value)
    }

    pub fn remove(account: &str) -> Option<Command> {
        let mut c = Command::new(crate::programs::Tool::SecretTool.find()?);
        c.args(["clear", "service", "singularity", "account", account]);
        Some(c)
    }
}

#[cfg(target_os = "macos")]
mod store {
    use std::process::Command;

    const SECURITY: &str = "/usr/bin/security";

    pub fn lookup(account: &str) -> Option<Command> {
        let mut c = Command::new(SECURITY);
        c.args(["find-generic-password", "-s", "singularity", "-a", account, "-w"]);
        Some(c)
    }

    /// `security -i` reads the command from stdin, the value hex-encoded (`-X`): never on a command line, no quoting.
    /// `-U` replaces an existing item.
    pub fn store(account: &str, value: &str) -> bool {
        if !account.chars().all(|c| c.is_ascii_alphanumeric()) {
            return false;
        }
        let hex: String = value.bytes().map(|b| format!("{b:02x}")).collect();
        let line = format!("add-generic-password -U -s singularity -a {account} -l \"Singularity: {account}\" -X {hex}\n");
        let mut c = Command::new(SECURITY);
        c.arg("-i");
        super::feed(c, &line)
    }

    pub fn remove(account: &str) -> Option<Command> {
        let mut c = Command::new(SECURITY);
        c.args(["delete-generic-password", "-s", "singularity", "-a", account]);
        Some(c)
    }
}

/// Write `value` to `path`, readable by its owner only (directory 0700, file 0600).
pub fn write_private(path: &std::path::Path, value: &str) -> std::io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(path)?;
    f.write_all(value.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s/secret");
        write_private(&p, "x").unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(p.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
    }
}
