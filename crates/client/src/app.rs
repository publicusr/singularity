//! Client state and input. Rendering lives in `ui`; the connection in `conn`.
//! Every interactive element registers a hit-box while drawing, so the mouse works everywhere.

use crate::covers::Covers;
use crate::gfx::Kitty;
use crate::home::{CardRef, HomeView};
use crate::layout::{Divider, Node};
use crate::meters::Meters;
use crate::visual::BlackHole;
use crate::visual::well::Hole;
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
    Like,
    NextVariant,
    PickDesign,
    PickTheme,
    SaveLayout,
    ToggleIdle,
    IdlePreview,
    ToggleNotifications,
    Login,
    Logout,
    RefreshHome,
    GoHome,
    Back,
    Forward,
    Search,
    Filter,
    SortTitle,
    SortArtist,
    SortAlbum,
    SortAdded,
    SortDuration,
    NextLook,
    Devices,
    ClearQueue,
    Health,
    ExportListening,
    DeleteListening,
    Setup,
    FocusNext,
    ResetLayout,
    Palette,
    Help,
    Quit,
}

impl Action {
    pub const ALL: [Action; 44] = [
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
        Action::Like,
        Action::NextVariant,
        Action::PickDesign,
        Action::PickTheme,
        Action::SaveLayout,
        Action::ToggleIdle,
        Action::IdlePreview,
        Action::ToggleNotifications,
        Action::Login,
        Action::Logout,
        Action::RefreshHome,
        Action::GoHome,
        Action::Back,
        Action::Forward,
        Action::Search,
        Action::Filter,
        Action::SortTitle,
        Action::SortArtist,
        Action::SortAlbum,
        Action::SortAdded,
        Action::SortDuration,
        Action::NextLook,
        Action::Devices,
        Action::ClearQueue,
        Action::Health,
        Action::ExportListening,
        Action::DeleteListening,
        Action::Setup,
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
            Action::Like => "Like / unlike the current track",
            Action::NextVariant => "Next palette (this design's colours, then your themes)",
            Action::PickDesign => "Design: pick one (layout + colours, live preview)",
            Action::PickTheme => "Theme: pick colours (live preview)",
            Action::SaveLayout => "Save layout as… (comes back in Design)",
            Action::ToggleIdle => "Idle takeover on / off (the hole drifts over the screen)",
            Action::IdlePreview => "Idle takeover: show it now",
            Action::ToggleNotifications => "Track-change notifications on / off",
            Action::Login => "Log in to Spotify",
            Action::Logout => "Log out of Spotify",
            Action::RefreshHome => "Refresh Home",
            Action::GoHome => "Go to Home",
            Action::Back => "Back",
            Action::Forward => "Forward",
            Action::Search => "Search",
            Action::Filter => "Filter this track list",
            Action::SortTitle => "Sort list by title",
            Action::SortArtist => "Sort list by artist",
            Action::SortAlbum => "Sort list by album",
            Action::SortAdded => "Sort list by date added",
            Action::SortDuration => "Sort list by duration",
            Action::NextLook => "Swap this Pane's Widget (the black hole: next Disk look)",
            Action::ClearQueue => "Clear the queue",
            Action::Devices => "Devices: play on another speaker or phone",
            Action::Health => "Health check (Engine, login, audio, stream quality)",
            Action::ExportListening => "Export listening data (log + latest library backup, to Downloads)",
            Action::DeleteListening => "Delete listening data (the log and every library backup)…",
            Action::Setup => "Setup: key, pairing, login, audio check",
            Action::FocusNext => "Focus next pane",
            Action::ResetLayout => "Reset layout",
            Action::Palette => "Command palette",
            Action::Help => "Help: which keys exist?",
            Action::Quit => "Quit (music keeps playing)",
        }
    }
    /// The name `keys.toml` uses.
    pub fn id(self) -> &'static str {
        match self {
            Action::TogglePlay => "play_pause",
            Action::Next => "next",
            Action::Previous => "previous",
            Action::SeekForward => "seek_forward",
            Action::SeekBack => "seek_back",
            Action::SeekForwardFine => "seek_forward_fine",
            Action::SeekBackFine => "seek_back_fine",
            Action::VolumeUp => "volume_up",
            Action::VolumeDown => "volume_down",
            Action::Shuffle => "shuffle",
            Action::Repeat => "repeat",
            Action::Like => "like",
            Action::NextVariant => "next_palette",
            Action::PickDesign => "design",
            Action::PickTheme => "theme",
            Action::SaveLayout => "save_layout",
            Action::ToggleIdle => "idle",
            Action::IdlePreview => "idle_preview",
            Action::ToggleNotifications => "notifications",
            Action::Login => "login",
            Action::Logout => "logout",
            Action::RefreshHome => "refresh_home",
            Action::GoHome => "home",
            Action::Back => "back",
            Action::Forward => "forward",
            Action::Search => "search",
            Action::Filter => "filter",
            Action::SortTitle => "sort_title",
            Action::SortArtist => "sort_artist",
            Action::SortAlbum => "sort_album",
            Action::SortAdded => "sort_added",
            Action::SortDuration => "sort_duration",
            Action::NextLook => "next_look",
            Action::Devices => "devices",
            Action::ClearQueue => "clear_queue",
            Action::Health => "health",
            Action::ExportListening => "export_listening",
            Action::DeleteListening => "delete_listening",
            Action::Setup => "setup",
            Action::FocusNext => "focus_next",
            Action::ResetLayout => "reset_layout",
            Action::Palette => "palette",
            Action::Help => "help",
            Action::Quit => "quit",
        }
    }

    pub fn from_id(id: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.id() == id)
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
    /// the gravity well: a click cycles the Disk look
    Well,
    /// ▶ Play on the open track list
    PlayView,
    /// a search filter chip
    Chip(usize),
    /// a device in the picker
    DeviceRow(usize),
    /// a recent search
    Recent(usize),
    /// a song in the search results
    SearchTrack(usize),
    /// a queue entry, by index in `playback.queue`
    QueueRow(usize),
    /// a synced lyric line: where it starts (ms)
    Lyric(u64),
    /// a name that opens its artist, album or playlist
    Link(singularity_proto::Link),
    /// ▶ Play on an artist page
    PlayArtist,
    /// a popular track on an artist page
    TopTrack(usize),
    /// ♥ / Save / Follow: toggles whether this is in the library
    Save(singularity_proto::Link),
    /// a row of the design or theme picker
    PickerItem(usize),
    /// a Pane's ◂ ▸: the Slot's Widget, and which way to swap it
    Swap(String, i32),
}

/// Two clicks on the same row within this time play it.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// What a context-menu entry does to its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAct {
    Play,
    /// play the open track list from this row
    PlayHere,
    PlayNext,
    Queue,
    Dequeue,
    ClearQueue,
    GoTo,
    CopyLink,
    /// like/unlike, save/remove, follow/unfollow
    Save,
}

/// What a context menu is about.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Card(CardRef),
    /// a row of the open track list, by view position
    Row(usize),
    /// a queue entry: its URI and uid, and whether the user queued it
    Queued { uri: String, uid: String, name: String, queued: bool },
    /// any other track (an artist's popular tracks)
    Track { uri: String, name: String },
}

/// What the main Pane shows; back/forward walk these.
#[derive(Clone, Debug, PartialEq)]
pub enum Page {
    Home,
    List { uri: String, title: String, subtitle: String },
    Artist { uri: String, name: String },
    Search,
}

