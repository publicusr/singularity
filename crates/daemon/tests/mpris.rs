//! Seam 1 for MPRIS (#21): a Daemon with the FakeEngine on a private D-Bus, driven the way playerctl and KDE's media
//! widget do. The bus has no service activation, so nothing on the machine is started by it.

use singularity_proto::{self as proto, ClientMsg, Command, DaemonMsg};
use singularityd::engine::FakeEngine;
use std::collections::HashMap;
use std::time::Duration;
use tokio::net::UnixStream;
use zbus::zvariant::{OwnedValue, Value};

struct Bus {
    child: std::process::Child,
    address: String,
    _dir: tempfile::TempDir,
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A private session bus (None if `dbus-daemon` isn't installed: the test then says so and passes).
fn private_bus() -> Option<Bus> {
    use std::io::BufRead;
    let dir = tempfile::tempdir().unwrap();
    let conf = dir.path().join("bus.conf");
    std::fs::write(
        &conf,
        format!(
            r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig><type>session</type><listen>unix:dir={}</listen><auth>EXTERNAL</auth>
<policy context="default"><allow send_destination="*" eavesdrop="true"/><allow eavesdrop="true"/><allow own="*"/></policy></busconfig>"#,
            dir.path().display()
        ),
    )
    .unwrap();
    let mut child = std::process::Command::new("dbus-daemon").arg(format!("--config-file={}", conf.display())).args(["--nofork", "--print-address=1"]).stdout(std::process::Stdio::piped()).spawn().ok()?;
    let mut line = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).ok()?;
    Some(Bus { child, address: line.trim().to_string(), _dir: dir })
}

/// Stands in for the desktop's notification server and records what it's shown.
struct FakeNotifications(tokio::sync::mpsc::UnboundedSender<(String, String)>);

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl FakeNotifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(&self, _app: &str, _replaces: u32, _icon: &str, summary: &str, body: &str, _actions: Vec<String>, _hints: HashMap<String, OwnedValue>, _timeout: i32) -> u32 {
        let _ = self.0.send((summary.into(), body.into()));
        7
    }
}

#[zbus::proxy(interface = "org.mpris.MediaPlayer2.Player", default_service = "org.mpris.MediaPlayer2.singularity", default_path = "/org/mpris/MediaPlayer2")]
trait Player {
    fn play_pause(&self) -> zbus::Result<()>;
    fn next(&self) -> zbus::Result<()>;
    fn seek(&self, offset: i64) -> zbus::Result<()>;
    fn open_uri(&self, uri: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn playback_status(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn metadata(&self) -> zbus::Result<HashMap<String, OwnedValue>>;
    #[zbus(property)]
    fn position(&self) -> zbus::Result<i64>;
    #[zbus(property)]
    fn volume(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn set_volume(&self, v: f64) -> zbus::Result<()>;
    #[zbus(property)]
    fn loop_status(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_loop_status(&self, v: &str) -> zbus::Result<()>;
}

/// Wait until `f` holds (properties follow the Engine a moment later).
async fn until<F: AsyncFn() -> bool>(what: &str, f: F) {
    for _ in 0..100 {
        if f().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    panic!("timed out waiting for {what}");
}

fn title(m: &HashMap<String, OwnedValue>) -> String {
    m.get("xesam:title").and_then(|v| String::try_from(v.clone()).ok()).unwrap_or_default()
}

#[tokio::test]
async fn playerctl_style_control_and_track_notifications() {
    let Some(bus) = private_bus() else {
        eprintln!("dbus-daemon not installed: skipping");
        return;
    };
    // the desktop's notification server
    let (ntx, mut notes) = tokio::sync::mpsc::unbounded_channel();
    let _desktop = zbus::connection::Builder::address(bus.address.as_str()).unwrap().name("org.freedesktop.Notifications").unwrap().serve_at("/org/freedesktop/Notifications", FakeNotifications(ntx)).unwrap().build().await.unwrap();
    // the Daemon
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m.sock");
    let listener = singularityd::bind(&path).unwrap();
    let opts = singularityd::mpris::Options { address: Some(bus.address.clone()), notify: true };
    tokio::spawn(singularityd::serve_with(listener, Box::new(FakeEngine), None, None, Some(opts)));
    let mut s = UnixStream::connect(&path).await.unwrap();
    proto::write_msg(&mut s, &ClientMsg::Hello { protocol: proto::PROTOCOL_VERSION, client: "test".into() }).await.unwrap();
    // drain the socket so the Daemon never blocks on us
    tokio::spawn(async move {
        let (mut r, mut w) = s.into_split();
        proto::write_msg(&mut w, &ClientMsg::Command(Command::Play { uri: Some("spotify:track:abc".into()) })).await.unwrap();
        while let Ok(Some(_)) = proto::read_msg::<_, DaemonMsg>(&mut r).await {}
        drop(w);
    });
    // the desktop's side: what playerctl does
    let desk = zbus::connection::Builder::address(bus.address.as_str()).unwrap().build().await.unwrap();
    let p = PlayerProxy::builder(&desk).cache_properties(zbus::proxy::CacheProperties::No).build().await.unwrap();
    until("Playing", async || p.playback_status().await.is_ok_and(|s| s == "Playing")).await;
    assert_eq!(title(&p.metadata().await.unwrap()), "abc");
    let m = p.metadata().await.unwrap();
    assert_eq!(m.get("mpris:trackid").map(|v| v.to_string()), Some(Value::from(zbus::zvariant::ObjectPath::try_from("/org/singularity/track/abc").unwrap()).to_string()));
    assert_eq!(m.get("xesam:url").and_then(|v| String::try_from(v.clone()).ok()).as_deref(), Some("https://open.spotify.com/track/abc"));
    // the notification for the new track
    let (summary, _) = tokio::time::timeout(Duration::from_secs(3), notes.recv()).await.unwrap().unwrap();
    assert_eq!(summary, "abc");
    // play-pause, seek, volume, loop
    p.play_pause().await.unwrap();
    until("Paused", async || p.playback_status().await.is_ok_and(|s| s == "Paused")).await;
    p.seek(30_000_000).await.unwrap();
    until("the seek", async || p.position().await.is_ok_and(|us| (30_000_000..31_000_000).contains(&us))).await;
    p.set_volume(0.25).await.unwrap();
    until("volume", async || p.volume().await.is_ok_and(|v| (v - 0.25).abs() < 0.01)).await;
    p.set_loop_status("Track").await.unwrap();
    until("loop", async || p.loop_status().await.is_ok_and(|l| l == "Track")).await;
    // a link from the browser plays
    p.open_uri("https://open.spotify.com/track/xyz?si=1").await.unwrap();
    until("the opened link", async || p.metadata().await.is_ok_and(|m| title(&m) == "xyz")).await;
    assert!(p.open_uri("https://example.com/nope").await.is_err());
}
