//! #167: an argument that isn't a switch, or a `SINGULARITY_DEMO` that isn't a yes, stops the Client with a short
//! error pointing at `--help`, before it starts a Daemon or touches the terminal; `--help` lists the switches.

/// The Client run with `args` (and `SINGULARITY_DEMO=demo`, if given) in a scratch dir: its exit code, stdout, stderr,
/// and whether a Daemon was started.
fn run(args: &[&str], demo: Option<&str>) -> (Option<i32>, String, String, bool) {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_singularity"));
    cmd.args(args)
        .env_clear()
        .env("HOME", dir.path())
        .env("SINGULARITY_SOCKET", &socket)
        .env("SINGULARITY_START_APP", "0")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(v) = demo {
        cmd.env("SINGULARITY_DEMO", v);
    }
    let mut child = cmd.spawn().unwrap();
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
    (out.status.code(), String::from_utf8_lossy(&out.stdout).into(), String::from_utf8_lossy(&out.stderr).into(), socket.exists())
}

#[test]
fn a_typo_or_a_bad_demo_value_stops_the_client_and_points_at_help() {
    for (args, demo, want) in [
        (&["--dmeo"][..], None, "singularity: unknown argument '--dmeo' (see singularity --help)"),
        (&["-demo"][..], None, "singularity: unknown argument '-demo' (see singularity --help)"),
        (&[][..], Some("maybe"), "singularity: SINGULARITY_DEMO='maybe' isn't 1, true or yes (see singularity --help)"),
    ] {
        let (code, out, err, daemon) = run(args, demo);
        assert_eq!((code, err.trim_end()), (Some(2), want), "{args:?} {demo:?}");
        assert!(out.is_empty(), "the terminal was written to: {out:?}");
        assert!(!daemon, "a Daemon was started");
    }
}

#[test]
fn help_lists_the_switches() {
    let (code, out, err, daemon) = run(&["--help"], None);
    assert_eq!(code, Some(0), "{err}");
    assert!(out.starts_with("Usage: singularity [--demo]") && out.contains("--version") && out.contains("--help"), "{out}");
    assert!(!daemon, "a Daemon was started");
}
