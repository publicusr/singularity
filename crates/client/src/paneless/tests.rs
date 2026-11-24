//! The library and the queue without a Pane, driven through the App with real keys and clicks (D87).

use super::*;
use crate::app::{Action, Hit, Page, Target};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use singularity_proto::Command;
use tokio::sync::mpsc::Receiver;

const W: u16 = 160;
const H: u16 = 48;

fn render(app: &mut App) -> String {
    let mut t = Terminal::new(TestBackend::new(W, H)).unwrap();
    t.draw(|f| crate::ui::draw(f, app)).unwrap();
    let buf = t.backend().buffer().clone();
    (0..H).map(|y| (0..W).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect::<Vec<_>>().join("\n")
}

fn mouse(app: &mut App, kind: MouseEventKind, x: u16, y: u16) {
    app.mouse(MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE });
}
fn click(app: &mut App, x: u16, y: u16) {
    mouse(app, MouseEventKind::Down(MouseButton::Left), x, y);
    mouse(app, MouseEventKind::Up(MouseButton::Left), x, y);
}
fn key(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}
fn typed(app: &mut App, s: &str) {
    s.chars().for_each(|c| key(app, KeyCode::Char(c)));
}
/// Ctrl+K, `what`, Enter: the palette's first match runs.
fn palette(app: &mut App, what: &str) {
    app.run(Action::Palette);
    typed(app, what);
    key(app, KeyCode::Enter);
}
fn sent(rx: &mut Receiver<Command>) -> Vec<Command> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

/// The Singularity design, drawn, with a library and four queued songs.
fn app() -> (App, Receiver<Command>) {
    use singularity_proto::{Item, Library, LibraryItem, QueueEntry};
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    let mut a = App::new(Some(tx));
    a.link = crate::app::Link::Connected { version: "test".into() };
    let item = |uri: &str, name: &str, kind: &str| LibraryItem { uri: uri.into(), name: name.into(), subtitle: String::new(), kind: kind.into(), image_url: None, depth: 0, own: false };
    a.hear(singularity_proto::Event::Library(Library { items: vec![item("spotify:collection", "Liked Songs", "liked"), item("spotify:playlist:p", "Night Drive", "playlist"), item("spotify:folder:f", "Moods", "folder")] }));
    a.playback.queue = (0..4).map(|i| QueueEntry { item: Item { uri: format!("spotify:track:q{i}"), name: format!("Song {i}"), duration_ms: 150_000, ..Default::default() }, source: "queue".into(), uid: format!("u{i}") }).collect();
    render(&mut a);
    (a, rx)
}

/// Close the Pane showing `w` with its ×, as a user does (and keep the edit unnamed, as "(edited)").
fn without(a: &mut App, w: Widget) {
    let id = a.layout.pane_of(w).unwrap();
    let r = a.hit_rect(&Hit::Close(id)).unwrap_or_else(|| panic!("{w} has no ×"));
    click(a, r.x + 1, r.y);
    if a.naming().is_some() {
        key(a, KeyCode::Esc);
    }
    assert!(!a.layout.has(w) && a.modal().is_none(), "{w} closed");
    render(a);
}

fn row(a: &App, h: Hit) -> (u16, u16) {
    let r = a.hit_rect(&h).unwrap_or_else(|| panic!("no {h:?} drawn"));
    (r.x + 2, r.y)
}

fn page_title(a: &App) -> Option<String> {
    match a.current_page() {
        Page::List { title, .. } => Some(title),
        _ => None,
    }
}

