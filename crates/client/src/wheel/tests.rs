//! Every Widget and every page of the main Pane, holding more than fits in a small Pane, under the wheel.

use super::Widget;
use crate::app::{Action, App, Hit, Link, Page};
use crate::layout::{Axis, Node, Size};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Cell;
use ratatui::crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use singularity_proto::{Card, HomeFeed, Item, LibraryItem, LyricLine, Lyrics, PlayStatus, QueueEntry, SearchResults, Shelf, StatsPeriod, StatsRow, TrackPage, TrackRow};

/// The screen: the smallest that isn't the mini-player, give or take, so every Pane is short.
const W: u16 = 100;
const H: u16 = 22;

fn app() -> App {
    let (tx, rx) = tokio::sync::mpsc::channel(4096);
    std::mem::forget(rx);
    let mut app = App::new(Some(tx));
    app.link = Link::Connected { version: "test".into() };
    app.playback.item = Some(Item { uri: "spotify:track:s".into(), name: "Song".into(), duration_ms: 200_000, ..Default::default() });
    // paused: nothing that follows the song moves by itself between frames
    app.playback.status = PlayStatus::Paused;
    app.playback.position_ms = 61_000;
    app.playback.timestamp_ms = singularity_proto::now_ms();
    app
}

fn render(app: &mut App) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(W, H)).unwrap();
    t.draw(|f| crate::ui::frame(f, app)).unwrap();
    t.backend().buffer().clone()
}

type Buffer = ratatui::buffer::Buffer;

fn text(buf: &Buffer, r: Rect) -> String {
    (r.y..r.bottom()).map(|y| (r.x..r.right()).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect::<Vec<_>>().join("\n")
}

fn cells(buf: &Buffer, r: Rect) -> Vec<Cell> {
    (r.y..r.bottom()).flat_map(|y| (r.x..r.right()).map(move |x| (x, y))).map(|(x, y)| buf[(x, y)].clone()).collect()
}

/// The rectangle of the Pane showing `w`, where the Screen plan has it now.
fn pane(app: &App, w: Widget) -> Rect {
    app.layout.pane_of(w).and_then(|id| app.plan().pane(id).map(|p| p.rect)).unwrap_or_else(|| panic!("no Pane shows {w}"))
}

fn wheel(app: &mut App, kind: MouseEventKind, x: u16, y: u16, notches: usize) {
    for _ in 0..notches {
        app.pointer(MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE }, x as f64 + 0.5, false);
    }
}

fn row(i: usize, kind: &str) -> TrackRow {
    TrackRow { uri: format!("spotify:{kind}:r{i}"), name: format!("Row number {i}"), artists: vec!["Nova Drift".into()], album: "Album".into(), duration_ms: 180_000, playable: true, ..Default::default() }
}

fn card(i: usize, kind: &str) -> Card {
    Card { uri: format!("spotify:{kind}:c{i}"), name: format!("Card {i}"), subtitle: "Nova Drift".into(), image_url: None, kind: kind.into() }
}

fn lyrics(app: &mut App, synced: bool) {
    let lines = (0..120u64).map(|i| LyricLine { start_ms: 1_000 + i * 3_000, text: format!("Line number {i} of the song"), ..Default::default() }).collect();
    app.lyrics_event(Lyrics { uri: "spotify:track:s".into(), synced, lines, source: "LRCLIB".into(), ..Default::default() });
}

fn stats(period: StatsPeriod) -> singularity_proto::Stats {
    let r = |kind: &str, i: usize| StatsRow { name: format!("{kind} {i}"), uri: Some(format!("spotify:{kind}:{i}")), track_uri: format!("spotify:track:t{i}"), subtitle: "Nova Drift".into(), minutes: 300 - i as u32 * 5, plays: 40, all_time_plays: 90 };
    singularity_proto::Stats {
        period,
        minutes: 1234,
        plays: 321,
        by_hour: vec![10; 24],
        top_artists: (0..40).map(|i| r("artist", i)).collect(),
        top_tracks: (0..40).map(|i| r("track", i)).collect(),
        top_albums: (0..40).map(|i| r("album", i)).collect(),
        ..Default::default()
    }
}

/// One thing to wheel: what it is, how to set it up, and how many notches in a row each move it (0: nothing to
/// scroll there).
struct Case {
    what: &'static str,
    set: fn(&mut App),
    moves: usize,
}

