//! Q95: the Soloist VM's runner (macOS's Soloist runner) against a fake VM, so its contract runs on Linux too: launch,
//! version, the WebSocket's port, a staged update, the LAN announcement and stopping; then the Soloist session's
//! supervisor driving it (the VM taken down on quit and after a failed start, a dead VM brought up again, an expired
//! build). The fake guest is a scratch home where the runner's scripts run in `sh`, with a stand-in `sgl-soloist`
//! (ensure / check / install, against a fake CDN folder) and a stand-in Soloist that prints its arguments; the real VM
//! runs only on the Mac.

use futures_util::{SinkExt, StreamExt};
use singularity_proto::{Command, EngineStatus};
use singularityd::engine::{Engine, EngineEvent};
use singularityd::soloist::SoloistEngine;
use singularityd::soloist::runner::vm::{GUEST_WS, Guest, InVm};
use singularityd::soloist::runner::{Announcement, Runner};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::AsyncBufReadExt;

/// A stand-in for the guest's update script (the real one is in `vm.rs`'s setup, its contract pinned by a test there):
/// the same commands and answers, but versions compared by their file (the real one compares the CDN's ETag), and the
/// CDN a folder.
const SGL_SOLOIST: &str = r#"#!/bin/sh
set -e
stage() {
  [ "$(cat "$FAKE_CDN/version")" = "$(cat soloist/version 2>/dev/null)" ] && { echo current; return; }
  rm -rf soloist.new && mkdir soloist.new && cp "$FAKE_CDN/soloist" "$FAKE_CDN/version" soloist.new/
  echo staged
}
install() { [ -d soloist.new ] || return 0; rm -rf soloist.old; [ -d soloist ] && mv soloist soloist.old; mv soloist.new soloist; echo installed; }
case "$1" in
  ensure) [ -x soloist/soloist ] || { stage >/dev/null; install; }; cat soloist/version ;;
  check) stage ;;
  install) install ;;
esac
"#;

/// A stand-in Soloist: says how it was started, then plays until stopped. A `dead` file in the control folder makes it
/// stop (with the VM), an `expire` file makes it stop as an expired build does (exit code 10), once.
const SOLOIST: &str = r#"#!/bin/sh
echo "started: $*"
while :; do
  [ -f "$FAKE_CTL/dead" ] && exit 1
  [ -f "$FAKE_CTL/expire" ] && { rm -f "$FAKE_CTL/expire"; exit 10; }
  sleep 0.1
done
"#;

/// What the fake VM was asked to do.
#[derive(Default)]
struct Seen {
    boots: u32,
    power_offs: u32,
    /// every script run in the guest
    scripts: Vec<String>,
    /// (address on the Mac, guest port)
    forwards: Vec<(String, u16)>,
    /// (loopback port, name)
    published: Vec<(u16, String)>,
}

/// The test's handle on the fake VM the runner owns.
#[derive(Clone)]
struct Fake {
    home: PathBuf,
    cdn: PathBuf,
    /// the stand-in Soloist's control files
    ctl: PathBuf,
    /// Soloist's WebSocket answers on the forward (logged in), as it does once Soloist runs
    answers_ws: Arc<AtomicBool>,
    /// powered on (a dead VM answers nothing)
    on: Arc<AtomicBool>,
    /// the next boot fails with this
    broken: Arc<Mutex<Option<String>>>,
    seen: Arc<Mutex<Seen>>,
    withdrawn: Arc<AtomicBool>,
}

impl Fake {
    fn new(root: &Path) -> Fake {
        let (home, cdn, ctl) = (root.join("guest"), root.join("cdn"), root.join("ctl"));
        std::fs::create_dir_all(home.join("bin")).unwrap();
        std::fs::create_dir_all(&cdn).unwrap();
        std::fs::create_dir_all(&ctl).unwrap();
        executable(&home.join("bin/sgl-soloist"), SGL_SOLOIST);
        executable(&cdn.join("soloist"), SOLOIST);
        let fake = Fake { home, cdn, ctl, answers_ws: Default::default(), on: Default::default(), broken: Default::default(), seen: Default::default(), withdrawn: Default::default() };
        fake.release("Soloist 1.0.0");
        fake
    }