#[test]
fn without_a_library_pane_your_library_opens_it_as_a_popup() {
    let (mut a, _rx) = app();
    without(&mut a, Widget::Library);
    palette(&mut a, "your library");
    assert_eq!(a.modal(), Some(Modal::Library));
    let shot = render(&mut a);
    assert!(shot.contains("Liked Songs") && shot.contains("Night Drive") && shot.contains("Esc close"), "{shot}");
    // a click on a playlist opens it in Home's Pane, and the popup has done its job
    let (x, y) = row(&a, Hit::Side(crate::sidebar::FIXED + 1));
    click(&mut a, x, y);
    assert_eq!(a.modal(), None);
    assert_eq!(page_title(&a).as_deref(), Some("Night Drive"));
    assert_eq!(a.focused(), Some(Widget::Home));
    // by keys: ↑ to Liked Songs, Enter
    palette(&mut a, "your library");
    render(&mut a);
    key(&mut a, KeyCode::Up);
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.modal(), None);
    assert_eq!(page_title(&a).as_deref(), Some("Liked Songs"));
    // a folder only says why it doesn't open; the popup stays
    palette(&mut a, "your library");
    render(&mut a);
    for _ in 0..5 {
        mouse(&mut a, MouseEventKind::ScrollDown, 0, 0);
    }
    assert_eq!(a.sidebar.sel, crate::sidebar::FIXED + 2, "the wheel moves its highlight, to its end");
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.modal(), Some(Modal::Library));
    assert!(a.toast.as_ref().is_some_and(|(t, _)| t.contains("Folders")), "{:?}", a.toast);
    // a right click on a row: its menu, as in the Pane
    render(&mut a);
    let (x, y) = row(&a, Hit::Side(crate::sidebar::FIXED + 1));
    mouse(&mut a, MouseEventKind::Down(MouseButton::Right), x, y);
    assert!(matches!(a.menu().map(|m| &m.target), Some(Target::Library { name, .. }) if name == "Night Drive"));
    key(&mut a, KeyCode::Esc);
    // Esc closes it; so does a click outside
    palette(&mut a, "your library");
    key(&mut a, KeyCode::Esc);
    assert_eq!(a.modal(), None);
    palette(&mut a, "your library");
    render(&mut a);
    click(&mut a, 0, H - 1);
    assert_eq!(a.modal(), None);
}

#[test]
fn with_their_panes_the_entries_focus_them() {
    let (mut a, _rx) = app();
    a.focus_on(Widget::Home);
    palette(&mut a, "your library");
    assert_eq!((a.modal(), a.focused()), (None, Some(Widget::Library)));
    palette(&mut a, "queue");
    assert_eq!((a.modal(), a.focused()), (None, Some(Widget::Queue)));
}

#[test]
fn without_a_queue_pane_queue_opens_it_as_a_popup_to_see_reorder_and_remove() {
    let (mut a, mut rx) = app();
    without(&mut a, Widget::Queue);
    sent(&mut rx);
    palette(&mut a, "queue");
    assert_eq!(a.modal(), Some(Modal::Queue));
    let shot = render(&mut a);
    assert!(shot.contains("Next in queue") && (0..4).all(|i| shot.contains(&format!("Song {i}"))) && shot.contains("Esc close"), "{shot}");
    // ↓ moves the highlight, as in the Pane
    key(&mut a, KeyCode::Down);
    assert_eq!(a.queue_view.sel, 1);
    // a drag: the last song onto the first row
    render(&mut a);
    let (x, from) = row(&a, Hit::QueueRow(3));
    let (_, to) = row(&a, Hit::QueueRow(0));
    mouse(&mut a, MouseEventKind::Down(MouseButton::Left), x, from);
    mouse(&mut a, MouseEventKind::Drag(MouseButton::Left), x, to);
    mouse(&mut a, MouseEventKind::Up(MouseButton::Left), x, to);
    assert_eq!(sent(&mut rx), vec![Command::MoveInQueue { uri: "spotify:track:q3".into(), uid: Some("u3".into()), to: 0, into_queue: false }]);
    let order: String = a.playback.queue.iter().map(|q| q.item.name.chars().last().unwrap()).collect();
    assert_eq!(order, "3012", "shown at once");
    assert_eq!(a.modal(), Some(Modal::Queue), "the drag leaves it open");
    // Delete removes the highlighted song
    key(&mut a, KeyCode::Delete);
    assert_eq!(sent(&mut rx), vec![Command::RemoveFromQueue { uri: "spotify:track:q3".into(), uid: Some("u3".into()) }]);
    // Esc closes it
    key(&mut a, KeyCode::Esc);
    assert_eq!(a.modal(), None);
}
