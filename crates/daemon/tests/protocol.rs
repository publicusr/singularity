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

#[tokio::test]
async fn a_client_that_stops_reading_is_let_go() {
    // (#71) it asks and asks (every Ping is answered) but never reads: once its answers back up for the write
    // deadline, the Daemon closes the session instead of holding it blocked for as long as it stays connected
    let (_dir, path) = start().await;
    let s = connect(&path).await;
    let (mut rd, mut wr) = s.into_split();
    let asking = tokio::spawn(async move {
        for id in 0.. {
            if proto::write_msg(&mut wr, &ClientMsg::Ping { id }).await.is_err() {
                return;
            }
        }
    });
    tokio::time::sleep(std::time::Duration::from_secs(7)).await;
    // what's buffered drains, then the connection ends (it would keep answering forever if it were still served)
    let drained = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Ok(Some(_)) = proto::read_msg::<_, DaemonMsg>(&mut rd).await {}
    })
    .await;
    asking.abort();
    assert!(drained.is_ok(), "the session was never closed");
}

#[tokio::test]
async fn the_socket_never_goes_through_a_symlinked_directory() {
    // (#71) someone else's directory, linked where ours should be, could hand Clients a socket of theirs
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, dir.path().join("sgl")).unwrap();
    let err = singularityd::bind(&dir.path().join("sgl/d.sock")).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    // our own directory: fine, and owner-only afterwards
    let ok = dir.path().join("own/d.sock");
    singularityd::bind(&ok).unwrap();
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(dir.path().join("own")).unwrap().permissions().mode() & 0o777, 0o700);
}

/// S5/A10: the check used to look at the last directory only; `create_dir_all` (and the socket) followed an
/// intermediate symlink, so the socket could be placed wherever that link pointed.
#[tokio::test]
async fn the_socket_never_goes_through_an_intermediate_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, dir.path().join("sgl")).unwrap();
    let err = singularityd::bind(&dir.path().join("sgl/inside/d.sock"));
    assert!(err.is_err(), "the socket was put inside the directory an intermediate symlink points at");
    assert!(!elsewhere.join("inside").exists(), "a directory was created behind the symlink");
}

/// A10: only the directory the Daemon created for the socket is made owner-only; an existing one keeps its mode
/// (`SINGULARITY_SOCKET=$HOME/daemon.sock` used to chmod `$HOME` to 0700).
#[tokio::test]
async fn a_socket_directory_that_already_exists_keeps_its_mode() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let own = dir.path().join("shared");
    std::fs::create_dir(&own).unwrap();
    std::fs::set_permissions(&own, std::fs::Permissions::from_mode(0o755)).unwrap();
    singularityd::bind(&own.join("d.sock")).unwrap();
    assert_eq!(std::fs::metadata(&own).unwrap().permissions().mode() & 0o777, 0o755, "the Daemon chmod'ed a directory it didn't create");
}

/// A10: a new socket directory is 0700 from the moment it exists (no umask window for another user to slip a file in).
#[tokio::test]
async fn a_new_socket_directory_is_owner_only_from_the_start() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("a/b");
    singularityd::bind(&nested.join("d.sock")).unwrap();
    for p in [dir.path().join("a"), nested.clone()] {
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o700, "{} isn't owner-only", p.display());
    }
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

