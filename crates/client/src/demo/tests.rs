//! Demo mode's filter (D90), event by event, with made-up personal values only.

use super::*;
use singularity_proto::{Card, CatalogueStatus, Device, EngineStatus, Health, Jam, JamMember, ListHeader, PlaybackState, Progress, Shelf, TrackRow};

fn redactor() -> Redactor {
    Redactor::from_env(Some("/home/zed".into()), Some("zed".into()), Some("zedbox".into()))
}

fn demo() -> Demo {
    Demo::new(DemoFile::default(), redactor())
}

fn item(uri: &str, name: &str, kind: &str, depth: u8, own: bool, authors: &[&str]) -> LibraryItem {
    let subtitle = match (kind, authors) {
        ("playlist", [a, ..]) => format!("Playlist · {a}"),
        (_, [a, ..]) => a.to_string(),
        _ => String::new(),
    };
    LibraryItem { uri: uri.into(), name: name.into(), subtitle, kind: kind.into(), image_url: None, depth, own, authors: authors.iter().map(|a| a.to_string()).collect() }
}

/// Liked Songs, a private playlist, the Demo folder (one own playlist, a friend's, a nested folder with one more), an
/// album and a podcast.
fn library() -> Library {
    Library {
        items: vec![
            item("spotify:collection", "Liked Songs", "liked", 0, false, &[]),
            item("spotify:playlist:diary", "Zed's Diary", "playlist", 0, true, &["Zed Quill"]),
            item("spotify:user:zed:folder:1", "Demo", "folder", 0, false, &[]),
            item("spotify:playlist:show1", "Showcase", "playlist", 1, true, &["Zed Quill"]),
            item("spotify:playlist:friend", "Road Trip", "playlist", 1, false, &["Bo Friend"]),
            item("spotify:user:zed:folder:2", "Inner", "folder", 1, false, &[]),
            item("spotify:playlist:deep", "Deep Cuts", "playlist", 2, true, &["Zed Quill"]),
            item("spotify:playlist:after", "After Hours", "playlist", 0, true, &["Zed Quill"]),
            item("spotify:album:a1", "Orbits", "album", 0, false, &["Nova Drift"]),
            item("spotify:show:pod", "Health Talk", "show", 0, false, &["Podcast · Some Publisher"]),
        ],
    }
}

fn names(l: &Library) -> Vec<&str> {
    l.items.iter().map(|i| i.name.as_str()).collect()
}

#[test]
fn the_switch_is_an_argument_or_the_environment() {
    assert!(asked(&["singularity".into(), "--demo".into()], None));
    assert!(asked(&["singularity".into()], Some("1")));
    assert!(!asked(&["singularity".into()], Some("0")));
    assert!(!asked(&["singularity".into()], None));
}

#[test]
fn the_redactor_replaces_what_it_learned_inside_longer_text() {
    let mut r = redactor();
    r.learn("zedq", "user");
    r.learn("Zed Quill", YOU);
    r.learn_device("Singularity on zedbox", ENGINE);
    r.learn_device("Zed's Phone", "Phone");
    assert_eq!(r.redact("Couldn't create the playlist: error sending request for url (https://spclient/user/ZEDQ/rootlist/changes)"), "Couldn't create the playlist: error sending request for url (https://spclient/user/user/rootlist/changes)");
    assert_eq!(r.redact("Made For Zed Quill"), "Made For you");
    assert_eq!(r.redact("\"Singularity on zedbox\" isn't on Connect"), "\"Singularity\" isn't on Connect", "the longer name first");
    assert_eq!(r.redact("Zed's Phone"), "Phone", "a device name as the whole text");
    assert_eq!(r.redact("Moving playback to 'Zed's Phone'"), "Moving playback to 'Phone'", "or quoted");
    assert_eq!(r.redact("Listening data exported to /home/zed/Downloads/x.json"), "Listening data exported to ~/Downloads/x.json");
    assert_eq!(r.redact("zedbox"), "computer");
    assert_eq!(r.redact("the socket in /run/user/zed belongs to zed"), "the socket in /run/user/user belongs to zed", "$USER only as a path's folder");
    assert_eq!(r.redact("zedboxes and zedq2 are other words"), "zedboxes and zedq2 are other words", "not inside a longer word");
}