pub struct Menu {
    /// where it opened (the pointer, or the selected item)
    pub at: (u16, u16),
    pub target: Target,
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

/// The config files being watched for saves.
struct ConfigFiles {
    config: singularity_config::Watched,
    keys: singularity_config::Watched,
    polled: Instant,
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
    /// the open track list (None: the main Pane shows Home or an artist)
    pub view: Option<TrackView>,
    /// the open artist page
    pub artist: Option<crate::artist::ArtistView>,
    /// the search page (kept while you browse its results, so back returns to them)
    pub search: Option<crate::search::SearchView>,
    search_kept: Option<crate::search::SearchView>,
    pub recent: Vec<String>,
    pub devices: Vec<singularity_proto::Device>,
    pub devices_open: bool,
    pub devices_sel: usize,
    artist_pages: HashMap<String, singularity_proto::ArtistPage>,
    /// history: pages to go back and forward to
    pub back: Vec<Page>,
    pub forward: Vec<Page>,
    /// each list's sort, filter and scroll, kept while you browse elsewhere
    list_views: HashMap<String, TrackView>,
    last_click: Option<(Instant, usize)>,
    pub meters: Meters,
    pub stream: Option<singularity_proto::StreamInfo>,
    pub health: Option<singularity_proto::Health>,
    /// the local listening log, summed up (#23)
    pub listening: Option<singularity_proto::ListeningSummary>,
    confirm_delete: Option<Instant>,
    pub health_open: bool,
    health_asked: Option<Instant>,
    pub setup: crate::setup::Setup,
    pub hole: BlackHole,
    pub queue_view: crate::queue::QueueView,
    pub lyrics: Option<singularity_proto::Lyrics>,
    /// first line shown of unsynced lyrics
    pub lyrics_scroll: usize,
    lyrics_asked: Option<String>,
    /// where the hole was drawn this frame (for bending the neighbouring borders)
    pub hole_at: Option<Hole>,
    /// what's liked, saved and followed (#22)
    pub(crate) saved: crate::saved::Saved,
    /// a saved playlist's Saved button was clicked once: a second click soon after removes it
    confirm_remove: Option<(String, Instant)>,
    /// `config.toml` and `keys.toml` (#20), and the files they came from (None in tests)
    pub config: singularity_config::Config,
    pub keymap: crate::keymap::Keymap,
    config_files: Option<ConfigFiles>,
    /// the design on screen (index into `design::DESIGNS`), the user's themes, the picker (#18)
    pub design: usize,
    pub user_themes: Vec<crate::theme::Theme>,
    pub theme_dir: Option<crate::looks::ThemeDir>,
    pub picker: Option<crate::looks::Picker>,
    /// state of the Widgets other designs bring (rain, scope, top)
    pub widgets: crate::widgets::State,
    /// the saved layout on screen (its file name), the ones there are, and the name being typed for a new one (#19)
    pub layout_name: Option<String>,
    pub saved_layouts: Vec<crate::layouts::Saved>,
    pub naming: Option<String>,
    pub layouts_dir: Option<std::path::PathBuf>,
    /// whether we asked the Daemon for spectrum frames
    spectrum_on: bool,
    /// the idle takeover (#17): the last input, since when the hole is out, a preview asked for, and where the hole
    /// sat in its Pane (where it breaks out from)
    last_input: Instant,
    idle_since: Option<Instant>,
    idle_preview: bool,
    pub hole_origin: Option<crate::visual::idle::Origin>,
    last_tick: Instant,
    told_backend: bool,
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
            artist: None,
            search: None,
            search_kept: None,
            recent: vec![],
            devices: vec![],
            devices_open: false,
            devices_sel: 0,
            artist_pages: HashMap::new(),
            back: vec![],
            forward: vec![],
            list_views: HashMap::new(),
            last_click: None,
            meters: Meters::default(),
            stream: None,
            health: None,
            listening: None,
            confirm_delete: None,
            health_open: false,
            health_asked: None,
            setup: Default::default(),
            hole: BlackHole::default(),
            queue_view: Default::default(),
            lyrics: None,
            lyrics_scroll: 0,
            lyrics_asked: None,
            hole_at: None,
            saved: Default::default(),
            confirm_remove: None,
            config: Default::default(),
            keymap: Default::default(),
            config_files: None,
            design: 0,
            user_themes: vec![],
            theme_dir: None,
            picker: None,
            widgets: Default::default(),
            layout_name: None,
            saved_layouts: vec![],
            naming: None,
            layouts_dir: None,
            spectrum_on: false,
            last_input: Instant::now(),
            idle_since: None,
            idle_preview: false,
            hole_origin: None,
            last_tick: Instant::now(),
            told_backend: false,
            commands,
        }
    }

    // ── config & keys ─────────────────────────────────────────────────

    /// Read `config.toml` and `keys.toml` (written with comments if missing) and keep watching them.
    pub fn load_config(&mut self) {
        self.load_config_from(singularity_config::config_path(), singularity_config::keys_path());
    }

    pub fn load_config_from(&mut self, cp: std::path::PathBuf, kp: std::path::PathBuf) {
        // the user's themes first, so a theme named in config.toml can be found
        if let Some(dir) = cp.parent() {
            self.init_theme_dir(dir.join("themes"));
            self.layouts_dir = Some(dir.join("layouts"));
            self.reload_layouts();
        }
        let c = singularity_config::load_config(&cp);
        let k = singularity_config::load_keys(&kp);
        self.config_files = Some(ConfigFiles { config: singularity_config::Watched::new(cp), keys: singularity_config::Watched::new(kp), polled: Instant::now() });
        self.apply_config(c);
        self.apply_keys(k);
    }

    /// A config as read: applied live, its problems shown.
    pub fn apply_config(&mut self, r: singularity_config::Read<singularity_config::Config>) {
        let high = r.value.visuals.gpu == singularity_config::GpuPower::HighPerformance;
        if high != self.hole.high_performance {
            // the next frame picks the adapter again, and says which
            self.hole.set_high_performance(high);
            self.told_backend = false;
        }
        let ui = r.value.ui.clone();
        self.config = r.value;
        self.apply_ui(&ui);
        if !r.problems.is_empty() {
            self.say(r.problems.join(" · "));
        }
    }

    pub fn config_path(&self) -> Option<std::path::PathBuf> {
        self.config_files.as_ref().map(|f| f.config.path.clone())
    }

    pub fn apply_keys(&mut self, r: singularity_config::Read<singularity_config::Keys>) {
        let mut problems = r.problems;
        self.keymap = crate::keymap::Keymap::from_keys(&r.value);
        problems.append(&mut self.keymap.problems.clone());
        if !problems.is_empty() {
            self.say(problems.join(" · "));
        }
    }

    /// Saved files apply at once (checked about once a second).
    fn poll_config(&mut self) {
        let Some(f) = self.config_files.as_mut().filter(|f| f.polled.elapsed() >= Duration::from_secs(1)) else { return };
        f.polled = Instant::now();
        let (config, keys) = (f.config.changed(), f.keys.changed());
        let (cp, kp) = (f.config.path.clone(), f.keys.path.clone());
        if config {
            self.apply_config(singularity_config::load_config(&cp));
        }
        if keys {
            self.apply_keys(singularity_config::load_keys(&kp));
        }
    }

    /// Flip track-change notifications, in `config.toml` (the Daemon picks it up from there).
    fn toggle_notifications(&mut self) {
        let on = !self.config.notifications.track_change;
        if let Some(f) = &self.config_files
            && let Err(e) = singularity_config::set(&f.config.path, singularity_config::DEFAULT_CONFIG, "notifications.track_change", on.into())
        {
            return self.say(format!("Couldn't change config.toml: {e}"));
        }
        self.config.notifications.track_change = on;
        self.say(if on { "Track notifications on" } else { "Track notifications off" });
    }

    /// Typing goes to a text field, so letters aren't shortcuts.
    fn typing(&self) -> bool {
        self.palette.is_some()
            || (crate::setup::wanted(self) && crate::setup::step(self) == crate::setup::Step::Key)
            || self.picker.is_some()
            || self.naming.is_some()
            || self.view.as_ref().is_some_and(|v| v.filtering)
            || (self.search.is_some() && self.focus == "home")
    }

    // ── idle takeover ─────────────────────────────────────────────────

    /// Seconds the hole has been out of its Pane, while the UI is idle.
    pub fn idle_secs(&self) -> Option<f32> {
        self.idle_since.map(|t| t.elapsed().as_secs_f32())
    }

    /// Tests: the hole has been out for `secs`.
    #[cfg(test)]
    pub(crate) fn idle_for(&mut self, secs: f32) {
        self.idle_since = Some(Instant::now() - Duration::from_secs_f32(secs));
    }

    /// Input arrived: the idle clock starts over. True if the UI was idle (that input is then swallowed).
    fn wake(&mut self) -> bool {
        self.last_input = Instant::now();
        self.idle_preview = false;
        self.idle_since.take().is_some()
    }

    /// Shift+I: the takeover off or on, in config.toml.
    fn toggle_idle(&mut self) {
        let on = !self.config.idle.enabled;
        if let Some(path) = self.config_path()
            && let Err(e) = singularity_config::set(&path, singularity_config::DEFAULT_CONFIG, "idle.enabled", on.into())
        {
            return self.say(format!("Couldn't change config.toml: {e}"));
        }
        self.config.idle.enabled = on;
        let mins = self.config.idle.minutes;
        self.say(if on { format!("Idle takeover on (after {mins} min)") } else { "Idle takeover off".into() });
    }

    // ── frame bookkeeping ─────────────────────────────────────────────

    pub fn begin_frame(&mut self) {
        self.hits.clear();
        self.overlays.clear();
        self.hole_at = None;
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

    /// After (re)connecting: ask for spectrum frames if an audio Widget is on screen.
    pub fn on_connected(&mut self) {
        self.spectrum_on = false;
        self.want_spectrum();
        self.send(Command::GetListening);
    }

    /// Spectrum frames flow only while a Widget that moves with the music is on screen (the Daemon's tap idles
    /// otherwise); called on connect and whenever the layout changes.
    pub fn want_spectrum(&mut self) {
        const AUDIO: [&str; 8] = ["fft", "loudness", "levels", "rain", "scope", "nowplaying", "well", "hole"];
        let on = self.layout.compute(Rect::new(0, 0, 200, 60)).slots.iter().any(|(s, _)| AUDIO.contains(&s.as_str()));
        if on != self.spectrum_on && matches!(self.link, Link::Connected { .. }) {
            self.spectrum_on = on;
            self.send(Command::Spectrum { on });
        }
    }

    pub fn say(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    pub(crate) fn send(&mut self, c: Command) {
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
            Action::Like => self.toggle_current(),
            Action::ToggleNotifications => self.toggle_notifications(),
            Action::NextVariant => self.next_variant(),
            Action::PickDesign => self.open_picker(crate::looks::PickerKind::Design),
            Action::PickTheme => self.open_picker(crate::looks::PickerKind::Theme),
            Action::SaveLayout => self.naming = Some(String::new()),
            Action::ToggleIdle => self.toggle_idle(),
            Action::IdlePreview => self.idle_preview = true,
            Action::Login => self.send(Command::Login),
            Action::Logout => self.send(Command::Logout),
            Action::RefreshHome => self.send(Command::GetHome),
            Action::GoHome => self.navigate(Page::Home),
            Action::Back => self.go_back(),
            Action::Search => self.navigate(Page::Search),
            Action::Forward => self.go_forward(),
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
            Action::Devices => {
                self.devices_open = true;
                self.devices_sel = self.devices.iter().position(|d| d.active).unwrap_or(0);
                if matches!(self.link, Link::Connected { .. }) {
                    self.send(Command::GetDevices);
                }
            }
            Action::ClearQueue => {
                if self.playback.queue.iter().any(|q| q.source == "queue") {
                    self.send(Command::ClearQueue);
                    self.say("Queue cleared");
                } else {
                    self.say("Nothing queued");
                }
            }
            Action::ExportListening => self.send(Command::ExportLocalData),
            Action::DeleteListening => {
                // a second time within 10 s confirms
                if self.confirm_delete.take().is_some_and(|t| t.elapsed() < Duration::from_secs(10)) {
                    self.send(Command::DeleteLocalData);
                } else {
                    self.confirm_delete = Some(Instant::now());
                    self.say("Run “Delete listening data” again within 10 s to delete the log and every backup");
                }
            }
            Action::Health => {
                self.health_open = true;
                self.health_asked = None;
            }
            Action::Setup => {
                self.setup.forced = true;
                self.setup.dismissed = false;
            }
            Action::NextLook => {
                let focus = self.focus.clone();
                if !self.swap_widget(&focus, 1) {
                    self.hole.next_look();
                    self.say(format!("Disk look: {}", self.hole.look().name));
                }
            }
            Action::FocusNext => {
                let slots: Vec<String> = self.layout.compute(Rect::new(0, 0, 200, 60)).slots.into_iter().map(|(s, _)| s).collect();
                let i = slots.iter().position(|s| *s == self.focus).map(|i| (i + 1) % slots.len()).unwrap_or(0);
                self.focus = slots[i].clone();
            }
            Action::ResetLayout => {
                let d = self.design;
                let theme = self.theme.id.clone();
                self.set_design(d);
                self.set_theme(&theme);
                self.layout_name = None;
                self.save_ui();
                self.say("Layout reset");
            }
            Action::Palette => self.palette = Some(Palette { query: String::new(), selected: 0 }),
            Action::Help => self.help = true,
            Action::Quit => self.quit = true,
        }
    }

    pub fn palette_items(&self) -> Vec<Action> {
        let q = self.palette.as_ref().map(|p| p.query.to_lowercase()).unwrap_or_default();
        let mut items: Vec<Action> = Action::ALL.into_iter().filter(|a| *a != Action::Palette && fuzzy(&a.label().to_lowercase(), &q)).collect();
        // what you typed, as typed: at the start first, then anywhere, then loose matches
        let q = q.trim();
        items.sort_by_key(|a| {
            let l = a.label().to_lowercase();
            if l.starts_with(q) { 0 } else if l.contains(q) { 1 } else { 2 }
        });
        items
    }

    // ── keyboard ──────────────────────────────────────────────────────

    pub fn key(&mut self, k: KeyEvent) {
        // the key that wakes the idle UI only wakes it
        if self.wake() {
            return;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        // the vim preset's movement keys (j is ↓), unless typing
        let k = match self.keymap.nav(k) {
            Some(code) if !self.typing() => KeyEvent::new(code, KeyModifiers::NONE),
            _ => k,
        };
        if self.palette.is_some() {
            return self.palette_key(k);
        }
        if self.picker.is_some() {
            return self.picker_key(k);
        }
        if self.naming.is_some() {
            return self.naming_key(k);
        }
        if crate::setup::wanted(self) {
            return self.wizard_key(k);
        }
        if self.devices_open {
            match k.code {
                KeyCode::Up => self.devices_sel = self.devices_sel.saturating_sub(1),
                KeyCode::Down => self.devices_sel = (self.devices_sel + 1).min(self.devices.len().saturating_sub(1)),
                KeyCode::Enter => {
                    let i = self.devices_sel;
                    self.transfer_to(i);
                }
                KeyCode::Esc | KeyCode::Char('d') | KeyCode::Char('q') => self.devices_open = false,
                _ => {}
            }
            return;
        }
        if self.health_open {
            if matches!(k.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q')) {
                self.health_open = false;
            }
            return;
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
        // shortcuts with Ctrl or Alt work everywhere (a Pane's own keys are plain)
        if k.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            && let Some(a) = self.keymap.action(k)
        {
            return self.run(a);
        }
        if self.focus == "home" && self.search.is_some() && self.search_key(k) {
            return;
        }
        if self.focus == "home" && !shift && self.view.is_some() && self.tracks_key(k) {
            return;
        }
        if self.focus == "home" && !shift && self.view.is_none() && self.home_key(k) {
            return;
        }
        // Esc on an artist or search page goes back, never quits
        if (self.artist.is_some() || self.search.is_some()) && matches!(k.code, KeyCode::Esc | KeyCode::Backspace) {
            return self.run(Action::Back);
        }
        if self.focus == "library" && !shift && self.side_key(k) {
            return;
        }
        if self.focus == "queue" && !shift && self.queue_key(k) {
            return;
        }
        if k.code == KeyCode::Esc && matches!(self.catalogue, Some(CatalogueStatus::LoginPending { .. })) {
            self.login_hidden = true;
            return;
        }
        // the keymap (keys.toml); arrows a focused Pane didn't use fall through to here and seek
        if let Some(a) = self.keymap.action(k) {
            self.run(a);
        }
    }

    /// Keys for the focused Home; returns false for keys it doesn't use.
    fn home_key(&mut self, k: KeyEvent) -> bool {
        if self.active_feed().is_none_or(|h| h.shelves.is_empty()) {
            return false;
        }
        let v = self.feed_view();
        match k.code {
            KeyCode::Left => v.move_sel(-1, 0),
            KeyCode::Right => v.move_sel(1, 0),
            KeyCode::Up => v.move_sel(0, -1),
            KeyCode::Down => v.move_sel(0, 1),
            KeyCode::PageUp => v.move_sel(0, -(v.visible.max(1) as i32)),
            KeyCode::PageDown => v.move_sel(0, v.visible.max(1) as i32),
            KeyCode::Enter => match v.sel {
                Some(c) => self.open_card(c),
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
        if p.uri == crate::tracks::LIKED {
            self.seed_liked(&p.tracks);
        }
        self.lists.entry(p.uri.clone()).or_default().add(&p);
    }

    /// Open sidebar row `row` (0 = Home, 1 = Search, then the library).
    fn open_side(&mut self, row: usize) {
        self.sidebar.sel = row;
        match row {
            0 => return self.run(Action::GoHome),
            1 => return self.run(Action::Search),
            _ => {}
        }
        let Some(it) = self.library.as_ref().and_then(|l| l.items.get(row - crate::sidebar::FIXED)).cloned() else { return };
        match it.kind.as_str() {
            "folder" => self.say("Folders are listed with their playlists indented below"),
            _ => self.open(&it.uri, &it.name, &it.subtitle, &it.kind),
        }
    }

    /// Open something in the main Pane: an album, playlist or Liked Songs shows its track list, an artist its page.
    pub fn open(&mut self, uri: &str, name: &str, subtitle: &str, kind: &str) {
        // Home's "Liked Songs" shortcut comes as the user's collection URI
        let uri = if kind == "liked" || uri.ends_with(":collection") || uri.contains(":collection:") { crate::tracks::LIKED } else { uri };
        let kind = if kind.is_empty() { uri.split(':').nth(1).unwrap_or("") } else { kind };
        let page = match kind {
            _ if uri == crate::tracks::LIKED => Page::List { uri: uri.into(), title: "Liked Songs".into(), subtitle: subtitle.into() },
            "playlist" | "album" => Page::List { uri: uri.into(), title: name.into(), subtitle: subtitle.into() },
            "artist" => Page::Artist { uri: uri.into(), name: name.into() },
            "show" | "episode" => return self.say("Podcasts arrive in v1.x"),
            _ => return,
        };
        self.navigate(page);
    }

    /// A link: its kind is in its URI.
    pub fn open_link(&mut self, l: &singularity_proto::Link) {
        self.open(&l.uri, &l.name, "", "");
    }

    pub fn current_page(&self) -> Page {
        if self.search.is_some() {
            return Page::Search;
        }
        if let Some(a) = &self.artist {
            return Page::Artist { uri: a.uri.clone(), name: a.name.clone() };
        }
        match &self.view {
            Some(v) => Page::List { uri: v.uri.clone(), title: v.title.clone(), subtitle: v.subtitle.clone() },
            None => Page::Home,
        }
    }

    /// Go to `p`, remembering where we were.
    pub fn navigate(&mut self, p: Page) {
        let here = self.current_page();
        if here != p {
            self.back.push(here);
            self.forward.clear();
        }
        self.show(p);
    }

    fn go_back(&mut self) {
        if let Some(p) = self.back.pop() {
            self.forward.push(self.current_page());
            self.show(p);
        }
    }

    fn go_forward(&mut self) {
        if let Some(p) = self.forward.pop() {
            self.back.push(self.current_page());
            self.show(p);
        }
    }

    /// Put `p` in the main Pane (no history change).
    fn show(&mut self, p: Page) {
        // keep the list we leave as it was (sort, filter, scroll)
        if let Some(v) = self.view.take() {
            self.list_views.insert(v.uri.clone(), v);
        }
        self.artist = None;
        if let Some(s) = self.search.take() {
            self.search_kept = Some(s);
        }
        self.focus = "home".into();
        match p {
            Page::Home => {}
            Page::Search => self.search = Some(self.search_kept.take().unwrap_or_default()),
            Page::List { uri, title, subtitle } => {
                let v = self.list_views.remove(&uri).unwrap_or_else(|| {
                    let mut v = TrackView::new(&uri, &title);
                    v.subtitle = subtitle;
                    v
                });
                self.view = Some(v);
                self.recheck_saved(&uri);
                if self.lists.get(&uri).is_none_or(|l| !l.complete()) {
                    self.lists.remove(&uri);
                    self.send(Command::GetTracks { uri });
                }
            }
            Page::Artist { uri, name } => {
                self.recheck_saved(&uri);
                let mut a = crate::artist::ArtistView::new(&uri, &name);
                match self.artist_pages.get(&uri) {
                    Some(p) => a.set_page(p.clone()),
                    None => self.send(Command::GetArtist { uri }),
                }
                self.artist = Some(a);
            }
        }
    }

    /// An artist page from the Daemon.
    pub fn artist_page(&mut self, p: singularity_proto::ArtistPage) {
        if let Some(a) = self.artist.as_mut().filter(|a| a.uri == p.uri) {
            a.set_page(p.clone());
        }
        self.artist_pages.insert(p.uri.clone(), p);
    }

    /// The Shelves on screen (Home's, or the artist page's) and their view state.
    fn active_feed(&self) -> Option<&singularity_proto::HomeFeed> {
        if let Some(s) = &self.search {
            return Some(&s.feed);
        }
        match &self.artist {
            Some(a) => Some(&a.feed),
            None => self.home.as_ref(),
        }
    }

    pub fn feed_view(&mut self) -> &mut HomeView {
        if let Some(s) = self.search.as_mut() {
            return &mut s.shelves;
        }
        match self.artist.as_mut() {
            Some(a) => &mut a.shelves,
            None => &mut self.home_view,
        }
    }

    /// Keys on the search page: typing goes to the query; arrows move through songs, then the Shelves.
    fn search_key(&mut self, k: KeyEvent) -> bool {
        let Some(s) = self.search.as_mut() else { return false };
        let songs = s.songs();
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL) || k.modifiers.contains(KeyModifiers::ALT);
        match k.code {
            KeyCode::Char(c) if !ctrl => {
                s.query.push(c);
                s.typed();
            }
            KeyCode::Backspace => {
                if s.query.pop().is_none() {
                    return false; // empty: Backspace means back
                }
                s.typed();
                if s.query.trim().is_empty() {
                    s.results = None;
                }
            }
            KeyCode::Esc if !s.query.is_empty() => {
                s.query.clear();
                s.results = None;
                s.typed();
            }
            KeyCode::Tab => {
                let c = s.chip + 1;
                s.set_chip(c);
            }
            KeyCode::BackTab => {
                let c = s.chip + crate::search::CHIPS.len() - 1;
                s.set_chip(c);
            }
            KeyCode::Down if s.query.trim().is_empty() => s.recent_sel = (s.recent_sel + 1).min(self.recent.len().saturating_sub(1)),
            KeyCode::Up if s.query.trim().is_empty() => s.recent_sel = s.recent_sel.saturating_sub(1),
            KeyCode::Enter if s.query.trim().is_empty() => {
                if let Some(q) = self.recent.get(s.recent_sel).cloned() {
                    s.query = q;
                    s.typed();
                }
            }
            KeyCode::Down if s.track_sel.is_none_or(|i| i + 1 < songs) && songs > 0 && s.shelves.sel.is_none() => s.track_sel = Some(s.track_sel.map_or(0, |i| i + 1)),
            KeyCode::Down if s.track_sel.is_some() => {
                // past the last song: into the Shelves
                s.track_sel = None;
                s.shelves.move_sel(0, 0);
            }
            KeyCode::Up if s.track_sel.is_some_and(|i| i > 0) => s.track_sel = s.track_sel.map(|i| i - 1),
            KeyCode::Enter if s.track_sel.is_some() => {
                let i = s.track_sel.unwrap_or(0);
                self.play_search_track(i);
            }
            _ => {
                // the Shelves take arrows and Enter like Home's
                return self.home_key(k);
            }
        }
        true
    }

    /// Move playback to device `i` of the list (and close the picker).
    fn transfer_to(&mut self, i: usize) {
        self.devices_open = false;
        let Some(d) = self.devices.get(i).cloned() else { return };
        if d.active {
            return self.say(format!("Already playing on {}", d.name));
        }
        self.send(Command::Transfer { device_id: d.id });
        self.say(format!("Moving playback to {}", d.name));
    }

    /// The device that's playing, when it isn't Singularity's own.
    pub fn remote_device(&self) -> Option<&singularity_proto::Device> {
        self.devices.iter().find(|d| d.active && !d.is_engine)
    }

    fn play_search_track(&mut self, i: usize) {
        let Some(s) = self.search.as_ref() else { return };
        let Some(t) = s.results.as_ref().and_then(|r| r.tracks.get(i)) else { return };
        let (uri, q) = (t.uri.clone(), s.query.clone());
        crate::search::remember(&mut self.recent, &q);
        crate::search::save_recent(&self.recent);
        self.send(Command::Play { uri: Some(uri) });
    }

    fn play_artist(&mut self) {
        if let Some(a) = &self.artist {
            let uri = a.uri.clone();
            self.send(Command::Play { uri: Some(uri) });
        }
    }

    /// Popular track `i`: the artist's context from that track.
    fn play_top(&mut self, i: usize) {
        let Some(a) = &self.artist else { return };
        let Some(t) = a.page.as_ref().and_then(|p| p.top_tracks.get(i)) else { return };
        let cmd = Command::PlayFrom { context_uri: a.uri.clone(), index: i as u32, track_uri: Some(t.uri.clone()) };
        self.send(cmd);
    }

    /// Open a Home Card (clicking never plays it: the list opens, with ▶ Play there).
    fn open_card(&mut self, c: CardRef) {
        self.feed_view().sel = Some(c);
        // opening a result is what makes a search worth remembering
        if let Some(q) = self.search.as_ref().map(|s| s.query.clone()) {
            crate::search::remember(&mut self.recent, &q);
            crate::search::save_recent(&self.recent);
        }
        if let Some(k) = self.card(c).cloned() {
            // a track has no page: it plays
            if k.kind == "track" {
                return self.send(Command::Play { uri: Some(k.uri) });
            }
            self.open(&k.uri, &k.name, &k.subtitle, &k.kind);
        }
    }

    /// ▶ Play on the open list: the whole context from its first track.
    fn play_view(&mut self) {
        let Some(v) = self.view.as_ref() else { return };
        let cmd = if v.uri == crate::tracks::LIKED {
            Command::PlayFrom { context_uri: v.uri.clone(), index: 0, track_uri: None }
        } else {
            Command::Play { uri: Some(v.uri.clone()) }
        };
        self.send(cmd);
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

    fn queue_key(&mut self, k: KeyEvent) -> bool {
        let n = self.playback.queue.len();
        if n == 0 {
            return false;
        }
        match k.code {
            KeyCode::Up => self.queue_view.move_sel(-1, n),
            KeyCode::Down => self.queue_view.move_sel(1, n),
            KeyCode::Delete | KeyCode::Backspace => {
                let Some(q) = self.playback.queue.get(self.queue_view.sel).cloned() else { return true };
                if q.source == "queue" {
                    let uid = (!q.uid.is_empty()).then_some(q.uid);
                    self.send(Command::RemoveFromQueue { uri: q.item.uri, uid });
                } else {
                    self.say("Only tracks you queued can be removed");
                }
            }
            KeyCode::Menu | KeyCode::F(10) | KeyCode::Enter => {
                let i = self.queue_view.sel;
                self.open_queue_menu(i, None);
            }
            _ => return false,
        }
        true
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
            KeyCode::Menu | KeyCode::F(10) => {
                let sel = v.sel;
                self.open_row_menu(sel, None);
            }
            // Esc clears the filter, then goes back to Home (it never quits from inside a list)
            KeyCode::Esc if !v.filter.is_empty() => v.set_filter(String::new()),
            KeyCode::Esc => self.run(Action::Back),
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
        self.active_feed()?.shelves.get(c.shelf)?.cards.get(c.card)
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
        let (kind, uri) = (card.kind.clone(), card.uri.clone());
        let mut items = vec![("Play".to_string(), MenuAct::Play)];
        if matches!(kind.as_str(), "track" | "episode") {
            items.push(("Play next".into(), MenuAct::PlayNext));
            items.push(("Add to queue".into(), MenuAct::Queue));
        }
        if matches!(kind.as_str(), "album" | "artist" | "playlist" | "show") {
            items.push((format!("Go to {kind}"), MenuAct::GoTo));
        }
        items.extend(self.save_entry(&uri));
        items.push(("Copy link".into(), MenuAct::CopyLink));
        let at = at.or_else(|| self.hit_rect(&Hit::Card(c)).map(|r| (r.x + 2, r.y + 1))).unwrap_or((0, 0));
        self.feed_view().sel = Some(c);
        self.menu = Some(Menu { at, target: Target::Card(c), items, selected: 0 });
    }

    /// Right-click on a track-list row.
    pub fn open_row_menu(&mut self, vi: usize, at: Option<(u16, u16)>) {
        if let Some(v) = self.view.as_mut() {
            v.sel = vi;
        }
        let mut items = vec![("Play from here".to_string(), MenuAct::PlayHere), ("Play next".into(), MenuAct::PlayNext), ("Add to queue".into(), MenuAct::Queue)];
        if let Some((uri, ..)) = self.target_track(&Target::Row(vi)) {
            items.extend(self.save_entry(&uri));
        }
        items.push(("Copy link".into(), MenuAct::CopyLink));
        let at = at.or_else(|| self.hit_rect(&Hit::Track(vi)).map(|r| (r.x + 4, r.y + 1))).unwrap_or((0, 0));
        self.menu = Some(Menu { at, target: Target::Row(vi), items, selected: 0 });
    }

    /// Right-click on a queue entry.
    pub fn open_queue_menu(&mut self, i: usize, at: Option<(u16, u16)>) {
        let Some(q) = self.playback.queue.get(i).cloned() else { return };
        self.queue_view.sel = i;
        let queued = q.source == "queue";
        let mut items = vec![];
        if queued {
            items.push(("Remove from queue".to_string(), MenuAct::Dequeue));
        } else {
            items.push(("Play next".to_string(), MenuAct::PlayNext));
        }
        if self.playback.queue.iter().any(|q| q.source == "queue") {
            items.push(("Clear queue".into(), MenuAct::ClearQueue));
        }
        items.extend(self.save_entry(&q.item.uri));
        items.push(("Copy link".into(), MenuAct::CopyLink));
        let at = at.or_else(|| self.hit_rect(&Hit::QueueRow(i)).map(|r| (r.x + 4, r.y + 1))).unwrap_or((0, 0));
        let target = Target::Queued { uri: q.item.uri.clone(), uid: q.uid.clone(), name: q.item.name.clone(), queued };
        self.menu = Some(Menu { at, target, items, selected: 0 });
    }

    /// Right-click on any other track (search results, an artist's popular tracks).
    fn open_track_menu(&mut self, uri: String, name: String, at: Option<(u16, u16)>) {
        let mut items = vec![("Play next".to_string(), MenuAct::PlayNext), ("Add to queue".into(), MenuAct::Queue)];
        items.extend(self.save_entry(&uri));
        items.push(("Copy link".into(), MenuAct::CopyLink));
        self.menu = Some(Menu { at: at.unwrap_or((0, 0)), target: Target::Track { uri, name }, items, selected: 0 });
    }

    /// A ♥ / Save / Follow button. Removing a playlist takes a second click (it may be your own).
    fn save_clicked(&mut self, l: singularity_proto::Link) {
        let playlist = l.uri.starts_with("spotify:playlist:") && self.is_saved(&l.uri) == Some(true);
        let confirmed = self.confirm_remove.take().is_some_and(|(u, t)| u == l.uri && t.elapsed() < Duration::from_secs(3));
        if playlist && !confirmed {
            self.confirm_remove = Some((l.uri, Instant::now()));
            return self.say("Click again to remove this playlist from Your Library");
        }
        self.toggle_saved(&l.uri, &l.name);
    }

    fn hit_rect(&self, h: &Hit) -> Option<Rect> {
        self.hits.iter().find(|(_, x)| x == h).map(|(r, _)| *r)
    }

    /// URI and name of a menu's target.
    pub(crate) fn target_track(&self, t: &Target) -> Option<(String, String, String)> {
        match t {
            Target::Card(c) => self.card(*c).map(|k| (k.uri.clone(), k.name.clone(), k.kind.clone())),
            Target::Row(vi) => {
                let order = self.view_order();
                let v = self.view.as_ref()?;
                let row = self.lists.get(&v.uri)?.rows.get(*order.get(*vi)?)?;
                Some((row.uri.clone(), row.name.clone(), "track".into()))
            }
            Target::Queued { uri, name, .. } | Target::Track { uri, name } => Some((uri.clone(), name.clone(), "track".into())),
        }
    }

    fn menu_run(&mut self, i: usize) {
        let Some(m) = self.menu.take() else { return };
        let Some((_, act)) = m.items.get(i).cloned() else { return };
        let Some((uri, name, kind)) = self.target_track(&m.target) else { return };
        let name = if name.is_empty() { "it".to_string() } else { name };
        match act {
            MenuAct::Play => match m.target {
                Target::Card(c) => self.play_card(c),
                _ => self.send(Command::Play { uri: Some(uri) }),
            },
            MenuAct::PlayHere => {
                if let Target::Row(vi) = m.target {
                    self.play_row(vi);
                }
            }
            MenuAct::PlayNext => {
                self.send(Command::PlayNext { uri });
                self.say(format!("{name} plays next"));
            }
            MenuAct::Queue => {
                self.send(Command::AddToQueue { uri });
                self.say(format!("Queued {name}"));
            }
            MenuAct::Dequeue => {
                let uid = match &m.target {
                    Target::Queued { uid, .. } if !uid.is_empty() => Some(uid.clone()),
                    _ => None,
                };
                self.send(Command::RemoveFromQueue { uri, uid });
            }
            MenuAct::ClearQueue => self.run(Action::ClearQueue),
            MenuAct::GoTo => match m.target {
                Target::Card(c) => self.open_card(c),
                _ => self.say(format!("{} pages arrive with #11", capitalise(&kind))),
            },
            MenuAct::CopyLink => {
                self.clipboard = Some(web_link(&uri));
                self.say("Link copied");
            }
            MenuAct::Save => self.toggle_saved(&uri, &name),
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

    /// Keys while the setup wizard is open.
    fn wizard_key(&mut self, k: KeyEvent) {
        use crate::setup::Step;
        let step = crate::setup::step(self);
        match k.code {
            KeyCode::Esc => {
                self.setup.dismissed = true;
                self.setup.forced = false;
                self.setup.rekey = false;
            }
            KeyCode::Char(c) if step == Step::Key => self.setup.key.push(c),
            KeyCode::Backspace if step == Step::Key => {
                self.setup.key.pop();
            }
            KeyCode::Enter if step == Step::Key => {
                let key = std::mem::take(&mut self.setup.key);
                if !key.trim().is_empty() {
                    self.setup.rekey = false;
                    self.send(Command::SetSoloistKey { key: key.trim().to_string() });
                }
            }
            KeyCode::Char('k') | KeyCode::Char('K') if step == Step::Engine => self.setup.rekey = true,
            KeyCode::Enter if step == Step::Login && !matches!(self.catalogue, Some(CatalogueStatus::LoginPending { .. })) => self.run(Action::Login),
            KeyCode::Char(' ') if step == Step::Audio => self.run(Action::TogglePlay),
            KeyCode::Enter if matches!(step, Step::Audio | Step::Done) => {
                self.setup.forced = false;
                self.setup.dismissed = true;
            }
            _ => {}
        }
    }

    /// Pasted text (bracketed paste): into the wizard's key field, the palette or the filter.
    pub fn paste(&mut self, s: &str) {
        if self.wake() {
            return;
        }
        let s = s.trim();
        if crate::setup::wanted(self) && crate::setup::step(self) == crate::setup::Step::Key {
            self.setup.key.push_str(s);
        } else if let Some(p) = self.palette.as_mut() {
            p.query.push_str(s);
            p.selected = 0;
        } else if let Some(v) = self.view.as_mut().filter(|v| v.filtering) {
            let f = format!("{}{s}", v.filter);
            v.set_filter(f);
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
        self.poll_config();
        self.poll_themes(false);
        let idle = &self.config.idle;
        let due = self.idle_preview || (idle.enabled && idle.minutes > 0.0 && self.last_input.elapsed().as_secs_f32() >= idle.minutes * 60.0);
        match (due, self.idle_since) {
            (true, None) => self.idle_since = Some(Instant::now()),
            (false, Some(_)) => self.idle_since = None,
            _ => {}
        }
        let dt = self.last_tick.elapsed().as_secs_f32().min(0.1);
        self.last_tick = Instant::now();
        self.hole.advance(dt, self.meters.frame.as_ref());
        // search while typing, once typing pauses
        if let Some(q) = self.search.as_mut().and_then(|s| s.due()) {
            self.send(Command::Search { query: q });
        }
        // lyrics follow the track, if a lyrics Pane is on screen
        if let Some(uri) = self.playback.item.as_ref().map(|i| i.uri.clone()).filter(|u| !u.is_empty())
            && self.lyrics_asked.as_deref() != Some(uri.as_str())
            && matches!(self.link, Link::Connected { .. })
            && self.layout.compute(Rect::new(0, 0, 200, 60)).slots.iter().any(|(s, _)| s == "lyrics")
        {
            self.lyrics_asked = Some(uri.clone());
            self.send(Command::GetLyrics { uri });
        }
        // the ♥ in the player bar follows the track
        if let Some(uri) = self.playback.item.as_ref().map(|i| i.uri.clone()).filter(|u| crate::saved::savable(u))
            && self.saved.checked_now.as_deref() != Some(uri.as_str())
            && matches!(self.link, Link::Connected { .. })
        {
            self.saved.checked_now = Some(uri.clone());
            self.recheck_saved(&uri);
        }
        // the health check refreshes itself while it's open
        if self.health_open && self.health_asked.is_none_or(|t| t.elapsed() > Duration::from_secs(3)) {
            self.health_asked = Some(Instant::now());
            if matches!(self.link, Link::Connected { .. }) {
                self.send(Command::GetHealth);
            }
        }
        if !self.told_backend
            && let Some(b) = self.hole.backend()
        {
            self.told_backend = true;
            self.say(format!("Visuals: {b}"));
        }
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
        // any mouse event wakes the idle UI, and does nothing else
        if self.wake() {
            return;
        }
        let coarse = m.modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::CONTROL | KeyModifiers::ALT);
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) if self.picker.is_some() => match self.hit_at(m.column, m.row) {
                Some(Hit::PickerItem(i)) => {
                    self.picker_select(i);
                    self.picker_keep();
                }
                _ => self.picker_cancel(),
            },
            MouseEventKind::Down(MouseButton::Left) if self.menu.is_some() => match self.hit_at(m.column, m.row) {
                Some(Hit::MenuItem(i)) => self.menu_run(i),
                // a click elsewhere only closes the menu
                _ => self.menu = None,
            },
            MouseEventKind::Down(MouseButton::Right) => {
                self.menu = None;
                let at = Some((m.column, m.row));
                match self.hit_at(m.column, m.row) {
                    Some(Hit::Card(c)) => {
                        self.focus = "home".into();
                        self.open_menu(c, at);
                    }
                    Some(Hit::Track(vi)) => {
                        self.focus = "home".into();
                        self.open_row_menu(vi, at);
                    }
                    Some(Hit::QueueRow(i)) => {
                        self.focus = "queue".into();
                        self.open_queue_menu(i, at);
                    }
                    Some(Hit::SearchTrack(i)) => {
                        let t = self.search.as_ref().and_then(|s| s.results.as_ref()?.tracks.get(i).cloned());
                        if let Some(t) = t {
                            self.open_track_menu(t.uri, t.name, at);
                        }
                    }
                    Some(Hit::TopTrack(i)) => {
                        let t = self.artist.as_ref().and_then(|a| a.page.as_ref()?.top_tracks.get(i).cloned());
                        if let Some(t) = t {
                            self.open_track_menu(t.uri, t.name, at);
                        }
                    }
                    _ => {}
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if self.help {
                    self.help = false;
                    return;
                }
                if self.devices_open && !matches!(self.hit_at(m.column, m.row), Some(Hit::DeviceRow(_))) {
                    self.devices_open = false;
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
                    Some(Hit::QueueRow(i)) => {
                        self.focus = "queue".into();
                        self.queue_view.sel = i;
                    }
                    // click a lyric line: jump there
                    Some(Hit::Lyric(ms)) => self.seek_to(ms),
                    Some(Hit::Well) => {
                        let hole = self.layout.compute(Rect::new(0, 0, 200, 60)).slots.iter().any(|(s, _)| s == "hole");
                        self.focus = if hole { "hole" } else { "well" }.into();
                        self.run(Action::NextLook);
                    }
                    Some(Hit::Card(c)) => {
                        self.focus = "home".into();
                        self.open_card(c);
                    }
                    Some(Hit::PlayView) => self.play_view(),
                    Some(Hit::ShelfScroll(row, d)) => self.feed_view().scroll_shelf(row, d),
                    Some(Hit::Link(l)) => self.open_link(&l),
                    Some(Hit::DeviceRow(i)) => self.transfer_to(i),
                    Some(Hit::Chip(i)) => {
                        if let Some(s) = self.search.as_mut() {
                            s.set_chip(i);
                        }
                    }
                    Some(Hit::Recent(i)) => {
                        let q = self.recent.get(i).cloned();
                        if let (Some(s), Some(q)) = (self.search.as_mut(), q) {
                            s.query = q;
                            s.typed();
                        }
                    }
                    Some(Hit::SearchTrack(i)) => {
                        let double = self.last_click.is_some_and(|(t, j)| j == usize::MAX / 2 - i && t.elapsed() < DOUBLE_CLICK);
                        if let Some(s) = self.search.as_mut() {
                            s.track_sel = Some(i);
                        }
                        if double {
                            self.last_click = None;
                            self.play_search_track(i);
                        } else {
                            self.last_click = Some((Instant::now(), usize::MAX / 2 - i));
                        }
                    }
                    Some(Hit::PlayArtist) => self.play_artist(),
                    Some(Hit::Save(l)) => self.save_clicked(l),
                    Some(Hit::TopTrack(i)) => {
                        // top tracks share the double-click memory with list rows, kept apart by index
                        let double = self.last_click.is_some_and(|(t, j)| j == usize::MAX - i && t.elapsed() < DOUBLE_CLICK);
                        if let Some(a) = self.artist.as_mut() {
                            a.top_sel = i;
                        }
                        if double {
                            self.last_click = None;
                            self.play_top(i);
                        } else {
                            self.last_click = Some((Instant::now(), usize::MAX - i));
                        }
                    }
                    Some(Hit::MenuItem(_) | Hit::PickerItem(_)) => {}
                    Some(Hit::Swap(slot, d)) => {
                        self.swap_widget(&slot, d);
                    }
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
                    self.feed_view().scroll_shelf(row, d);
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
                if matches!(&hit, Some(Hit::Focus(s)) if s == "lyrics") && self.lyrics.as_ref().is_some_and(|l| !l.synced) {
                    self.lyrics_scroll = (self.lyrics_scroll as i64 - sign as i64 * 3).max(0) as usize;
                    return;
                }
                if matches!(&hit, Some(Hit::QueueRow(_))) || matches!(&hit, Some(Hit::Focus(s)) if s == "queue") {
                    let lines = crate::queue::lines(self).len();
                    self.queue_view.scroll(-sign * 3, lines);
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
                        Some(row) if coarse => self.feed_view().scroll_shelf(row, -sign as i32),
                        _ => self.feed_view().scroll_rows(-sign as i32),
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
                    Some(Hit::PickerItem(_)) => {
                        let sel = self.picker.as_ref().map(|p| p.sel).unwrap_or(0);
                        self.picker_select(if up { sel.saturating_sub(1) } else { sel + 1 });
                    }
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
            Hit::Card(c) => {
                let rows = match (&self.search, &self.artist) {
                    (Some(s), _) => &s.shelves.rows,
                    (_, Some(a)) => &a.shelves.rows,
                    _ => &self.home_view.rows,
                };
                rows.iter().position(|r| r.cards.contains(&c))
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn press(app: &mut App, code: KeyCode, mods: KeyModifiers) {
        app.key(KeyEvent::new(code, mods));
    }

    fn with_files() -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(None);
        app.load_config_from(dir.path().join("config.toml"), dir.path().join("keys.toml"));
        (app, dir)
    }

    /// Saves land within a second; tests don't wait for it.
    fn saved(app: &mut App) {
        if let Some(f) = app.config_files.as_mut() {
            f.polled = Instant::now() - Duration::from_secs(2);
        }
        app.tick();
    }

    #[test]
    fn first_run_writes_both_files_with_their_comments() {
        let (app, dir) = with_files();
        let c = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        let k = std::fs::read_to_string(dir.path().join("keys.toml")).unwrap();
        assert!(c.contains("# which graphics chip") && k.contains("preset = \"standard\""));
        assert_eq!(app.keymap.preset, "standard");
    }

    #[test]
    fn keys_toml_applies_live_and_reports_conflicts() {
        let (mut app, dir) = with_files();
        app.playback.item = Some(singularity_proto::Item { uri: "spotify:track:a".into(), name: "A".into(), ..Default::default() });
        let keys = dir.path().join("keys.toml");
        let text = std::fs::read_to_string(&keys).unwrap().replace("# like = \"f\"", "like = \"f\"\nshuffle = \"f\"");
        std::fs::write(&keys, text).unwrap();
        saved(&mut app);
        assert!(app.toast.as_ref().is_some_and(|(t, _)| t.contains("f is bound to both like and shuffle")), "{:?}", app.toast);
        press(&mut app, KeyCode::Char('f'), KeyModifiers::NONE);
        assert_eq!(app.is_saved("spotify:track:a"), Some(true), "f likes now");
        press(&mut app, KeyCode::Char('l'), KeyModifiers::NONE);
        assert_eq!(app.is_saved("spotify:track:a"), Some(true), "l no longer does");
    }

    #[test]
    fn the_vim_preset_moves_lists_but_not_while_typing() {
        let (mut app, dir) = with_files();
        std::fs::write(dir.path().join("keys.toml"), "schema = 1\npreset = \"vim\"\n").unwrap();
        saved(&mut app);
        assert_eq!(app.keymap.preset, "vim");
        app.view = Some(TrackView::new("spotify:collection", "Liked Songs"));
        let rows = (0..5).map(|i| singularity_proto::TrackRow { uri: format!("spotify:track:{i}"), name: format!("Song {i}"), playable: true, ..Default::default() }).collect();
        app.track_page(singularity_proto::TrackPage { uri: "spotify:collection".into(), offset: 0, total: 5, tracks: rows, header: None });
        press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
        press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(app.view.as_ref().unwrap().sel, 2);
        press(&mut app, KeyCode::Char('G'), KeyModifiers::SHIFT);
        assert_eq!(app.view.as_ref().unwrap().sel, 4);
        // in the filter, j is a letter
        press(&mut app, KeyCode::Char('/'), KeyModifiers::NONE);
        press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(app.view.as_ref().unwrap().filter, "j");
    }

    #[test]
    fn the_notifications_toggle_edits_config_toml_and_keeps_its_comments() {
        let (mut app, dir) = with_files();
        let path = dir.path().join("config.toml");
        let before = std::fs::read_to_string(&path).unwrap();
        app.run(Action::ToggleNotifications);
        assert!(app.config.notifications.track_change);
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after, before.replace("track_change = false", "track_change = true"));
        // the GPU setting, edited by hand, applies on the next frame
        std::fs::write(&path, after.replace("gpu = \"low-power\"", "gpu = \"high-performance\"")).unwrap();
        saved(&mut app);
        assert!(app.hole.high_performance);
    }

    /// The UI's last input was long enough ago for the takeover, and a frame has passed.
    fn idled(app: &mut App) {
        app.last_input = Instant::now() - Duration::from_secs_f32(app.config.idle.minutes * 60.0 + 1.0);
        app.tick();
    }

    #[test]
    fn the_key_that_wakes_the_idle_ui_is_not_acted_on() {
        let (tx, mut rx) = mpsc::channel(8);
        let mut app = App::new(Some(tx));
        app.link = Link::Connected { version: "t".into() };
        idled(&mut app);
        assert!(app.idle_secs().is_some(), "idle after idle.minutes");
        press(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
        assert!(app.idle_secs().is_none() && rx.try_recv().is_err(), "the first n only wakes");
        press(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
        assert!(matches!(rx.try_recv(), Ok(Command::Next)), "the second n skips");
        // a click wakes it the same way, and does nothing else
        idled(&mut app);
        app.mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: 3, row: 3, modifiers: KeyModifiers::NONE });
        assert!(app.idle_secs().is_none() && rx.try_recv().is_err());
    }

    #[test]
    fn input_keeps_the_ui_awake_and_the_preview_shows_it_at_once() {
        let mut app = App::new(None);
        app.tick();
        assert!(app.idle_secs().is_none());
        app.run(Action::IdlePreview);
        app.tick();
        assert!(app.idle_secs().is_some(), "the palette's preview doesn't wait");
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        app.tick();
        assert!(app.idle_secs().is_none(), "and a key ends it");
    }

    #[test]
    fn shift_i_turns_the_takeover_off_in_config_toml() {
        let (mut app, dir) = with_files();
        press(&mut app, KeyCode::Char('I'), KeyModifiers::SHIFT);
        assert!(!app.config.idle.enabled);
        let text = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(text.contains("enabled = false") && text.contains("# "), "kept its comments:\n{text}");
        idled(&mut app);
        assert!(app.idle_secs().is_none());
        // a restarted App reads it back
        let mut again = App::new(None);
        again.load_config_from(dir.path().join("config.toml"), dir.path().join("keys.toml"));
        idled(&mut again);
        assert!(!again.config.idle.enabled && again.idle_secs().is_none());
    }

    #[test]
    fn deleting_listening_data_takes_a_second_go() {
        let (tx, mut rx) = mpsc::channel(8);
        let mut app = App::new(Some(tx));
        app.link = Link::Connected { version: "t".into() };
        app.run(Action::DeleteListening);
        assert!(rx.try_recv().is_err() && app.toast.as_ref().is_some_and(|(t, _)| t.contains("again")));
        app.run(Action::DeleteListening);
        assert!(matches!(rx.try_recv(), Ok(Command::DeleteLocalData)));
        app.run(Action::ExportListening);
        assert!(matches!(rx.try_recv(), Ok(Command::ExportLocalData)));
    }
}