#[tokio::test]
async fn queue_edits_and_bare_entries_get_names() {
    use singularity_proto::{Command, HomeFeed};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("q.sock");
    let listener = singularityd::bind(&path).unwrap();
    let catalogue = singularityd::catalogue::spawn_fake(HomeFeed::default());
    tokio::spawn(singularityd::serve(listener, Box::new(singularityd::engine::FakeEngine), Some(catalogue), None));
    let mut a = connect(&path).await;
    let cmd = |c| ClientMsg::Command(c);
    async fn queue_where(s: &mut UnixStream, ok: impl Fn(&[singularity_proto::QueueEntry]) -> bool) -> Vec<singularity_proto::QueueEntry> {
        loop {
            let p = next_playback(s).await;
            if ok(&p.queue) {
                return p.queue;
            }
        }
    }
    proto::write_msg(&mut a, &cmd(Command::AddToQueue { uri: "spotify:track:x1".into() })).await.unwrap();
    proto::write_msg(&mut a, &cmd(Command::PlayNext { uri: "spotify:track:x2".into() })).await.unwrap();
    // the second goes first; both arrive bare and get their names from the Catalogue
    let q = queue_where(&mut a, |q| q.len() == 2 && q.iter().all(|e| !e.item.name.is_empty())).await;
    assert_eq!(q.iter().map(|e| e.item.uri.as_str()).collect::<Vec<_>>(), ["spotify:track:x2", "spotify:track:x1"]);
    assert_eq!(q[0].item.name, "Described x2");
    // move the second to the front, and back
    proto::write_msg(&mut a, &cmd(Command::MoveInQueue { uri: "spotify:track:x1".into(), uid: Some(q[1].uid.clone()), to: 0, into_queue: false })).await.unwrap();
    queue_where(&mut a, |q| q.first().is_some_and(|e| e.item.uri == "spotify:track:x1")).await;
    proto::write_msg(&mut a, &cmd(Command::MoveInQueue { uri: "spotify:track:x1".into(), uid: Some(q[1].uid.clone()), to: 5, into_queue: false })).await.unwrap();
    let q = queue_where(&mut a, |q| q.first().is_some_and(|e| e.item.uri == "spotify:track:x2")).await;
    proto::write_msg(&mut a, &cmd(Command::RemoveFromQueue { uri: "spotify:track:x2".into(), uid: Some(q[0].uid.clone()) })).await.unwrap();
    let q = queue_where(&mut a, |q| q.len() == 1).await;
    assert_eq!(q[0].item.uri, "spotify:track:x1");
    proto::write_msg(&mut a, &cmd(Command::ClearQueue)).await.unwrap();
    queue_where(&mut a, |q| q.is_empty()).await;
}

#[tokio::test]
async fn lyrics_round_trip() {
    use singularity_proto::{Command, HomeFeed};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ly.sock");
    let listener = singularityd::bind(&path).unwrap();
    tokio::spawn(singularityd::serve(listener, Box::new(NoEngine { reason: "test".into() }), Some(singularityd::catalogue::spawn_fake(HomeFeed::default())), None));
    let mut a = connect(&path).await;
    proto::write_msg(&mut a, &ClientMsg::Command(Command::GetLyrics { uri: "spotify:track:l".into() })).await.unwrap();
    loop {
        if let DaemonMsg::Event(Event::Lyrics(l)) = proto::read_msg::<_, DaemonMsg>(&mut a).await.unwrap().unwrap() {
            assert_eq!(l.uri, "spotify:track:l");
            assert!(l.synced && !l.lines.is_empty());
            break;
        }
    }
}

#[tokio::test]
async fn devices_and_transfer() {
    use singularity_proto::{Command, HomeFeed};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dv.sock");
    let listener = singularityd::bind(&path).unwrap();
    tokio::spawn(singularityd::serve(listener, Box::new(NoEngine { reason: "test".into() }), Some(singularityd::catalogue::spawn_fake(HomeFeed::default())), None));
    let mut a = connect(&path).await;
    async fn devices(s: &mut UnixStream) -> Vec<singularity_proto::Device> {
        loop {
            if let DaemonMsg::Event(Event::Devices { devices }) = proto::read_msg::<_, DaemonMsg>(s).await.unwrap().unwrap() {
                return devices;
            }
        }
    }
    proto::write_msg(&mut a, &ClientMsg::Command(Command::GetDevices)).await.unwrap();
    let d = devices(&mut a).await;
    assert!(d.iter().any(|d| d.is_engine && d.active));
    proto::write_msg(&mut a, &ClientMsg::Command(Command::Transfer { device_id: "phone".into() })).await.unwrap();
    let d = devices(&mut a).await;
    assert!(d.iter().any(|d| d.id == "phone" && d.active));
}

