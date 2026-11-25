//! Without `HOME` (and the `XDG_*` folders that would stand in for it) the Client stops at start and says why (Q93),
//! before it starts a Daemon or touches the terminal.

#[test]
fn without_home_the_client_stops_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_singularity"))
        .env_clear()
        .env("SINGULARITY_SOCKET", &socket)
        .env("SINGULARITY_START_APP", "0")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // it must stop on its own; one that went on is stopped here
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let out = child.wait_with_output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert!(err.contains("singularity: HOME isn't set"), "{err}");
    assert!(out.stdout.is_empty(), "the terminal was written to: {:?}", String::from_utf8_lossy(&out.stdout));
    assert!(!socket.exists(), "a Daemon was started");
}
