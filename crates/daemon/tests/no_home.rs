//! Without `HOME` (and the `XDG_*` folders that would stand in for it) the Daemon stops at start and says why (Q93),
//! before it takes the socket.

#[test]
fn without_home_the_daemon_stops_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_singularityd"))
        .env_clear()
        .env("SINGULARITY_SOCKET", &socket)
        .env("SINGULARITY_ENGINE", "fake")
        .env("SINGULARITY_NOWPLAYING", "off")
        .env("SINGULARITY_NO_KEYRING", "1")
        .arg("--no-catalogue")
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert!(err.contains("singularityd: HOME isn't set"), "{err}");
    assert!(!socket.exists(), "the socket was taken");
}