#[tokio::test]
async fn saving_settles_on_spotifys_answer_and_failures_carry_the_old_value() {
    use singularity_proto::{Command, HomeFeed, SavedFlag};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sv.sock");
    let listener = singularityd::bind(&path).unwrap();
    tokio::spawn(singularityd::serve(listener, Box::new(NoEngine { reason: "test".into() }), Some(singularityd::catalogue::spawn_fake(HomeFeed::default())), None));
    let mut a = connect(&path).await;
    // the error (if any) and the flags, up to the next Saved
    async fn answer(s: &mut UnixStream) -> (Option<String>, Vec<SavedFlag>) {
        let mut err = None;
        loop {
            match proto::read_msg::<_, DaemonMsg>(s).await.unwrap().unwrap() {
                DaemonMsg::Event(Event::Error { message }) => err = Some(message),
                DaemonMsg::Event(Event::Saved { entries }) => return (err, entries),
                _ => {}
            }
        }
    }
    let flag = |uri: &str, saved| SavedFlag { uri: uri.into(), saved };
    proto::write_msg(&mut a, &ClientMsg::Command(Command::CheckSaved { uris: vec!["spotify:track:t1".into(), "spotify:artist:a0".into()] })).await.unwrap();
    assert_eq!(answer(&mut a).await, (None, vec![flag("spotify:track:t1", true), flag("spotify:artist:a0", false)]));
    proto::write_msg(&mut a, &ClientMsg::Command(Command::SetSaved { uri: "spotify:artist:a0".into(), saved: true })).await.unwrap();
    assert_eq!(answer(&mut a).await, (None, vec![flag("spotify:artist:a0", true)]));
    proto::write_msg(&mut a, &ClientMsg::Command(Command::SetSaved { uri: "spotify:track:fail".into(), saved: true })).await.unwrap();
    let (err, flags) = answer(&mut a).await;
    assert!(err.is_some(), "the failure is told");
    assert_eq!(flags, vec![flag("spotify:track:fail", false)], "the old value comes back for the rollback");
}

#[tokio::test]
async fn playlists_are_created_and_deleted_and_only_your_own() {
    use singularity_proto::{Command, HomeFeed, Library};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pl.sock");
    let listener = singularityd::bind(&path).unwrap();
    tokio::spawn(singularityd::serve(listener, Box::new(NoEngine { reason: "test".into() }), Some(singularityd::catalogue::spawn_fake(HomeFeed::default())), None));
    let mut a = connect(&path).await;
    // events up to the next Library that passes `ok`: (errors, created, that library)
    async fn until_library(s: &mut UnixStream, ok: impl Fn(&Library) -> bool) -> (Vec<String>, Option<(String, String)>, Library) {
        let (mut errs, mut made) = (vec![], None);
        loop {
            match proto::read_msg::<_, DaemonMsg>(s).await.unwrap().unwrap() {
                DaemonMsg::Event(Event::Error { message }) => errs.push(message),
                DaemonMsg::Event(Event::PlaylistCreated { uri, name }) => made = Some((uri, name)),
                DaemonMsg::Event(Event::Library(l)) if ok(&l) => return (errs, made, l),
                _ => {}
            }
        }
    }
    let send = |c| ClientMsg::Command(c);
    let (_, _, lib) = until_library(&mut a, |_| true).await;
    assert!(lib.items.iter().any(|i| i.own), "the fake user owns one");
    proto::write_msg(&mut a, &send(Command::CreatePlaylist { name: "Road trip".into(), uris: vec![] })).await.unwrap();
    let (errs, made, lib) = until_library(&mut a, |l| l.items.iter().any(|i| i.name == "Road trip")).await;
    let (uri, name) = made.expect("created");
    assert!(errs.is_empty() && name == "Road trip");
    assert_eq!((lib.items[1].uri.as_str(), lib.items[1].own), (uri.as_str(), true), "at the top, under Liked Songs");
    proto::write_msg(&mut a, &send(Command::DeletePlaylist { uri: uri.clone() })).await.unwrap();
    let (errs, _, lib) = until_library(&mut a, |l| !l.items.iter().any(|i| i.uri == uri)).await;
    assert!(errs.is_empty());
    // someone else's: refused, the library unchanged
    let theirs = lib.items.iter().find(|i| i.kind == "playlist" && !i.own).map(|i| i.uri.clone());
    if let Some(theirs) = theirs {
        proto::write_msg(&mut a, &send(Command::DeletePlaylist { uri: theirs.clone() })).await.unwrap();
        let (errs, _, lib) = until_library(&mut a, |_| true).await;
        assert!(!errs.is_empty() && lib.items.iter().any(|i| i.uri == theirs));
    }
    proto::write_msg(&mut a, &send(Command::CreatePlaylist { name: "fail".into(), uris: vec![] })).await.unwrap();
    let (errs, made, _) = until_library(&mut a, |_| true).await;
    assert!(!errs.is_empty() && made.is_none(), "a failure is told");
}

