//! Client state and input. Rendering lives in `ui`; the connection in `conn`.
//! Every interactive element registers a hit-box while drawing, so the mouse works everywhere.

use crate::covers::Covers;
use crate::gfx::Kitty;
use crate::home::{CardRef, HomeView};
use crate::layout::{Divider, Node};
use crate::scrub::{Bar, Scrub};
use crate::sidebar::Sidebar;
use crate::tracks::{Col, TrackList, TrackView};
use std::collections::HashMap;
use crate::theme::Theme;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use singularity_proto::{CatalogueStatus, Command, EngineStatus, HomeFeed, PlaybackState};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

#[derive(Clone, Debug, PartialEq)]
pub enum Link {
    Connecting,
    Connected { version: String },
    Incompatible(String),
}

/// Everything a user can do. The palette lists these; keys and buttons trigger them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    TogglePlay,
    Next,
    Previous,
    SeekForward,
    SeekBack,
    SeekForwardFine,
    SeekBackFine,
    VolumeUp,
    VolumeDown,
    Shuffle,
    Repeat,
    Login,
    Logout,
    RefreshHome,
    GoHome,
    Filter,
    SortTitle,
    SortArtist,
    SortAlbum,
    SortAdded,
    SortDuration,
    FocusNext,
    ResetLayout,
    Palette,
    Help,
    Quit,
}

impl Action {
    pub const ALL: [Action; 26] = [
        Action::TogglePlay,
        Action::Next,
        Action::Previous,
        Action::SeekForward,
        Action::SeekBack,
        Action::SeekForwardFine,
        Action::SeekBackFine,
        Action::VolumeUp,
        Action::VolumeDown,
        Action::Shuffle,
        Action::Repeat,
        Action::Login,
        Action::Logout,
        Action::RefreshHome,
        Action::GoHome,
        Action::Filter,
        Action::SortTitle,
        Action::SortArtist,
        Action::SortAlbum,
        Action::SortAdded,
        Action::SortDuration,
        Action::FocusNext,
        Action::ResetLayout,
        Action::Palette,
        Action::Help,
        Action::Quit,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Action::TogglePlay => "Play / pause",
            Action::Next => "Next track",
            Action::Previous => "Previous track",
            Action::SeekForward => "Seek forward 5 s",
            Action::SeekBack => "Seek back 5 s",
            Action::SeekForwardFine => "Seek forward 1 s",
            Action::SeekBackFine => "Seek back 1 s",
            Action::VolumeUp => "Volume up",
            Action::VolumeDown => "Volume down",
            Action::Shuffle => "Toggle shuffle",
            Action::Repeat => "Cycle repeat (off → all → one)",
            Action::Login => "Log in to Spotify",
            Action::Logout => "Log out of Spotify",
            Action::RefreshHome => "Refresh Home",
            Action::GoHome => "Go to Home",
            Action::Filter => "Filter this track list",
            Action::SortTitle => "Sort list by title",
            Action::SortArtist => "Sort list by artist",
            Action::SortAlbum => "Sort list by album",
            Action::SortAdded => "Sort list by date added",
            Action::SortDuration => "Sort list by duration",
            Action::FocusNext => "Focus next pane",
            Action::ResetLayout => "Reset layout",
            Action::Palette => "Command palette",
            Action::Help => "Help: which keys exist?",
            Action::Quit => "Quit (music keeps playing)",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Action::TogglePlay => "Space",
            Action::Next => "n",
            Action::Previous => "b",
            Action::SeekForward => "Shift+→",
            Action::SeekBack => "Shift+←",
            Action::SeekForwardFine => ".",
            Action::SeekBackFine => ",",
            Action::VolumeUp => "+",
            Action::VolumeDown => "-",
            Action::Shuffle => "s",
            Action::Repeat => "r",
            Action::Login | Action::Logout | Action::RefreshHome => "",
            Action::GoHome => "Backspace",
            Action::Filter => "/",
            Action::SortTitle | Action::SortArtist | Action::SortAlbum | Action::SortAdded | Action::SortDuration => "",
            Action::FocusNext => "Tab",
            Action::ResetLayout => "",
            Action::Palette => "Ctrl+K",
            Action::Help => "?",
            Action::Quit => "q",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    Focus(String),
    Divider(usize),
    Action(Action),
    PaletteItem(usize),
    Bar(Bar, Rect),
    Card(CardRef),
    /// a Shelf's ‹ › buttons: row index, direction
    ShelfScroll(usize, i32),
    MenuItem(usize),
    /// a sidebar row (0 = Home)
    Side(usize),
    /// a track-list row, by position in the view
    Track(usize),
    SortCol(Col),
}

/// Two clicks on the same row within this time play it.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// What a context-menu entry does to its Card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAct {
    Play,
    Queue,
    GoTo,
    CopyLink,
}