/// Review of #141: names that are also ordinary words stay where they are words (an album, a band, a Shelf).
#[test]
fn device_computer_and_user_names_are_not_replaced_as_ordinary_words() {
    let mut r = Redactor::from_env(Some("/home/richter".into()), Some("richter".into()), Some("echo".into()));
    r.learn_device("Speaker", "Speaker 2");
    assert_eq!(r.redact("Album · Max Richter"), "Album · Max Richter");
    assert_eq!(r.redact("Echo & the Bunnymen"), "Echo & the Bunnymen");
    assert_eq!(r.redact("Speaker & the Bunnymen"), "Speaker & the Bunnymen");
    assert_eq!(r.redact("see /data/richter/log"), "see /data/user/log");
    let mut d = Demo::new(DemoFile::default(), r);
    let h = HomeFeed { fetched_unix_ms: 1, shelves: vec![shelf("", "Echo & the Bunnymen radio", "shelf", &["spotify:playlist:x"])] };
    assert_eq!(d.scrub(Event::Library(library())), d.clone().scrub(Event::Library(library())));
    assert_eq!(d.home(h).shelves.len(), 1, "not dropped as personal");
}

#[test]
fn the_redactor_ignores_names_too_short_or_their_own_stand_in() {
    let mut r = Redactor::default();
    r.learn("a", "user");
    r.learn("Singularity", ENGINE);
    assert_eq!(r.redact("a Singularity"), "a Singularity");
}

#[test]
fn the_library_is_the_demo_folder_flattened_with_neutral_owners() {
    let mut d = demo();
    let l = d.library(library());
    assert_eq!(names(&l), ["Showcase", "Road Trip", "Deep Cuts"], "the folder's playlists, nested ones too; no folders");
    assert!(l.items.iter().all(|i| i.depth == 0));
    assert_eq!(l.items[0].subtitle, "Playlist · you");
    assert_eq!(l.items[0].authors, ["you"]);
    assert_eq!(l.items[1].subtitle, "Playlist · a friend");
    assert_eq!(d.redact("Made For Zed Quill"), "Made For you", "the owner's display name is learned from own playlists");
}

#[test]
fn without_a_demo_folder_the_library_is_a_hint() {
    let mut d = demo();
    let mut l = library();
    l.items.retain(|i| i.kind != "folder" && i.depth == 0);
    let l = d.library(l);
    assert_eq!(names(&l), [HINT]);
    assert_eq!(l.items[0].kind, "hint");
}

#[test]
fn demo_toml_adds_albums_podcasts_and_liked_songs() {
    let mut d = Demo::new(DemoFile { library: vec!["spotify:album:a1".into()], liked: true, ..Default::default() }, redactor());
    assert_eq!(names(&d.library(library())), ["Liked Songs", "Showcase", "Road Trip", "Deep Cuts", "Orbits"]);
    let mut d = Demo::new(DemoFile { library: vec!["spotify:show:pod".into()], ..Default::default() }, redactor());
    assert!(!names(&d.library(library())).contains(&"Liked Songs"), "Liked Songs only with liked = true");
}

#[test]
fn a_playlist_made_in_this_session_stays() {
    let mut d = demo();
    d.scrub(Event::Library(library()));
    d.created("spotify:playlist:new");
    let mut l = library();
    l.items.insert(1, item("spotify:playlist:new", "Fresh", "playlist", 0, true, &["Zed Quill"]));
    assert_eq!(names(&d.library(l)), ["Fresh", "Showcase", "Road Trip", "Deep Cuts"]);
    assert!(d.library_again().is_some());
}

#[test]
fn demo_toml_reads_and_adds_without_losing_comments() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("demo.toml");
    assert_eq!(load(&p), (DemoFile::default(), None), "no file: an empty Demo set");
    std::fs::write(&p, "# mine\nliked = true\n[home]\nhide = [\"Late night\"]\n").unwrap();
    assert!(add_to_file(&p, "spotify:album:a1").unwrap());
    assert!(!add_to_file(&p, "spotify:album:a1").unwrap(), "already there");
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(text.starts_with("# mine"), "{text}");
    let (f, problem) = load(&p);
    assert_eq!(problem, None);
    assert_eq!(f.library, ["spotify:album:a1"]);
    assert!(f.liked);
    assert_eq!(f.home.hide, ["Late night"]);
    std::fs::write(&p, "liked = 3").unwrap();
    assert!(load(&p).1.is_some_and(|e| e.starts_with("demo.toml")));
}