#[tokio::test]
async fn a_playlist_is_renamed_and_redescribed_and_a_refusal_is_told() {
    use singularity_proto::{Command, HomeFeed};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ed.sock");
    let listener = singularityd::bind(&path).unwrap();
    tokio::spawn(singularityd::serve(listener, Box::new(NoEngine { reason: "test".into() }), Some(singularityd::catalogue::spawn_fake(HomeFeed::default())), None));
    let mut a = connect(&path).await;
    // the answer to an edit, then the Library after it
    async fn edited(s: &mut UnixStream) -> ((String, String, bool), singularity_proto::Library) {
        let mut got = None;
        loop {
            match proto::read_msg::<_, DaemonMsg>(s).await.unwrap().unwrap() {
                DaemonMsg::Event(Event::PlaylistEdited { name, description, ok, .. }) => got = Some((name, description, ok)),
                DaemonMsg::Event(Event::Library(l)) if got.is_some() => return (got.unwrap(), l),
                _ => {}
            }
        }
    }
    let mine = "spotify:playlist:fakemine".to_string();
    let edit = |name: &str, description: &str| ClientMsg::Command(Command::EditPlaylist { uri: mine.clone(), name: name.into(), description: description.into() });
    proto::write_msg(&mut a, &edit("Mixtape II", "")).await.unwrap();
    let ((name, description, ok), lib) = edited(&mut a).await;
    assert_eq!((name.as_str(), description.as_str(), ok), ("Mixtape II", "", true));
    assert!(lib.items.iter().any(|i| i.uri == mine && i.name == "Mixtape II"));
    proto::write_msg(&mut a, &edit("fail", "x")).await.unwrap();
    let ((_, _, ok), lib) = edited(&mut a).await;
    assert!(!ok && lib.items.iter().any(|i| i.uri == mine && i.name == "Mixtape II"), "refused: the old name stays");
}