pub struct Menu {
    /// where it opened (the pointer, or the selected Card)
    pub at: (u16, u16),
    pub card: CardRef,
    pub items: Vec<(String, MenuAct)>,
    pub selected: usize,
}

/// Wheel and key nudges are coalesced: the bar moves at once, the command goes out once the nudging pauses.
pub(crate) const NUDGE_SETTLE: Duration = Duration::from_millis(180);

#[derive(Default)]
struct Nudge {
    seek: Option<u64>,
    volume: Option<u8>,
    due: Option<Instant>,
}

pub struct Palette {
    pub query: String,
    pub selected: usize,
}

pub struct App {
    pub theme: Theme,
    pub layout: Node,
    pub link: Link,
    pub engine: Option<EngineStatus>,
    pub playback: PlaybackState,
    pub catalogue: Option<CatalogueStatus>,
    pub home: Option<HomeFeed>,
    pub focus: String,
    pub palette: Option<Palette>,
    pub help: bool,
    pub toast: Option<(String, Instant)>,
    pub quit: bool,
    /// the user dismissed the login overlay (the login itself keeps waiting)
    pub login_hidden: bool,
    pub dividers: Vec<Divider>,
    pub(crate) hits: Vec<(Rect, Hit)>,
    drag: Option<Hit>,
    /// boundary minus grab position, so a divider moves exactly with the mouse whichever grip cell was grabbed
    drag_offset: i32,
    /// the seek or volume bar being dragged
    pub scrub: Option<Scrub>,
    nudge: Nudge,
    pub home_view: HomeView,
    pub menu: Option<Menu>,
    pub covers: Covers,
    /// the pixel tier, when the terminal speaks the kitty graphics protocol
    pub kitty: Option<Kitty>,
    /// one cell's size in pixels, when the terminal reports it
    pub cell_px: Option<(f64, f64)>,
    /// screen areas covered by popups this frame: pixel images there fall back to cells
    pub overlays: Vec<Rect>,
    /// text to put on the system clipboard (OSC 52), written by the main loop
    pub clipboard: Option<String>,
    pub library: Option<singularity_proto::Library>,
    pub sidebar: Sidebar,
    /// track lists received so far, by URI
    pub lists: HashMap<String, TrackList>,
    /// the open track list (None: the main Pane shows Home)
    pub view: Option<TrackView>,
    last_click: Option<(Instant, usize)>,
    commands: Option<mpsc::Sender<Command>>,
}

impl App {
    pub fn new(commands: Option<mpsc::Sender<Command>>) -> App {
        App {
            theme: Theme::singularity(),
            layout: Node::singularity(),
            link: Link::Connecting,
            engine: None,
            playback: PlaybackState::default(),
            catalogue: None,
            home: None,
            focus: "home".into(),
            palette: None,
            help: false,
            toast: None,
            quit: false,
            login_hidden: false,
            dividers: vec![],
            hits: vec![],
            drag: None,
            drag_offset: 0,
            scrub: None,
            nudge: Nudge::default(),
            home_view: HomeView::default(),
            menu: None,
            covers: Covers::offline(),
            kitty: None,
            cell_px: None,
            overlays: vec![],
            clipboard: None,
            library: None,
            sidebar: Sidebar::default(),
            lists: HashMap::new(),
            view: None,
            last_click: None,
            commands,
        }
    }

    // ── frame bookkeeping ─────────────────────────────────────────────

    pub fn begin_frame(&mut self) {
        self.hits.clear();
        self.overlays.clear();
        self.covers.poll();
        if self.toast.as_ref().is_some_and(|(_, t)| t.elapsed() > Duration::from_millis(2500)) {
            self.toast = None;
        }
    }