fn shelf(id: &str, title: &str, kind: &str, cards: &[&str]) -> Shelf {
    Shelf {
        id: id.into(),
        title: title.into(),
        kind: kind.into(),
        cards: cards.iter().map(|u| Card { uri: u.to_string(), name: u.to_string(), subtitle: "By Zed Quill".into(), image_url: None, kind: "playlist".into() }).collect(),
    }
}

#[test]
fn home_keeps_only_editorial_shelves_without_the_owners_other_playlists() {
    let mut d = demo();
    d.scrub(Event::Library(library()));
    let h = HomeFeed {
        fetched_unix_ms: 1,
        shelves: vec![
            shelf("", "", "shortcuts", &["spotify:playlist:after"]),
            shelf("spotify:section:a", "Made For Zed Quill", "shelf", &["spotify:playlist:mix"]),
            shelf("", "Recently played", "shelf", &["spotify:playlist:x"]),
            shelf("", "Because you listened to Nova Drift", "feed", &["spotify:playlist:x"]),
            shelf("", "Your top mixes", "shelf", &["spotify:playlist:x"]),
            shelf("", "Charts", "shelf", &["spotify:playlist:top50", "spotify:playlist:diary"]),
            shelf("", "Only mine", "shelf", &["spotify:playlist:diary"]),
            shelf("spotify:section:quiet", "Late night", "shelf", &["spotify:playlist:night"]),
            shelf("", "Hi Zed Quill", "shelf", &["spotify:playlist:night"]),
        ],
    };
    let out = d.home(h.clone());
    let titles: Vec<_> = out.shelves.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles, ["Charts", "Late night"]);
    assert_eq!(out.shelves[0].cards.len(), 1, "the owner's own playlist outside the Demo set is dropped");
    assert_eq!(out.shelves[0].cards[0].subtitle, "By you");
    // demo.toml: hide by id, show by title
    d.file.home = HomeRules { hide: vec!["spotify:section:quiet".into()], show: vec!["Your top mixes".into()] };
    let titles: Vec<_> = d.home(h).shelves.iter().map(|s| s.title.clone()).collect();
    assert_eq!(titles, ["Your top mixes", "Charts"]);
}

#[test]
fn home_learns_the_owners_playlists_when_the_library_comes_later() {
    let mut d = demo();
    let h = HomeFeed { fetched_unix_ms: 1, shelves: vec![shelf("", "Charts", "shelf", &["spotify:playlist:top50", "spotify:playlist:diary"])] };
    let Event::Home(first) = d.scrub(Event::Home(h)) else { panic!() };
    assert_eq!(first.shelves[0].cards.len(), 2, "not known yet");
    d.scrub(Event::Library(library()));
    assert_eq!(d.home_again().unwrap().shelves[0].cards.len(), 1);
}

fn row(added: &str, progress: Option<Progress>) -> TrackRow {
    TrackRow { uri: "spotify:track:1".into(), name: "Song".into(), added_at: Some(added.into()), progress, ..Default::default() }
}

#[test]
fn lists_lose_their_dates_and_say_whose_they_are() {
    let mut d = demo();
    d.scrub(Event::Library(library()));
    let page = |uri: &str, sub: &str| TrackPage { uri: uri.into(), total: 1, tracks: vec![row("2024-02-03T10:00:00Z", None)], header: Some(ListHeader { subtitle: sub.into(), ..Default::default() }), ..Default::default() };
    let own = d.tracks(page("spotify:playlist:show1", "Playlist · Zed Quill · 3 likes"));
    assert_eq!(own.tracks[0].added_at, None, "the Added column is blank");
    assert_eq!(own.header.unwrap().subtitle, "Playlist · you · 3 likes");
    assert_eq!(d.tracks(page("spotify:playlist:friend", "Playlist · Bo Friend · 9 likes")).header.unwrap().subtitle, "Playlist · a friend · 9 likes");
    assert_eq!(d.tracks(page("spotify:playlist:top50", "Playlist · Spotify · 1 234 likes")).header.unwrap().subtitle, "Playlist · Spotify · 1 234 likes");
    assert_eq!(d.tracks(page("spotify:playlist:friend", "Playlist · 3 likes")).header.unwrap().subtitle, "Playlist · 3 likes", "no owner: the likes stay");
}