/// What each Widget holds that doesn't fit, as cases. No catch-all arm: a new Widget needs its cases here.
fn cases(w: Widget) -> Vec<Case> {
    let still = |what| vec![Case { what, set: |_| {}, moves: 0 }];
    match w {
        // the main Pane's pages have cases of their own (`pages`)
        Widget::Home => vec![],
        Widget::Library => vec![Case {
            what: "a long library",
            set: |app| {
                let item = |n: usize| LibraryItem { uri: format!("spotify:playlist:p{n}"), name: format!("Playlist {n}"), subtitle: "Playlist".into(), kind: "playlist".into(), image_url: None, depth: 0, own: false, authors: vec![] };
                app.hear(singularity_proto::Event::Library(singularity_proto::Library { items: (0..60).map(item).collect() }));
            },
            moves: 20,
        }],
        Widget::Queue => vec![Case {
            what: "a long queue",
            set: |app| {
                app.playback.queue = (0..40).map(|i| QueueEntry { item: Item { uri: format!("spotify:track:q{i}"), name: format!("Queued {i}"), duration_ms: 150_000, ..Default::default() }, source: "queue".into(), uid: format!("u{i}") }).collect();
            },
            moves: 20,
        }],
        Widget::Lyrics => vec![Case { what: "synced lyrics", set: |app| lyrics(app, true), moves: 12 }, Case { what: "unsynced lyrics", set: |app| lyrics(app, false), moves: 12 }],
        Widget::Karaoke => vec![Case { what: "karaoke, synced: the line being sung", set: |app| lyrics(app, true), moves: 0 }, Case { what: "karaoke, unsynced: the whole text", set: |app| lyrics(app, false), moves: 12 }],
        Widget::TopWeek => vec![
            Case {
                what: "top this week (tracks)",
                set: |app| {
                    app.stats.insert(StatsPeriod::Week, stats(StatsPeriod::Week));
                    app.top_week_tracks = true;
                },
                moves: 20,
            },
            Case { what: "top this week (artists)", set: |app| drop(app.stats.insert(StatsPeriod::Week, stats(StatsPeriod::Week))), moves: 20 },
        ],
        Widget::Stream => still("stream"),
        Widget::NowPlaying => still("now playing"),
        Widget::Top => still("top"),
        Widget::Listening => still("listening"),
        Widget::Today => still("today"),
        Widget::Marquee => still("marquee"),
        Widget::Glow => still("glow"),
        Widget::Fft => still("fft"),
        Widget::Loudness => still("loudness"),
        Widget::Levels => still("levels"),
        Widget::Scope => still("scope"),
        Widget::Rain => still("rain"),
        Widget::Starfield => still("starfield"),
        Widget::Well => still("well"),
        Widget::Hole => still("hole"),
        Widget::Unknown(_) => vec![],
    }
}

/// Which `Page` a case shows, as a number: no catch-all arm, so a new page needs cases in `pages`.
fn page_index(p: &Page) -> usize {
    match p {
        Page::Home => 0,
        Page::List { .. } => 1,
        Page::Artist(_) => 2,
        Page::Search => 3,
        Page::Stats(_) => 4,
        Page::Settings(_) => 5,
    }
}
const PAGES: usize = 6;

fn search(app: &mut App, chip: usize) {
    app.run(Action::Search);
    let s = app.searching_mut().unwrap();
    s.type_text("orbit");
    let cards = |kind: &str| (0..30).map(|i| card(i, kind)).collect::<Vec<_>>();
    s.receive(SearchResults {
        query: "orbit".into(),
        top: cards("album"),
        tracks: (0..30).map(|i| row(i, "track")).collect(),
        artists: cards("artist"),
        albums: cards("album"),
        playlists: cards("playlist"),
        podcasts: cards("show"),
        episodes: (0..30).map(|i| row(i, "episode")).collect(),
        audiobooks: cards("show"),
        ..Default::default()
    });
    s.set_chip(chip);
}

fn list(app: &mut App, uri: &str, header: Option<singularity_proto::ListHeader>) {
    app.open(uri, "A long list", "", "");
    let kind = if uri.contains(":show:") { "episode" } else { "track" };
    app.track_page(TrackPage { uri: uri.into(), offset: 0, total: 200, tracks: (0..200).map(|i| row(i, kind)).collect(), header });
}

