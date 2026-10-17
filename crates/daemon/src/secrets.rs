//! Secrets (spec D24, #5): the system keyring (Secret Service via `secret-tool`: KWallet, GNOME Keyring, KeePassXC…),
//! with an owner-only file fallback that warns. Items: `service=singularity account=<name>` (`soloist`, `spotify`).

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

fn fallback_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config")).join("singularity/secrets")
}

/// `SINGULARITY_NO_KEYRING=1` skips the keyring (tests, or a deliberate file-only setup).
fn keyring() -> bool {
    std::env::var_os("SINGULARITY_NO_KEYRING").is_none()
}

/// Read a secret: keyring first, then the fallback file.
pub fn get(account: &str) -> Option<String> {
    let out = keyring()
        .then(|| Command::new("secret-tool").args(["lookup", "service", "singularity", "account", account]).stderr(Stdio::null()).output().ok())
        .flatten();
    if let Some(o) = out {
        let v = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if o.status.success() && !v.is_empty() {
            return Some(v);
        }
    }
    let v = std::fs::read_to_string(fallback_dir().join(account)).ok()?.trim().to_string();
    (!v.is_empty()).then_some(v)
}

/// Store a secret in the keyring, or (if there is none) in an owner-only file.
pub fn set(account: &str, value: &str) -> std::io::Result<Stored> {
    let label = format!("Singularity: {account}");
    let child = keyring().then(|| {
        Command::new("secret-tool")
            .args(["store", "--label", &label, "service", "singularity", "account", account])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    });
    if let Some(Ok(mut c)) = child {
        // the value goes over stdin, never on a command line
        let wrote = c.stdin.take().map(|mut s| s.write_all(value.as_bytes()).is_ok()).unwrap_or(false);
        if wrote && c.wait().is_ok_and(|s| s.success()) && get(account).as_deref() == Some(value) {
            let _ = std::fs::remove_file(fallback_dir().join(account));
            return Ok(Stored::Keyring);
        }
    }
    write_private(&fallback_dir().join(account), value)?;
    Ok(Stored::File(fallback_dir().join(account)))
}

/// Remove a secret everywhere.
pub fn clear(account: &str) {
    if keyring() {
        let _ = Command::new("secret-tool").args(["clear", "service", "singularity", "account", account]).stderr(Stdio::null()).status();
    }
    let _ = std::fs::remove_file(fallback_dir().join(account));
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