#[test]
fn episode_progress_shows_only_for_shows_in_the_demo_set() {
    let done = Some(Progress { position_ms: 5, done: true });
    let show = |uri: &str| TrackPage { uri: uri.into(), total: 1, tracks: vec![row("2025-01-01", done)], ..Default::default() };
    let mut d = Demo::new(DemoFile { library: vec!["spotify:show:pod".into()], ..Default::default() }, redactor());
    d.scrub(Event::Library(library()));
    let kept = d.tracks(show("spotify:show:pod"));
    assert_eq!((kept.tracks[0].progress, kept.tracks[0].added_at.as_deref()), (done, Some("2025-01-01")), "a release date is public");
    assert_eq!(d.tracks(show("spotify:show:other")).tracks[0].progress, None);
}

#[test]
fn search_drops_the_owners_other_playlists_and_progress() {
    let mut d = demo();
    d.scrub(Event::Library(library()));
    let card = |u: &str| Card { uri: u.into(), name: u.into(), subtitle: "Zed Quill".into(), image_url: None, kind: "playlist".into() };
    let r = SearchResults { query: "x".into(), top: vec![card("spotify:playlist:diary"), card("spotify:playlist:pub")], playlists: vec![card("spotify:playlist:diary"), card("spotify:playlist:show1")], episodes: vec![row("2025", Some(Progress::default()))], error: Some("failed: /home/zed/cache".into()), ..Default::default() };
    let r = d.search(r);
    assert_eq!(r.top.iter().map(|c| c.uri.as_str()).collect::<Vec<_>>(), ["spotify:playlist:pub"]);
    assert_eq!(r.playlists.iter().map(|c| c.uri.as_str()).collect::<Vec<_>>(), ["spotify:playlist:show1"]);
    assert_eq!(r.top[0].subtitle, "you");
    assert_eq!(r.episodes[0].progress, None);
    assert_eq!(r.error.as_deref(), Some("failed: ~/cache"));
}

#[test]
fn an_artist_page_drops_the_owners_other_playlists() {
    let mut d = demo();
    d.scrub(Event::Library(library()));
    let page = singularity_proto::ArtistPage { uri: "spotify:artist:a".into(), shelves: vec![shelf("", "Discovered on", "shelf", &["spotify:playlist:diary", "spotify:playlist:pub"]), shelf("", "Only mine", "shelf", &["spotify:playlist:after"])], ..Default::default() };
    let Event::Artist(p) = d.scrub(Event::Artist(page)) else { panic!() };
    assert_eq!(p.shelves.len(), 1);
    assert_eq!(p.shelves[0].cards.iter().map(|c| c.uri.as_str()).collect::<Vec<_>>(), ["spotify:playlist:pub"]);
    assert_eq!(p.shelves[0].cards[0].subtitle, "By you");
}

#[test]
fn devices_get_neutral_names_by_kind_kept_for_the_session() {
    let mut d = demo();
    let dev = |id: &str, name: &str, kind: &str, engine: bool| Device { id: id.into(), name: name.into(), kind: kind.into(), is_engine: engine, ..Default::default() };
    let list = vec![dev("e", "Singularity on zedbox", "computer", true), dev("p1", "Zed's Phone", "smartphone", false), dev("s", "Kitchen", "speaker", false), dev("p2", "Zed's Old Phone", "smartphone", false), dev("t", "Living Room", "tv", false)];
    let Event::Devices { devices } = d.scrub(Event::Devices { devices: list.clone() }) else { panic!() };
    assert_eq!(devices.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["Singularity", "Phone", "Speaker", "Phone 2", "TV"]);
    // again, in another order: the same names
    let Event::Devices { devices } = d.scrub(Event::Devices { devices: list.into_iter().rev().collect() }) else { panic!() };
    assert_eq!(devices.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["TV", "Phone 2", "Speaker", "Phone", "Singularity"]);
    assert_eq!(d.redact("Zed's Old Phone"), "Phone 2");
    assert_eq!(neutral_name("automobile", 1), "Car");
    assert_eq!(neutral_name("weird", 3), "Device 3");
}

