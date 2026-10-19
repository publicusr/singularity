//! Secrets without a keyring (#64): an owner-only file, the same on Linux and macOS. Its own test binary, so setting
//! the environment can't race other tests.

use singularityd::secrets::{self, Stored};

#[test]
fn without_a_keyring_secrets_go_to_an_owner_only_file() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: this binary's only test sets them before anything reads them
    unsafe {
        std::env::set_var("SINGULARITY_NO_KEYRING", "1");
        std::env::set_var("XDG_CONFIG_HOME", dir.path());
    }
    let file = dir.path().join("singularity/secrets/probe");
    assert_eq!(secrets::set("probe", "{\"a\": \"b c\"}").unwrap(), Stored::File(file.clone()));
    assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(secrets::get("probe").as_deref(), Some("{\"a\": \"b c\"}"));
    assert_eq!(secrets::in_keyring("probe"), None, "the keyring is never asked");
    secrets::clear("probe");
    assert_eq!(secrets::get("probe"), None);
}
