use crate::audio::{Audio, AudioFrame};
use crate::gfx::{self, Fb, KittyImage, Rgb};
use crate::mock::{self, Track};
use crate::ui::NAMES;
use crate::viz;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hero {
    /// index into blackhole::LOOKS
    BlackHole(usize),
    Warp,
    Spectrum,
    Matrix,
    Scope,
    History,
}

pub const LOOKS: [crate::blackhole::DiskLook; 7] = [
    crate::blackhole::INFERNO,
    crate::blackhole::GARGANTUA,
    crate::blackhole::M87,
    crate::blackhole::QUASAR,
    crate::blackhole::BLAZAR,
    crate::blackhole::EMBER,
    crate::blackhole::PIXEL_EMBER,
];

impl Hero {
    pub fn name(self) -> String {
        match self {
            Hero::BlackHole(i) => format!("black hole · {}", LOOKS[i % LOOKS.len()].name),
            Hero::Warp => "warp starfield".into(),
                        Hero::Spectrum => "spectrum".into(),
            Hero::Matrix => "matrix rain".into(),
            Hero::Scope => "oscilloscope".into(),
            Hero::History => "loudness history".into(),
        }
    }
}

/// Each design curates the visuals that suit its slot, so swapping never goes off-brand.
pub fn slot_options(dir: usize) -> &'static [Hero] {
    match dir {
        0 => &[Hero::BlackHole(6), Hero::BlackHole(0), Hero::BlackHole(1), Hero::BlackHole(2), Hero::BlackHole(3), Hero::Warp, Hero::Spectrum],
        1 => &[Hero::Spectrum, Hero::Scope, Hero::History],
        2 => &[Hero::Spectrum, Hero::History, Hero::BlackHole(4)],
        3 => &[Hero::BlackHole(6), Hero::BlackHole(0), Hero::BlackHole(1), Hero::BlackHole(2), Hero::BlackHole(3), Hero::BlackHole(4)],
        _ => &[Hero::Matrix, Hero::BlackHole(1), Hero::Spectrum, Hero::Warp],
    }
}

/// Seconds without input before the black hole drifts in over the UI (screensaver).
pub const IDLE_SECS: f32 = 30.0;

#[derive(Clone, Debug)]
pub enum Act {
    None,
    Dir(usize),
    PlayPause,
    Next,
    Prev,
    Seek(Rect),
    SeekTo(f32),
    Volume(Rect),
    Like,
    Sidebar(usize),
    SidebarScroll,
    CycleHero,
    ToggleHole,
    Variant,
    TogglePixel,
    ToggleAudio,
    OpenPalette,
    Help,
    Palette(Box<Act>),
}

pub struct PaletteState {
    pub query: String,
    pub sel: usize,
}

pub struct App {
    pub dir: usize,
    pub heroes: [usize; 6],
    pub track_idx: usize,
    pub playing: bool,
    pub pos: f32,
    pub volume: f32,
    pub liked: bool,
    pub t: f32,
    pub dt: f32,
    pub fps: f32,
    pub audio: Audio,
    pub frame: AudioFrame,
    pub phase: f32,
    pub scroll: f32,
    pub warp: viz::Warp,
    pub matrix: viz::Matrix,
    pub scope: viz::Scope,
    pub history: viz::History,
    hist_acc: f32,
    /// the roaming hole that lenses the UI (on by default in the Singularity design)
    /// colour variant per design (t cycles)
    pub variant: [usize; 6],
    pub idle_for: f32,
    pub idle_forced: bool,
    pub gpu: Option<crate::gpu::Gpu>,
    pub sidebar_sel: usize,
    pub palette: Option<PaletteState>,
    pub help: bool,
    pub toast: Option<(String, f32)>,
    pub quit: bool,
    /// brightness multiplier for animated visuals 
    pub dim: f32,
    // render tiers
    pub pixel: bool,
    pub kitty_ok: bool,
    pub cell_px: (f32, f32),
    pub kitty_queue: Vec<KittyImage>,
    pub kitty_sent: HashMap<u32, u64>,
    pub kitty_keys_this_frame: HashMap<u32, u64>,
    hits: Vec<(Rect, Act)>,
    drag: Option<Act>,
}