#[test]
fn the_jam_has_a_fake_invite_as_long_as_the_real_one_and_numbered_guests() {
    let mut d = demo();
    let real = "https://open.spotify.com/socialsession/SecretToken9876";
    let jam = Jam {
        session_id: "s".into(),
        owner: true,
        host: Some("Zed's Phone".into()),
        host_is_engine: false,
        invite: Some(real.into()),
        queue_only: false,
        members: vec![
            JamMember { id: "o".into(), name: "Zed Quill".into(), owner: true, listening: true },
            JamMember { id: "g1".into(), name: "Bo Friend".into(), owner: false, listening: true },
            JamMember { id: "g2".into(), name: "Cy Other".into(), owner: false, listening: false },
        ],
    };
    d.scrub(Event::Devices { devices: vec![Device { id: "p".into(), name: "Zed's Phone".into(), kind: "smartphone".into(), ..Default::default() }] });
    let Event::Jam { jam: Some(j) } = d.scrub(Event::Jam { jam: Some(jam) }) else { panic!() };
    let invite = j.invite.unwrap();
    assert!(invite.starts_with(FAKE_INVITE) && !invite.contains("SecretToken"), "{invite}");
    assert_eq!(invite.len(), real.len(), "the same length: a QR as big");
    assert_eq!(crate::jam::qr_rows(&invite).len(), crate::jam::qr_rows(real).len());
    assert_eq!(j.host.as_deref(), Some("Phone"));
    assert_eq!(j.members.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["you", "Guest 1", "Guest 2"]);
    assert_eq!(d.redact("Bo Friend joined"), "Guest 1 joined");
    // a host that isn't one of the account's devices: someone else's, unnamed
    let other = Jam { host: Some("Cy's Laptop".into()), host_is_engine: false, ..Default::default() };
    let Event::Jam { jam: Some(j) } = d.scrub(Event::Jam { jam: Some(other) }) else { panic!() };
    assert_eq!(j.host.as_deref(), Some(FRIENDS_DEVICE));
}

#[test]
fn statuses_lose_names_paths_and_the_audio_output() {
    let mut d = demo();
    assert_eq!(d.scrub(Event::Catalogue(CatalogueStatus::Ready { username: "zedq".into(), country: "SE".into(), premium: true })), Event::Catalogue(CatalogueStatus::Ready { username: String::new(), country: "SE".into(), premium: true }));
    assert_eq!(d.scrub(Event::Engine(EngineStatus::Ready { device_name: "Zed's Desk".into(), active: true, version: "1".into(), expires_in_days: None })), Event::Engine(EngineStatus::Ready { device_name: ENGINE.into(), active: true, version: "1".into(), expires_in_days: None }));
    assert_eq!(d.scrub(Event::Engine(EngineStatus::Failed { message: "\"Zed's Desk\" failed: see /home/zed/vm/console.log".into() })), Event::Engine(EngineStatus::Failed { message: "\"Singularity\" failed: see ~/vm/console.log".into() }));
    assert_eq!(d.scrub(Event::Error { message: "Spotify login failed: zedq".into() }), Event::Error { message: "Spotify login failed: user".into() });
    let Event::Health(h) = d.scrub(Event::Health(Health { soloist_key: "/home/zed/.local/key".into(), spotify_login: "keyring".into(), audio_output: Some("Zed's AirPods".into()), ..Default::default() })) else { panic!() };
    assert_eq!((h.soloist_key.as_str(), h.audio_output.as_deref()), ("~/.local/key", Some("playing")));
}

#[test]
fn a_context_outside_the_demo_set_is_a_playlist() {
    let mut d = demo();
    d.scrub(Event::Library(library()));
    let ctx = |uri: &str| {
        let Event::Playback(p) = d.clone().scrub(Event::Playback(PlaybackState { context_uri: Some(uri.into()), context_name: Some("Real Name".into()), ..Default::default() })) else { panic!() };
        p.context_name.unwrap()
    };
    assert_eq!(ctx("spotify:playlist:diary"), SOME_PLAYLIST);
    assert_eq!(ctx("spotify:playlist:show1"), "Real Name");
    assert_eq!(ctx("spotify:album:x"), "Real Name");
    assert_eq!(ctx("spotify:user:zed:collection"), "Real Name");
}

// ── the whole Client: what's drawn ───────────────────────────────