    pub fn hit(&mut self, r: Rect, h: Hit) {
        self.hits.push((r, h));
    }

    fn hit_at(&self, x: u16, y: u16) -> Option<Hit> {
        self.hits.iter().rev().find(|(r, _)| x >= r.x && x < r.right() && y >= r.y && y < r.bottom()).map(|(_, h)| h.clone())
    }

    /// The pixel tier's cell size, if `r` can be drawn with it this frame.
    pub fn pixel_tier_for(&self, r: Rect) -> Option<(f64, f64)> {
        let px = self.cell_px?;
        (self.kitty.is_some() && !self.overlays.iter().any(|o| o.intersects(r))).then_some(px)
    }

    pub fn say(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    fn send(&mut self, c: Command) {
        let ok = match (&self.link, &self.commands) {
            (Link::Connected { .. }, Some(tx)) => tx.try_send(c).is_ok(),
            _ => false,
        };
        if !ok {
            self.say("Not connected to singularityd");
        }
    }

    // ── actions ───────────────────────────────────────────────────────

    pub fn run(&mut self, a: Action) {
        match a {
            Action::TogglePlay => self.send(Command::TogglePlay),
            Action::Next => self.send(Command::Next),
            Action::Previous => self.send(Command::Previous),
            Action::SeekForward => self.nudge_seek(5000),
            Action::SeekBack => self.nudge_seek(-5000),
            Action::SeekForwardFine => self.nudge_seek(1000),
            Action::SeekBackFine => self.nudge_seek(-1000),
            Action::VolumeUp => self.nudge_volume(5),
            Action::VolumeDown => self.nudge_volume(-5),
            Action::Shuffle => self.send(Command::SetShuffle { enabled: !self.playback.shuffle }),
            Action::Repeat => {
                use singularity_proto::RepeatMode::*;
                let mode = match self.playback.repeat { Off => Context, Context => Track, Track => Off };
                self.send(Command::SetRepeat { mode });
            }
            Action::Login => self.send(Command::Login),
            Action::Logout => self.send(Command::Logout),
            Action::RefreshHome => self.send(Command::GetHome),
            Action::GoHome => {
                self.view = None;
                self.focus = "home".into();
            }
            Action::Filter => match self.view.as_mut() {
                Some(v) => {
                    v.filtering = true;
                    self.focus = "home".into();
                }
                None => self.say("Open a playlist, album or Liked Songs to filter it"),
            },
            Action::SortTitle => self.sort(Col::Title),
            Action::SortArtist => self.sort(Col::Artist),
            Action::SortAlbum => self.sort(Col::Album),
            Action::SortAdded => self.sort(Col::Added),
            Action::SortDuration => self.sort(Col::Duration),
            Action::FocusNext => {
                let slots: Vec<String> = self.layout.compute(Rect::new(0, 0, 200, 60)).slots.into_iter().map(|(s, _)| s).collect();
                let i = slots.iter().position(|s| *s == self.focus).map(|i| (i + 1) % slots.len()).unwrap_or(0);
                self.focus = slots[i].clone();
            }
            Action::ResetLayout => {
                self.layout = Node::singularity();
                self.say("Layout reset");
            }
            Action::Palette => self.palette = Some(Palette { query: String::new(), selected: 0 }),
            Action::Help => self.help = true,
            Action::Quit => self.quit = true,
        }
    }

    pub fn palette_items(&self) -> Vec<Action> {
        let q = self.palette.as_ref().map(|p| p.query.to_lowercase()).unwrap_or_default();
        Action::ALL.into_iter().filter(|a| *a != Action::Palette && fuzzy(&a.label().to_lowercase(), &q)).collect()
    }

    // ── keyboard ──────────────────────────────────────────────────────

    pub fn key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if self.palette.is_some() {
            return self.palette_key(k);
        }
        if self.menu.is_some() {
            return self.menu_key(k);
        }
        if self.help {
            self.help = false;
            return;
        }
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        if self.view.as_ref().is_some_and(|v| v.filtering) {
            return self.filter_key(k);
        }
        if self.focus == "home" && !shift && self.view.is_some() && self.tracks_key(k) {
            return;
        }
        if self.focus == "home" && !shift && self.view.is_none() && self.home_key(k) {
            return;
        }
        if self.focus == "library" && !shift && self.side_key(k) {
            return;
        }
        match k.code {
            KeyCode::Char('k') if ctrl => self.run(Action::Palette),
            KeyCode::Char(':') => self.run(Action::Palette),
            KeyCode::Char('?') => self.run(Action::Help),
            KeyCode::Esc if matches!(self.catalogue, Some(CatalogueStatus::LoginPending { .. })) => self.login_hidden = true,
            KeyCode::Char('q') | KeyCode::Esc => self.run(Action::Quit),
            KeyCode::Char(' ') => self.run(Action::TogglePlay),
            KeyCode::Char('n') => self.run(Action::Next),
            KeyCode::Char('b') => self.run(Action::Previous),
            KeyCode::Left if k.modifiers.contains(KeyModifiers::ALT) => self.run(Action::GoHome),
            // arrows navigate a focused Pane that has something to navigate; elsewhere they seek
            KeyCode::Right => self.run(Action::SeekForward),
            KeyCode::Left => self.run(Action::SeekBack),
            KeyCode::Char('/') => self.run(Action::Filter),
            KeyCode::Char('f') if ctrl => self.run(Action::Filter),
            KeyCode::Backspace => self.run(Action::GoHome),
            KeyCode::Char('.') => self.run(Action::SeekForwardFine),
            KeyCode::Char(',') => self.run(Action::SeekBackFine),
            KeyCode::Char('+') | KeyCode::Char('=') => self.run(Action::VolumeUp),
            KeyCode::Char('-') => self.run(Action::VolumeDown),
            KeyCode::Char('s') => self.run(Action::Shuffle),
            KeyCode::Char('r') => self.run(Action::Repeat),
            KeyCode::Tab => self.run(Action::FocusNext),
            _ => {}
        }
    }

