//! Seam for #77: the Daemon runs its programs from fixed system locations, never from `PATH`.

use singularityd::programs::Tool;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

#[test]
fn a_fake_pgrep_earlier_on_the_path_is_not_used() {
    let scratch = tempfile::tempdir().unwrap();
    let fake = scratch.path().join("pgrep");
    std::fs::write(&fake, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let old = std::env::var_os("PATH");
    let mut path = std::ffi::OsString::from(scratch.path());
    if let Some(old) = &old {
        path.push(":");
        path.push(old);
    }
    // SAFETY: this test binary has no other test that reads `PATH`.
    unsafe { std::env::set_var("PATH", &path) };
    let found = Tool::Pgrep.find();
    match &old {
        Some(old) => unsafe { std::env::set_var("PATH", old) },
        None => unsafe { std::env::remove_var("PATH") },
    }
    assert_ne!(found.as_deref(), Some(fake.as_path()), "the fake on PATH was used");
    if let Some(found) = found {
        assert!(Tool::DIRS.iter().any(|d| found == Path::new(d).join("pgrep")), "{} is not a fixed location", found.display());
    }
}
