//! Seam 1 (spec, Testing Decisions): drive a real Daemon purely through the protocol.

use singularity_proto::{self as proto, ClientMsg, DaemonMsg, Event, EngineStatus, PROTOCOL_VERSION};
use singularityd::engine::NoEngine;
use tokio::net::UnixStream;

async fn start() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s").join("daemon.sock");
    let listener = singularityd::bind(&path).unwrap();
    tokio::spawn(singularityd::serve(listener, Box::new(NoEngine { reason: "test".into() })));
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