    /// Keys for the focused Home; returns false for keys it doesn't use.
    fn home_key(&mut self, k: KeyEvent) -> bool {
        if self.home.as_ref().is_none_or(|h| h.shelves.is_empty()) {
            return false;
        }
        let v = &mut self.home_view;
        match k.code {
            KeyCode::Left => v.move_sel(-1, 0),
            KeyCode::Right => v.move_sel(1, 0),
            KeyCode::Up => v.move_sel(0, -1),
            KeyCode::Down => v.move_sel(0, 1),
            KeyCode::PageUp => v.move_sel(0, -(v.visible.max(1) as i32)),
            KeyCode::PageDown => v.move_sel(0, v.visible.max(1) as i32),
            KeyCode::Enter => match v.sel {
                Some(c) => self.play_card(c),
                None => v.move_sel(0, 0),
            },
            KeyCode::Menu | KeyCode::F(10) => {
                if let Some(c) = v.sel {
                    self.open_menu(c, None);
                }
            }
            _ => return false,
        }
        true
    }

    // ── library & track lists ──────────────────────────────────────

    /// A page from the Daemon.
    pub fn track_page(&mut self, p: singularity_proto::TrackPage) {
        self.lists.entry(p.uri.clone()).or_default().add(&p);
    }

    /// Open sidebar row `row` (0 = Home).
    fn open_side(&mut self, row: usize) {
        self.sidebar.sel = row;
        if row == 0 {
            return self.run(Action::GoHome);
        }
        let Some(it) = self.library.as_ref().and_then(|l| l.items.get(row - 1)).cloned() else { return };
        match it.kind.as_str() {
            "liked" | "playlist" | "album" => {
                if self.view.as_ref().is_none_or(|v| v.uri != it.uri) {
                    self.view = Some(TrackView::new(&it.uri, &it.name));
                }
                if self.lists.get(&it.uri).is_none_or(|l| !l.complete()) {
                    self.lists.remove(&it.uri);
                    self.send(Command::GetTracks { uri: it.uri.clone() });
                }
            }
            "folder" => self.say("Folders are listed with their playlists indented below"),
            "artist" => self.say("Artist pages arrive with #11"),
            _ => {}
        }
    }

    fn sort(&mut self, col: Col) {
        match self.view.as_mut() {
            Some(v) => v.cycle_sort(col),
            None => self.say("Open a playlist, album or Liked Songs to sort it"),
        }
    }

