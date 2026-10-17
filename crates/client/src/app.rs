//! Client state and input. Rendering lives in `ui`; the connection in `conn`.
//! Every interactive element registers a hit-box while drawing, so the mouse works everywhere.

use crate::layout::{Divider, Node};
use crate::scrub::{Bar, Scrub};
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
    FocusNext,
    ResetLayout,
    Palette,
    Help,
    Quit,
}

impl Action {
    pub const ALL: [Action; 19] = [
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
            Action::SeekForward => "→",
            Action::SeekBack => "←",
            Action::SeekForwardFine => "Shift+→",
            Action::SeekBackFine => "Shift+←",
            Action::VolumeUp => "+",
            Action::VolumeDown => "-",
            Action::Shuffle => "s",
            Action::Repeat => "r",
            Action::Login | Action::Logout | Action::RefreshHome => "",
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
            commands,
        }
    }

    // ── frame bookkeeping ─────────────────────────────────────────────

    pub fn begin_frame(&mut self) {
        self.hits.clear();
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
        if self.help {
            self.help = false;
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
            KeyCode::Right if k.modifiers.contains(KeyModifiers::SHIFT) => self.run(Action::SeekForwardFine),
            KeyCode::Left if k.modifiers.contains(KeyModifiers::SHIFT) => self.run(Action::SeekBackFine),
            KeyCode::Right => self.run(Action::SeekForward),
            KeyCode::Left => self.run(Action::SeekBack),
            KeyCode::Char('+') | KeyCode::Char('=') => self.run(Action::VolumeUp),
            KeyCode::Char('-') => self.run(Action::VolumeDown),
            KeyCode::Char('s') => self.run(Action::Shuffle),
            KeyCode::Char('r') => self.run(Action::Repeat),
            KeyCode::Tab => self.run(Action::FocusNext),
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
            MouseEventKind::Down(MouseButton::Left) => {
                if self.help {
                    self.help = false;
                    return;
                }
                match self.hit_at(m.column, m.row) {
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
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let up = matches!(m.kind, MouseEventKind::ScrollUp);
                let sign = if up { 1 } else { -1 };
                match self.hit_at(m.column, m.row) {
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

fn fuzzy(hay: &str, q: &str) -> bool {
    let mut it = hay.chars();
    q.chars().filter(|c| !c.is_whitespace()).all(|c| it.any(|h| h == c))
}
