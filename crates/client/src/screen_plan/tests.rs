//! The Screen plan's tests: a title's chrome as plain values, and clicks and the wheel with no frame drawn.

use super::*;
use crate::layout::{Axis, Node, Size};
use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

const WINDOW: Rect = Rect { x: 0, y: 0, width: 120, height: 40 };

/// The plain values a plan is worked out from: `layout` in a 120×40 window, nothing focused, held or numbered.
fn facts(layout: &Arrangement) -> Facts<'_> {
    Facts { window: WINDOW, arranging: false, layout, focus: None, moving: None, design: 0, numbered: false, main_title: None, top_week_tracks: false, history: false }
}

/// Home on the left, `w` on the right (`cells` wide, or half the screen).
fn beside_home(w: Option<Widget>, cells: Option<u16>) -> Arrangement {
    let size = cells.map_or(Size::Ratio(0.5), Size::Last);
    let right = match w {
        Some(w) => Node::pane(w),
        None => Node::Pane(Pane { id: crate::layout::PaneId(77), widget: None }),
    };
    Node::split(Axis::Horizontal, size, Node::pane(Widget::Home), right).into()
}

/// The plan's Pane showing `w` (None: the empty one).
fn of(plan: &ScreenPlan, w: Option<Widget>) -> PanePlan {
    plan.panes.iter().find(|p| p.pane.widget == w).cloned().unwrap_or_else(|| panic!("no Pane shows {w:?}"))
}

fn cells(r: Rect, x_from_right: u16, w: u16) -> Option<Rect> {
    Some(Rect::new(r.right() - x_from_right, r.y, w, 1))
}

#[test]
fn a_closable_pane_has_its_name_and_a_cross_at_the_right() {
    let l = beside_home(Some(Widget::Library), None);
    let p = of(&plan(&facts(&l)), Some(Widget::Library));
    let r = p.rect;
    assert_eq!((p.title.lead.as_str(), p.title.text.as_str()), (" ", "~/library "));
    assert_eq!(p.title.bar, Rect::new(r.x + 1, r.y, r.width - 2, 1));
    assert_eq!(p.title.name, Some((Rect::new(r.x + 2, r.y, 9, 1), Hit::TitleName(p.pane.id))));
    assert_eq!(p.title.close, cells(r, 5, 3));
    assert_eq!((p.title.swap, p.title.history, p.title.split), (None, None, None), "the library gives way to no other Widget");
    assert!(!p.short && !p.bare && !p.own_frame && !p.title.lit);
}

#[test]
fn a_widget_that_gives_way_keeps_its_arrows_left_of_the_cross() {
    let l = beside_home(Some(Widget::Marquee), None);
    let p = of(&plan(&facts(&l)), Some(Widget::Marquee));
    assert_eq!(p.title.swap, cells(p.rect, 10, 5));
    assert_eq!(p.title.close, cells(p.rect, 5, 3));
    let hits: Vec<Hit> = p.title.hits(p.pane.id).into_iter().map(|(_, h)| h).collect();
    let id = p.pane.id;
    assert_eq!(hits, [Hit::Title(id), Hit::TitleName(id), Hit::Swap(id, -1), Hit::Swap(id, 1), Hit::Close(id)], "the buttons win over the name, the name over the bar");
}

#[test]
fn the_main_pane_is_titled_after_its_page_with_back_and_forward_and_never_closes() {
    let l = beside_home(Some(Widget::Library), None);
    let f = Facts { main_title: Some("search".into()), history: true, focus: l.pane_of(Widget::Home), ..facts(&l) };
    let p = of(&plan(&f), Some(Widget::Home));
    assert_eq!(p.title.text, "search ");
    assert_eq!(p.title.history, cells(p.rect, 7, 5));
    assert_eq!((p.title.close, p.title.name.clone(), p.title.swap), (None, None, None), "Home is Required: no ×, nothing replaces it");
    assert!(p.title.lit && p.focused);
    // nowhere to go back or forward: no arrows
    let p = of(&plan(&Facts { history: false, ..facts(&l) }), Some(Widget::Home));
    assert_eq!(p.title.history, None);
}

