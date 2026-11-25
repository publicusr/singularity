//! The Gesture: one press or drag at a time, and a release the Client never heard ends any of them (D89, Q91).

use crate::app::{App, Hit, Link};
use crate::scrub::Bar;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use singularity_proto::{Command, Item, Library, LibraryItem, QueueEntry, TrackPage, TrackRow};
use tokio::sync::mpsc::Receiver;

const W: u16 = 140;
const H: u16 = 40;

fn render(app: &mut App) {
    let mut t = Terminal::new(TestBackend::new(W, H)).unwrap();
    t.draw(|f| crate::ui::frame(f, app)).unwrap();
}

fn mouse(app: &mut App, kind: MouseEventKind, x: u16, y: u16) {
    app.mouse(MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE });
}

/// What the Daemon was sent (not the page fetches drawing asks for).
fn sent(rx: &mut Receiver<Command>) -> Vec<Command> {
    std::iter::from_fn(|| rx.try_recv().ok()).filter(|c| !matches!(c, Command::GetTracks { .. } | Command::CheckSaved { .. })).collect()
}

/// Whether a press or drag is still held.
fn held(a: &App) -> bool {
    a.gesture.is_some()
}

fn hit(a: &App, h: Hit) -> ratatui::layout::Rect {
    a.hit_rect(&h).unwrap_or_else(|| panic!("{h:?} isn't drawn"))
}

/// An App with a song playing (paused) and a queue of 12, linked to a Daemon.
fn app() -> (App, Receiver<Command>) {
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    let mut a = App::new(Some(tx));
    a.link = Link::Connected { version: "test".into() };
    a.playback.item = Some(Item { uri: "spotify:track:now".into(), name: "Song".into(), duration_ms: 240_000, ..Default::default() });
    a.playback.queue = (0..12)
        .map(|i| QueueEntry { item: Item { uri: format!("spotify:track:{i}"), name: format!("Song {i}"), ..Default::default() }, source: "queue".into(), uid: format!("q{i}") })
        .collect();
    render(&mut a);
    (a, rx)
}

/// A seek-bar drag whose release went elsewhere (another window): the next bare move lets go, seeking where the bar
/// was let go of, as a release there would.
#[test]
fn a_seek_drag_whose_release_was_missed_lets_go() {
    let (mut a, mut rx) = app();
    let bar = a.all_hits().iter().find_map(|(r, h)| matches!(h, Hit::Bar(Bar::Seek, _)).then_some(*r)).expect("the seek bar");
    let x = bar.x + 10;
    mouse(&mut a, MouseEventKind::Down(MouseButton::Left), x, bar.y);
    mouse(&mut a, MouseEventKind::Drag(MouseButton::Left), x + 5, bar.y);
    assert!(held(&a) && sent(&mut rx).is_empty(), "held, nothing sent yet");
    mouse(&mut a, MouseEventKind::Moved, x + 20, bar.y - 5);
    assert!(!held(&a), "a bare move: the release was missed");
    assert!(matches!(sent(&mut rx)[..], [Command::Seek { .. }]), "let go where it was");
}

/// A queued song dragged whose release was missed: the next bare move lets go, and the song stays where it was (as a
/// held Widget goes back, D84).
#[test]
fn a_queue_drag_whose_release_was_missed_lets_go() {
    let (mut a, mut rx) = app();
    let (r1, r4) = (hit(&a, Hit::QueueRow(1)), hit(&a, Hit::QueueRow(4)));
    mouse(&mut a, MouseEventKind::Down(MouseButton::Left), r1.x + 2, r1.y);
    mouse(&mut a, MouseEventKind::Drag(MouseButton::Left), r4.x + 2, r4.y);
    assert!(held(&a));
    mouse(&mut a, MouseEventKind::Moved, r4.x + 2, r4.y + 1);
    assert!(!held(&a), "a bare move: the release was missed");
    assert!(sent(&mut rx).is_empty(), "nothing moved");
    assert_eq!(a.playback.queue[1].uid, "q1");
    // and a fresh press after it is a press of its own
    mouse(&mut a, MouseEventKind::Down(MouseButton::Left), r4.x + 2, r4.y);
    mouse(&mut a, MouseEventKind::Up(MouseButton::Left), r4.x + 2, r4.y);
    assert!(!held(&a) && sent(&mut rx).is_empty());
}