    /// Spotify's CDN now has this build.
    fn release(&self, version: &str) {
        std::fs::write(self.cdn.join("version"), format!("{version}\n")).unwrap();
    }

    fn seen<T>(&self, f: impl FnOnce(&Seen) -> T) -> T {
        f(&self.seen.lock().unwrap())
    }

    /// The guest port's forward on the Mac.
    fn forward_of(&self, port: u16) -> Option<String> {
        self.seen(|s| s.forwards.iter().rev().find(|f| f.1 == port).map(|f| f.0.clone()))
    }
}

fn executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Released when dropped: the announcement is withdrawn.
struct Withdraw(Arc<AtomicBool>);

impl Drop for Withdraw {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl Guest for Fake {
    async fn boot(&self, say: &(dyn Fn(String) + Sync)) -> anyhow::Result<()> {
        self.seen.lock().unwrap().boots += 1;
        if let Some(e) = self.broken.lock().unwrap().take() {
            anyhow::bail!(e);
        }
        say("Starting the Soloist VM…".into());
        let _ = std::fs::remove_file(self.ctl.join("dead"));
        self.on.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn power_off(&self) {
        self.seen.lock().unwrap().power_offs += 1;
        self.on.store(false, Ordering::SeqCst);
    }

    fn command(&self, script: &str) -> tokio::process::Command {
        self.seen.lock().unwrap().scripts.push(script.to_string());
        let mut c = tokio::process::Command::new("sh");
        if self.on.load(Ordering::SeqCst) {
            c.arg("-c").arg(script);
        } else {
            c.args(["-c", "echo 'connect: Connection refused' >&2; exit 255"]);
        }
        let path = format!("{}:{}", self.home.join("bin").display(), std::env::var("PATH").unwrap_or_default());
        c.current_dir(&self.home).env("PATH", path).env("FAKE_CDN", &self.cdn).env("FAKE_CTL", &self.ctl).stdin(std::process::Stdio::null()).kill_on_drop(true);
        c
    }

    /// The forward listens on a port the system picks, held from the start (#118: a port picked and let go can be
    /// taken by a parallel test before it is bound). Only Soloist's WebSocket answers, once `answers_ws` is set.
    async fn expose(&self, port: u16) -> anyhow::Result<u16> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let local = listener.local_addr()?.port();
        self.seen.lock().unwrap().forwards.push((format!("127.0.0.1:{local}"), port));
        let answers = self.answers_ws.clone();
        tokio::spawn(async move {
            while let Ok((s, _)) = listener.accept().await {
                if port != GUEST_WS || !answers.load(Ordering::SeqCst) {
                    continue;
                }
                tokio::spawn(async move {
                    let Ok(mut ws) = tokio_tungstenite::accept_async(s).await else { return };
                    let hello = serde_json::json!({"type": "auth_state", "logged_in": true, "is_active": true});
                    let _ = ws.send(tokio_tungstenite::tungstenite::Message::text(hello.to_string())).await;
                    while let Some(Ok(_)) = ws.next().await {}
                });
            }
        });
        Ok(local)
    }

