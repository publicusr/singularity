//! With `HOME` unset but all four `XDG_*_HOME` set the Daemon runs; an export with nothing naming the Downloads
//! folder says so in its own words, not the start's "HOME isn't set" (#97). Its own test binary, so changing the
//! environment can't race other tests.

mod common;

use singularity_proto::{self as proto, ClientMsg, Command, DaemonMsg, Event, PROTOCOL_VERSION};
use singularityd::engine::NoEngine;

#[tokio::test]
async fn an_export_without_a_downloads_folder_says_what_to_set() {
    let dir = common::scratch();
    // SAFETY: this binary's only test sets them before anything reads them
    unsafe {
        std::env::remove_var("HOME");
        std::env::remove_var("XDG_DOWNLOAD_DIR");
        for v in ["XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"] {
            std::env::set_var(v, dir.path().join(v));
        }
    }
    assert!(singularity_config::app_folders::AppFolders::from_env().is_ok(), "the Daemon starts");
    let path = dir.path().join("x.sock");
    let listener = singularityd::bind(&path).unwrap();
    let extras = singularityd::Extras { store: Some(dir.path().join("data")), ..Default::default() };
    tokio::spawn(singularityd::serve_with(listener, Box::new(NoEngine { reason: "test".into() }), None, None, extras));
    let mut s = tokio::net::UnixStream::connect(&path).await.unwrap();
    proto::write_msg(&mut s, &ClientMsg::Hello { protocol: PROTOCOL_VERSION, client: "test".into(), demo: false }).await.unwrap();
    proto::write_msg(&mut s, &ClientMsg::Command(Command::ExportLocalData)).await.unwrap();
    let message = loop {
        match proto::read_msg::<_, DaemonMsg>(&mut s).await.unwrap().unwrap() {
            DaemonMsg::Event(Event::Error { message }) => break message,
            DaemonMsg::Event(Event::Notice { message }) => panic!("exported: {message}"),
            _ => {}
        }
    };
    assert_eq!(message, "Local data: no Downloads folder: set HOME or XDG_DOWNLOAD_DIR");
}