    /// The view's visible order and the list length.
    fn view_order(&self) -> Vec<usize> {
        match (&self.view, self.view.as_ref().and_then(|v| self.lists.get(&v.uri))) {
            (Some(v), Some(l)) => v.order(&l.rows),
            _ => vec![],
        }
    }

    /// Play the row at view position `vi`: the context starts at that track's place in the original list.
    fn play_row(&mut self, vi: usize) {
        let order = self.view_order();
        let (Some(v), Some(&ri)) = (self.view.as_ref(), order.get(vi)) else { return };
        let Some(row) = self.lists.get(&v.uri).and_then(|l| l.rows.get(ri)) else { return };
        if !row.playable {
            return self.say("This track isn't available");
        }
        let cmd = Command::PlayFrom { context_uri: v.uri.clone(), index: ri as u32, track_uri: Some(row.uri.clone()) };
        self.send(cmd);
    }

    fn tracks_key(&mut self, k: KeyEvent) -> bool {
        let len = self.view_order().len();
        let Some(v) = self.view.as_mut() else { return false };
        let page = v.visible.max(1) as i64;
        match k.code {
            KeyCode::Up => v.move_sel(-1, len),
            KeyCode::Down => v.move_sel(1, len),
            KeyCode::PageUp => v.move_sel(-page, len),
            KeyCode::PageDown => v.move_sel(page, len),
            KeyCode::Home => v.move_sel(-(len as i64), len),
            KeyCode::End => v.move_sel(len as i64, len),
            KeyCode::Enter => {
                let sel = v.sel;
                self.play_row(sel);
            }
            // Esc clears the filter, then goes back to Home (it never quits from inside a list)
            KeyCode::Esc if !v.filter.is_empty() => v.set_filter(String::new()),
            KeyCode::Esc => self.run(Action::GoHome),
            _ => return false,
        }
        true
    }

    fn filter_key(&mut self, k: KeyEvent) {
        let len = self.view_order().len();
        let Some(v) = self.view.as_mut() else { return };
        match k.code {
            KeyCode::Esc => {
                v.filtering = false;
                v.set_filter(String::new());
            }
            KeyCode::Enter => v.filtering = false,
            KeyCode::Backspace => {
                let mut f = v.filter.clone();
                f.pop();
                v.set_filter(f);
            }
            KeyCode::Up => v.move_sel(-1, len),
            KeyCode::Down => v.move_sel(1, len),
            KeyCode::Char(c) => {
                let f = format!("{}{c}", v.filter);
                v.set_filter(f);
            }
            _ => {}
        }
    }

    fn side_key(&mut self, k: KeyEvent) -> bool {
        let n = crate::sidebar::len(self);
        match k.code {
            KeyCode::Up => self.sidebar.move_sel(-1, n),
            KeyCode::Down => self.sidebar.move_sel(1, n),
            KeyCode::Enter | KeyCode::Right => {
                let row = self.sidebar.sel;
                self.open_side(row);
                if row > 0 && self.view.is_some() {
                    self.focus = "home".into();
                }
            }
            _ => return false,
        }
        true
    }

    fn card(&self, c: CardRef) -> Option<&singularity_proto::Card> {
        self.home.as_ref()?.shelves.get(c.shelf)?.cards.get(c.card)
    }

    fn play_card(&mut self, c: CardRef) {
        self.home_view.sel = Some(c);
        if let Some(uri) = self.card(c).map(|k| k.uri.clone()) {
            self.send(Command::Play { uri: Some(uri) });
        }
    }

    /// Right-click (or the Menu key) on a Card. `at` is the pointer; None opens it under the selected Card.
    pub fn open_menu(&mut self, c: CardRef, at: Option<(u16, u16)>) {
        let Some(card) = self.card(c) else { return };
        let kind = card.kind.clone();
        let mut items = vec![("Play".to_string(), MenuAct::Play)];
        // Soloist queues single items; queueing whole albums and playlists needs their tracks (#12)
        if matches!(kind.as_str(), "track" | "episode") {
            items.push(("Add to queue".into(), MenuAct::Queue));
        }
        if matches!(kind.as_str(), "album" | "artist" | "playlist" | "show") {
            items.push((format!("Go to {kind}"), MenuAct::GoTo));
        }
        items.push(("Copy link".into(), MenuAct::CopyLink));
        let at = at.or_else(|| self.hits.iter().find(|(_, h)| *h == Hit::Card(c)).map(|(r, _)| (r.x + 2, r.y + 1))).unwrap_or((0, 0));
        self.home_view.sel = Some(c);
        self.menu = Some(Menu { at, card: c, items, selected: 0 });
    }