    async fn publish(&self, port: u16, name: &str) -> anyhow::Result<Announcement> {
        self.seen.lock().unwrap().published.push((port, name.to_string()));
        Ok(Announcement::held(Withdraw(self.withdrawn.clone())))
    }
}

/// A VM runner on a fresh fake VM, and the notices its `prepare` sent.
fn runner(root: &Path) -> (InVm<Fake>, Fake) {
    let fake = Fake::new(root);
    (InVm::new(fake.clone(), root.join("data")), fake)
}

async fn prepare(vm: &InVm<Fake>) -> (anyhow::Result<()>, Vec<String>) {
    let said = Mutex::new(Vec::new());
    let r = vm.prepare(&|m| said.lock().unwrap().push(m)).await;
    (r, said.into_inner().unwrap())
}

#[tokio::test]
async fn the_first_prepare_boots_the_vm_forwards_its_websocket_and_fetches_a_build() {
    let dir = tempfile::tempdir().unwrap();
    let (vm, fake) = runner(dir.path());
    assert!(vm.in_vm());
    let (r, said) = prepare(&vm).await;
    r.unwrap();
    assert_eq!(said, ["Starting the Soloist VM…", "Downloading Soloist from Spotify into the VM…"], "the VM's progress and the download go to the status bar");
    assert_eq!(fake.seen(|s| s.boots), 1);
    assert!(fake.forward_of(GUEST_WS).is_some_and(|a| a.starts_with("127.0.0.1:")), "Soloist's WebSocket on the Mac's loopback");
    assert_eq!(vm.version().await, "Soloist 1.0.0");
    assert!(!vm.update_waiting());
    // the VM stays up between runs: no second boot, and the build is there
    let (r, said) = prepare(&vm).await;
    r.unwrap();
    assert!(said.is_empty(), "{said:?}");
    assert_eq!(fake.seen(|s| s.boots), 1);
}

#[tokio::test]
async fn a_build_staged_in_the_guest_goes_in_at_the_next_prepare() {
    let dir = tempfile::tempdir().unwrap();
    let (vm, fake) = runner(dir.path());
    prepare(&vm).await.0.unwrap();
    fake.release("Soloist 1.1.0");
    let staged = Arc::new(tokio::sync::Notify::new());
    let checker = tokio::spawn(vm.keep_current(tokio::sync::watch::channel(Some(80)).1, staged.clone()));
    tokio::time::timeout(Duration::from_secs(10), staged.notified()).await.expect("the new build is announced as staged");
    checker.abort();
    assert!(vm.update_waiting());
    assert_eq!(vm.version().await, "Soloist 1.0.0", "the running build stays until Soloist is restarted");
    prepare(&vm).await.0.unwrap();
    assert_eq!(vm.version().await, "Soloist 1.1.0");
    assert!(!vm.update_waiting());
    // an expired build is the guest's to replace: Soloist starts again at once
    assert!(!vm.expired());
}

#[tokio::test]
async fn launch_hands_the_key_in_on_stdin_and_closing_it_stops_soloist() {
    let dir = tempfile::tempdir().unwrap();
    let (vm, fake) = runner(dir.path());
    prepare(&vm).await.0.unwrap();
    vm.check_free().unwrap();
    let mut run = vm.launch("sekrit-key", "Singularity on the Mac (lossless)", Some(55)).await.unwrap();
    let mut out = tokio::io::BufReader::new(run.child.stdout.take().unwrap()).lines();
    let line = tokio::time::timeout(Duration::from_secs(10), out.next_line()).await.unwrap().unwrap().unwrap();
    assert!(line.starts_with("started: --device-name Singularity on the Mac (lossless) --api-key sekrit-key --initial-volume 55 --ws 0.0.0.0:7700 "), "{line}");
    assert!(fake.seen(|s| s.scripts.iter().all(|c| !c.contains("sekrit-key"))), "the key never goes on a command line");
    // Soloist runs while the session's stdin is held
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(run.child.try_wait().unwrap().is_none());
    drop(run.hold.take().expect("the session's stdin is held for the run"));
    let exit = tokio::time::timeout(Duration::from_secs(10), run.child.wait()).await.expect("closing stdin stops Soloist");
    exit.unwrap();
    // no saved volume: Soloist's own 40 %
    let mut run = vm.launch("k", "n", None).await.unwrap();
    let mut out = tokio::io::BufReader::new(run.child.stdout.take().unwrap()).lines();
    let line = tokio::time::timeout(Duration::from_secs(10), out.next_line()).await.unwrap().unwrap().unwrap();
    assert!(line.contains("--initial-volume 40 "), "{line}");
}

#[tokio::test]
async fn the_websocket_is_ready_once_it_answers_through_the_forward() {
    let dir = tempfile::tempdir().unwrap();
    let (vm, fake) = runner(dir.path());
    prepare(&vm).await.0.unwrap();
    assert_eq!(vm.ws_addr().await, None, "nothing answers on the forward yet");
    let local = fake.forward_of(GUEST_WS).unwrap();
    // Soloist's WebSocket comes up behind the forward
    fake.answers_ws.store(true, Ordering::SeqCst);
    assert_eq!(vm.ws_addr().await, Some(format!("ws://{local}")));
}

#[tokio::test]
async fn pairing_forwards_the_zeroconf_port_and_announces_it_until_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let (vm, fake) = runner(dir.path());
    prepare(&vm).await.0.unwrap();
    let announced = vm.announce(4070, "Singularity on the Mac (lossless)").await.unwrap();
    let local = fake.forward_of(4070).expect("the guest's ZeroConf port forwarded to the Mac");
    let port: u16 = local.strip_prefix("127.0.0.1:").expect("on loopback").parse().unwrap();
    assert_eq!(fake.seen(|s| s.published.clone()), [(port, "Singularity on the Mac (lossless)".to_string())], "the forward is what's announced");
    assert!(!fake.withdrawn.load(Ordering::SeqCst));
    drop(announced);
    assert!(fake.withdrawn.load(Ordering::SeqCst), "logged in: the announcement goes");
}

