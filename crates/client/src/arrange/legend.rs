//! Arrange mode's legend pinned, exactly, at the widths the owner uses (#86): a change to its keys that pushes one off
//! the line (as "drag a title…" and "^Z undo" once went at 160) fails here and has to be explained in its PR.
use super::*;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::KeyModifiers;

const H: u16 = 50;

/// The last row (the legend's) of a frame drawn `w` wide, its blanks at the end trimmed.
fn legend(app: &mut App, w: u16) -> String {
    let mut t = Terminal::new(TestBackend::new(w, H)).unwrap();
    t.draw(|f| crate::ui::frame(f, app)).unwrap();
    let buf = t.backend().buffer();
    (0..w).map(|x| buf[(x, H - 1)].symbol().to_string()).collect::<String>().trim_end().to_string()
}

fn key(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn tray_focus(app: &App) -> Option<TrayStop> {
    app.arrange.on.as_ref().and_then(|a| a.tray_focus)
}

/// ↓ from the queue's Pane until the keyboard's focus is on the Tray.
fn down_to_tray(app: &mut App, w: u16) {
    app.focus_on(Widget::Queue);
    for _ in 0..10 {
        if tray_focus(app).is_some() {
            return;
        }
        key(app, KeyCode::Down);
        legend(app, w);
    }
    panic!("{w}: ↓ never reached the Tray");
}

/// The states the legend tells apart, each reached as the owner would, at `w` columns.
fn state(name: &str, w: u16) -> App {
    let mut a = App::new(None);
    legend(&mut a, w);
    match name {
        "nothing picked up" => key(&mut a, KeyCode::Char('m')),
        "a Widget carried" => {
            key(&mut a, KeyCode::Char('m'));
            a.focus_on(Widget::Queue);
            key(&mut a, KeyCode::Enter);
            assert!(a.arrange.tile.as_ref().is_some_and(|t| t.aim.is_some()), "{w}: Enter picked it up");
        }
        "s waiting for its arrow" => {
            key(&mut a, KeyCode::Char('m'));
            a.focus_on(Widget::Queue);
            key(&mut a, KeyCode::Char('s'));
            assert!(a.arrange.on.as_ref().is_some_and(|r| r.split_next), "{w}: s waits");
        }
        "a Tray Widget focused" => {
            key(&mut a, KeyCode::Char('m'));
            a.focus_on(Widget::Fft);
            key(&mut a, KeyCode::Char('t'));
            assert_eq!(a.layout.tray(), &[Widget::Fft]);
            legend(&mut a, w);
            down_to_tray(&mut a, w);
            for _ in 0..10 {
                if tray_focus(&a) == Some(TrayStop::Widget(Widget::Fft)) {
                    break;
                }
                key(&mut a, KeyCode::Left);
            }
            assert_eq!(tray_focus(&a), Some(TrayStop::Widget(Widget::Fft)), "← reaches the Tray's Widget");
        }
        "+ Widget focused" => {
            key(&mut a, KeyCode::Char('m'));
            down_to_tray(&mut a, w);
            assert_eq!(tray_focus(&a), Some(TrayStop::Add), "{w}: the empty Tray's one stop");
        }
        "Home still needs a Pane" => {
            a.run(Action::NewDesign);
            assert!(a.done_blocked().is_some(), "{w}: Home waits in the Tray");
        }
        _ => unreachable!("{name}"),
    }
    assert!(a.arrange.on.is_some(), "{w} {name}: arranging");
    a
}

/// Today's legend (`main` when #86 was built, after the D87 merge), state by state and width by width: the keys between the ` ARRANGE ` tag
/// and the right end, which is ` Done ` (after "Home still needs a Pane" while Home waits in the Tray).
const PINNED: [(&str, u16, &str); 30] = [
    ("nothing picked up", 90, "←↑↓→ Enter pick up · t Tray · ⊞ s+arrow split · × x close · Esc done"),
    ("nothing picked up", 120, "←↑↓→ Enter pick up · t Tray · ⊞ s+arrow split · × x close · +−[]size · ^Z undo · ^Y redo · Esc done"),
    ("nothing picked up", 160, "drag a title onto a Pane or the Tray · ←↑↓→ Enter pick up · t Tray · ⊞ s+arrow split · × x close · +−[]size · ^Z undo · ^Y redo · Esc done"),
    ("nothing picked up", 180, "drag a title onto a Pane or the Tray · ←↑↓→ Enter pick up · t Tray · ⊞ s+arrow split · × x close · +−[]size · w + Widget · ^Z undo · ^Y redo · Esc done"),
    ("nothing picked up", 264, "drag a title onto a Pane or the Tray · ←↑↓→ Enter pick up · t Tray · ⊞ s+arrow split · × x close · +−[]size · w + Widget · ^Z undo · ^Y redo · Esc done"),
    ("a Widget carried", 90, "←↑↓→ aim · Enter drop · Esc put back"),
    ("a Widget carried", 120, "←↑↓→ aim · Enter drop · Esc put back"),
    ("a Widget carried", 160, "←↑↓→ aim · Enter drop · Esc put back"),
    ("a Widget carried", 180, "←↑↓→ aim · Enter drop · Esc put back"),
    ("a Widget carried", 264, "←↑↓→ aim · Enter drop · Esc put back"),
    ("s waiting for its arrow", 90, "←↑↓→ split that way"),
    ("s waiting for its arrow", 120, "←↑↓→ split that way"),
    ("s waiting for its arrow", 160, "←↑↓→ split that way"),
    ("s waiting for its arrow", 180, "←↑↓→ split that way"),
    ("s waiting for its arrow", 264, "←↑↓→ split that way"),
    ("a Tray Widget focused", 90, "←↑↓→ Enter pick up · t Tray · ⊞ s+arrow split · x remove · Esc done"),
    ("a Tray Widget focused", 120, "←↑↓→ Enter pick up · t Tray · ⊞ s+arrow split · x remove · +−[]size · ^Z undo · ^Y redo · Esc done"),
    ("a Tray Widget focused", 160, "drag a title onto a Pane or the Tray · ←↑↓→ Enter pick up · t Tray · ⊞ s+arrow split · x remove · +−[]size · ^Z undo · ^Y redo · Esc done"),
    ("a Tray Widget focused", 180, "drag a title onto a Pane or the Tray · ←↑↓→ Enter pick up · t Tray · ⊞ s+arrow split · x remove · +−[]size · w + Widget · ^Z undo · ^Y redo · Esc done"),
    ("a Tray Widget focused", 264, "drag a title onto a Pane or the Tray · ←↑↓→ Enter pick up · t Tray · ⊞ s+arrow split · x remove · +−[]size · w + Widget · ^Z undo · ^Y redo · Esc done"),
    ("+ Widget focused", 90, "←↑↓→ Enter pick up · t Tray · ⊞ s+arrow split · × x close · Esc done"),
    ("+ Widget focused", 120, "←↑↓→ Enter pick up · t Tray · ⊞ s+arrow split · × x close · +−[]size · ^Z undo · ^Y redo · Esc done"),
    ("+ Widget focused", 160, "drag a title onto a Pane or the Tray · ←↑↓→ Enter pick up · t Tray · ⊞ s+arrow split · × x close · +−[]size · ^Z undo · ^Y redo · Esc done"),
    ("+ Widget focused", 180, "drag a title onto a Pane or the Tray · ←↑↓→ Enter pick up · t Tray · ⊞ s+arrow split · × x close · +−[]size · w + Widget · ^Z undo · ^Y redo · Esc done"),
    ("+ Widget focused", 264, "drag a title onto a Pane or the Tray · ←↑↓→ Enter pick up · t Tray · ⊞ s+arrow split · × x close · +−[]size · w + Widget · ^Z undo · ^Y redo · Esc done"),
    ("Home still needs a Pane", 90, "←↑↓→ Enter pick up · t Tray · Esc done"),
    ("Home still needs a Pane", 120, "←↑↓→ Enter pick up · t Tray · ⊞ s+arrow split · × x close · Esc done"),
    ("Home still needs a Pane", 160, "←↑↓→ Enter pick up · t Tray · ⊞ s+arrow split · × x close · +−[]size · ^Z undo · ^Y redo · Esc done"),
    ("Home still needs a Pane", 180, "←↑↓→ Enter pick up · t Tray · ⊞ s+arrow split · × x close · +−[]size · ^Z undo · ^Y redo · Esc done"),
    ("Home still needs a Pane", 264, "drag a title onto a Pane or the Tray · ←↑↓→ Enter pick up · t Tray · ⊞ s+arrow split · × x close · +−[]size · w + Widget · ^Z undo · ^Y redo · Esc done"),
];

/// The whole row each state draws at each width, cell by cell: the tag, the keys, blanks, the right end.
#[test]
fn the_arrange_legend_is_pinned_at_the_owners_widths() {
    let mut wrong = Vec::new();
    for (name, w, keys) in PINNED {
        let right = if name == "Home still needs a Pane" { "Home still needs a Pane   Done" } else { "Done" };
        let left = format!("  ARRANGE  {keys}");
        // ` Done ` ends a cell short of the edge (the row's blanks after it are trimmed)
        let Some(gap) = (w as usize - 2).checked_sub(left.chars().count() + right.chars().count()) else {
            wrong.push(format!("{name} at {w}: the pinned keys don't fit {w} columns"));
            continue;
        };
        let want = format!("{left}{}{right}", " ".repeat(gap));
        let mut a = state(name, w);
        let got = legend(&mut a, w);
        if got != want {
            wrong.push(format!("{name} at {w}:\n  want {want:?}\n  got  {got:?}"));
        }
    }
    assert!(wrong.is_empty(), "the legend changed (explain it in the PR, then update PINNED):\n{}", wrong.join("\n"));
}