/// Every page of the main Pane, holding more than fits.
fn pages() -> Vec<Case> {
    vec![
        Case {
            what: "Home's Shelves",
            set: |app| {
                let shelf = |s: usize| Shelf { id: String::new(), title: format!("Shelf {s}"), kind: "shelf".into(), cards: (0..12).map(|i| card(i, "album")).collect() };
                app.hear(singularity_proto::Event::Home(HomeFeed { fetched_unix_ms: 0, shelves: (0..8).map(shelf).collect() }));
            },
            moves: 20,
        },
        Case { what: "a playlist", set: |app| list(app, "spotify:playlist:p", None), moves: 20 },
        Case { what: "Liked Songs", set: |app| list(app, "spotify:collection", None), moves: 20 },
        Case { what: "a Show page", set: |app| list(app, "spotify:show:s", Some(Default::default())), moves: 20 },
        Case { what: "a Book page", set: |app| list(app, "spotify:show:b", Some(singularity_proto::ListHeader { book: true, ..Default::default() })), moves: 20 },
        Case {
            what: "an artist page",
            set: |app| {
                app.open("spotify:artist:a", "Nova Drift", "", "artist");
                let shelf = |s: usize| Shelf { id: String::new(), title: format!("Releases {s}"), kind: "shelf".into(), cards: (0..12).map(|i| card(i, "album")).collect() };
                let page = singularity_proto::ArtistPage { uri: "spotify:artist:a".into(), name: "Nova Drift".into(), top_tracks: (0..10).map(|i| row(i, "track")).collect(), shelves: (0..8).map(shelf).collect(), ..Default::default() };
                app.page.artist_mut().unwrap().set_page(page);
            },
            // ten popular tracks: nine notches from the first to the last
            moves: 9,
        },
        Case {
            what: "an artist page without Shelves",
            set: |app| {
                app.open("spotify:artist:a", "Nova Drift", "", "artist");
                let page = singularity_proto::ArtistPage { uri: "spotify:artist:a".into(), name: "Nova Drift".into(), top_tracks: (0..10).map(|i| row(i, "track")).collect(), ..Default::default() };
                app.page.artist_mut().unwrap().set_page(page);
            },
            moves: 9,
        },
        Case {
            what: "search: recent searches",
            set: |app| {
                app.search = crate::search::Search::with_recent((0..12).map(|i| format!("query {i}")).collect());
                app.run(Action::Search);
            },
            moves: 11,
        },
        // five songs above the Shelves
        Case { what: "search: All", set: |app| search(app, 0), moves: 4 },
        Case { what: "search: Songs", set: |app| search(app, 1), moves: 20 },
        Case { what: "search: Artists", set: |app| search(app, 2), moves: 20 },
        Case { what: "search: Albums", set: |app| search(app, 3), moves: 20 },
        Case { what: "search: Playlists", set: |app| search(app, 4), moves: 20 },
        Case { what: "search: Podcasts", set: |app| search(app, 5), moves: 20 },
        Case { what: "search: Audiobooks", set: |app| search(app, 6), moves: 20 },
        Case {
            what: "the Stats page",
            set: |app| {
                app.run(Action::Stats);
                let p = app.page.stats().unwrap().period;
                app.stats.insert(p, stats(p));
            },
            moves: 20,
        },
        // from the first setting to the last
        Case { what: "the Settings page", set: |app| app.run(Action::Settings), moves: singularity_config::SETTINGS.len() - 1 },
    ]
}

/// The layout for `slot`: it beside Home (or the library beside Home), each half the screen.
fn layout(slot: Widget) -> Node {
    let other = if slot == Widget::Home { Widget::Library } else { Widget::Home };
    Node::split(Axis::Horizontal, Size::Ratio(0.5), Node::pane(other), Node::pane(slot))
}

/// Wheel `case` in the Pane showing `slot` at a few places; returns what went wrong.
fn check(slot: Widget, case: &Case) -> Vec<String> {
    let mut bad = vec![];
    let other = if slot == Widget::Home { Widget::Library } else { Widget::Home };
    // the top of the body, the middle, the bottom (the main Pane's pages have songs on top and Shelves below)
    for at in 0..3 {
        let mut app = app();
        app.layout = layout(slot).into();
        (case.set)(&mut app);
        app.focus_on(other);
        render(&mut app);
        let r = pane(&app, slot);
        let inner = Rect::new(r.x + 1, r.y + 1, r.width.saturating_sub(2), r.height.saturating_sub(2));
        let y = [inner.y + 4, inner.y + inner.height / 2, inner.bottom().saturating_sub(2)][at];
        let x = inner.x + inner.width / 3;
        let what = format!("{} in {slot} ({}x{} Pane), wheeled at row {}", case.what, r.width, r.height, y - inner.y);
        // the first notch focuses the Pane, as a click does (so a highlight shows)
        wheel(&mut app, MouseEventKind::ScrollDown, x, y, 1);
        if case.moves == 0 {
            render(&mut app);
            continue;
        }
        if !app.is_focused(slot) {
            bad.push(format!("{what}: the wheel left the focus on {:?}", app.focused()));
        }
        // every notch moves it, all the way (a highlight that leaves the window, or a window that stops short, doesn't)
        let mut last = render(&mut app);
        for n in 2..=case.moves {
            wheel(&mut app, MouseEventKind::ScrollDown, x, y, 1);
            let now = render(&mut app);
            if cells(&now, inner) == cells(&last, inner) {
                bad.push(format!("{what}: notch {n} down moved nothing\n{}", text(&now, inner)));
                break;
            }
            last = now;
        }
        wheel(&mut app, MouseEventKind::ScrollUp, x, y, 1);
        let up = render(&mut app);
        if cells(&up, inner) == cells(&last, inner) {
            bad.push(format!("{what}: a notch up moved nothing\n{}", text(&up, inner)));
        }
    }
    bad
}