#[tokio::test]
async fn a_vm_that_went_away_is_seen_and_brought_up_again() {
    let dir = tempfile::tempdir().unwrap();
    let (vm, fake) = runner(dir.path());
    fake.answers_ws.store(true, Ordering::SeqCst);
    prepare(&vm).await.0.unwrap();
    assert!(vm.alive().await);
    assert!(vm.ws_addr().await.is_some());
    // the VM dies under Soloist
    fake.on.store(false, Ordering::SeqCst);
    assert!(!vm.alive().await);
    vm.down().await;
    assert_eq!(fake.seen(|s| s.power_offs), 1);
    // (the fake's old forward still answers: the runner mustn't look there)
    assert_eq!(vm.ws_addr().await, None, "a VM taken down has no WebSocket forward");
    prepare(&vm).await.0.unwrap();
    assert_eq!(fake.seen(|s| s.boots), 2, "the next round boots it again");
    assert_eq!(vm.version().await, "Soloist 1.0.0");
}

#[tokio::test]
async fn a_vm_that_doesnt_come_up_says_so_and_is_tried_again() {
    let dir = tempfile::tempdir().unwrap();
    let (vm, fake) = runner(dir.path());
    *fake.broken.lock().unwrap() = Some("vfkit stopped".into());
    let (r, _) = prepare(&vm).await;
    assert_eq!(r.unwrap_err().to_string(), "the Soloist VM: vfkit stopped", "the Engine's status");
    assert_eq!(vm.unavailable(), "the Soloist VM isn't up");
    vm.down().await;
    prepare(&vm).await.0.unwrap();
    assert_eq!(fake.seen(|s| s.boots), 2);
}

// The supervisor (the Soloist session's loop) driving the VM's runner.

/// The VM's Soloist Engine on a fresh fake VM (its first boot failing with `broken`), started: its commands, its
/// events, the fake.
fn supervised(root: &Path, broken: Option<&str>) -> (tokio::sync::mpsc::Sender<Command>, tokio::sync::mpsc::Receiver<EngineEvent>, Fake) {
    static KEY: std::sync::Once = std::sync::Once::new();
    // SAFETY: set once, the same value for every test here, before an Engine reads it
    KEY.call_once(|| unsafe { std::env::set_var("SOLOIST_API_KEY", "test-key-not-real") });
    let (vm, fake) = runner(root);
    fake.answers_ws.store(true, Ordering::SeqCst);
    *fake.broken.lock().unwrap() = broken.map(String::from);
    let engine = SoloistEngine::with_runner("Singularity on the Mac (lossless)".into(), vm);
    assert!(engine.soloist_in_vm(), "the Daemon tells Clients (Q94)");
    let (tx, rx) = Box::new(engine).spawn();
    (tx, rx, fake)
}