fn render(app: &mut crate::app::App, w: u16, h: u16) -> String {
    let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
    t.draw(|f| crate::ui::frame(f, app)).unwrap();
    let buf = t.backend().buffer().clone();
    (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect::<Vec<_>>().join("\n")
}

/// The made-up personal values the account below carries: none may be drawn in demo mode.
const SECRETS: &[&str] = &["zedq", "Zed Quill", "Zed's Desk", "Zed's Phone", "Bo Friend", "SecretToken", "Zed's Diary", "Zed's AirPods"];

/// A Client that heard a made-up account with personal values, its device picker (and Jam) open.
fn heard(demo: bool) -> crate::app::App {
    let mut app = crate::app::App::new(None);
    if demo {
        app.demo = Some(Demo::new(DemoFile::default(), redactor()));
    }
    app.hear(Event::Catalogue(CatalogueStatus::Ready { username: "zedq".into(), country: "SE".into(), premium: true }));
    app.hear(Event::Engine(EngineStatus::Ready { device_name: "Zed's Desk".into(), active: true, version: "1".into(), expires_in_days: None }));
    app.hear(Event::Library(library()));
    app.hear(Event::Home(HomeFeed { fetched_unix_ms: 1, shelves: vec![shelf("", "Made For Zed Quill", "shelf", &["spotify:playlist:diary"]), shelf("", "Charts", "shelf", &["spotify:playlist:top50"])] }));
    app.hear(Event::Devices { devices: vec![Device { id: "e".into(), name: "Zed's Desk".into(), kind: "computer".into(), is_engine: true, active: true, ..Default::default() }, Device { id: "p".into(), name: "Zed's Phone".into(), kind: "smartphone".into(), ..Default::default() }] });
    app.hear(Event::Jam {
        jam: Some(Jam {
            session_id: "s".into(),
            owner: true,
            host: Some("Zed's Desk".into()),
            host_is_engine: true,
            invite: Some("https://open.spotify.com/socialsession/SecretToken9876".into()),
            members: vec![JamMember { id: "g".into(), name: "Bo Friend".into(), owner: false, listening: true }],
            ..Default::default()
        }),
    });
    app.run(crate::app::Action::Devices);
    app
}

#[test]
fn nothing_personal_is_drawn_in_demo_mode() {
    let mut app = heard(true);
    let screen = render(&mut app, 160, 48);
    for s in SECRETS {
        assert!(!screen.contains(s), "{s} is on screen:\n{screen}");
    }
    for s in [" DEMO ", "Premium", LINK_HIDDEN, "Singularity (this one)", "Phone", "Guest 1", "Charts", "Showcase"] {
        assert!(screen.contains(s), "{s} missing:\n{screen}");
    }
    assert!(screen.contains('▀') || screen.contains('▄'), "a QR still shows:\n{screen}");
    app.say("Couldn't reach zedq's rootlist from \"Zed's Phone\"");
    assert_eq!(app.toast.as_ref().unwrap().0, "Couldn't reach user's rootlist from \"Phone\"");
}

#[test]
fn without_demo_mode_everything_shows_as_before() {
    let mut app = heard(false);
    let screen = render(&mut app, 160, 48);
    for s in ["zedq · Premium", "Zed's Phone", "Bo Friend", "socialsession/SecretToken9876", "Zed's Diary"] {
        assert!(screen.contains(s), "{s} missing:\n{screen}");
    }
    assert!(!screen.contains(" DEMO "));
}

fn connected(app: &mut crate::app::App, demo: bool, account: &str) {
    app.receive(crate::conn::ConnEvent::Connected { daemon_version: "1".into(), engine: Default::default(), soloist_in_vm: false, demo, account: account.into() });
}

/// Review of #141: a Daemon too old to know demo mode would answer with the real log, and delete it: until the
/// Welcome confirms demo mode no stats show, and export and delete are refused.
#[test]
fn without_the_daemons_word_a_demo_client_shows_no_stats_and_refuses_delete() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let mut app = crate::app::App::new(Some(tx));
    app.demo = Some(demo());
    connected(&mut app, false, "");
    app.hear(Event::Listening(Default::default()));
    app.hear(Event::Stats(Default::default()));
    assert!(app.listening.is_none() && app.stats.is_empty(), "the real log's answers aren't shown");
    while rx.try_recv().is_ok() {}
    for c in [singularity_proto::Command::DeleteLocalData, singularity_proto::Command::ExportLocalData] {
        app.send(c);
        assert_eq!(app.toast.as_ref().unwrap().0, TOO_OLD);
    }
    assert!(!std::iter::from_fn(|| rx.try_recv().ok()).any(|c| matches!(c, singularity_proto::Command::DeleteLocalData | singularity_proto::Command::ExportLocalData)), "nothing was sent");
    connected(&mut app, true, "");
    app.hear(Event::Listening(Default::default()));
    assert!(app.listening.is_some());
    app.send(singularity_proto::Command::DeleteLocalData);
    assert!(std::iter::from_fn(|| rx.try_recv().ok()).any(|c| c == singularity_proto::Command::DeleteLocalData));
}