#[tokio::test]
async fn the_listening_log_sums_up_exports_and_deletes_through_the_protocol() {
    use singularity_proto::{Command, Item};
    use singularityd::store::{Play, Store};
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let now = proto::now_ms();
    {
        let s = Store::open(&data).unwrap();
        for (i, back) in [0u64, 0, 1].into_iter().enumerate() {
            let item = Item { uri: format!("spotify:track:{i}"), name: "T".into(), artists: vec!["Nova Drift".into()], duration_ms: 600_000, ..Default::default() };
            s.record(&Play { started_ms: now - back * 86_400_000, item, context_uri: None, played_ms: 120_000, device: "here".into() }).unwrap();
        }
    }
    // the export lands in the Downloads folder (only this test reads XDG_DOWNLOAD_DIR)
    let downloads = dir.path().join("dl");
    unsafe { std::env::set_var("XDG_DOWNLOAD_DIR", &downloads) };
    let path = dir.path().join("ld.sock");
    let listener = singularityd::bind(&path).unwrap();
    let extras = singularityd::Extras { store: Some(data.clone()), ..Default::default() };
    tokio::spawn(singularityd::serve_with(listener, Box::new(NoEngine { reason: "test".into() }), None, None, extras));
    let mut a = connect(&path).await;
    async fn next(s: &mut UnixStream, want: fn(&Event) -> bool) -> Event {
        loop {
            if let DaemonMsg::Event(e) = proto::read_msg::<_, DaemonMsg>(s).await.unwrap().unwrap()
                && want(&e)
            {
                return e;
            }
        }
    }
    proto::write_msg(&mut a, &ClientMsg::Command(Command::GetListening)).await.unwrap();
    let Event::Listening(s) = next(&mut a, |e| matches!(e, Event::Listening(_))).await else { unreachable!() };
    assert_eq!((s.plays, s.streak_days, s.top_artist.as_deref()), (3, 2, Some("Nova Drift")));
    assert_eq!(s.minutes[s.minutes.len() - 2..], [2, 4]);

    // stats for a period (#35)
    proto::write_msg(&mut a, &ClientMsg::Command(Command::GetStats { period: singularity_proto::StatsPeriod::All })).await.unwrap();
    let Event::Stats(st) = next(&mut a, |e| matches!(e, Event::Stats(_))).await else { unreachable!() };
    assert_eq!((st.period, st.plays, st.minutes, st.top_artists[0].name.as_str(), st.top_tracks.len()), (singularity_proto::StatsPeriod::All, 3, 6, "Nova Drift", 3));

    proto::write_msg(&mut a, &ClientMsg::Command(Command::ExportLocalData)).await.unwrap();
    let Event::Notice { message } = next(&mut a, |e| matches!(e, Event::Notice { .. } | Event::Error { .. })).await else { panic!("export failed") };
    let file = std::fs::read_dir(&downloads).unwrap().next().unwrap().unwrap().path();
    assert!(message.contains(&file.display().to_string()), "{message}");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    let doc: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(doc["plays"].as_array().unwrap().len(), 3);

    // deleting tells every Client: the Pane empties
    proto::write_msg(&mut a, &ClientMsg::Command(Command::DeleteLocalData)).await.unwrap();
    let Event::Listening(s) = next(&mut a, |e| matches!(e, Event::Listening(_))).await else { unreachable!() };
    assert_eq!((s.plays, s.minutes.iter().sum::<u32>()), (0, 0));
    assert_eq!(Store::open(&data).unwrap().summary(now).unwrap().plays, 0, "gone from disk");
}