#[test]
fn arrange_mode_puts_a_split_button_before_the_cross_and_takes_the_arrows_away() {
    let l = beside_home(Some(Widget::Marquee), None);
    let f = Facts { arranging: true, history: true, ..facts(&l) };
    let plan = plan(&f);
    let m = of(&plan, Some(Widget::Marquee));
    assert_eq!((m.title.split, m.title.close, m.title.swap), (cells(m.rect, 8, 3), cells(m.rect, 5, 3), None));
    let home = of(&plan, Some(Widget::Home));
    assert_eq!((home.title.split, home.title.close, home.title.history), (cells(home.rect, 5, 3), None, None), "Home's ⊞ sits where a × would");
    // the Panes give the Tray its rows
    assert_eq!(plan.rows.tray.height, crate::arrange::TRAY_H);
    assert_eq!(plan.rows.body.height, WINDOW.height - 5 - crate::arrange::TRAY_H);
}

#[test]
fn a_narrow_pane_or_the_only_one_has_no_cross() {
    let l = beside_home(Some(Widget::Library), Some(13));
    let p = of(&plan(&facts(&l)), Some(Widget::Library));
    assert_eq!(p.rect.width, 13);
    assert_eq!(p.title.close, None, "no room for a ×");
    let only: Arrangement = Arrangement::new(Node::pane(Widget::Library), vec![Widget::Home]);
    let p = of(&plan(&facts(&only)), Some(Widget::Library));
    assert_eq!(p.title.close, None, "the only Pane");
}

#[test]
fn a_long_title_is_cut_before_the_buttons() {
    let l = beside_home(Some(Widget::Library), Some(90));
    let f = Facts { main_title: Some("a very long playlist name that goes on".into()), history: true, ..facts(&l) };
    let home = of(&plan(&f), Some(Widget::Home));
    // room: its width less the corners, the lead, the arrows (8) and the 3 cells the arrows keep for a ×; the title
    // takes all but two of it, then "…" and its space
    assert_eq!(home.rect.width, 30);
    let room = 30 - 2 - 1 - 8 - 3;
    assert_eq!(home.title.text, "a very long pl… ");
    assert_eq!(home.title.text.chars().count(), room);
    assert!(home.title.text.ends_with("… "), "{:?}", home.title.text);
}

#[test]
fn an_empty_pane_has_no_title_but_closes() {
    let l = beside_home(None, None);
    let p = of(&plan(&facts(&l)), None);
    assert_eq!((p.title.lead.as_str(), p.title.text.as_str(), p.title.name.clone()), ("", "", None));
    assert_eq!(p.title.close, cells(p.rect, 5, 3));
}

#[test]
fn top_this_weeks_title_is_its_switch_unless_its_pane_is_too_small_for_it() {
    let l = beside_home(Some(Widget::TopWeek), None);
    let p = of(&plan(&Facts { top_week_tracks: true, ..facts(&l) }), Some(Widget::TopWeek));
    assert_eq!(p.title.text, "top tracks · week ⇄ ");
    assert_eq!(p.title.name, Some((Rect::new(p.rect.x + 1, p.rect.y, 21, 1), Hit::TopWeekToggle)));
    let l = beside_home(Some(Widget::TopWeek), Some(16));
    let p = of(&plan(&facts(&l)), Some(Widget::TopWeek));
    assert!(p.short && plan(&facts(&l)).short(p.pane.id), "{:?}", p.rect);
    assert!(matches!(p.title.name, Some((_, Hit::TitleName(_)))), "a hidden list has no switch, only its name: {:?}", p.title.name);
}

#[test]
fn numbered_themes_number_the_panes_and_a_held_widget_lights_its_title() {
    let l = beside_home(Some(Widget::Library), None);
    let lib = l.pane_of(Widget::Library);
    let plan = plan(&Facts { numbered: true, moving: lib, ..facts(&l) });
    assert_eq!(of(&plan, Some(Widget::Home)).title.lead, " ¹");
    let p = of(&plan, Some(Widget::Library));
    assert_eq!(p.title.lead, " ²");
    assert!(p.held && p.title.lit && !p.focused);
    assert_eq!(p.title.name.unwrap().0.x, p.rect.x + 3, "the name starts after the number");
}

#[test]
fn the_well_draws_its_own_frame_and_a_tiny_window_has_no_panes() {
    let l = beside_home(Some(Widget::Well), None);
    let p = of(&plan(&facts(&l)), Some(Widget::Well));
    assert!(p.own_frame && !p.short);
    assert_eq!(p.title.close, cells(p.rect, 5, 3));
    let tiny = plan(&Facts { window: Rect::new(0, 0, 30, 8), ..facts(&l) });
    assert!(tiny.mini && tiny.panes.is_empty() && tiny.dividers.is_empty());
    let unsized_ = plan(&Facts { window: Rect::default(), ..facts(&l) });
    assert!(unsized_.panes.is_empty(), "before the first frame");
}

