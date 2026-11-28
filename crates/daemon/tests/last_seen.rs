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
    daemon_with(dir.join("ls.sock"), file, None)
}

/// `daemon` on the socket `path`, with its LocalStore in `store`.
fn daemon_with(path: std::path::PathBuf, file: &std::path::Path, store: Option<std::path::PathBuf>) -> (std::path::PathBuf, tokio::sync::mpsc::Sender<CatEvent>) {
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(16);
    // (kept open: a Catalogue that hasn't answered anything yet)
    std::mem::forget(cmd_rx);
    let (ev_tx, ev_rx) = tokio::sync::mpsc::channel(16);
    let listener = singularityd::bind(&path).unwrap();
    let extras = singularityd::Extras { last_seen: Some(file.to_path_buf()), store, ..Default::default() };
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

/// #167: "Delete listening data" also takes the copy and the demo Listening log, files and all; a fresh Home isn't
/// written back, and the next Daemon shows no library until the Catalogue fetches one.
#[tokio::test]
async fn deleting_the_listening_data_takes_the_copy_and_the_demo_log() {
    use singularity_proto::{Command, Item};
    use singularityd::store::{Play, Store};
    let dir = common::scratch();
    let file = dir.path().join("cache").join("home-and-library.json");
    let data = dir.path().join("data");
    write(&file, &alice());
    let item = Item { uri: "spotify:track:d".into(), name: "D".into(), artists: vec!["Demo Artist".into()], duration_ms: 600_000, ..Default::default() };
    Store::open_demo(&data).unwrap().record(&Play { started_ms: proto::now_ms(), item, context_uri: None, played_ms: 120_000, device: "here".into() }).unwrap();
    let demo_log = data.join("listening-demo.db");
    let (path, ev) = daemon_with(dir.path().join("a.sock"), &file, Some(data.clone()));
    let (mut a, _) = snapshot(&path).await;
    ev.send(ready("alice")).await.unwrap();
    assert_eq!(next(&mut a, |e| matches!(e, Event::Library(_))).await, Event::Library(library("Old")));

    proto::write_msg(&mut a, &ClientMsg::Command(Command::DeleteLocalData)).await.unwrap();
    let said = next(&mut a, |e| matches!(e, Event::Notice { .. } | Event::Error { .. })).await;
    assert_eq!(said, Event::Notice { message: "Deleted the listening log and every library backup".into() });
    assert!(gone(&file).await, "the copy is gone");
    assert!(!demo_log.exists(), "the demo log is gone");
    // what the Daemon fetches next isn't written back, also after a reconnect names the same account again
    ev.send(CatEvent::Home(home("Today"))).await.unwrap();
    next(&mut a, |e| matches!(e, Event::Home(_))).await;
    ev.send(ready("alice")).await.unwrap();
    ev.send(CatEvent::Account("alice".into())).await.unwrap();
    ev.send(CatEvent::Home(home("Later"))).await.unwrap();
    ev.send(CatEvent::Library(library("Later"))).await.unwrap();
    next(&mut a, |e| matches!(e, Event::Library(_))).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!file.exists(), "written back");

    // the next Daemon: nothing shows when the account is named, until the Catalogue fetches the library
    let (path, ev) = daemon_with(dir.path().join("b.sock"), &file, Some(data.clone()));
    let (mut b, _) = snapshot(&path).await;
    ev.send(ready("alice")).await.unwrap();
    next(&mut b, |e| matches!(e, Event::Catalogue(CatalogueStatus::Ready { .. }))).await;
    let (_c, evs) = snapshot(&path).await;
    assert!(!evs.iter().any(|e| matches!(e, Event::Home(_) | Event::Library(_))), "{evs:?}");
    ev.send(CatEvent::Library(library("Fresh"))).await.unwrap();
    assert_eq!(next(&mut b, |e| matches!(e, Event::Library(_))).await, Event::Library(library("Fresh")));
}

/// #167: a Delete before the Catalogue names the account: the copy read at start is never shown, and what is fetched
/// after isn't written.
#[tokio::test]
async fn deleting_before_the_account_is_named_shows_and_keeps_nothing() {
    use singularity_proto::Command;
    let dir = common::scratch();
    let file = dir.path().join("cache").join("home-and-library.json");
    let data = dir.path().join("data");
    write(&file, &alice());
    let (path, ev) = daemon_with(dir.path().join("a.sock"), &file, Some(data.clone()));
    let (mut a, _) = snapshot(&path).await;
    proto::write_msg(&mut a, &ClientMsg::Command(Command::DeleteLocalData)).await.unwrap();
    let said = next(&mut a, |e| matches!(e, Event::Notice { .. } | Event::Error { .. })).await;
    assert_eq!(said, Event::Notice { message: "Deleted the listening log and every library backup".into() });
    assert!(gone(&file).await);
    ev.send(ready("alice")).await.unwrap();
    next(&mut a, |e| matches!(e, Event::Catalogue(CatalogueStatus::Ready { .. }))).await;
    let (_b, evs) = snapshot(&path).await;
    assert!(!evs.iter().any(|e| matches!(e, Event::Home(_) | Event::Library(_))), "{evs:?}");
    ev.send(CatEvent::Library(library("Fresh"))).await.unwrap();
    next(&mut a, |e| matches!(e, Event::Library(_))).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!file.exists(), "written back");
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