    fn menu_run(&mut self, i: usize) {
        let Some(m) = self.menu.take() else { return };
        let Some((_, act)) = m.items.get(i).cloned() else { return };
        let Some(card) = self.card(m.card).cloned() else { return };
        match act {
            MenuAct::Play => self.play_card(m.card),
            MenuAct::Queue => {
                self.send(Command::AddToQueue { uri: card.uri });
                self.say(format!("Queued {}", card.name));
            }
            MenuAct::GoTo => {
                let ticket = if card.kind == "playlist" { "#9" } else { "#11" };
                self.say(format!("{} pages arrive with {ticket}", capitalise(&card.kind)));
            }
            MenuAct::CopyLink => {
                self.clipboard = Some(web_link(&card.uri));
                self.say("Link copied");
            }
        }
    }

    fn menu_key(&mut self, k: KeyEvent) {
        let Some(m) = self.menu.as_mut() else { return };
        match k.code {
            KeyCode::Esc => self.menu = None,
            KeyCode::Up => m.selected = m.selected.saturating_sub(1),
            KeyCode::Down => m.selected = (m.selected + 1).min(m.items.len().saturating_sub(1)),
            KeyCode::Enter => {
                let i = m.selected;
                self.menu_run(i);
            }
            _ => {}
        }
    }

    fn palette_key(&mut self, k: KeyEvent) {
        let n = self.palette_items().len();
        let Some(p) = self.palette.as_mut() else { return };
        match k.code {
            KeyCode::Esc => self.palette = None,
            KeyCode::Up => p.selected = p.selected.saturating_sub(1),
            KeyCode::Down => p.selected = (p.selected + 1).min(n.saturating_sub(1)),
            KeyCode::Backspace => {
                p.query.pop();
                p.selected = 0;
            }
            KeyCode::Char(c) => {
                p.query.push(c);
                p.selected = 0;
            }
            KeyCode::Enter => {
                let sel = p.selected;
                if let Some(a) = self.palette_items().get(sel).copied() {
                    self.palette = None;
                    self.run(a);
                }
            }
            _ => {}
        }
    }

    // ── seek & volume ─────────────────────────────────────────────────

    fn duration_ms(&self) -> u64 {
        self.playback.item.as_ref().map(|i| i.duration_ms).unwrap_or(0)
    }

    /// Where the seek bar stands: the drag target, a pending nudge, or the interpolated playback position.
    pub fn display_position(&self, now_ms: u64) -> u64 {
        match &self.scrub {
            Some(s) if s.bar == Bar::Seek => s.value,
            _ => self.nudge.seek.unwrap_or_else(|| self.playback.position_at(now_ms)),
        }
    }

    pub fn display_volume(&self) -> u8 {
        match &self.scrub {
            Some(s) if s.bar == Bar::Volume => s.value as u8,
            _ => self.nudge.volume.unwrap_or(self.playback.volume),
        }
    }

    fn nudge_seek(&mut self, delta_ms: i64) {
        let dur = self.duration_ms();
        if dur == 0 {
            return;
        }
        let base = self.display_position(singularity_proto::now_ms()) as i64;
        self.nudge.seek = Some((base + delta_ms).clamp(0, dur as i64) as u64);
        self.nudge.due = Some(Instant::now() + NUDGE_SETTLE);
    }

    fn nudge_volume(&mut self, delta: i64) {
        self.nudge.volume = Some((self.display_volume() as i64 + delta).clamp(0, 100) as u8);
        self.nudge.due = Some(Instant::now() + NUDGE_SETTLE);
    }

    /// Called every frame: sends nudges once they've settled.
    pub fn tick(&mut self) {
        if self.nudge.due.is_some_and(|d| Instant::now() >= d) {
            self.flush_nudge();
        }
    }