#[tokio::test]
async fn tracks_are_added_to_a_playlist_and_a_duplicate_asks_first() {
    use singularity_proto::{AddOutcome, Command, HomeFeed};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("add.sock");
    let listener = singularityd::bind(&path).unwrap();
    tokio::spawn(singularityd::serve(listener, Box::new(NoEngine { reason: "test".into() }), Some(singularityd::catalogue::spawn_fake(HomeFeed::default())), None));
    let mut a = connect(&path).await;
    async fn outcome(s: &mut UnixStream) -> (String, AddOutcome) {
        loop {
            if let DaemonMsg::Event(Event::AddedToPlaylist { playlist, outcome, .. }) = proto::read_msg::<_, DaemonMsg>(s).await.unwrap().unwrap() {
                return (playlist, outcome);
            }
        }
    }
    let mine = "spotify:playlist:fakemine".to_string();
    let add = |uri: &str, anyway| ClientMsg::Command(Command::AddToPlaylist { playlist: mine.clone(), name: "Fake Mixtape".into(), uris: vec![uri.into()], anyway });
    proto::write_msg(&mut a, &add("spotify:track:new1", false)).await.unwrap();
    assert_eq!(outcome(&mut a).await.1, AddOutcome::Added);
    proto::write_msg(&mut a, &add("spotify:track:new1", false)).await.unwrap();
    assert_eq!(outcome(&mut a).await.1, AddOutcome::AlreadyThere, "added once already");
    proto::write_msg(&mut a, &add("spotify:track:new1", true)).await.unwrap();
    assert_eq!(outcome(&mut a).await.1, AddOutcome::Added, "anyway");
    // the playlist now ends with them
    proto::write_msg(&mut a, &ClientMsg::Command(Command::GetTracks { uri: mine.clone() })).await.unwrap();
    let page = loop {
        if let DaemonMsg::Event(Event::Tracks(p)) = proto::read_msg::<_, DaemonMsg>(&mut a).await.unwrap().unwrap() {
            break p;
        }
    };
    assert_eq!((page.total, page.tracks.last().map(|t| t.uri.as_str())), (42, Some("spotify:track:new1")));
    // someone else's: refused
    proto::write_msg(&mut a, &ClientMsg::Command(Command::AddToPlaylist { playlist: "spotify:playlist:theirs".into(), name: "x".into(), uris: vec!["spotify:track:a".into()], anyway: false })).await.unwrap();
    assert_eq!(outcome(&mut a).await.1, AddOutcome::Failed);
    // a new playlist made with a track in it
    proto::write_msg(&mut a, &ClientMsg::Command(Command::CreatePlaylist { name: "With one".into(), uris: vec!["spotify:track:b".into()] })).await.unwrap();
    let (uri, o) = outcome(&mut a).await;
    assert!(uri.contains(":fakenew") && o == AddOutcome::Added);
}

#[tokio::test]
async fn one_row_is_removed_by_its_uid_and_a_refusal_is_told() {
    use singularity_proto::{Command, HomeFeed};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rm.sock");
    let listener = singularityd::bind(&path).unwrap();
    tokio::spawn(singularityd::serve(listener, Box::new(NoEngine { reason: "test".into() }), Some(singularityd::catalogue::spawn_fake(HomeFeed::default())), None));
    let mut a = connect(&path).await;
    let mine = "spotify:playlist:fakemine".to_string();
    async fn answer(s: &mut UnixStream) -> bool {
        loop {
            if let DaemonMsg::Event(Event::RemovedFromPlaylist { ok, .. }) = proto::read_msg::<_, DaemonMsg>(s).await.unwrap().unwrap() {
                return ok;
            }
        }
    }
    async fn page(s: &mut UnixStream, uri: &str) -> singularity_proto::TrackPage {
        proto::write_msg(s, &ClientMsg::Command(Command::GetTracks { uri: uri.into() })).await.unwrap();
        loop {
            if let DaemonMsg::Event(Event::Tracks(p)) = proto::read_msg::<_, DaemonMsg>(s).await.unwrap().unwrap() {
                return p;
            }
        }
    }
    let before = page(&mut a, &mine).await;
    assert!(before.tracks.iter().all(|t| t.uid.is_some()), "playlist rows carry uids");
    let uid = before.tracks[3].uid.clone().unwrap();
    proto::write_msg(&mut a, &ClientMsg::Command(Command::RemoveFromPlaylist { playlist: mine.clone(), uids: vec![uid.clone()] })).await.unwrap();
    assert!(answer(&mut a).await);
    let after = page(&mut a, &mine).await;
    assert_eq!(after.total, before.total - 1);
    assert!(!after.tracks.iter().any(|t| t.uid.as_deref() == Some(uid.as_str())));
    proto::write_msg(&mut a, &ClientMsg::Command(Command::RemoveFromPlaylist { playlist: mine.clone(), uids: vec!["fail".into()] })).await.unwrap();
    assert!(!answer(&mut a).await);
}