/// The wheel over every Widget (`Widget::ALL`) and every page of the main Pane, each holding more than fits in a
/// short Pane, wherever the pointer is in it: what scrolls moves (and focuses its Pane), and comes back up.
#[test]
fn the_wheel_scrolls_every_widget_and_page_that_holds_more_than_fits() {
    let mut bad = vec![];
    for &w in Widget::ALL {
        for case in cases(w) {
            bad.extend(check(w, &case));
        }
    }
    let mut seen = [false; PAGES];
    for case in pages() {
        let mut app = app();
        (case.set)(&mut app);
        seen[page_index(&app.page)] = true;
        bad.extend(check(Widget::Home, &case));
    }
    assert!(seen.iter().all(|s| *s), "every page has a case: {seen:?}");
    assert!(bad.is_empty(), "{}", bad.join("\n\n"));
}

/// A big queue (#59: each song on two rows beside its cover): the pointer finds the song under either of its rows
/// (hover, click, right-click), and the wheel moves its highlight one song a notch, the list following, all the way.
#[test]
fn a_big_queues_two_row_songs_take_the_pointer_and_the_wheel_song_by_song() {
    use crate::app::{Popup, Target};
    use ratatui::crossterm::event::MouseButton;
    let (w, h) = (200, 48);
    let draw = |app: &mut App| {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| crate::ui::frame(f, app)).unwrap();
        t.backend().buffer().clone()
    };
    let mut app = app();
    app.layout = layout(Widget::Queue).into();
    app.playback.queue = (0..40).map(|i| QueueEntry { item: Item { uri: format!("spotify:track:q{i}"), name: format!("Queued {i}"), artists: vec!["Nova Drift".into()], duration_ms: 150_000, ..Default::default() }, source: "queue".into(), uid: format!("u{i}") }).collect();
    app.focus_on(Widget::Home);
    draw(&mut app);
    let r = pane(&app, Widget::Queue);
    assert!(crate::queue::is_big(Rect::new(r.x + 1, r.y + 1, r.width - 2, r.height - 2)), "{r:?}");
    let row = |app: &App, i: usize| app.hit_rect(&Hit::QueueRow(i)).unwrap_or_else(|| panic!("song {i} isn't drawn"));
    // hover: both rows of each song shown are that song
    for i in 0..5 {
        let rr = row(&app, i);
        assert_eq!(rr.height, 2);
        for y in [rr.y, rr.y + 1] {
            assert_eq!(app.hit_at(rr.x + rr.width / 2, y), Some(Hit::QueueRow(i)), "song {i}, row {y}");
        }
    }
    // a click on a song's second row highlights that song; a right-click there opens its menu
    let rr = row(&app, 3);
    let press = |app: &mut App, b: MouseButton, x: u16, y: u16| {
        app.pointer(MouseEvent { kind: MouseEventKind::Down(b), column: x, row: y, modifiers: KeyModifiers::NONE }, x as f64 + 0.5, false);
        app.pointer(MouseEvent { kind: MouseEventKind::Up(b), column: x, row: y, modifiers: KeyModifiers::NONE }, x as f64 + 0.5, false);
    };
    press(&mut app, MouseButton::Left, rr.x + 20, rr.y + 1);
    assert_eq!((app.queue_view.sel, app.focused()), (3, Some(Widget::Queue)));
    draw(&mut app);
    let rr = row(&app, 2);
    press(&mut app, MouseButton::Right, rr.x + 20, rr.y + 1);
    match app.popup.as_ref() {
        Some(Popup::Menu(m)) => assert!(matches!(&m.target, Target::Queued { name, .. } if name == "Queued 2"), "{:?}", m.target),
        _ => panic!("no menu"),
    }
    app.popup = None;
    // the wheel: one song a notch, the highlighted song always drawn, the list moving once it has to
    app.queue_view.sel = 0;
    let mut last = draw(&mut app);
    let (x, y) = (r.x + r.width / 3, r.y + r.height / 2);
    for n in 1..40 {
        wheel(&mut app, MouseEventKind::ScrollDown, x, y, 1);
        let now = draw(&mut app);
        assert_eq!(app.queue_view.sel, n);
        assert!(app.hits.iter().any(|(_, h)| *h == Hit::QueueRow(n)), "notch {n}: the highlighted song isn't drawn");
        assert!(cells(&now, r) != cells(&last, r), "notch {n} moved nothing");
        last = now;
    }
    assert_eq!(app.queue_view.top + app.queue_view.visible, 41, "at the end, the last song at the bottom");
    wheel(&mut app, MouseEventKind::ScrollUp, x, y, 1);
    assert!(cells(&draw(&mut app), r) != cells(&last, r), "a notch up moved nothing");
    assert_eq!(app.queue_view.sel, 38);
}