impl App {
    pub fn new(kitty_ok: bool, cell_px: (f32, f32)) -> App {
        App {
            dir: 0,
            heroes: [0; 6],
            track_idx: 0,
            playing: true,
            pos: 31.0,
            volume: 0.7,
            liked: true,
            t: 0.0,
            dt: 0.016,
            fps: 0.0,
            audio: Audio::start(),
            frame: AudioFrame::default(),
            phase: 0.0,
            scroll: 0.0,
            warp: viz::Warp::new(),
            matrix: viz::Matrix::new(),
            scope: viz::Scope::new(),
            history: viz::History::new(),
            hist_acc: 0.0,
            variant: [0; 6],
            idle_for: 0.0,
            idle_forced: false,
            gpu: crate::gpu::Gpu::new(),
            sidebar_sel: 5,
            palette: None,
            help: false,
            toast: None,
            quit: false,
            dim: 1.0,
            pixel: kitty_ok,
            kitty_ok,
            cell_px,
            kitty_queue: vec![],
            kitty_sent: HashMap::new(),
            kitty_keys_this_frame: HashMap::new(),
            hits: vec![],
            drag: None,
        }
    }

    pub fn track(&self) -> &'static Track {
        if self.track_idx == 0 { &mock::NOW } else { &mock::QUEUE[(self.track_idx - 1) % mock::QUEUE.len()] }
    }

    pub fn upcoming(&self) -> Vec<&'static Track> {
        (1..=mock::QUEUE.len()).map(|k| {
            let i = (self.track_idx + k) % (mock::QUEUE.len() + 1);
            if i == 0 { &mock::NOW } else { &mock::QUEUE[i - 1] }
        }).collect()
    }

    pub fn tick(&mut self, dt: f32) {
        self.dt = dt;
        self.t += dt;
        self.fps = self.fps * 0.95 + (1.0 / dt.max(1e-3)) * 0.05;
        if self.playing {
            self.pos += dt;
            if self.pos >= self.track().secs as f32 {
                self.skip(1);
            }
        }
        self.frame = self.audio.update(self.t, dt, self.playing).clone();
        let f = &self.frame;
        self.phase += dt * (0.45 + f.energy * 0.5);
        self.scroll += dt * (0.6 + f.energy * 2.5 + f.beat * 1.2);
        self.warp.step(dt, &self.frame.clone());
        self.idle_for += dt;
        self.hist_acc += dt;
        if self.hist_acc > 0.1 {
            self.hist_acc = 0.0;
            self.history.push(self.frame.rms * 3.0 + self.frame.bass * 0.4);
        }
        if let Some((_, ttl)) = &mut self.toast {
            *ttl -= dt;
            if *ttl <= 0.0 {
                self.toast = None;
            }
        }
    }

    fn skip(&mut self, d: i32) {
        let n = (mock::QUEUE.len() + 1) as i32;
        self.track_idx = ((self.track_idx as i32 + d).rem_euclid(n)) as usize;
        self.pos = 0.0;
    }

    pub fn say(&mut self, s: impl Into<String>) {
        self.toast = Some((s.into(), 2.0));
    }

    // ───────────── render-tier plumbing ─────────────

    pub fn begin_frame(&mut self) {
        self.hits.clear();
        self.kitty_queue.clear();
        self.kitty_keys_this_frame.clear();
    }

    pub fn hit(&mut self, r: Rect, a: Act) {
        self.hits.push((r, a));
    }

    /// Pixel tier is used only when supported, enabled, and no overlay is covering the screen.
    pub fn pixel_active(&self) -> bool {
        self.pixel && self.kitty_ok && self.palette.is_none() && !self.help
    }

    /// A framebuffer sized for `r` in the active render tier.
    pub fn fb_for(&self, r: Rect, _animated: bool, max_w: usize) -> Fb {
        if self.pixel_active() {
            let pw = r.width as f32 * self.cell_px.0;
            let ph = r.height as f32 * self.cell_px.1;
            let s = (max_w as f32 / pw).min(1.0);
            Fb::new((pw * s) as usize, (ph * s) as usize)
        } else {
            Fb::new(r.width as usize, r.height as usize * 2)
        }
    }

    pub fn idle(&self) -> bool {
        self.idle_forced || self.idle_for > IDLE_SECS
    }

    /// Any input wakes the UI; returns true if it was idle (the input is then swallowed).
    fn wake(&mut self) -> bool {
        let was = self.idle();
        self.idle_for = 0.0;
        self.idle_forced = false;
        was
    }

    /// Render the black hole on the GPU if there is one, else on the CPU. Pixel-art looks render at
    /// a quarter of the resolution, get dithered onto a small palette, and are upscaled crisply.
    pub fn render_bh(&mut self, r: Rect, max_w: usize, st: crate::blackhole::Style) -> Fb {
        let full = self.fb_for(r, true, max_w);
        let pixel = st.look.name == "pixel ember";
        let k = if pixel { if self.pixel_active() { 5 } else { 1 } } else { 1 };
        let mut fb = Fb::new((full.w / k).max(1), (full.h / k).max(1));
        let (t, fr) = (self.phase, self.frame.clone());
        let levels = if pixel { 5.0 } else { 0.0 };
        if let Some(g) = self.gpu.as_mut() {
            g.render(&mut fb, t, &fr, &st, levels);
        } else {
            crate::blackhole::render(&mut fb, t, &fr, &st);
            if pixel {
                crate::blackhole::quantize(&mut fb, levels, &crate::gpu::PIXEL_RAMP);
            }
        }
        if k > 1 { fb.upscale(k) } else { fb }
    }

    pub fn hero(&self) -> Hero {
        let opts = slot_options(self.dir);
        opts[self.heroes[self.dir] % opts.len()]
    }

    pub fn present(&mut self, buf: &mut Buffer, r: Rect, id: u32, animated: bool, mut fb: Fb) {
        if animated && self.dim < 1.0 {
            for p in fb.px.iter_mut() {
                *p = p.scale(self.dim);
            }
        }
        if self.pixel_active() {
            let overlay = !fb.alpha.is_empty();
            if !overlay {
                gfx::fill(buf, r, ratatui::style::Color::Reset);
            }
            let key = if animated {
                (self.t * 1000.0) as u64 ^ 0xa5a5
            } else {
                // static content: only re-send when geometry changes
                (r.x as u64) | (r.y as u64) << 16 | (r.width as u64) << 32 | (r.height as u64) << 48
            };
            self.kitty_keys_this_frame.insert(id, key);
            if self.kitty_sent.get(&id) != Some(&key) {
                let z = if overlay { 5 } else if animated { -2 } else { -1 };
                self.kitty_queue.push(KittyImage { id, rect: r, fb, z });
            }
        } else {
            fb.blit_halfblocks(buf, r);
        }
    }

    pub fn pixel_pane(&mut self, buf: &mut Buffer, r: Rect, id: u32, animated: bool, max_w: usize, render: impl FnOnce(&mut Fb)) {
        if r.width == 0 || r.height == 0 {
            return;
        }
        // skip expensive re-render of static images that are already on screen
        if self.pixel_active() && !animated {
            let key = (r.x as u64) | (r.y as u64) << 16 | (r.width as u64) << 32 | (r.height as u64) << 48;
            if self.kitty_sent.get(&id) == Some(&key) {
                gfx::fill(buf, r, ratatui::style::Color::Reset);
                self.kitty_keys_this_frame.insert(id, key);
                return;
            }
        }
        let mut fb = self.fb_for(r, animated, max_w);
        render(&mut fb);
        self.present(buf, r, id, animated, fb);
    }

    pub fn bh_style(&self, look: usize, rh: f32, center: (f32, f32), alpha: bool) -> crate::blackhole::Style {
        let tint = if self.dir == 4 { Some(Rgb::hex(0x33ff66)) } else { None };
        crate::blackhole::Style { look: LOOKS[look % LOOKS.len()], tint, rh, center, alpha }
    }

    // ───────────── input ─────────────

    pub fn palette_items(&self) -> Vec<(String, String, Act)> {
        let mut items: Vec<(String, String, Act)> = NAMES
            .iter()
            .enumerate()
            .map(|(i, n)| (format!("Design: {n}"), format!("{}", i + 1), Act::Dir(i)))
            .collect();
        items.extend([
            (if self.playing { "Pause".into() } else { "Play".into() }, "Space".into(), Act::PlayPause),
            ("Next track".into(), "n".into(), Act::Next),
            ("Previous track".into(), "b".into(), Act::Prev),
            ("Like / unlike current track".into(), "l".into(), Act::Like),
            ("Swap visual in hero slot".into(), "v".into(), Act::CycleHero),
            ("Preview idle screensaver (black hole)".into(), "i".into(), Act::ToggleHole),
            ("Cycle colour palette of this design".into(), "t".into(), Act::Variant),
            ("Toggle pixel / cell render tier".into(), "p".into(), Act::TogglePixel),
            ("Toggle live / synthetic audio".into(), "a".into(), Act::ToggleAudio),
            ("Go to: Liked Songs".into(), "".into(), Act::Sidebar(0)),
            ("Go to: Discover Weekly".into(), "".into(), Act::Sidebar(3)),
            ("Go to: night drive".into(), "".into(), Act::Sidebar(5)),
            ("Help: what keys exist?".into(), "?".into(), Act::Help),
        ]);
        let q = self.palette.as_ref().map(|p| p.query.to_lowercase()).unwrap_or_default();
        items.retain(|(l, _, _)| fuzzy(&l.to_lowercase(), &q));
        items
    }

    pub fn run(&mut self, a: Act) {
        match a {
            Act::None | Act::SidebarScroll => {}
            Act::Dir(i) => {
                self.dir = i;
                self.say(format!("Design {}: {}", i + 1, NAMES[i]));
            }
            Act::PlayPause => self.playing = !self.playing,
            Act::Next => self.skip(1),
            Act::Prev => {
                if self.pos > 3.0 { self.pos = 0.0 } else { self.skip(-1) }
            }
            Act::Seek(_) | Act::Volume(_) => {}
            Act::SeekTo(s) => self.pos = s.clamp(0.0, self.track().secs as f32 - 1.0),
            Act::Like => {
                self.liked = !self.liked;
                self.say(if self.liked { "Added to Liked Songs" } else { "Removed from Liked Songs" });
            }
            Act::Sidebar(i) => {
                self.sidebar_sel = i;
                self.say(format!("Open: {}", mock::LIBRARY[i].1));
            }
            Act::CycleHero => {
                self.heroes[self.dir] += 1;
                self.say(format!("Visual: {}", self.hero().name()));
            }
            Act::ToggleHole => {
                self.idle_forced = true;
            }
            Act::Variant => {
                let n = crate::directions::variant_count(self.dir);
                if n <= 1 {
                    self.say("This design has a single palette");
                } else {
                    self.variant[self.dir] = (self.variant[self.dir] + 1) % n;
                    self.say(format!("Palette: {}", crate::directions::variant_name(self.dir, self.variant[self.dir])));
                }
            }
            Act::TogglePixel => {
                if !self.kitty_ok {
                    self.say("Pixel tier needs kitty/ghostty/wezterm");
                } else {
                    self.pixel = !self.pixel;
                    self.say(if self.pixel { "Pixel tier (kitty graphics)" } else { "Cell tier (half-blocks)" });
                }
            }
            Act::ToggleAudio => {
                self.audio.force_synthetic = !self.audio.force_synthetic;
                self.say(if self.audio.force_synthetic { "Synthetic audio" } else { "Live system audio" });
            }
            Act::OpenPalette => self.palette = Some(PaletteState { query: String::new(), sel: 0 }),
            Act::Help => {
                self.palette = None;
                self.help = true;
            }
            Act::Palette(inner) => {
                self.palette = None;
                self.run(*inner);
            }
        }
    }

    pub fn key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if self.wake() && !(ctrl && k.code == KeyCode::Char('c')) {
            return;
        }
        if ctrl && k.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if let Some(p) = &mut self.palette {
            match k.code {
                KeyCode::Esc => self.palette = None,
                KeyCode::Up => p.sel = p.sel.saturating_sub(1),
                KeyCode::Down => p.sel += 1,
                KeyCode::Backspace => {
                    p.query.pop();
                    p.sel = 0;
                }
                KeyCode::Enter => {
                    let items = self.palette_items();
                    let sel = self.palette.as_ref().map(|p| p.sel).unwrap_or(0);
                    if let Some((_, _, a)) = items.get(sel.min(items.len().saturating_sub(1))) {
                        let a = a.clone();
                        self.palette = None;
                        self.run(a);
                    }
                }
                KeyCode::Char(c) => {
                    p.query.push(c);
                    p.sel = 0;
                }
                _ => {}
            }
            let n = self.palette_items().len();
            if let Some(p) = self.palette.as_mut() {
                p.sel = p.sel.min(n.saturating_sub(1));
            }
            return;
        }
        if self.help {
            self.help = false;
            return;
        }
        match k.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('k') if ctrl => self.run(Act::OpenPalette),
            KeyCode::Char(':') => self.run(Act::OpenPalette),
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char(' ') => self.run(Act::PlayPause),
            KeyCode::Right => self.pos = (self.pos + 5.0).min(self.track().secs as f32 - 1.0),
            KeyCode::Left => self.pos = (self.pos - 5.0).max(0.0),
            KeyCode::Char('n') => self.run(Act::Next),
            KeyCode::Char('b') => self.run(Act::Prev),
            KeyCode::Char('l') => self.run(Act::Like),
            KeyCode::Char('v') => self.run(Act::CycleHero),
            KeyCode::Char('i') => self.run(Act::ToggleHole),
            KeyCode::Char('t') => self.run(Act::Variant),
            KeyCode::Char('p') => self.run(Act::TogglePixel),
            KeyCode::Char('a') => self.run(Act::ToggleAudio),
            KeyCode::Char('+') | KeyCode::Char('=') => self.volume = (self.volume + 0.05).min(1.0),
            KeyCode::Char('-') => self.volume = (self.volume - 0.05).max(0.0),
            KeyCode::Tab => self.run(Act::Dir((self.dir + 1) % NAMES.len())),
            KeyCode::BackTab => self.run(Act::Dir((self.dir + NAMES.len() - 1) % NAMES.len())),
            KeyCode::Up => self.sidebar_sel = self.sidebar_sel.saturating_sub(1),
            KeyCode::Down => self.sidebar_sel = (self.sidebar_sel + 1).min(mock::LIBRARY.len() - 1),
            KeyCode::Char(c @ '1'..='5') => self.run(Act::Dir(c as usize - '1' as usize)),
            _ => {}
        }
    }

    fn hit_at(&self, x: u16, y: u16) -> Option<Act> {
        self.hits.iter().rev().find(|(r, _)| x >= r.x && x < r.right() && y >= r.y && y < r.bottom()).map(|(_, a)| a.clone())
    }

    fn apply_drag(&mut self, a: &Act, x: u16) {
        match a {
            Act::Seek(r) => {
                let f = (x.saturating_sub(r.x) as f32 / r.width.max(1) as f32).clamp(0.0, 1.0);
                self.pos = f * (self.track().secs as f32 - 1.0);
            }
            Act::Volume(r) => {
                self.volume = ((x.saturating_sub(r.x) + 1) as f32 / r.width.max(1) as f32).clamp(0.0, 1.0);
            }
            _ => {}
        }
    }

    pub fn mouse(&mut self, m: MouseEvent) {
        if !matches!(m.kind, MouseEventKind::Moved) && self.wake() {
            return;
        }
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if self.help {
                    self.help = false;
                    return;
                }
                let Some(a) = self.hit_at(m.column, m.row) else {
                    self.palette = None;
                    return;
                };
                if matches!(a, Act::Seek(_) | Act::Volume(_)) {
                    self.apply_drag(&a, m.column);
                    self.drag = Some(a);
                } else {
                    self.run(a);
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(a) = self.drag.clone() {
                    self.apply_drag(&a, m.column);
                }
            }
            MouseEventKind::Up(_) => self.drag = None,
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let down = matches!(m.kind, MouseEventKind::ScrollDown);
                match self.hit_at(m.column, m.row) {
                    Some(Act::Sidebar(_)) | Some(Act::SidebarScroll) => {
                        self.sidebar_sel = if down { (self.sidebar_sel + 1).min(mock::LIBRARY.len() - 1) } else { self.sidebar_sel.saturating_sub(1) };
                    }
                    Some(Act::Volume(_)) => self.volume = (self.volume + if down { -0.05 } else { 0.05 }).clamp(0.0, 1.0),
                    Some(Act::Palette(_)) => {
                        if let Some(p) = &mut self.palette {
                            p.sel = if down { p.sel + 1 } else { p.sel.saturating_sub(1) };
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