#[tokio::test]
async fn a_row_moves_before_another_and_a_refusal_is_told() {
    use singularity_proto::{Command, HomeFeed};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mv.sock");
    let listener = singularityd::bind(&path).unwrap();
    tokio::spawn(singularityd::serve(listener, Box::new(NoEngine { reason: "test".into() }), Some(singularityd::catalogue::spawn_fake(HomeFeed::default())), None));
    let mut a = connect(&path).await;
    let mine = "spotify:playlist:fakemine".to_string();
    async fn answer(s: &mut UnixStream) -> bool {
        loop {
            if let DaemonMsg::Event(Event::MovedInPlaylist { ok, .. }) = proto::read_msg::<_, DaemonMsg>(s).await.unwrap().unwrap() {
                return ok;
            }
        }
    }
    async fn uids(s: &mut UnixStream, uri: &str) -> Vec<String> {
        proto::write_msg(s, &ClientMsg::Command(Command::GetTracks { uri: uri.into() })).await.unwrap();
        loop {
            if let DaemonMsg::Event(Event::Tracks(p)) = proto::read_msg::<_, DaemonMsg>(s).await.unwrap().unwrap() {
                return p.tracks.into_iter().filter_map(|t| t.uid).collect();
            }
        }
    }
    let before = uids(&mut a, &mine).await;
    let mv = |uid: &str, before: Option<&str>, after: Option<&str>| ClientMsg::Command(Command::MoveInPlaylist { playlist: mine.clone(), uids: vec![uid.into()], before: before.map(String::from), after: after.map(String::from) });
    // the first row to just before the fourth
    proto::write_msg(&mut a, &mv(&before[0], Some(&before[3]), None)).await.unwrap();
    assert!(answer(&mut a).await);
    let after = uids(&mut a, &mine).await;
    assert_eq!(after[..4], [before[1].clone(), before[2].clone(), before[0].clone(), before[3].clone()]);
    // after the last
    let last = after.last().unwrap().clone();
    proto::write_msg(&mut a, &mv(&after[0], None, Some(&last))).await.unwrap();
    assert!(answer(&mut a).await);
    assert_eq!(uids(&mut a, &mine).await.last(), Some(&after[0]));
    proto::write_msg(&mut a, &mv("fail", None, None)).await.unwrap();
    assert!(!answer(&mut a).await);
}

#[tokio::test]
async fn a_cover_is_set_put_back_and_refused_when_it_isnt_a_jpeg() {
    use singularity_proto::{Command, HomeFeed};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cv.sock");
    let listener = singularityd::bind(&path).unwrap();
    tokio::spawn(singularityd::serve(listener, Box::new(NoEngine { reason: "test".into() }), Some(singularityd::catalogue::spawn_fake(HomeFeed::default())), None));
    let mut a = connect(&path).await;
    // the answer, and whether an error came first
    async fn answer(s: &mut UnixStream) -> (bool, bool) {
        let mut err = false;
        loop {
            match proto::read_msg::<_, DaemonMsg>(s).await.unwrap().unwrap() {
                DaemonMsg::Event(Event::Error { .. }) => err = true,
                DaemonMsg::Event(Event::PlaylistCoverSet { ok, .. }) => return (ok, err),
                _ => {}
            }
        }
    }
    let set = |jpeg: Option<&str>| ClientMsg::Command(Command::SetPlaylistCover { playlist: "spotify:playlist:fakemine".into(), jpeg: jpeg.map(String::from) });
    // "/9j/" is FF D8 FF: a JPEG's start
    proto::write_msg(&mut a, &set(Some("/9j/4AAQ"))).await.unwrap();
    assert_eq!(answer(&mut a).await, (true, false));
    proto::write_msg(&mut a, &set(None)).await.unwrap();
    assert_eq!(answer(&mut a).await, (true, false), "the mosaic back");
    proto::write_msg(&mut a, &set(Some("iVBORw0K"))).await.unwrap();
    assert_eq!(answer(&mut a).await, (false, true), "a PNG isn't taken");
    // unreadable base64: told at once, nothing sent on
    proto::write_msg(&mut a, &set(Some("not base64!"))).await.unwrap();
    loop {
        if let DaemonMsg::Event(Event::Error { message }) = proto::read_msg::<_, DaemonMsg>(&mut a).await.unwrap().unwrap() {
            assert!(message.contains("readable"));
            break;
        }
    }
}
