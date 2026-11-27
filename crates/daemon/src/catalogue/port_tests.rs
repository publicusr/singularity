//! The real Catalogue at its own interface (D78), `CatCmd` in and `CatEvent` out, over a fixtures adapter for the
//! query port. It runs logged out of Spotify: queries go through the port either way, and nothing else is reached.
//! What these catch: what is asked, in what order, what is kept and reused, how pages and failures come out. What
//! they can't: Spotify's real answers changing shape (the recorded fixtures stay as they were).

use super::*;
use std::sync::{Arc, Mutex};

/// How a fixture answers one query.
type Answer = Arc<dyn Fn(&str, &Value) -> anyhow::Result<Value> + Send + Sync>;

/// The query port answered from recorded fixtures by a rule, every question written down.
#[derive(Clone)]
struct Fixtures {
    answer: Answer,
    asked: Arc<Mutex<Vec<(String, Value)>>>,
    /// how long each answer takes (Spotify's latency, for the ordering tests)
    latency: Duration,
}

impl Fixtures {
    fn new(answer: impl Fn(&str, &Value) -> anyhow::Result<Value> + Send + Sync + 'static) -> Fixtures {
        Fixtures { answer: Arc::new(answer), asked: Default::default(), latency: Duration::ZERO }
    }

    /// Every answer takes `latency`.
    fn slow(mut self, latency: Duration) -> Fixtures {
        self.latency = latency;
        self
    }

    fn asked(&self, op: &str) -> usize {
        self.asked.lock().unwrap().iter().filter(|(o, _)| o == op).count()
    }

    /// The operations asked for, in order, a search with its words.
    fn order(&self) -> Vec<String> {
        self.asked.lock().unwrap().iter().map(|(op, v)| format!("{op} {}", v["searchTerm"].as_str().unwrap_or("")).trim_end().to_string()).collect()
    }
}

impl Fetch for Fixtures {
    async fn query(&mut self, op: &str, vars: Value) -> anyhow::Result<Value> {
        self.asked.lock().unwrap().push((op.into(), vars.clone()));
        if !self.latency.is_zero() {
            tokio::time::sleep(self.latency).await;
        }
        (self.answer)(op, &vars)
    }
}

fn fixture(name: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap()
}

/// A Catalogue over `f`, in a scratch dir.
fn catalogue(f: &Fixtures) -> (tempfile::TempDir, mpsc::Sender<CatCmd>, mpsc::Receiver<CatEvent>) {
    let dir = tempfile::tempdir().unwrap();
    let dirs = Dirs { credentials: dir.path().join("creds"), queries: dir.path().join("q.json") };
    let f = f.clone();
    let (tx, ev) = spawn_with(dirs, Identity::default(), move |_| f.clone());
    (dir, tx, ev)
}