    fn flush_nudge(&mut self) {
        let n = std::mem::take(&mut self.nudge);
        if let Some(ms) = n.seek {
            self.seek_to(ms);
        }
        if let Some(v) = n.volume {
            self.set_volume(v);
        }
    }

    /// Sends the seek and moves the local position anchor, so the bar doesn't snap back while the Daemon answers.
    fn seek_to(&mut self, ms: u64) {
        self.send(Command::Seek { position_ms: ms });
        self.playback.position_ms = ms;
        self.playback.timestamp_ms = singularity_proto::now_ms();
    }

    fn set_volume(&mut self, percent: u8) {
        if percent != self.playback.volume {
            self.send(Command::SetVolume { percent });
            self.playback.volume = percent;
        }
    }

    // ── mouse ─────────────────────────────────────────────────────────

    /// A mouse event in cell units (tests, and terminals without pixel reports).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn mouse(&mut self, m: MouseEvent) {
        self.pointer(m, m.column as f64 + 0.5, false);
    }

    /// A mouse event; `fx` is the pointer's fractional column, `precise` whether it has real sub-cell resolution.
    pub fn pointer(&mut self, m: MouseEvent, fx: f64, precise: bool) {
        let coarse = m.modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::CONTROL | KeyModifiers::ALT);
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) if self.menu.is_some() => match self.hit_at(m.column, m.row) {
                Some(Hit::MenuItem(i)) => self.menu_run(i),
                // a click elsewhere only closes the menu
                _ => self.menu = None,
            },
            MouseEventKind::Down(MouseButton::Right) => {
                self.menu = None;
                if let Some(Hit::Card(c)) = self.hit_at(m.column, m.row) {
                    self.focus = "home".into();
                    self.open_menu(c, Some((m.column, m.row)));
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if self.help {
                    self.help = false;
                    return;
                }
                match self.hit_at(m.column, m.row) {
                    Some(Hit::Side(row)) => {
                        self.focus = "library".into();
                        self.open_side(row);
                    }
                    Some(Hit::Track(vi)) => {
                        self.focus = "home".into();
                        // a second click on the same row soon after plays it
                        let double = self.last_click.is_some_and(|(t, i)| i == vi && t.elapsed() < DOUBLE_CLICK);
                        if let Some(v) = self.view.as_mut() {
                            v.sel = vi;
                        }
                        if double {
                            self.last_click = None;
                            self.play_row(vi);
                        } else {
                            self.last_click = Some((Instant::now(), vi));
                        }
                    }
                    Some(Hit::SortCol(c)) => self.sort(c),
                    Some(Hit::Card(c)) => {
                        self.focus = "home".into();
                        self.play_card(c);
                    }
                    Some(Hit::ShelfScroll(row, d)) => self.home_view.scroll_shelf(row, d),
                    Some(Hit::MenuItem(_)) => {}
                    Some(Hit::Focus(s)) => self.focus = s,
                    Some(Hit::Divider(i)) => {
                        if let Some(d) = self.dividers.get(i) {
                            let (boundary, grab) = match d.axis {
                                crate::layout::Axis::Horizontal => (d.grip.x + 1, m.column),
                                crate::layout::Axis::Vertical => (d.grip.y + 1, m.row),
                            };
                            self.drag_offset = boundary as i32 - grab as i32;
                        }
                        self.drag = Some(Hit::Divider(i));
                    }
                    Some(Hit::Action(a)) => self.run(a),
                    Some(Hit::PaletteItem(i)) => {
                        if let Some(a) = self.palette_items().get(i).copied() {
                            self.palette = None;
                            self.run(a);
                        }
                    }
                    Some(Hit::Bar(bar, r)) => {
                        let max = match bar {
                            Bar::Seek => self.duration_ms(),
                            Bar::Volume => 100,
                        };
                        if max > 0 {
                            self.nudge = Nudge::default();
                            self.scrub = Some(Scrub::press(bar, r, max, fx, precise));
                        }
                    }
                    None => self.palette = None,
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(s) = self.scrub.as_mut() {
                    s.drag(fx, coarse);
                    // volume is heard while dragging (deduped per percent); the seek waits for the release
                    if s.bar == Bar::Volume {
                        let v = s.value as u8;
                        self.set_volume(v);
                    }
                } else if let Some(Hit::Divider(i)) = self.drag.clone()
                    && let Some(d) = self.dividers.get(i).cloned()
                {
                    let pos = if d.axis == crate::layout::Axis::Vertical { m.row } else { m.column };
                    self.layout.drag(&d, (pos as i32 + self.drag_offset).max(0) as u16);
                }
            }
            MouseEventKind::Up(_) => {
                self.drag = None;
                if let Some(s) = self.scrub.take() {
                    match s.bar {
                        Bar::Seek => self.seek_to(s.value),
                        Bar::Volume => self.set_volume(s.value as u8),
                    }
                }
            }
            MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => {
                let d = if m.kind == MouseEventKind::ScrollLeft { -1 } else { 1 };
                if let Some(row) = self.home_row_at(m.column, m.row) {
                    self.home_view.scroll_shelf(row, d);
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let up = matches!(m.kind, MouseEventKind::ScrollUp);
                let sign = if up { 1 } else { -1 };
                let hit = self.hit_at(m.column, m.row);
                let over_list = matches!(&hit, Some(Hit::Track(_) | Hit::SortCol(_))) || (self.view.is_some() && matches!(&hit, Some(Hit::Focus(s)) if s == "home"));
                if over_list {
                    let len = self.view_order().len();
                    if let Some(v) = self.view.as_mut() {
                        v.scroll(-sign * 3, len);
                    }
                    return;
                }
                if matches!(&hit, Some(Hit::Side(_))) || matches!(&hit, Some(Hit::Focus(s)) if s == "library") {
                    let n = crate::sidebar::len(self);
                    self.sidebar.scroll(-sign * 3, n);
                    return;
                }
                let over_home = matches!(&hit, Some(Hit::Card(_) | Hit::ShelfScroll(..))) || matches!(&hit, Some(Hit::Focus(s)) if s == "home");
                if over_home && self.menu.is_none() {
                    // Shift+wheel scrolls the Shelf under the pointer sideways
                    match self.home_row_at(m.column, m.row) {
                        Some(row) if coarse => self.home_view.scroll_shelf(row, -sign as i32),
                        _ => self.home_view.scroll_rows(-sign as i32),
                    }
                    return;
                }
                match hit {
                    Some(Hit::MenuItem(_)) => {
                        if let Some(menu) = self.menu.as_mut() {
                            menu.selected = if up { menu.selected.saturating_sub(1) } else { (menu.selected + 1).min(menu.items.len().saturating_sub(1)) };
                        }
                    }
                    Some(Hit::Bar(Bar::Seek, _)) => self.nudge_seek(sign * if coarse { 5000 } else { 1000 }),
                    Some(Hit::Bar(Bar::Volume, _)) => self.nudge_volume(sign * if coarse { 5 } else { 1 }),
                    Some(Hit::PaletteItem(_)) => {
                        let n = self.palette_items().len();
                        if let Some(p) = self.palette.as_mut() {
                            p.selected = if up { p.selected.saturating_sub(1) } else { (p.selected + 1).min(n.saturating_sub(1)) };
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

impl App {
    /// The Home row under the pointer: a Card's row, or a Shelf title's ‹ › buttons.
    fn home_row_at(&self, x: u16, y: u16) -> Option<usize> {
        match self.hit_at(x, y)? {
            Hit::Card(c) => self.home_view.rows.iter().position(|r| r.cards.contains(&c)),
            Hit::ShelfScroll(row, _) => Some(row),
            _ => None,
        }
    }
}

/// `spotify:album:abc` → `https://open.spotify.com/album/abc`
pub fn web_link(uri: &str) -> String {
    let parts: Vec<&str> = uri.split(':').collect();
    match parts.as_slice() {
        ["spotify", rest @ ..] if rest.len() >= 2 => format!("https://open.spotify.com/{}", rest.join("/")),
        _ => uri.to_string(),
    }
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

fn fuzzy(hay: &str, q: &str) -> bool {
    let mut it = hay.chars();
    q.chars().filter(|c| !c.is_whitespace()).all(|c| it.any(|h| h == c))
}