/// The events up to and including the first that `stop` picks (15 s at most).
async fn until(rx: &mut tokio::sync::mpsc::Receiver<EngineEvent>, stop: impl Fn(&EngineEvent) -> bool) -> Vec<EngineEvent> {
    let mut seen = Vec::new();
    loop {
        let e = tokio::time::timeout(Duration::from_secs(15), rx.recv()).await.unwrap_or_else(|_| panic!("timed out; so far: {seen:?}"));
        let e = e.unwrap_or_else(|| panic!("the Engine ended; so far: {seen:?}"));
        let done = stop(&e);
        seen.push(e);
        if done {
            return seen;
        }
    }
}

fn ready(e: &EngineEvent) -> bool {
    matches!(e, EngineEvent::Status(EngineStatus::Ready { .. }))
}

/// The VM has been powered off `n` times (15 s at most).
async fn powered_off(fake: &Fake, n: u32) {
    for _ in 0..150 {
        if fake.seen(|s| s.power_offs) >= n {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(fake.seen(|s| s.power_offs), n);
}

/// How often Soloist was started in the guest.
fn launches(fake: &Fake) -> usize {
    fake.seen(|s| s.scripts.iter().filter(|c| c.starts_with("read -r k")).count())
}

/// How often the runner asked whether the VM is still there.
fn alive_checks(fake: &Fake) -> usize {
    fake.seen(|s| s.scripts.iter().filter(|c| *c == "true").count())
}

#[tokio::test]
async fn quitting_takes_the_vm_down() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx, fake) = supervised(dir.path(), None);
    until(&mut rx, ready).await;
    assert_eq!(fake.seen(|s| (s.boots, s.power_offs)), (1, 0));
    // the Daemon quits: the VM goes with it
    drop(tx);
    powered_off(&fake, 1).await;
}

#[tokio::test]
async fn a_vm_that_doesnt_come_up_is_taken_down_and_commands_are_told_why() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx, fake) = supervised(dir.path(), Some("vfkit stopped"));
    let seen = until(&mut rx, |e| matches!(e, EngineEvent::Status(EngineStatus::Failed { .. }))).await;
    assert!(matches!(seen.last(), Some(EngineEvent::Status(EngineStatus::Failed { message })) if message == "the Soloist VM: vfkit stopped"), "{seen:?}");
    tx.send(Command::Next).await.unwrap();
    until(&mut rx, |e| matches!(e, EngineEvent::Error(m) if m == "the Soloist VM isn't up")).await;
    assert_eq!(fake.seen(|s| s.power_offs), 1, "a failed start takes the VM down");
    // and again when the Daemon quits while it waits
    drop(tx);
    powered_off(&fake, 2).await;
}

#[tokio::test]
async fn a_vm_that_died_is_taken_down_and_booted_again() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx, fake) = supervised(dir.path(), None);
    until(&mut rx, ready).await;
    // the VM dies under Soloist
    fake.on.store(false, Ordering::SeqCst);
    std::fs::write(fake.ctl.join("dead"), "").unwrap();
    until(&mut rx, ready).await;
    assert_eq!(fake.seen(|s| (s.boots, s.power_offs)), (2, 1), "seen dead, taken down, booted again");
    assert!(alive_checks(&fake) >= 1);
    assert_eq!(launches(&fake), 2);
    drop(tx);
    powered_off(&fake, 2).await;
}

#[tokio::test]
async fn an_expired_build_in_the_vm_starts_again_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx, fake) = supervised(dir.path(), None);
    until(&mut rx, ready).await;
    let checks = alive_checks(&fake);
    std::fs::write(fake.ctl.join("expire"), "").unwrap();
    let seen = until(&mut rx, ready).await;
    assert!(!seen.iter().any(|e| matches!(e, EngineEvent::Status(EngineStatus::Failed { .. }))), "no status for it: {seen:?}");
    assert!(seen.iter().any(|e| matches!(e, EngineEvent::Status(EngineStatus::Starting))), "{seen:?}");
    assert_eq!(alive_checks(&fake), checks, "straight back up: no restart wait, no look at the VM");
    assert_eq!(fake.seen(|s| (s.boots, s.power_offs)), (1, 0));
    assert_eq!(launches(&fake), 2);
    drop(tx);
    powered_off(&fake, 1).await;
}
