//! Seam 1 (spec, Testing Decisions): drive a real Daemon purely through the protocol.

use singularity_proto::{self as proto, ClientMsg, DaemonMsg, Event, EngineStatus, PROTOCOL_VERSION};
use singularityd::engine::NoEngine;
use tokio::net::UnixStream;

async fn start() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s").join("daemon.sock");
    let listener = singularityd::bind(&path).unwrap();
    tokio::spawn(singularityd::serve(listener, Box::new(NoEngine { reason: "test".into() }), None, None));
    (dir, path)
}

#[tokio::test]
async fn handshake_ping_and_initial_snapshot() {
    let (_dir, path) = start().await;
    let mut s = UnixStream::connect(&path).await.unwrap();
    proto::write_msg(&mut s, &ClientMsg::Hello { protocol: PROTOCOL_VERSION, client: "test".into() }).await.unwrap();
    let welcome: DaemonMsg = proto::read_msg(&mut s).await.unwrap().unwrap();
    assert!(matches!(welcome, DaemonMsg::Welcome { protocol: PROTOCOL_VERSION, .. }));
    // engine status + playback snapshot arrive without asking
    let mut saw_status = false;
    let mut saw_playback = false;
    for _ in 0..2 {
        match proto::read_msg::<_, DaemonMsg>(&mut s).await.unwrap().unwrap() {
            DaemonMsg::Event(Event::Engine(EngineStatus::NotConfigured { .. })) => saw_status = true,
            DaemonMsg::Event(Event::Playback(_)) => saw_playback = true,
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(saw_status && saw_playback);
    proto::write_msg(&mut s, &ClientMsg::Ping { id: 7 }).await.unwrap();
    let pong: DaemonMsg = proto::read_msg(&mut s).await.unwrap().unwrap();
    assert_eq!(pong, DaemonMsg::Pong { id: 7 });
}

#[tokio::test]
async fn incompatible_protocol_gets_a_clear_message() {
    let (_dir, path) = start().await;
    let mut s = UnixStream::connect(&path).await.unwrap();
    proto::write_msg(&mut s, &ClientMsg::Hello { protocol: PROTOCOL_VERSION + 1, client: "future".into() }).await.unwrap();
    match proto::read_msg::<_, DaemonMsg>(&mut s).await.unwrap().unwrap() {
        DaemonMsg::Incompatible { message, .. } => assert!(message.contains("protocol")),
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn second_daemon_refuses_to_bind() {
    let (_dir, path) = start().await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let err = singularityd::bind(&path).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);
}

async fn connect(path: &std::path::Path) -> UnixStream {
    let mut s = UnixStream::connect(path).await.unwrap();
    proto::write_msg(&mut s, &ClientMsg::Hello { protocol: PROTOCOL_VERSION, client: "test".into() }).await.unwrap();
    let _welcome: DaemonMsg = proto::read_msg(&mut s).await.unwrap().unwrap();
    s
}

async fn next_playback(s: &mut UnixStream) -> singularity_proto::PlaybackState {
    loop {
        if let DaemonMsg::Event(Event::Playback(p)) = proto::read_msg::<_, DaemonMsg>(s).await.unwrap().unwrap() {
            return p;
        }
    }
}

#[tokio::test]
async fn playback_flow_through_the_protocol() {
    use singularity_proto::{Command, PlayStatus};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("d.sock");
    let listener = singularityd::bind(&path).unwrap();
    tokio::spawn(singularityd::serve(listener, Box::new(singularityd::engine::FakeEngine), None, None));
    let mut a = connect(&path).await;
    let mut b = connect(&path).await; // a second Client sees the same events
    let _ = next_playback(&mut a).await; // initial snapshots
    let _ = next_playback(&mut b).await;

    let send = |c: Command| ClientMsg::Command(c);
    proto::write_msg(&mut a, &send(Command::Play { uri: Some("spotify:track:fake2".into()) })).await.unwrap();
    let p = next_playback(&mut a).await;
    assert_eq!((p.status, p.item.as_ref().unwrap().name.as_str()), (PlayStatus::Playing, "Track 2"));
    assert_eq!(next_playback(&mut b).await.item.unwrap().name, "Track 2");

    proto::write_msg(&mut a, &send(Command::Next)).await.unwrap();
    assert_eq!(next_playback(&mut a).await.item.unwrap().name, "Track 3");
    proto::write_msg(&mut a, &send(Command::Seek { position_ms: 90_000 })).await.unwrap();
    assert_eq!(next_playback(&mut a).await.position_ms, 90_000);
    proto::write_msg(&mut a, &send(Command::SetVolume { percent: 33 })).await.unwrap();
    assert_eq!(next_playback(&mut a).await.volume, 33);
    proto::write_msg(&mut a, &send(Command::TogglePlay)).await.unwrap();
    assert_eq!(next_playback(&mut a).await.status, PlayStatus::Paused);
}

#[tokio::test]
async fn library_track_pages_and_play_from_track_n() {
    use singularity_proto::{Command, HomeFeed};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("l.sock");
    let listener = singularityd::bind(&path).unwrap();
    let catalogue = singularityd::catalogue::spawn_fake(HomeFeed::default());
    tokio::spawn(singularityd::serve(listener, Box::new(singularityd::engine::FakeEngine), Some(catalogue), None));
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let mut a = connect(&path).await;
    // the library is part of the snapshot once the Catalogue has it
    let lib = loop {
        if let DaemonMsg::Event(Event::Library(l)) = proto::read_msg::<_, DaemonMsg>(&mut a).await.unwrap().unwrap() {
            break l;
        }
    };
    assert_eq!((lib.items[0].kind.as_str(), lib.items[0].uri.as_str()), ("liked", "spotify:collection"));

    // a long list arrives page by page, in order
    proto::write_msg(&mut a, &ClientMsg::Command(Command::GetTracks { uri: "spotify:collection".into() })).await.unwrap();
    let mut got = 0;
    while got < 250 {
        if let DaemonMsg::Event(Event::Tracks(p)) = proto::read_msg::<_, DaemonMsg>(&mut a).await.unwrap().unwrap() {
            assert_eq!((p.offset, p.total), (got, 250));
            got += p.tracks.len() as u32;
        }
    }

    // without Connect (fake Catalogue), play-from falls back to the Engine
    let cmd = Command::PlayFrom { context_uri: "spotify:collection".into(), index: 41, track_uri: Some("spotify:track:fake41".into()) };
    proto::write_msg(&mut a, &ClientMsg::Command(cmd)).await.unwrap();
    let p = loop {
        let p = next_playback(&mut a).await;
        if p.item.is_some() {
            break p;
        }
    };
    assert_eq!(p.item.unwrap().uri, "spotify:track:fake41");
    assert_eq!(p.context_uri.as_deref(), Some("spotify:collection"));
}

#[tokio::test]
async fn two_clients_share_one_tap_and_frames_stop_when_paused() {
    use singularity_proto::Command;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.sock");
    let listener = singularityd::bind(&path).unwrap();
    tokio::spawn(singularityd::serve(listener, Box::new(singularityd::engine::FakeEngine), None, Some(singularityd::tap::TapSource::Synthetic)));
    let mut a = connect(&path).await;
    let mut b = connect(&path).await;
    let cmd = |c| ClientMsg::Command(c);
    proto::write_msg(&mut a, &cmd(Command::Play { uri: None })).await.unwrap();
    for s in [&mut a, &mut b] {
        proto::write_msg(s, &cmd(Command::Spectrum { on: true })).await.unwrap();
    }
    async fn frames(s: &mut UnixStream, want: usize) -> Vec<singularity_proto::SpectrumFrame> {
        let mut out = vec![];
        while out.len() < want {
            let m = tokio::time::timeout(std::time::Duration::from_secs(3), proto::read_msg::<_, DaemonMsg>(s)).await.expect("frames keep coming").unwrap().unwrap();
            if let DaemonMsg::Event(Event::Spectrum(f)) = m {
                out.push(f);
            }
        }
        out
    }
    // ~30 Hz to both, with music in them
    let t = std::time::Instant::now();
    let fa = frames(&mut a, 30).await;
    assert!(t.elapsed() < std::time::Duration::from_millis(2000), "{:?} for 30 frames", t.elapsed());
    let fb = frames(&mut b, 30).await;
    assert!(fa.iter().chain(&fb).any(|f| f.bands.iter().any(|v| *v > 100)));
    assert!(fa.iter().any(|f| f.beat), "the kick is found");
    // pause: bars fall, one zero frame, then silence on the wire
    proto::write_msg(&mut a, &cmd(Command::Pause)).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
    let mut last = None;
    while let Ok(Ok(Some(m))) = tokio::time::timeout(std::time::Duration::from_millis(400), proto::read_msg::<_, DaemonMsg>(&mut b)).await {
        if let DaemonMsg::Event(Event::Spectrum(f)) = m {
            last = Some(f);
        }
    }
    assert!(last.is_some_and(|f| f.bands.iter().all(|v| *v == 0)), "the last frame clears the bars");
}

#[tokio::test]
async fn shutdown_makes_way_for_a_newer_daemon() {
    use singularity_proto::Command;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.sock");
    let listener = singularityd::bind(&path).unwrap();
    let served = tokio::spawn(singularityd::serve(listener, Box::new(NoEngine { reason: "test".into() }), None, None));
    let mut a = connect(&path).await;
    proto::write_msg(&mut a, &ClientMsg::Command(Command::Shutdown)).await.unwrap();
    let r = tokio::time::timeout(std::time::Duration::from_secs(2), served).await.expect("serve returns").unwrap().unwrap();
    assert_eq!(r, singularityd::Served::Replaced);
}
