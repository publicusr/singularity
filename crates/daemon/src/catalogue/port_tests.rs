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

    /// The operations asked for, in order, a search with its words.
    fn order(&self) -> Vec<String> {
        self.asked.lock().unwrap().iter().map(|(op, v)| format!("{op} {}", v["searchTerm"].as_str().unwrap_or("")).trim_end().to_string()).collect()
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

/// #81 with #126: a search typed while the start's Home and library load is made at once, beside them, not after
/// the whole library; the library still comes, whole.
#[tokio::test]
async fn a_search_during_the_start_fetch_does_not_wait_for_it() {
    // forty pages at the top, 200 ms each, four at a time: about two seconds
    let tree = Arc::new(HashMap::from([("".to_string(), (0..2000).map(|i| ("playlist", format!("p{i}"))).collect::<Vec<_>>())]));
    let t = tree.clone();
    let f = Fixtures::new(move |op, vars| Ok(if op == "libraryV3" { paged_library(&t, vars) } else { json!({}) }))
        .slow(|op| Duration::from_millis(if op == "searchDesktop" { 0 } else { 200 }));
    let (_dir, tx, mut ev) = catalogue(&f);
    tx.send(CatCmd::HomeAndLibrary).await.unwrap();
    until_asked(&f, "libraryV3", 2).await;
    let typed = std::time::Instant::now();
    tx.send(CatCmd::Search("nova".into())).await.unwrap();
    let first = next(&mut ev, |e| match e {
        CatEvent::Search(_) => Some("search"),
        CatEvent::Library(_) => Some("library"),
        _ => None,
    })
    .await;
    assert_eq!(first, "search", "the search waited for the library");
    assert!(typed.elapsed() < Duration::from_millis(500), "{:?}", typed.elapsed());
    let lib = next(&mut ev, |e| if let CatEvent::Library(l) = e { Some(l) } else { None }).await;
    assert_eq!(lib.items.len(), 2000);
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

/// Until `f` has been asked `op` at least `n` times (the work holding it is under way).
async fn until_asked(f: &Fixtures, op: &str, n: usize) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while f.asked(op) < n {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("never asked");
}

/// A raw query as a command (it waits in browsing's queue like any other); its answer is dropped.
fn query(op: &str) -> CatCmd {
    CatCmd::Query { op: op.into(), vars: json!({}), reply: tokio::sync::oneshot::channel().0 }
}

/// Where `what` was asked, among everything asked.
fn at(order: &[String], what: &str) -> usize {
    order.iter().position(|o| o == what).unwrap_or_else(|| panic!("{what} wasn't asked: {order:?}"))
}

/// A Catalogue over `f`, in a scratch dir.
fn catalogue(f: &Fixtures) -> (tempfile::TempDir, mpsc::Sender<CatCmd>, mpsc::Receiver<CatEvent>) {
    no_real_login();
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

/// #128: demo Home's quiet look at a long playlist asks for its first page only, and a failure is answered with
/// neither page, not with a toast.
#[tokio::test]
async fn a_peek_asks_for_one_page_and_fails_quietly() {
    let f = Fixtures::new(|op, v| match op {
        "fetchPlaylist" => {
            let mut v = fixture("playlist.json");
            v["data"]["playlistV2"]["content"]["totalCount"] = json!(250);
            Ok(v)
        }
        _ => anyhow::bail!("offline ({})", v["uri"]),
    });
    let (_dir, tx, mut ev) = catalogue(&f);
    tx.send(CatCmd::Peek("spotify:playlist:p".into())).await.unwrap();
    let (tracks, artist) = next(&mut ev, |e| if let CatEvent::Peeked { tracks, artist, .. } = e { Some((tracks.clone(), artist.clone())) } else { None }).await;
    let page = tracks.expect("the first page");
    assert_eq!((page.offset, page.total, artist.is_none()), (0, 250, true));
    assert_eq!(f.asked("fetchPlaylist"), 1, "one page, not the whole list");
    tx.send(CatCmd::Peek("spotify:artist:a".into())).await.unwrap();
    let got = next(&mut ev, |e| match e {
        CatEvent::Peeked { tracks, artist, .. } => Some(Some((tracks.is_none(), artist.is_none()))),
        CatEvent::Error(_) => Some(None),
        _ => None,
    })
    .await;
    assert_eq!(got, Some((true, true)), "a failure: neither, and no toast first");
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
    .slow(|_| Duration::from_millis(20));
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
    let f = Fixtures::new(|_, _| Ok(json!({}))).slow(|_| Duration::from_millis(20));
    let (_dir, tx, mut ev) = catalogue(&f);
    tx.send(CatCmd::Describe((0..30).map(|i| format!("spotify:track:t{i}")).collect())).await.unwrap();
    until_asked(&f, "getTrack", 1).await;
    tx.send(CatCmd::Search("nova".into())).await.unwrap();
    next(&mut ev, |e| matches!(e, CatEvent::Search(_)).then_some(())).await;
    let order = f.order();
    assert!(at(&order, "searchDesktop nova") <= 2, "the search waited for the whole queue: {order:?}");
    next(&mut ev, |e| matches!(e, CatEvent::DescribeFailed(_)).then_some(())).await;
    assert_eq!(f.asked("getTrack"), 30, "and the queue is still named, every track");
}

/// #126: the ♥ checks a page sends (50 at a time) let a search through between two batches.
#[tokio::test]
async fn a_search_is_made_between_two_batches_of_heart_checks() {
    let f = Fixtures::new(|_, _| Ok(json!({}))).slow(|_| Duration::from_millis(20));
    let (_dir, tx, mut ev) = catalogue(&f);
    tx.send(CatCmd::CheckSaved((0..150).map(|i| format!("spotify:track:t{i}")).collect())).await.unwrap();
    next(&mut ev, |e| matches!(e, CatEvent::Saved(_)).then_some(())).await;
    tx.send(CatCmd::Search("nova".into())).await.unwrap();
    next(&mut ev, |e| matches!(e, CatEvent::Search(_)).then_some(())).await;
    let order = f.order();
    assert!(at(&order, "searchDesktop nova") <= 2, "the search waited for every batch: {order:?}");
    until_asked(&f, "areEntitiesInLibrary", 3).await;
}

/// #126 (review): a login or logout queued behind long work isn't jumped by a search made between two of the work's
/// requests: the search would go out on the old session. (A reconnect stands in for both here: it reaches nothing.)
#[tokio::test]
async fn a_search_never_goes_ahead_of_a_queued_session_change() {
    let f = Fixtures::new(|op, _| {
        let mut v = fixture("playlist.json");
        v["data"]["playlistV2"]["content"]["totalCount"] = json!(500);
        Ok(if op == "fetchPlaylist" { v } else { json!({}) })
    })
    .slow(|_| Duration::from_millis(20));
    let (_dir, tx, mut ev) = catalogue(&f);
    tx.send(CatCmd::Tracks("spotify:playlist:p".into())).await.unwrap();
    next(&mut ev, |e| matches!(e, CatEvent::Tracks(_)).then_some(())).await;
    tx.send(CatCmd::Reconnect).await.unwrap();
    tx.send(CatCmd::Search("nova".into())).await.unwrap();
    next(&mut ev, |e| matches!(e, CatEvent::Search(_)).then_some(())).await;
    let order = f.order();
    assert_eq!(f.asked("fetchPlaylist"), 5, "the list finished first: {order:?}");
    assert_eq!(order.last().map(String::as_str), Some("searchDesktop nova"));
}

/// #126 (review): searches go ahead of queued work at most `SEARCHES_IN_A_ROW` times in a row; then a queued command
/// runs before the next one, so typing can't hold everything else back.
#[tokio::test]
async fn a_stream_of_searches_lets_queued_work_through() {
    let f = Fixtures::new(|_, _| Ok(json!({}))).slow(|_| Duration::from_millis(20));
    let (_dir, tx, _ev) = catalogue(&f);
    for c in [query("qA"), query("qB"), CatCmd::Search("s1".into())] {
        tx.send(c).await.unwrap();
    }
    // each next search comes while the one before is with Spotify
    type_while_searching(&f, &tx, &["s2", "s3", "s4"]).await;
    until_asked(&f, "qB", 1).await;
    let order = f.order();
    let order: Vec<&str> = order.iter().map(|o| o.strip_prefix("searchDesktop ").unwrap_or(o)).collect();
    assert_eq!(order, ["s1", "s2", "s3", "qA", "s4", "qB"]);
}

/// Send each of `words` as a search while the one before it is with Spotify.
async fn type_while_searching(f: &Fixtures, tx: &mpsc::Sender<CatCmd>, words: &[&str]) {
    for (i, q) in words.iter().enumerate() {
        until_asked(f, "searchDesktop", i + 1).await;
        tx.send(CatCmd::Search(q.to_string())).await.unwrap();
    }
}

/// #126 (review): a lyrics lookup lets a search through before going to LRCLIB (which can take seconds). Three
/// searches have just gone ahead, so the lyrics run first and the fourth search comes in the middle of them.
#[tokio::test]
async fn a_search_is_made_before_a_lyrics_lookup_goes_to_lrclib() {
    let f = Fixtures::new(|_, _| Ok(json!({}))).slow(|_| Duration::from_millis(20));
    let (_dir, tx, mut ev) = catalogue(&f);
    // no name for the track: it is looked up (`getTrack`), finds none, and LRCLIB is never reached
    let lyrics = CatCmd::Lyrics { uri: "spotify:track:t".into(), meta: None, words: false };
    for c in [lyrics, CatCmd::Search("s1".into())] {
        tx.send(c).await.unwrap();
    }
    type_while_searching(&f, &tx, &["s2", "s3", "s4"]).await;
    next(&mut ev, |e| matches!(e, CatEvent::Lyrics(_)).then_some(())).await;
    let order = f.order();
    let order: Vec<&str> = order.iter().map(|o| o.strip_prefix("searchDesktop ").unwrap_or(o)).collect();
    assert_eq!(order, ["s1", "s2", "s3", "s4", "getTrack"]);
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

/// A browsing State as `run` makes one, not yet started, with a session that logged in `ago` and reaches nothing
/// (never connected), and `clients` Clients connected; idle browsing looks every 10 ms.
fn ended_soon(ago: Duration, clients: usize) -> (State<Fixtures>, Session, mpsc::Receiver<CatEvent>, mpsc::UnboundedSender<CatCmd>, tempfile::TempDir) {
    no_real_login();
    let dir = tempfile::tempdir().unwrap();
    let dirs = Dirs { credentials: dir.path().join("creds"), queries: dir.path().join("q.json") };
    let (ev_tx, ev) = mpsc::channel(64);
    let (connect_tx, connect_rx) = mpsc::unbounded_channel();
    std::mem::forget(connect_rx);
    let (browse_tx, browse_rx) = mpsc::unbounded_channel();
    let mut st = State::new(dirs, Fixtures::new(|_, _| Ok(json!({}))), session_slot(), ev_tx, connect_tx, Default::default(), browse_rx);
    st.redial.every = Duration::from_millis(10);
    st.clients = Box::leak(Box::new(std::sync::atomic::AtomicUsize::new(clients)));
    let session = Session::new(SessionConfig::default(), None);
    st.set_session(Some(session.clone()));
    st.redial.logged_in(Instant::now() - ago);
    (st, session, ev, browse_tx, dir)
}

/// The next status from the Catalogue within `wait`, if any.
async fn status_within(ev: &mut mpsc::Receiver<CatEvent>, wait: Duration) -> Option<CatalogueStatus> {
    tokio::time::timeout(wait, next(ev, |e| if let CatEvent::Status(s) = e { Some(s) } else { None })).await.ok()
}

/// Q171: with a Client connected, Spotify ending the session is seen while idle and the login is tried again then,
/// with no request to set it off. With no saved login found then (a keyring that doesn't answer), it isn't taken for
/// a logout (that forgets Home and the library): it's a failed try, tried again later.
#[tokio::test]
async fn a_session_spotify_ended_is_replaced_without_a_request() {
    let (st, session, mut ev, _browse, _dir) = ended_soon(STEADY, 1);
    tokio::spawn(browse(st));
    // the start: no saved login in the scratch dirs
    assert!(matches!(status_within(&mut ev, Duration::from_secs(5)).await, Some(CatalogueStatus::LoggedOut)));
    // nothing is asked of it; Spotify ends the session
    session.shutdown();
    assert!(matches!(status_within(&mut ev, Duration::from_secs(5)).await, Some(CatalogueStatus::Unavailable { .. })), "a failed try, not a logout");
    assert!(status_within(&mut ev, Duration::from_millis(300)).await.is_none(), "the next try waits");
}

/// Q171: with no Client connected, an ended session isn't replaced in the background; the next request does it.
#[tokio::test]
async fn with_no_client_the_next_request_logs_in_again() {
    let (st, session, mut ev, browse_tx, _dir) = ended_soon(STEADY, 0);
    tokio::spawn(browse(st));
    assert!(matches!(status_within(&mut ev, Duration::from_secs(5)).await, Some(CatalogueStatus::LoggedOut)));
    session.shutdown();
    assert!(status_within(&mut ev, Duration::from_millis(300)).await.is_none(), "no one's here: no login in the background");
    browse_tx.send(query("x")).unwrap();
    assert!(matches!(status_within(&mut ev, Duration::from_secs(5)).await, Some(CatalogueStatus::Unavailable { .. })), "the request tried");
}

/// Q171 (review): a session that ends soon after its login waits before the background login, and says so; a request
/// meanwhile still logs in again at once.
#[tokio::test]
async fn a_request_logs_in_again_at_once_while_the_background_waits() {
    let (st, session, mut ev, browse_tx, _dir) = ended_soon(Duration::ZERO, 1);
    tokio::spawn(browse(st));
    assert!(matches!(status_within(&mut ev, Duration::from_secs(5)).await, Some(CatalogueStatus::LoggedOut)));
    session.shutdown();
    match status_within(&mut ev, Duration::from_secs(5)).await {
        Some(CatalogueStatus::Unavailable { message }) => assert!(message.contains("logging in again in 5 s"), "{message}"),
        other => panic!("the wait isn't said: {other:?}"),
    }
    assert!(status_within(&mut ev, Duration::from_millis(300)).await.is_none(), "the background waits");
    browse_tx.send(query("x")).unwrap();
    match status_within(&mut ev, Duration::from_millis(500)).await {
        Some(CatalogueStatus::Unavailable { message }) => assert!(message.contains("saved Spotify login"), "the request tried at once: {message}"),
        other => panic!("the request didn't log in again: {other:?}"),
    }
}