/// Every Widget's name in a saved Design reads back as that Widget.
#[test]
fn every_widget_name_reads_back() {
    for &w in Widget::ALL {
        assert_eq!(Widget::of(w.name()), Some(w));
    }
}

/// Search in the smallest Homes (owner, after #55; review of #79): the songs and the Shelves take turns under the chips.
/// Notch by notch the wheel goes down the songs, on into the Shelves (several of them) to their end, and all the way
/// back up to the first song; every notch moves something, and only one highlight is ever live.
#[test]
fn a_short_homes_search_wheels_through_the_songs_and_every_shelf_and_back() {
    let (sw, sh) = crate::needs::smallest(Widget::Home);
    for (w, h) in [(sw, sh), (40, sh), (40, sh + 1)] {
        let mut app = app();
        app.layout = Node::split(Axis::Vertical, Size::Cells(h), Node::split(Axis::Horizontal, Size::Cells(w), Node::pane(Widget::Home), Node::pane(Widget::Library)), Node::pane(Widget::Queue)).into();
        search(&mut app, 0);
        // songs whose names still tell them apart in a 12-column row
        let s = app.searching_mut().unwrap();
        let cards = |kind: &str| (0..30).map(|i| card(i, kind)).collect::<Vec<_>>();
        let tracks = (0..30).map(|i| TrackRow { name: format!("S{i}"), ..row(i, "track") }).collect();
        s.receive(SearchResults { query: "orbit".into(), top: cards("album"), tracks, artists: cards("artist"), albums: cards("album"), playlists: cards("playlist"), ..Default::default() });
        render(&mut app);
        let r = pane(&app, Widget::Home);
        let inner = Rect::new(r.x + 1, r.y + 1, r.width - 2, r.height - 2);
        // over the first result row
        let (x, y) = (inner.x + 3, inner.y + 2);
        let what = format!("{w}x{h} Home");
        let one = |app: &App| app.searching().is_some_and(|s| !(s.track_sel.is_some() && s.shelves.sel.is_some()));
        let songs = app.searching().unwrap().songs();
        let mut last = render(&mut app);
        let (mut down, mut shelves_seen) = (0, false);
        loop {
            wheel(&mut app, MouseEventKind::ScrollDown, x, y, 1);
            let now = render(&mut app);
            assert!(one(&app), "{what}: two highlights after notch {}", down + 1);
            if cells(&now, inner) == cells(&last, inner) {
                break;
            }
            shelves_seen |= app.searching().unwrap().track_sel.is_none();
            down += 1;
            last = now;
            assert!(down < 200, "{what}: the wheel never stops");
        }
        // every song, then the Shelves (more than a notch of them: several Shelves of Cards under All)
        assert!(shelves_seen && down > songs + 2, "{what}: {down} notches down\n{}", text(&last, inner));
        let mut up = 0;
        while app.searching().unwrap().track_sel != Some(0) {
            wheel(&mut app, MouseEventKind::ScrollUp, x, y, 1);
            let now = render(&mut app);
            assert!(one(&app), "{what}: two highlights after notch {} up", up + 1);
            assert!(cells(&now, inner) != cells(&last, inner), "{what}: notch {} up moved nothing\n{}", up + 1, text(&now, inner));
            last = now;
            up += 1;
            assert!(up < 200, "{what}: the wheel never gets back to the first song");
        }
    }
}
