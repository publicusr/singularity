//! #81: a fresh Daemon shows the last Home and library at once, while its Catalogue is still logging in.

mod common;

use singularity_proto::{self as proto, CatalogueStatus, ClientMsg, DaemonMsg, Event, HomeFeed, Library, LibraryItem, PROTOCOL_VERSION, Shelf};
use singularityd::catalogue::CatEvent;
use singularityd::engine::NoEngine;
use singularityd::last_seen::Seen;
use std::time::Duration;
use tokio::net::UnixStream;

fn home(title: &str) -> HomeFeed {
    HomeFeed { fetched_unix_ms: 1, shelves: vec![Shelf { id: String::new(), title: title.into(), kind: "shelf".into(), cards: vec![] }] }
}

fn library(name: &str) -> Library {
    Library { items: vec![LibraryItem { uri: "spotify:playlist:p".into(), name: name.into(), subtitle: String::new(), kind: "playlist".into(), image_url: None, depth: 0, own: true, authors: vec![] }] }
}

async fn connect(path: &std::path::Path) -> UnixStream {
    let mut s = UnixStream::connect(path).await.unwrap();
    proto::write_msg(&mut s, &ClientMsg::Hello { protocol: PROTOCOL_VERSION, client: "test".into(), demo: false }).await.unwrap();
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

/// A Daemon whose Catalogue the test speaks for, keeping the last copy at `file`.
fn daemon(dir: &std::path::Path, file: &std::path::Path) -> (std::path::PathBuf, tokio::sync::mpsc::Sender<CatEvent>) {
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(16);
    // (kept open: a Catalogue that hasn't answered anything yet)
    std::mem::forget(cmd_rx);
    let (ev_tx, ev_rx) = tokio::sync::mpsc::channel(16);
    let path = dir.join("ls.sock");
    let listener = singularityd::bind(&path).unwrap();
    let extras = singularityd::Extras { last_seen: Some(file.to_path_buf()), ..Default::default() };
    tokio::spawn(singularityd::serve_with(listener, Box::new(NoEngine { reason: "test".into() }), Some((cmd_tx, ev_rx)), None, extras));
    (path, ev_tx)
}

fn write(file: &std::path::Path, seen: &Seen) {
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, serde_json::to_vec(seen).unwrap()).unwrap();
}

fn alice() -> Seen {
    Seen { account: Some("alice".into()), home: Some(home("Yesterday")), library: Some(library("Old")) }
}

fn ready(username: &str) -> CatEvent {
    CatEvent::Status(CatalogueStatus::Ready { username: username.into(), country: "SE".into(), premium: true })
}

#[tokio::test]
async fn the_last_home_and_library_show_once_their_account_is_named_then_are_replaced_and_go_on_logout() {
    let dir = common::scratch();
    let file = dir.path().join("cache").join("home-and-library.json");
    write(&file, &alice());
    let (path, ev) = daemon(dir.path(), &file);

    // before the Catalogue names the account, nothing is shown
    let (mut a, evs) = snapshot(&path).await;
    assert!(!evs.iter().any(|e| matches!(e, Event::Home(_) | Event::Library(_))), "{evs:?}");
    // the saved login is alice's: her last Home and library show at once, to Clients there and to new ones
    ev.send(CatEvent::Account("alice".into())).await.unwrap();
    assert_eq!(next(&mut a, |e| matches!(e, Event::Home(_))).await, Event::Home(home("Yesterday")));
    let (_b, evs) = snapshot(&path).await;
    assert!(evs.contains(&Event::Home(home("Yesterday"))) && evs.contains(&Event::Library(library("Old"))), "{evs:?}");

    // Ready as alice changes nothing; the fresh Home replaces the old one, on screen and on disk
    ev.send(ready("alice")).await.unwrap();
    ev.send(CatEvent::Home(home("Today"))).await.unwrap();
    assert_eq!(next(&mut a, |e| matches!(e, Event::Home(_))).await, Event::Home(home("Today")));
    assert_eq!(kept(&file, |s| s.home == Some(home("Today"))).await, Some(Seen { home: Some(home("Today")), ..alice() }));

    // logged out: the file goes, and so does everything the Daemon holds of hers, fresh or not
    ev.send(CatEvent::Status(CatalogueStatus::LoggedOut)).await.unwrap();
    next(&mut a, |e| matches!(e, Event::Catalogue(CatalogueStatus::LoggedOut))).await;
    assert!(gone(&file).await);
    let (_c, evs) = snapshot(&path).await;
    assert!(!evs.iter().any(|e| matches!(e, Event::Home(_) | Event::Library(_))), "{evs:?}");

    // bob logs in and his library arrives first: the file holds his library, never her Home
    ev.send(ready("bob")).await.unwrap();
    ev.send(CatEvent::Library(library("Bob's"))).await.unwrap();
    let want = Seen { account: Some("bob".into()), home: None, library: Some(library("Bob's")) };
    assert_eq!(kept(&file, |s| s.account.as_deref() == Some("bob")).await, Some(want));
}

#[tokio::test]
async fn another_accounts_copy_is_never_shown_and_is_deleted() {
    let dir = common::scratch();
    let file = dir.path().join("home-and-library.json");
    write(&file, &alice());
    let (path, ev) = daemon(dir.path(), &file);
    let (mut a, _) = snapshot(&path).await;
    ev.send(ready("bob")).await.unwrap();
    next(&mut a, |e| matches!(e, Event::Catalogue(CatalogueStatus::Ready { .. }))).await;
    let (_b, evs) = snapshot(&path).await;
    assert!(!evs.iter().any(|e| matches!(e, Event::Home(_) | Event::Library(_))), "{evs:?}");
    assert!(gone(&file).await, "alice's copy is gone");
}

/// What the file holds once the Daemon's writer has done what `done` waits for (it writes on its own thread).
async fn kept(file: &std::path::Path, done: fn(&Seen) -> bool) -> Option<Seen> {
    let read = || std::fs::read(file).ok().and_then(|b| serde_json::from_slice::<Seen>(&b).ok());
    for _ in 0..50 {
        match read() {
            Some(s) if done(&s) => return Some(s),
            _ => tokio::time::sleep(Duration::from_millis(20)).await,
        }
    }
    read()
}

/// Whether the file is gone once the Daemon's writer has caught up.
async fn gone(file: &std::path::Path) -> bool {
    for _ in 0..50 {
        if !file.exists() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}