/// The next event `pick` takes, skipping the rest (status changes); a test fails rather than wait for ever.
async fn next<T>(ev: &mut mpsc::Receiver<CatEvent>, pick: impl Fn(CatEvent) -> Option<T>) -> T {
    let wait = async {
        loop {
            if let Some(t) = pick(ev.recv().await.expect("the Catalogue stopped")) {
                return t;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), wait).await.expect("no such event from the Catalogue")
}

#[tokio::test]
async fn the_library_is_walked_into_its_folders() {
    let f = Fixtures::new(|op, vars| {
        assert_eq!(op, "libraryV3");
        // the top level as recorded, and an empty folder
        Ok(if vars["folderUri"].is_string() { json!({}) } else { fixture("library.json") })
    });
    let (_dir, tx, mut ev) = catalogue(&f);
    tx.send(CatCmd::Library).await.unwrap();
    let lib = next(&mut ev, |e| if let CatEvent::Library(l) = e { Some(l) } else { None }).await;
    let kinds: Vec<&str> = lib.items.iter().map(|i| i.kind.as_str()).collect();
    assert_eq!(kinds, ["liked", "playlist", "folder", "album", "artist"]);
    assert_eq!(f.asked("libraryV3"), 2, "the top level, then the folder");
}

#[tokio::test]
async fn a_search_is_kept_and_asked_for_once() {
    let f = Fixtures::new(|op, _| {
        assert_eq!(op, "searchDesktop");
        Ok(json!({}))
    });
    let (_dir, tx, mut ev) = catalogue(&f);
    for q in ["nova", " Nova "] {
        tx.send(CatCmd::Search(q.into())).await.unwrap();
        let r = next(&mut ev, |e| if let CatEvent::Search(r) = e { Some(r) } else { None }).await;
        assert_eq!(r.query, q, "the answer carries the words as typed");
        assert!(r.error.is_none());
    }
    assert_eq!(f.asked("searchDesktop"), 1, "matched ignoring case and outer spaces");
}

#[tokio::test]
async fn a_failed_search_is_answered_with_its_error_and_not_kept() {
    let f = Fixtures::new(|_, _| anyhow::bail!("Spotify didn't answer within 12 s"));
    let (_dir, tx, mut ev) = catalogue(&f);
    for _ in 0..2 {
        tx.send(CatCmd::Search("nova".into())).await.unwrap();
        let r = next(&mut ev, |e| if let CatEvent::Search(r) = e { Some(r) } else { None }).await;
        assert!(r.error.as_deref().is_some_and(|e| e.contains("12 s")), "{:?}", r.error);
    }
    assert_eq!(f.asked("searchDesktop"), 2, "a failure is asked again");
}

#[tokio::test]
async fn a_track_spotify_cannot_describe_fails_alone_and_is_not_asked_about_again() {
    let f = Fixtures::new(|op, vars| {
        assert_eq!(op, "getTrack");
        if vars["uri"] == "spotify:track:gone" { anyhow::bail!("404") } else { Ok(json!({})) }
    });
    let (_dir, tx, mut ev) = catalogue(&f);
    tx.send(CatCmd::Describe(vec!["spotify:track:gone".into()])).await.unwrap();
    let failed = next(&mut ev, |e| if let CatEvent::DescribeFailed(u) = e { Some(u) } else { None }).await;
    assert_eq!(failed, ["spotify:track:gone"]);
    tx.send(CatCmd::Describe(vec!["spotify:track:gone".into()])).await.unwrap();
    let failed = next(&mut ev, |e| if let CatEvent::DescribeFailed(u) = e { Some(u) } else { None }).await;
    assert_eq!(failed, ["spotify:track:gone"], "still reported, so nothing waits on it");
    assert_eq!(f.asked("getTrack"), 1, "not asked again within DESCRIBE_RETRY");
}

#[tokio::test]
async fn a_track_list_is_sent_and_kept() {
    let f = Fixtures::new(|op, _| {
        assert_eq!(op, "fetchPlaylist");
        Ok(fixture("playlist.json"))
    });
    let (_dir, tx, mut ev) = catalogue(&f);
    let uri = "spotify:playlist:p";
    for _ in 0..2 {
        tx.send(CatCmd::Tracks(uri.into())).await.unwrap();
        let page = next(&mut ev, |e| if let CatEvent::Tracks(p) = e { Some(p) } else { None }).await;
        assert_eq!(page.uri, uri);
        assert!(!page.tracks.is_empty());
    }
    assert_eq!(f.asked("fetchPlaylist"), 1, "the second open comes from what was kept");
}

#[tokio::test]
async fn a_long_track_list_comes_page_by_page() {
    let f = Fixtures::new(|op, _| {
        assert_eq!(op, "fetchPlaylist");
        let mut v = fixture("playlist.json");
        v["data"]["playlistV2"]["content"]["totalCount"] = json!(250);
        Ok(v)
    });
    let (_dir, tx, mut ev) = catalogue(&f);
    tx.send(CatCmd::Tracks("spotify:playlist:p".into())).await.unwrap();
    let mut offsets = vec![];
    for _ in 0..3 {
        offsets.push(next(&mut ev, |e| if let CatEvent::Tracks(p) = e { Some(p.offset) } else { None }).await);
    }
    assert_eq!(offsets, [0, 100, 200], "each page sent as it arrives");
    let asked: Vec<u64> = f.asked.lock().unwrap().iter().map(|(_, v)| v["offset"].as_u64().unwrap()).collect();
    assert_eq!(asked, [0, 100, 200]);
}

/// While typing, only the newest search matters: the ones it replaces aren't asked for. It goes ahead of the browsing
/// work waiting (#126), and what waited runs after it, in the order it was asked for.
#[tokio::test]
async fn the_newest_search_goes_first_and_the_rest_keep_their_order() {
    let f = Fixtures::new(|op, _| Ok(if op == "libraryV3" { fixture("library.json") } else { json!({}) }));
    let (_dir, tx, mut ev) = catalogue(&f);
    // all four wait in the channel before the Catalogue reads any (this test's runtime has one thread)
    for c in [CatCmd::Tracks("spotify:playlist:p".into()), CatCmd::Search("no".into()), CatCmd::Library, CatCmd::Search("nova".into())] {
        tx.send(c).await.unwrap();
    }
    next(&mut ev, |e| matches!(e, CatEvent::Library(_)).then_some(())).await;
    assert_eq!(f.order()[..3], ["searchDesktop nova", "fetchPlaylist", "libraryV3"]);
}

/// #126: a search typed while a long track list loads is made between two of its pages, not after the last.
#[tokio::test]
async fn a_search_is_made_between_the_pages_of_a_long_track_list() {
    let f = Fixtures::new(|op, _| {
        let mut v = fixture("playlist.json");
        v["data"]["playlistV2"]["content"]["totalCount"] = json!(1000);
        Ok(if op == "searchDesktop" { json!({}) } else { v })
    })
    .slow(Duration::from_millis(20));
    let (_dir, tx, mut ev) = catalogue(&f);
    tx.send(CatCmd::Tracks("spotify:playlist:p".into())).await.unwrap();
    next(&mut ev, |e| matches!(e, CatEvent::Tracks(_)).then_some(())).await;
    tx.send(CatCmd::Search("nova".into())).await.unwrap();
    next(&mut ev, |e| matches!(e, CatEvent::Search(_)).then_some(())).await;
    let order = f.order();
    let at = order.iter().position(|o| o == "searchDesktop nova").unwrap();
    assert!(at <= 2, "the search waited for the whole list: {order:?}");
    assert!(next(&mut ev, |e| if let CatEvent::Tracks(p) = e { Some(p.offset) } else { None }).await > 0, "and the list carries on where it was");
}

/// #126: naming a whole queue (a `getTrack` each) lets a search through between two tracks.
#[tokio::test]
async fn a_search_is_made_between_the_tracks_of_a_queue_being_named() {
    let f = Fixtures::new(|_, _| Ok(json!({}))).slow(Duration::from_millis(20));
    let (_dir, tx, mut ev) = catalogue(&f);
    tx.send(CatCmd::Describe((0..30).map(|i| format!("spotify:track:t{i}")).collect())).await.unwrap();
    tx.send(CatCmd::Search("nova".into())).await.unwrap();
    next(&mut ev, |e| matches!(e, CatEvent::Search(_)).then_some(())).await;
    let order = f.order();
    let at = order.iter().position(|o| o == "searchDesktop nova").unwrap();
    assert!(at <= 1, "the search waited for the whole queue: {order:?}");
    next(&mut ev, |e| matches!(e, CatEvent::DescribeFailed(_)).then_some(())).await;
    assert_eq!(f.asked("getTrack"), 30, "and the queue is still named, every track");
}

/// #8: a search waited behind Connect work in the Catalogue's one loop, here the resume after an update, which
/// retries for 6.5 s while this computer's device comes back (an Engine's own commands could wait 20 s for it).
#[tokio::test]
async fn a_search_is_answered_while_connect_is_busy() {
    let f = Fixtures::new(|_, _| Ok(json!({})));
    let (_dir, tx, mut ev) = catalogue(&f);
    let resume = crate::engine::Resume { context_uri: None, track_uri: "spotify:track:t".into(), position_ms: 0, paused: false };
    tx.send(CatCmd::Resume(resume)).await.unwrap();
    tx.send(CatCmd::Search("nova".into())).await.unwrap();
    let answered = tokio::time::timeout(Duration::from_secs(1), next(&mut ev, |e| if let CatEvent::Search(r) = e { Some(r) } else { None })).await;
    assert!(answered.is_ok(), "the search waited for Connect");
}

#[test]
fn connect_work_goes_to_connect_and_browsing_stays() {
    let resume = crate::engine::Resume { context_uri: None, track_uri: "spotify:track:t".into(), position_ms: 0, paused: false };
    let play_from = CatCmd::PlayFrom { context_uri: "spotify:playlist:p".into(), index: 0, track_uri: None };
    for c in [CatCmd::Devices, CatCmd::Transfer("d".into()), CatCmd::StartJam, CatCmd::EndJam, CatCmd::Remote(Command::Next), CatCmd::Engine(Command::Pause), CatCmd::Queue(QueueEdit::Clear), play_from, CatCmd::Resume(resume)] {
        assert!(is_connect(&c), "{c:?}");
    }
    for c in [CatCmd::Search("x".into()), CatCmd::Library, CatCmd::Home, CatCmd::Describe(vec![]), CatCmd::Login, CatCmd::Logout, CatCmd::Reconnect] {
        assert!(!is_connect(&c), "{c:?}");
    }
}
