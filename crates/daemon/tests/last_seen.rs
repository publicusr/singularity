//! #81: a fresh Daemon shows the last Home and library at once, while its Catalogue is still logging in.

mod common;

use singularity_proto::{self as proto, CatalogueStatus, ClientMsg, DaemonMsg, Event, HomeFeed, Library, LibraryItem, PROTOCOL_VERSION, Shelf};
use singularityd::catalogue::CatEvent;
use singularityd::engine::NoEngine;
use singularityd::last_seen::{LastSeen, Seen};
use std::time::Duration;
use tokio::net::UnixStream;

fn home(title: &str) -> HomeFeed {
    HomeFeed { fetched_unix_ms: 1, shelves: vec![Shelf { title: title.into(), kind: "shelf".into(), cards: vec![] }] }
}

fn library(name: &str) -> Library {
    Library { items: vec![LibraryItem { uri: "spotify:playlist:p".into(), name: name.into(), subtitle: String::new(), kind: "playlist".into(), image_url: None, depth: 0, own: true, authors: vec![] }] }
}

async fn connect(path: &std::path::Path) -> UnixStream {
    let mut s = UnixStream::connect(path).await.unwrap();
    proto::write_msg(&mut s, &ClientMsg::Hello { protocol: PROTOCOL_VERSION, client: "test".into() }).await.unwrap();
    let _welcome: DaemonMsg = proto::read_msg(&mut s).await.unwrap().unwrap();
    s
}

/// The snapshot a Client gets on connecting: the events before the answer to a Ping sent right after the hello.
async fn snapshot(path: &std::path::Path) -> (UnixStream, Vec<Event>) {
    let mut s = connect(path).await;
    proto::write_msg(&mut s, &ClientMsg::Ping { id: 1 }).await.unwrap();
    let mut evs = vec![];
    loop {
        match tokio::time::timeout(Duration::from_secs(5), proto::read_msg::<_, DaemonMsg>(&mut s)).await.expect("no snapshot").unwrap().unwrap() {
            DaemonMsg::Pong { .. } => return (s, evs),
            DaemonMsg::Event(e) => evs.push(e),
            _ => {}
        }
    }
}

async fn next(s: &mut UnixStream, want: fn(&Event) -> bool) -> Event {
    let wait = async {
        loop {
            if let DaemonMsg::Event(e) = proto::read_msg::<_, DaemonMsg>(s).await.unwrap().unwrap()
                && want(&e)
            {
                return e;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), wait).await.expect("no such event")
}

#[tokio::test]
async fn the_last_home_and_library_are_shown_at_once_then_replaced_and_go_on_logout() {
    let dir = common::scratch();
    let file = dir.path().join("cache").join("home-and-library.json");
    LastSeen::at(file.clone()).write(&Seen { home: Some(home("Yesterday")), library: Some(library("Old")) }, true);
    // a Catalogue that hasn't answered anything yet (still logging in)
    let (cmd_tx, _cmd_rx) = tokio::sync::mpsc::channel(16);
    let (ev_tx, ev_rx) = tokio::sync::mpsc::channel(16);
    let path = dir.path().join("ls.sock");
    let listener = singularityd::bind(&path).unwrap();
    let extras = singularityd::Extras { last_seen: Some(file.clone()), ..Default::default() };
    tokio::spawn(singularityd::serve_with(listener, Box::new(NoEngine { reason: "test".into() }), Some((cmd_tx, ev_rx)), None, extras));

    let (mut a, evs) = snapshot(&path).await;
    assert!(evs.contains(&Event::Home(home("Yesterday"))), "{evs:?}");
    assert!(evs.contains(&Event::Library(library("Old"))), "{evs:?}");

    // the fresh Home replaces it, on screen and on disk
    ev_tx.send(CatEvent::Home(home("Today"))).await.unwrap();
    assert_eq!(next(&mut a, |e| matches!(e, Event::Home(_))).await, Event::Home(home("Today")));
    assert_eq!(kept(&file).await, (Some(home("Today")), Some(library("Old"))));

    // logged out: the file goes, and so does what was still only from it (the library); a Client connecting now
    // isn't shown the account that left
    ev_tx.send(CatEvent::Status(CatalogueStatus::LoggedOut)).await.unwrap();
    next(&mut a, |e| matches!(e, Event::Catalogue(CatalogueStatus::LoggedOut))).await;
    assert!(!file.exists());
    let (_b, evs) = snapshot(&path).await;
    assert!(!evs.iter().any(|e| matches!(e, Event::Library(_))), "{evs:?}");
}

#[tokio::test]
async fn without_a_file_nothing_is_shown_until_the_catalogue_answers() {
    let dir = common::scratch();
    let file = dir.path().join("home-and-library.json");
    let (cmd_tx, _cmd_rx) = tokio::sync::mpsc::channel(16);
    let (ev_tx, ev_rx) = tokio::sync::mpsc::channel(16);
    let path = dir.path().join("ls.sock");
    let listener = singularityd::bind(&path).unwrap();
    let extras = singularityd::Extras { last_seen: Some(file.clone()), ..Default::default() };
    tokio::spawn(singularityd::serve_with(listener, Box::new(NoEngine { reason: "test".into() }), Some((cmd_tx, ev_rx)), None, extras));
    let (mut a, evs) = snapshot(&path).await;
    assert!(!evs.iter().any(|e| matches!(e, Event::Home(_) | Event::Library(_))), "{evs:?}");
    ev_tx.send(CatEvent::Library(library("New"))).await.unwrap();
    next(&mut a, |e| matches!(e, Event::Library(_))).await;
    assert_eq!(kept(&file).await, (None, Some(library("New"))), "kept for the next start");
}

/// What the file holds once the Daemon has written it (just after it told the Clients).
async fn kept(file: &std::path::Path) -> (Option<HomeFeed>, Option<Library>) {
    let mut last = (None, None);
    for _ in 0..50 {
        let s = LastSeen::at(file.to_path_buf()).read();
        last = (s.home, s.library);
        if last.0.as_ref().is_some_and(|h| h.shelves[0].title == "Today") || last.1.as_ref().is_some_and(|l| l.items[0].name == "New") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    last
}