fn app() -> App {
    let (tx, rx) = tokio::sync::mpsc::channel(4096);
    std::mem::forget(rx);
    let mut app = App::new(Some(tx));
    app.window_resized(WINDOW.width, WINDOW.height);
    app
}

fn mouse(app: &mut App, kind: MouseEventKind, x: u16, y: u16) {
    app.mouse(MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE });
}

fn click(app: &mut App, r: Rect) {
    mouse(app, MouseEventKind::Down(MouseButton::Left), r.x + r.width / 2, r.y);
    mouse(app, MouseEventKind::Up(MouseButton::Left), r.x + r.width / 2, r.y);
}

#[test]
fn clicks_and_the_wheel_find_their_pane_with_no_frame_drawn() {
    let mut app = app();
    assert!(app.hits.is_empty(), "nothing drawn");
    // the ×
    let fft = app.layout.pane_of(Widget::Fft).unwrap();
    let close = app.plan().pane(fft).unwrap().title.close.unwrap();
    assert_eq!(app.hit_at(close.x + 1, close.y), Some(Hit::Close(fft)));
    click(&mut app, close);
    assert!(!app.layout.has(Widget::Fft), "the × closed it");
    // the Widget's name opens the Widget picker
    let queue = app.layout.pane_of(Widget::Queue).unwrap();
    let (name, _) = app.plan().pane(queue).unwrap().title.name.clone().unwrap();
    click(&mut app, name);
    assert!(app.widget_picker().is_some_and(|p| p.target == queue));
    app.key(ratatui::crossterm::event::KeyEvent::new(ratatui::crossterm::event::KeyCode::Esc, KeyModifiers::NONE));
    // a click in a Pane's body focuses it
    let lib = app.layout.pane_of(Widget::Library).unwrap();
    let r = app.plan().pane(lib).unwrap().rect;
    click(&mut app, Rect::new(r.x + 3, r.y + 4, 1, 1));
    assert_eq!(app.focus, Some(lib));
    // the wheel over the queue: its Pane takes it
    mouse(&mut app, MouseEventKind::ScrollDown, r.x + 3, r.y + 4);
    let q = app.plan().pane(queue).unwrap().rect;
    mouse(&mut app, MouseEventKind::ScrollDown, q.x + 4, q.y + 3);
    assert_eq!(app.focus, Some(queue), "the wheel focused the queue");
    // a divider's grip is found where the plan has it
    let d = app.dividers()[0].clone();
    assert_eq!(app.plan().divider_at(d.grip.x, d.grip.y + d.grip.height / 2), Some(0));
}

#[test]
fn drawing_changes_nothing_the_plan_is_worked_out_from() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let mut app = app();
    // a Widget held over another Pane in Arrange mode, so the plan has a Tray, a target, a label and a chip
    let stream = app.layout.pane_of(Widget::Stream).unwrap();
    let s = app.plan().pane(stream).unwrap().rect;
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), s.x + 3, s.y);
    let q = app.layout.compute(app.pane_area()).rect_of(Widget::Queue).unwrap();
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), q.x + 5, q.y + 4);
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), q.x + 6, q.y + 5);
    let mut t = Terminal::new(TestBackend::new(WINDOW.width, WINDOW.height)).unwrap();
    for _ in 0..2 {
        let mut planned = None;
        t.draw(|f| {
            let plan = app.frame_begins(f.area());
            let before = (app.layout.clone(), app.focus, app.body, app.window, app.mini, app.arrange.is_on(), app.moving(), app.arrange.tray_first(), app.label_row(app.layout.pane_of(Widget::Queue).unwrap()));
            crate::ui::draw(f, &mut app, &plan);
            let after = (app.layout.clone(), app.focus, app.body, app.window, app.mini, app.arrange.is_on(), app.moving(), app.arrange.tray_first(), app.label_row(app.layout.pane_of(Widget::Queue).unwrap()));
            assert_eq!(before, after, "drawing changed the App");
            assert_eq!(app.plan(), plan, "drawing changed what the plan is worked out from");
            planned = Some(plan);
        })
        .unwrap();
        let plan = planned.unwrap();
        assert!(plan.tray.is_some() && plan.drag.as_ref().is_some_and(|d| d.target.is_some() && d.label.is_some() && d.chip.is_some()), "{:?}", plan.drag);
    }
}
