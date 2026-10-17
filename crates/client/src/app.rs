//! Client state and input. Rendering lives in `ui`; the connection in `conn`.
//! Every interactive element registers a hit-box while drawing, so the mouse works everywhere.

use crate::layout::{Divider, Node};
use crate::theme::Theme;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use singularity_proto::{Command, EngineStatus, PlaybackState};
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
    VolumeUp,
    VolumeDown,
    FocusNext,
    ResetLayout,
    Palette,
    Help,
    Quit,
}

impl Action {
    pub const ALL: [Action; 12] = [
        Action::TogglePlay,
        Action::Next,
        Action::Previous,
        Action::SeekForward,
        Action::SeekBack,
        Action::VolumeUp,
        Action::VolumeDown,
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
            Action::VolumeUp => "Volume up",
            Action::VolumeDown => "Volume down",
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
            Action::VolumeUp => "+",
            Action::VolumeDown => "-",
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
    Seek(Rect),
    Volume(Rect),
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
    pub focus: String,
    pub palette: Option<Palette>,
    pub help: bool,
    pub toast: Option<(String, Instant)>,
    pub quit: bool,
    pub dividers: Vec<Divider>,
    hits: Vec<(Rect, Hit)>,
    drag: Option<Hit>,
    /// boundary minus grab position, so a divider moves exactly with the mouse whichever grip cell was grabbed
    drag_offset: i32,
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
            focus: "home".into(),
            palette: None,
            help: false,
            toast: None,
            quit: false,
            dividers: vec![],
            hits: vec![],
            drag: None,
            drag_offset: 0,
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
        let pos = self.playback.position_at(singularity_proto::now_ms());
        match a {
            Action::TogglePlay => self.send(Command::TogglePlay),
            Action::Next => self.send(Command::Next),
            Action::Previous => self.send(Command::Previous),
            Action::SeekForward => self.send(Command::Seek { position_ms: pos + 5000 }),
            Action::SeekBack => self.send(Command::Seek { position_ms: pos.saturating_sub(5000) }),
            Action::VolumeUp => self.send(Command::SetVolume { percent: self.playback.volume.saturating_add(5).min(100) }),
            Action::VolumeDown => self.send(Command::SetVolume { percent: self.playback.volume.saturating_sub(5) }),
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
            KeyCode::Char('q') | KeyCode::Esc => self.run(Action::Quit),
            KeyCode::Char(' ') => self.run(Action::TogglePlay),
            KeyCode::Char('n') => self.run(Action::Next),
            KeyCode::Char('b') => self.run(Action::Previous),
            KeyCode::Right => self.run(Action::SeekForward),
            KeyCode::Left => self.run(Action::SeekBack),
            KeyCode::Char('+') | KeyCode::Char('=') => self.run(Action::VolumeUp),
            KeyCode::Char('-') => self.run(Action::VolumeDown),
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

    // ── mouse ─────────────────────────────────────────────────────────

    pub fn mouse(&mut self, m: MouseEvent) {
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
                    Some(h @ (Hit::Seek(_) | Hit::Volume(_))) => {
                        self.apply_drag(&h, m.column);
                        self.drag = Some(h);
                    }
                    None => self.palette = None,
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(h) = self.drag.clone() {
                    self.apply_drag(&h, if matches!(h, Hit::Divider(i) if self.dividers.get(i).is_some_and(|d| d.axis == crate::layout::Axis::Vertical)) { m.row } else { m.column });
                }
            }
            MouseEventKind::Up(_) => self.drag = None,
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let up = matches!(m.kind, MouseEventKind::ScrollUp);
                match self.hit_at(m.column, m.row) {
                    Some(Hit::Volume(_)) => self.run(if up { Action::VolumeUp } else { Action::VolumeDown }),
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

    fn apply_drag(&mut self, h: &Hit, pos: u16) {
        match h {
            Hit::Divider(i) => {
                if let Some(d) = self.dividers.get(*i).cloned() {
                    self.layout.drag(&d, (pos as i32 + self.drag_offset).max(0) as u16);
                }
            }
            Hit::Seek(r) => {
                let dur = self.playback.item.as_ref().map(|i| i.duration_ms).unwrap_or(0);
                if dur > 0 {
                    let f = (pos.saturating_sub(r.x) as f64 / r.width.max(1) as f64).clamp(0.0, 1.0);
                    self.send(Command::Seek { position_ms: (f * dur as f64) as u64 });
                }
            }
            Hit::Volume(r) => {
                let f = ((pos.saturating_sub(r.x) + 1) as f64 / r.width.max(1) as f64).clamp(0.0, 1.0);
                self.send(Command::SetVolume { percent: (f * 100.0).round() as u8 });
            }
            _ => {}
        }
    }
}

fn fuzzy(hay: &str, q: &str) -> bool {
    let mut it = hay.chars();
    q.chars().filter(|c| !c.is_whitespace()).all(|c| it.any(|h| h == c))
}
