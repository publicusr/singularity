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
    /// how long each operation takes to answer (Spotify's latency, #81)
    latency: Arc<dyn Fn(&str) -> Duration + Send + Sync>,
    /// queries out right now, and the most there have been at once
    out: Arc<std::sync::atomic::AtomicUsize>,
    most_out: Arc<std::sync::atomic::AtomicUsize>,
}

impl Fixtures {
    fn new(answer: impl Fn(&str, &Value) -> anyhow::Result<Value> + Send + Sync + 'static) -> Fixtures {
        Fixtures { answer: Arc::new(answer), asked: Default::default(), latency: Arc::new(|_| Duration::ZERO), out: Default::default(), most_out: Default::default() }
    }

    /// The same answers, each `latency(op)` late.
    fn slow(self, latency: impl Fn(&str) -> Duration + Send + Sync + 'static) -> Fixtures {
        Fixtures { latency: Arc::new(latency), ..self }
    }

    fn asked(&self, op: &str) -> usize {
        self.asked.lock().unwrap().iter().filter(|(o, _)| o == op).count()
    }

    fn most_out(&self) -> usize {
        self.most_out.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Fetch for Fixtures {
    async fn query(&mut self, op: &str, vars: Value) -> anyhow::Result<Value> {
        use std::sync::atomic::Ordering::SeqCst;
        self.asked.lock().unwrap().push((op.into(), vars.clone()));
        let now = self.out.fetch_add(1, SeqCst) + 1;
        self.most_out.fetch_max(now, SeqCst);
        let wait = (self.latency)(op);
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        self.out.fetch_sub(1, SeqCst);
        (self.answer)(op, &vars)
    }
}

/// A library as Spotify pages it (`libraryV3`): each folder's items by its URI ("" the top level), as (kind, name);
/// a folder's URI is its name.
fn paged_library(tree: &HashMap<String, Vec<(&'static str, String)>>, vars: &Value) -> Value {
    let level = tree.get(vars["folderUri"].as_str().unwrap_or("")).cloned().unwrap_or_default();
    let (offset, limit) = (vars["offset"].as_u64().unwrap() as usize, vars["limit"].as_u64().unwrap() as usize);
    let items: Vec<Value> = level
        .iter()
        .skip(offset)
        .take(limit)
        .map(|(kind, name)| match *kind {
            "folder" => json!({ "item": { "data": { "__typename": "Folder", "name": name, "uri": name } } }),
            _ => json!({ "item": { "data": { "__typename": "Playlist", "name": name, "uri": format!("spotify:playlist:{name}") } } }),
        })
        .collect();
    json!({ "data": { "me": { "libraryV3": { "totalCount": level.len(), "items": items } } } })
}

/// The library as a walk one request at a time gives it: each folder's contents right after it.
fn walked(tree: &HashMap<String, Vec<(&'static str, String)>>, folder: &str, out: &mut Vec<String>) {
    for (kind, name) in tree.get(folder).cloned().unwrap_or_default() {
        out.push(name.clone());
        if kind == "folder" {
            walked(tree, &name, out);
        }
    }
}

/// A top level of three pages with three folders: one of two pages holding a folder, one small, one empty.
fn big_library() -> HashMap<String, Vec<(&'static str, String)>> {
    let pl = |prefix: &str, n: usize| (0..n).map(|i| ("playlist", format!("{prefix}{i}"))).collect::<Vec<_>>();
    let mut top = pl("top", 120);
    top[10] = ("folder", "spotify:folder:a".into());
    top[70] = ("folder", "spotify:folder:b".into());
    top[110] = ("folder", "spotify:folder:c".into());
    let mut a = pl("a", 60);
    a[55] = ("folder", "spotify:folder:a-inner".into());
    HashMap::from([("".into(), top), ("spotify:folder:a".into(), a), ("spotify:folder:b".into(), pl("b", 5)), ("spotify:folder:c".into(), vec![]), ("spotify:folder:a-inner".into(), pl("inner", 3))])
}

/// #81: a fresh Daemon fetched its library one request after another. The pages of a level and the folders of a
/// depth now go out together (never more than `PARALLEL`), and the sidebar comes out in the same order.
#[tokio::test]
async fn the_library_pages_and_folders_are_asked_for_together_in_the_same_order() {
    let tree = Arc::new(big_library());
    let t = tree.clone();
    let latency = Duration::from_millis(200);
    let f = Fixtures::new(move |_, vars| Ok(paged_library(&t, vars))).slow(move |_| latency);
    let (_dir, tx, mut ev) = catalogue(&f);
    let began = std::time::Instant::now();
    tx.send(CatCmd::Library).await.unwrap();
    let lib = next(&mut ev, |e| if let CatEvent::Library(l) = e { Some(l) } else { None }).await;
    let took = began.elapsed();
    let mut want = vec![];
    walked(&tree, "", &mut want);
    let got: Vec<String> = lib.items.iter().map(|i| i.name.clone()).collect();
    assert_eq!(got, want, "the same order as one request at a time");
    let inner = lib.items.iter().find(|i| i.name == "inner0").unwrap();
    assert_eq!(inner.depth, 2, "a folder's folder is one level deeper still");
    // 3 top-level pages, 2 + 1 + 1 for the folders, 1 for the folder inside one
    let asked = f.asked("libraryV3");
    assert_eq!(asked, 8);
    assert!((2..=PARALLEL).contains(&f.most_out()), "out at once: {}", f.most_out());
    // one at a time it takes 8 × latency; together, five rounds (top, its pages, the folders, a's second page, a's folder)
    eprintln!("the library: {} ms for {asked} requests of {} ms each (one at a time: {} ms)", took.as_millis(), latency.as_millis(), asked as u128 * latency.as_millis());
    assert!(took < latency * 7, "{took:?}");
}

/// #81: after a login, Home and the library are asked for at once, each sent as soon as it's in.
#[tokio::test]
async fn home_and_the_library_are_fetched_side_by_side() {
    // the top level as recorded, its folder empty
    let f = Fixtures::new(|op, vars| Ok(if op == "libraryV3" && !vars["folderUri"].is_string() { fixture("library.json") } else { json!({}) }))
        .slow(|op| Duration::from_millis(if op == "home" { 400 } else { 100 }));
    let (_dir, tx, mut ev) = catalogue(&f);
    let began = std::time::Instant::now();
    tx.send(CatCmd::HomeAndLibrary).await.unwrap();
    let mut order = vec![];
    while order.len() < 2 {
        let e = next(&mut ev, |e| match e {
            CatEvent::Home(_) => Some("home"),
            CatEvent::Library(_) => Some("library"),
            _ => None,
        })
        .await;
        order.push(e);
    }
    let took = began.elapsed();
    assert_eq!(order, ["library", "home"], "the library didn't wait for the slower Home");
    // Home 400 ms alongside the library's two requests of 100 ms each (top level, then its folder)
    eprintln!("Home and the library: {} ms (one after the other: 600 ms)", took.as_millis());
    assert!(took < Duration::from_millis(580), "{took:?}");
}

/// Spotify asking to slow down (a 429, or librespot's own limiter): the page is asked once more after a pause, and
/// a second refusal fails the library instead of asking again and again.
#[tokio::test]
async fn a_library_page_refused_for_the_rate_is_asked_once_more_after_a_pause() {
    let refusals = Arc::new(std::sync::atomic::AtomicUsize::new(1));
    let left = refusals.clone();
    let f = Fixtures::new(move |_, vars| {
        if vars["folderUri"].is_string() {
            return Ok(json!({}));
        }
        if left.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            left.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            return Err(librespot_core::Error::resource_exhausted("rate limited for at least another 3 seconds").into());
        }
        Ok(fixture("library.json"))
    });
    let (_dir, tx, mut ev) = catalogue(&f);
    let began = std::time::Instant::now();
    tx.send(CatCmd::Library).await.unwrap();
    let lib = next(&mut ev, |e| if let CatEvent::Library(l) = e { Some(l) } else { None }).await;
    assert!(began.elapsed() >= BACKOFF, "it waited before asking again");
    assert_eq!(lib.items.len(), 5);
    assert_eq!(f.asked("libraryV3"), 3, "the top level twice, then its folder");
    // refused twice: the library fails, asked no more than that
    refusals.store(2, std::sync::atomic::Ordering::SeqCst);
    tx.send(CatCmd::Library).await.unwrap();
    let err = next(&mut ev, |e| if let CatEvent::Error(m) = e { Some(m) } else { None }).await;
    assert!(err.contains("Library unavailable"), "{err}");
    assert_eq!(f.asked("libraryV3"), 5);
}

/// Spotify's side failing (a 503) on Home: asked once more after a pause, like a 429; a request it got wrong (a 400)
/// is not asked again.
#[tokio::test]
async fn home_failing_on_spotifys_side_is_asked_once_more_and_a_bad_request_is_not() {
    let fails = Arc::new(std::sync::atomic::AtomicUsize::new(1));
    let left = fails.clone();
    let f = Fixtures::new(move |_, _| {
        if left.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            left.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            return Err(librespot_core::Error::unavailable("Response status code: 503 Service Unavailable").into());
        }
        Ok(json!({}))
    });
    let (_dir, tx, mut ev) = catalogue(&f);
    let began = std::time::Instant::now();
    tx.send(CatCmd::Home).await.unwrap();
    next(&mut ev, |e| matches!(e, CatEvent::Home(_)).then_some(())).await;
    assert!(began.elapsed() >= BACKOFF);
    assert_eq!(f.asked("home"), 2);
    let f = Fixtures::new(|_, _| Err(librespot_core::Error::invalid_argument("Response status code: 400 Bad Request").into()));
    let (_dir, tx, mut ev) = catalogue(&f);
    tx.send(CatCmd::Home).await.unwrap();
    let err = next(&mut ev, |e| if let CatEvent::Error(m) = e { Some(m) } else { None }).await;
    assert!(err.contains("Home feed unavailable"), "{err}");
    assert_eq!(f.asked("home"), 1);
}

/// Home and the library share one gate: never more than `PARALLEL` out between them, however big the library.
#[tokio::test]
async fn home_and_a_big_library_never_have_more_than_the_gate_out() {
    // eight pages at the top: seven wanted at once after the first, beside Home
    let tree = Arc::new(HashMap::from([("".to_string(), (0..400).map(|i| ("playlist", format!("p{i}"))).collect::<Vec<_>>())]));
    let t = tree.clone();
    let f = Fixtures::new(move |op, vars| Ok(if op == "libraryV3" { paged_library(&t, vars) } else { json!({}) }))
        .slow(|op| Duration::from_millis(if op == "home" { 300 } else { 50 }));
    let (_dir, tx, mut ev) = catalogue(&f);
    tx.send(CatCmd::HomeAndLibrary).await.unwrap();
    next(&mut ev, |e| matches!(e, CatEvent::Library(_)).then_some(())).await;
    assert_eq!(f.most_out(), PARALLEL, "Home counts against the gate too");
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

/// While typing, only the newest search matters: the ones it replaces aren't asked for, and what waited behind
/// them runs after it, in the order it was asked for.
#[tokio::test]
async fn the_newest_search_goes_first_and_the_rest_keep_their_order() {
    let f = Fixtures::new(|op, _| Ok(if op == "libraryV3" { fixture("library.json") } else { json!({}) }));
    let (_dir, tx, mut ev) = catalogue(&f);
    // all four wait in the channel before the Catalogue reads any (this test's runtime has one thread)
    for c in [CatCmd::Tracks("spotify:playlist:p".into()), CatCmd::Search("no".into()), CatCmd::Library, CatCmd::Search("nova".into())] {
        tx.send(c).await.unwrap();
    }
    next(&mut ev, |e| matches!(e, CatEvent::Library(_)).then_some(())).await;
    let asked: Vec<String> = f.asked.lock().unwrap().iter().map(|(op, v)| format!("{op} {}", v["searchTerm"].as_str().unwrap_or(""))).collect();
    let asked: Vec<&str> = asked.iter().map(|s| s.trim_end()).collect();
    assert_eq!(asked[..3], ["fetchPlaylist", "searchDesktop nova", "libraryV3"]);
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
    for c in [CatCmd::Search("x".into()), CatCmd::Library, CatCmd::Home, CatCmd::HomeAndLibrary, CatCmd::Describe(vec![]), CatCmd::Login, CatCmd::Logout, CatCmd::Reconnect] {
        assert!(!is_connect(&c), "{c:?}");
    }
}