/// A row of your own playlist dragged whose release was missed: the next bare move lets go, and nothing moved.
#[test]
fn a_list_row_drag_whose_release_was_missed_lets_go() {
    let (mut a, mut rx) = app();
    let own = |uri: &str, name: &str| LibraryItem { uri: uri.into(), name: name.into(), subtitle: String::new(), kind: "playlist".into(), image_url: None, depth: 0, own: true, authors: vec![] };
    a.hear(singularity_proto::Event::Library(Library { items: vec![own("spotify:playlist:mine", "Mine")] }));
    a.open("spotify:playlist:mine", "Mine", "", "playlist");
    let row = |n: usize| TrackRow { uri: format!("t{n}"), name: format!("Song {n}"), uid: Some(format!("u{n}")), playable: true, ..Default::default() };
    a.track_page(TrackPage { uri: "spotify:playlist:mine".into(), offset: 0, total: 5, tracks: (0..5).map(row).collect(), header: None });
    render(&mut a);
    sent(&mut rx);
    let (r0, r3) = (hit(&a, Hit::Track(0)), hit(&a, Hit::Track(3)));
    mouse(&mut a, MouseEventKind::Down(MouseButton::Left), r0.x + 2, r0.y);
    mouse(&mut a, MouseEventKind::Drag(MouseButton::Left), r3.x + 2, r3.y);
    assert!(held(&a));
    mouse(&mut a, MouseEventKind::Moved, r3.x + 2, r3.y + 1);
    assert!(!held(&a), "a bare move: the release was missed");
    assert!(sent(&mut rx).is_empty(), "nothing moved");
    let uids: Vec<_> = a.open.list("spotify:playlist:mine").unwrap().rows().iter().map(|r| r.uid.clone().unwrap()).collect();
    assert_eq!(uids, ["u0", "u1", "u2", "u3", "u4"]);
}

fn seek_bar(a: &App) -> ratatui::layout::Rect {
    a.all_hits().iter().find_map(|(r, h)| matches!(h, Hit::Bar(Bar::Seek, _)).then_some(*r)).expect("the seek bar")
}

fn key(a: &mut App, code: ratatui::crossterm::event::KeyCode, m: KeyModifiers) {
    a.key(ratatui::crossterm::event::KeyEvent::new(code, m));
}

/// In Arrange mode, Enter while the seek bar is dragged picks nothing up (keys wait while something is held, D84):
/// the release seeks as ever.
#[test]
fn enter_while_seeking_in_arrange_mode_picks_nothing_up() {
    use ratatui::crossterm::event::KeyCode;
    let (mut a, mut rx) = app();
    key(&mut a, KeyCode::Char('m'), KeyModifiers::NONE);
    render(&mut a);
    assert!(a.arrange.is_on());
    let bar = seek_bar(&a);
    mouse(&mut a, MouseEventKind::Down(MouseButton::Left), bar.x + 10, bar.y);
    mouse(&mut a, MouseEventKind::Drag(MouseButton::Left), bar.x + 20, bar.y);
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(a.gesture.as_ref().is_some_and(|g| matches!(g, crate::gesture::Gesture::Scrub(_))), "still the seek bar's");
    mouse(&mut a, MouseEventKind::Up(MouseButton::Left), bar.x + 20, bar.y);
    assert!(!held(&a));
    assert!(matches!(sent(&mut rx)[..], [Command::Seek { .. }]), "the release seeks");
}

/// A seek-bar drag under a popup (Ctrl+K opened mid-drag) whose release was missed ends unsent, as a release heard
/// there does.
#[test]
fn a_seek_drag_under_a_popup_whose_release_was_missed_ends_unsent() {
    use ratatui::crossterm::event::KeyCode;
    let (mut a, mut rx) = app();
    let bar = seek_bar(&a);
    mouse(&mut a, MouseEventKind::Down(MouseButton::Left), bar.x + 10, bar.y);
    mouse(&mut a, MouseEventKind::Drag(MouseButton::Left), bar.x + 15, bar.y);
    key(&mut a, KeyCode::Char('k'), KeyModifiers::CONTROL);
    assert!(a.is_open(crate::app::Modal::Palette));
    mouse(&mut a, MouseEventKind::Moved, bar.x + 30, bar.y - 4);
    assert!(!held(&a));
    assert!(sent(&mut rx).iter().all(|c| !matches!(c, Command::Seek { .. })), "no seek");
}

/// A double click is known by what was clicked: the same thing twice soon after is one, the same place in another
/// list isn't (the old shared index kept them apart with offsets), and a third click starts again.
#[test]
fn a_double_click_is_the_same_thing_clicked_twice() {
    use crate::gesture::Clicked;
    let (mut a, _rx) = app();
    assert!(!a.double_click(Clicked::StatsRow(2)));
    assert!(!a.double_click(Clicked::TopWeek(2)), "another list's row 2");
    assert!(!a.double_click(Clicked::SearchTrack(2)));
    assert!(a.double_click(Clicked::SearchTrack(2)), "the same search track again");
    assert!(!a.double_click(Clicked::SearchTrack(2)), "a third click is a first one");
    let id = |s: &str| crate::openlist::RowId::of(&TrackRow { uri: s.into(), ..Default::default() });
    assert!(!a.double_click(Clicked::Row(id("t1"), 0)));
    assert!(!a.double_click(Clicked::Row(id("t2"), 0)), "another song now in that place (D81)");
    assert!(a.double_click(Clicked::Row(id("t2"), 0)));
    assert!(!a.double_click(Clicked::TopTrack(0)));
    a.last_click = a.last_click.take().map(|(t, c)| (t - crate::app::DOUBLE_CLICK, c));
    assert!(!a.double_click(Clicked::TopTrack(0)), "too slow");
}
