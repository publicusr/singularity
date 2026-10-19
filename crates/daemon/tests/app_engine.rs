//! AppEngine (#65, D51), seam 1: a Daemon whose Engine is the Spotify app, with a stand-in Catalogue (bare channels)
//! in place of Spotify. Commands must reach the Catalogue as `CatCmd::Engine`; the app's playback, queue and stream
//! quality, read from a Connect cluster, must reach Clients as the Engine's own.

use librespot_protocol::connect::{Cluster, DeviceInfo};
use librespot_protocol::player::{BitrateLevel, PlaybackQuality, PlayerState, ProvidedTrack};
use singularity_proto::{self as proto, ClientMsg, Command, DaemonMsg, EngineStatus, Event, PROTOCOL_VERSION, PlayStatus};
use singularityd::app_engine::AppEngine;
use singularityd::catalogue::{CatCmd, CatEvent, app_engine_events};
use singularityd::engine::Identity;
use tokio::net::UnixStream;
use tokio::sync::mpsc;

const MAC: &str = "Owner’s MacBook Air";

fn identity() -> Identity {
    Identity { device_name: MAC.into(), via_connect: true, version: "1.2.94.583".into(), player: "the Spotify app" }
}

fn device(name: &str) -> DeviceInfo {
    let mut d = DeviceInfo::new();
    d.name = name.into();
    d.volume = 65535;
    d
}

/// The app's device and a phone; `active` plays a track with two queued and one context track next.
fn cluster(active: &str) -> Cluster {
    let mut c = Cluster::new();
    c.device.insert("mac".into(), device(MAC));
    c.device.insert("phone".into(), device("Pixel"));
    c.active_device_id = active.into();
    let mut ps = PlayerState::new();
    ps.is_playing = true;
    ps.duration = 200_000;
    ps.context_uri = "spotify:album:a".into();
    let mut t = ProvidedTrack::new();
    t.uri = "spotify:track:now".into();
    t.metadata.insert("title".into(), "Now".into());
    ps.track = Some(t).into();
    for (uri, provider, uid) in [("spotify:track:q1", "queue", "u1"), ("spotify:delimiter", "context", ""), ("spotify:track:c1", "context", "u2")] {
        let mut n = ProvidedTrack::new();
        (n.uri, n.provider, n.uid) = (uri.into(), provider.into(), uid.into());
        ps.next_tracks.push(n);
    }
    let mut q = PlaybackQuality::new();
    q.bitrate_level = BitrateLevel::hifi.into();
    ps.playback_quality = Some(q).into();
    c.player_state = Some(ps).into();
    c
}

async fn connect(path: &std::path::Path) -> UnixStream {
    let mut s = UnixStream::connect(path).await.unwrap();
    proto::write_msg(&mut s, &ClientMsg::Hello { protocol: PROTOCOL_VERSION, client: "test".into() }).await.unwrap();
    let _welcome: DaemonMsg = proto::read_msg(&mut s).await.unwrap().unwrap();
    s
}

async fn next_event(s: &mut UnixStream, want: impl Fn(&Event) -> bool) -> Event {
    loop {
        if let DaemonMsg::Event(e) = proto::read_msg::<_, DaemonMsg>(s).await.unwrap().unwrap()
            && want(&e)
        {
            return e;
        }
    }
}

#[tokio::test]
async fn the_spotify_app_plays_through_the_catalogue() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("d.sock");
    let listener = singularityd::bind(&path).unwrap();
    let (cat_tx, mut cat_rx) = mpsc::channel::<CatCmd>(16);
    let (ev_tx, ev_rx) = mpsc::channel::<CatEvent>(16);
    let engine = AppEngine { identity: identity(), catalogue: Some(cat_tx.clone()), start_app: false };
    tokio::spawn(singularityd::serve(listener, Box::new(engine), Some((cat_tx, ev_rx)), None));
    let mut s = connect(&path).await;

    // a restart is the AppEngine's own business (here, with no real app, nothing): it never reaches the Catalogue
    proto::write_msg(&mut s, &ClientMsg::Command(Command::RestartEngine)).await.unwrap();
    // transport commands and "add to queue" go to the app's Connect device, through the Catalogue
    for cmd in [Command::TogglePlay, Command::Seek { position_ms: 5000 }, Command::AddToQueue { uri: "spotify:track:z".into() }] {
        proto::write_msg(&mut s, &ClientMsg::Command(cmd.clone())).await.unwrap();
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(2), cat_rx.recv()).await.expect("reached the Catalogue").unwrap() {
                CatCmd::Engine(got) => {
                    assert_eq!(got, cmd);
                    break;
                }
                _ => continue, // the Catalogue's own business (Describe, Devices…)
            }
        }
    }

    // the app is the active device: its playback is the Engine's, with what plays next (no delimiter), and lossless
    for e in app_engine_events(&cluster("mac"), &identity(), proto::now_ms()) {
        ev_tx.send(CatEvent::Engine(e)).await.unwrap();
    }
    let ready = next_event(&mut s, |e| matches!(e, Event::Engine(EngineStatus::Ready { .. }))).await;
    assert_eq!(ready, Event::Engine(EngineStatus::Ready { device_name: MAC.into(), active: true, version: "1.2.94.583".into(), expires_in_days: None }));
    let Event::Playback(p) = next_event(&mut s, |e| matches!(e, Event::Playback(p) if p.item.is_some())).await else { unreachable!() };
    assert_eq!((p.status, p.item.unwrap().name.as_str(), p.context_uri.as_deref()), (PlayStatus::Playing, "Now", Some("spotify:album:a")));
    let next: Vec<_> = p.queue.iter().map(|q| (q.item.uri.as_str(), q.source.as_str(), q.uid.as_str())).collect();
    assert_eq!(next, [("spotify:track:q1", "queue", "u1"), ("spotify:track:c1", "context", "u2")]);
    let Event::Stream(info) = next_event(&mut s, |e| matches!(e, Event::Stream(_))).await else { unreachable!() };
    assert_eq!((info.codec.as_str(), info.sample_format.as_str()), ("flac", "16-bit"));
}

#[test]
fn the_app_engine_reads_its_status_from_the_cluster() {
    let id = identity();
    let status = |c: &Cluster| app_engine_events(c, &id, 0).into_iter().find_map(|e| match e {
        singularityd::engine::EngineEvent::Status(s) => Some(s),
        _ => None,
    });
    // the phone plays: the app is there, not active, and reports no playback of its own (the phone's is `Remote`)
    let phone = cluster("phone");
    assert!(matches!(status(&phone), Some(EngineStatus::Ready { active: false, .. })));
    assert_eq!(app_engine_events(&phone, &id, 0).len(), 1);
    // nothing plays anywhere: idle
    let idle = app_engine_events(&cluster(""), &id, 0);
    assert!(idle.iter().any(|e| matches!(e, singularityd::engine::EngineEvent::Playback(p) if p.status == PlayStatus::Idle && p.item.is_none())));
    // the app isn't on Connect (not running, or logged in elsewhere): said so
    let mut gone = cluster("phone");
    gone.device.remove("mac");
    assert!(matches!(status(&gone), Some(EngineStatus::Failed { message }) if message.contains(MAC) && message.contains("logged in")));
}