/// Review of #141: errors before the login is Ready (a failed login) already hide the saved login's name.
#[test]
fn the_saved_logins_name_is_redacted_from_the_start() {
    let mut app = crate::app::App::new(None);
    app.demo = Some(demo());
    connected(&mut app, true, "zedq");
    app.hear(Event::Error { message: "Spotify login failed: bad credentials for zedq".into() });
    assert_eq!(app.toast.as_ref().unwrap().0, "Spotify login failed: bad credentials for user");
}

/// Review of #141: Home waits for the library, so the owner's playlists outside the Demo set never flash.
#[test]
fn home_waits_for_the_library_in_demo_mode() {
    let mut app = crate::app::App::new(None);
    app.demo = Some(demo());
    app.hear(Event::Home(HomeFeed { fetched_unix_ms: 1, shelves: vec![shelf("", "Charts", "shelf", &["spotify:playlist:top50", "spotify:playlist:diary"])] }));
    assert!(app.home.is_none());
    app.hear(Event::Library(library()));
    assert_eq!(app.home.as_ref().unwrap().shelves[0].cards.len(), 1);
}

/// Review of #141: a playlist another Client makes during the demo doesn't join the Demo set.
#[test]
fn a_playlist_made_by_another_client_stays_out() {
    let mut app = crate::app::App::new(None);
    app.demo = Some(demo());
    app.hear(Event::Library(library()));
    app.hear(Event::PlaylistCreated { uri: "spotify:playlist:new".into(), name: "Elsewhere".into() });
    let mut l = library();
    l.items.insert(1, item("spotify:playlist:new", "Elsewhere", "playlist", 0, true, &["Zed Quill"]));
    app.hear(Event::Library(l));
    assert!(!app.library.as_ref().unwrap().items.iter().any(|i| i.name == "Elsewhere"));
}

/// Review of #141: the redactor is on before the config is read, which teaches it the owner's device name.
#[test]
fn the_config_is_read_with_the_redactor_on() {
    let dir = tempfile::tempdir().unwrap();
    let (cp, kp) = (dir.path().join("config.toml"), dir.path().join("keys.toml"));
    std::fs::write(&cp, "schema = 6\n[engine]\ndevice = \"Zed's Desk\"\n").unwrap();
    let mut app = crate::app::App::new(None);
    app.start_demo(dir.path().join("demo.toml"));
    app.load_config_from(cp, kp);
    assert_eq!(app.demo.as_ref().unwrap().redact("Zed's Desk"), ENGINE);
}

#[test]
fn a_login_in_demo_mode_does_not_open_the_browser() {
    let pending = Event::Catalogue(CatalogueStatus::LoginPending { url: "https://example.invalid/pair".into(), code: "ABCD".into() });
    let mut app = crate::app::App::new(None);
    app.demo = Some(demo());
    app.hear(pending.clone());
    assert_eq!(app.open_url, None);
    assert!(matches!(app.catalogue, Some(CatalogueStatus::LoginPending { .. })), "the link and code still show");
    let mut app = crate::app::App::new(None);
    app.hear(pending);
    assert!(app.open_url.is_some());
}

#[test]
fn add_to_demo_is_offered_only_in_demo_mode_and_writes_demo_toml() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = crate::app::App::new(None);
    assert_eq!(app.demo_entry("album", "spotify:album:a1"), None);
    app.start_demo(dir.path().join("demo.toml"));
    app.hear(Event::Library(library()));
    assert!(app.demo_entry("album", "spotify:album:a1").is_some());
    assert_eq!(app.demo_entry("playlist", "spotify:playlist:x"), None, "playlists come from the Demo folder");
    app.add_to_demo("spotify:album:a1", "Orbits");
    assert_eq!(load(&dir.path().join("demo.toml")).0.library, ["spotify:album:a1"]);
    assert!(app.library.as_ref().unwrap().items.iter().any(|i| i.name == "Orbits"), "in the library at once");
    assert_eq!(app.demo_entry("album", "spotify:album:a1"), None, "already in");
}
