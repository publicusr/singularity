//! Config files (#20, D21): `~/.config/singularity/config.toml` and `keys.toml`, read by the Daemon and the Client.
//!
//! - Written with comments on first run, so the file documents itself.
//! - A `schema` number, with automatic migrations that edit the document in place (comments and formatting survive).
//! - Edits from the app (`set`) go through the same document, so the files stay the one source of truth.
//! - Hot reload: `Watched` notices a save (by mtime and size, which also catches editors that save by renaming).
//! - A bad value never stops the app: what can't be read falls back to its default, and the problem is reported.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
pub use toml_edit;
use toml_edit::{DocumentMut, Item, Value};

/// The `config.toml` schema this build writes and reads.
pub const CONFIG_SCHEMA: i64 = 5;
/// The `keys.toml` schema.
pub const KEYS_SCHEMA: i64 = 1;

/// `$XDG_CONFIG_HOME/singularity` (else `~/.config/singularity`).
pub fn dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))).unwrap_or_else(std::env::temp_dir);
    base.join("singularity")
}

pub fn config_path() -> PathBuf {
    dir().join("config.toml")
}

pub fn keys_path() -> PathBuf {
    dir().join("keys.toml")
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub ui: Ui,
    pub notifications: Notifications,
    pub visuals: Visuals,
    pub idle: Idle,
    pub engine: EngineChoice,
}

/// Which Engine plays (D51); read when the Daemon starts.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct EngineChoice {
    pub kind: EngineKind,
    /// the player's Connect device name (the app, librespot); empty: the default for that Engine
    pub device: String,
    /// Soloist's cache of played songs, in GB; 0: no limit (#71)
    pub cache_gb: f32,
}

impl Default for EngineChoice {
    fn default() -> Self {
        EngineChoice { kind: EngineKind::Auto, device: String::new(), cache_gb: 5.0 }
    }
}

impl EngineChoice {
    /// What Soloist's `--cache-size` gets: MB, at least its minimum of 100; 0 is its own "no limit". A value that
    /// makes no sense (negative) is the default.
    pub fn soloist_cache_mb(&self) -> u32 {
        let gb = if self.cache_gb >= 0.0 { self.cache_gb } else { EngineChoice::default().cache_gb };
        if gb == 0.0 { 0 } else { ((gb as f64 * 1024.0).round() as u32).max(100) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EngineKind {
    /// this OS's own: Soloist on Linux, librespot on macOS (D54)
    #[default]
    Auto,
    Soloist,
    SpotifyApp,
    /// librespot inside the Daemon (D52): light, every OS, at most 320 kbps
    Librespot,
}

impl EngineKind {
    /// What `Auto` means here.
    pub fn resolved(self) -> EngineKind {
        match self {
            EngineKind::Auto if cfg!(target_os = "macos") => EngineKind::Librespot,
            EngineKind::Auto => EngineKind::Soloist,
            k => k,
        }
    }
}

/// The idle takeover: after `minutes` without input the black hole leaves its Pane and drifts over the screen.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Idle {
    pub enabled: bool,
    pub minutes: f32,
}

impl Default for Idle {
    fn default() -> Self {
        Idle { enabled: true, minutes: 3.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Ui {
    /// singularity, event-horizon, classic, cockpit or terminal
    pub design: String,
    /// a built-in theme or one from `themes/`; empty: the design's own
    pub theme: String,
    /// the terminal's own background instead of black
    pub transparent: bool,
    /// a layout saved in `layouts/` (its file name); empty: the design's own
    pub layout: String,
    /// covers and the black hole in the Theme's colours, if the Theme tints
    pub tint_art: bool,
}

impl Default for Ui {
    fn default() -> Self {
        Ui { design: "singularity".into(), theme: String::new(), transparent: false, layout: String::new(), tint_art: true }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct Notifications {
    /// a desktop notification when a new track starts
    pub track_change: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Visuals {
    pub gpu: GpuPower,
    /// frames per second while the terminal isn't focused
    pub unfocused_fps: u32,
}

impl Default for Visuals {
    fn default() -> Self {
        Visuals { gpu: GpuPower::LowPower, unfocused_fps: 30 }
    }
}

/// Which graphics chip draws the Visuals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GpuPower {
    /// the built-in graphics: plenty for the black hole, and frugal
    #[default]
    LowPower,
    /// the dedicated card
    HighPerformance,
}

/// What a fresh `config.toml` says.
pub const DEFAULT_CONFIG: &str = r#"# Singularity settings. Changes apply as soon as you save this file.
# (Don't change `schema`: Singularity uses it to update this file when settings change between versions.)
schema = 5

[ui]
# the design: "singularity", "event-horizon", "classic", "cockpit" or "terminal" (Ctrl+K → Design shows them)
design = "singularity"
# colours: empty for the design's own, else a built-in theme or a file in themes/ (Ctrl+K → Theme, or t)
theme = ""
# let the terminal's own background (and its transparency) show instead of black
transparent = false
# a layout you saved (Ctrl+K → "Save layout as…"), by file name in layouts/; empty for the design's own
layout = ""
# covers and the black hole in the theme's colours, for themes that tint (Cockpit's and Terminal's); false: real colours
tint_art = true

[notifications]
# a desktop notification when a new track starts
track_change = false

[visuals]
# which graphics chip draws the black hole: "low-power" (built-in graphics) or "high-performance" (dedicated card)
gpu = "low-power"
# frames per second while the terminal isn't focused (1–30; lower saves a little CPU, but looks choppier)
unfocused_fps = 30

[idle]
# after this many minutes without input the black hole leaves its Pane and drifts over the screen; any key wakes it
# (Shift+I switches it off and on)
enabled = true
minutes = 3

[engine]
# what plays the music: "auto" (Soloist on Linux, librespot on macOS), "soloist" (lossless; on macOS it runs in a
# small Linux VM, ~300 MB), "spotify-app" (the official Spotify app, started hidden and driven over Connect) or
# "librespot" (built in: the lightest, but at most 320 kbps, never lossless); restart the Daemon after changing it
kind = "auto"
# the player's name in device pickers; empty: the default (the app: this computer's name; librespot: "Singularity on
# <computer>"; Soloist: "Singularity" on Linux, "Singularity on <computer> (lossless)" on macOS)
device = ""
# disk space for Soloist's cache of played songs, in GB; 0 = no limit. Replays come from it instead of the network (a
# lossless song is ~20–40 MB). Applies when Soloist next starts; Soloist's VM on macOS keeps its own 200 MB.
cache_gb = 5
"#;

/// What a fresh `keys.toml` says.
pub const DEFAULT_KEYS: &str = r#"# Singularity keys. Changes apply as soon as you save this file.
schema = 1

# "standard" (arrows, Enter, Space, Ctrl+K) or "vim" (adds h/j/k/l, g/G, Ctrl+D/U for moving around)
preset = "standard"

# Your own bindings, on top of the preset: action = "key", or a list of keys.
# Keys look like "l", "Space", "Ctrl+K", "Shift+Right", "Alt+Left", "F10". A key you give an action is taken away
# from whatever had it before; a key bound twice here is reported, and the first binding wins.
# The actions and their keys are listed in the help screen (?) and the command palette (Ctrl+K).
[bindings]
# like = "f"
# next = ["n", "Ctrl+Right"]
"#;

/// One migration: edits a document of schema N into schema N + 1 (in place, so comments stay).
pub type Migration = fn(&mut DocumentMut);

/// `config.toml`'s migrations: entry `i` takes schema `i + 1` to `i + 2`.
pub const CONFIG_MIGRATIONS: &[Migration] = &[add_tint_art, add_engine, add_cache_gb, smooth_unfocused];

/// 1 → 2: `ui.tint_art` (#33) appears at the end of `[ui]`, with its comment, so it can be found.
fn add_tint_art(doc: &mut DocumentMut) {
    let Some(ui) = doc.get_mut("ui").and_then(Item::as_table_mut) else { return };
    if ui.contains_key("tint_art") {
        return;
    }
    let mut key = toml_edit::Key::new("tint_art");
    key.leaf_decor_mut().set_prefix("# covers and the black hole in the theme's colours, for themes that tint (Cockpit's and Terminal's); false: real colours\n");
    ui.insert_formatted(&key, toml_edit::value(true));
}

/// 2 → 3: `[engine]` (#65) appears at the end, as a fresh file has it.
fn add_engine(doc: &mut DocumentMut) {
    if doc.contains_key("engine") {
        return;
    }
    let fresh: DocumentMut = DEFAULT_CONFIG.parse().expect("the default config parses");
    let mut engine = fresh["engine"].as_table().expect("[engine] in the default file").clone();
    engine.decor_mut().set_prefix("\n");
    doc.insert("engine", Item::Table(engine));
}

/// 3 → 4: `engine.cache_gb` (#71) appears at the end of `[engine]`, with its comment.
fn add_cache_gb(doc: &mut DocumentMut) {
    let Some(engine) = doc.get_mut("engine").and_then(Item::as_table_mut) else { return };
    if engine.contains_key("cache_gb") {
        return;
    }
    let fresh: DocumentMut = DEFAULT_CONFIG.parse().expect("the default config parses");
    let (key, item) = fresh["engine"].as_table().and_then(|t| t.get_key_value("cache_gb")).expect("cache_gb in the default file");
    engine.insert_formatted(key, item.clone());
}

/// 4 → 5: the old default of 10 unfocused frames a second looked choppy (D62); a file that still says 10 (and its
/// comment) gets the new default of 30. Any other number was the user's own and stays.
fn smooth_unfocused(doc: &mut DocumentMut) {
    let Some(visuals) = doc.get_mut("visuals").and_then(Item::as_table_mut) else { return };
    if visuals.get("unfocused_fps").and_then(Item::as_integer) != Some(10) {
        return;
    }
    if let Some(v) = visuals.get_mut("unfocused_fps").and_then(Item::as_value_mut) {
        let decor = v.decor().clone();
        *v = Value::from(30);
        *v.decor_mut() = decor;
    }
    if let Some(mut key) = visuals.key_mut("unfocused_fps")
        && let Some(old) = key.leaf_decor().prefix().and_then(|p| p.as_str()).map(String::from)
    {
        key.leaf_decor_mut().set_prefix(old.replace("(1–30; the Visuals keep moving, slower)", "(1–30; lower saves a little CPU, but looks choppier)"));
    }
}

/// Bring `doc` up to `target` with `migrations`. Returns whether anything changed; a document from a newer
/// Singularity is left alone (and reported).
pub fn migrate(doc: &mut DocumentMut, migrations: &[Migration], target: i64) -> Result<bool, String> {
    let from = doc.get("schema").and_then(Item::as_integer).unwrap_or(1);
    if from > target {
        return Err(format!("written by a newer Singularity (schema {from}, this one knows {target}); unknown settings are ignored"));
    }
    for v in from..target {
        let m = migrations.get((v - 1) as usize).ok_or_else(|| format!("no migration from schema {v}"))?;
        m(doc);
        set_value(doc, "schema", Value::from(v + 1));
    }
    Ok(from < target)
}

/// A file as read: the value, what was wrong with it, and the migrated text if a migration ran.
#[derive(Debug)]
pub struct Read<T> {
    pub value: T,
    pub problems: Vec<String>,
    pub migrated: Option<String>,
}

/// Parse a config file's text (migrating it to `target` first).
pub fn parse<T: Default + for<'de> Deserialize<'de>>(name: &str, text: &str, migrations: &[Migration], target: i64) -> Read<T> {
    let mut problems = vec![];
    let mut doc = match text.parse::<DocumentMut>() {
        Ok(d) => d,
        Err(e) => return Read { value: T::default(), problems: vec![format!("{name}: {}", one_line(&e.to_string()))], migrated: None },
    };
    let migrated = match migrate(&mut doc, migrations, target) {
        Ok(true) => Some(doc.to_string()),
        Ok(false) => None,
        Err(e) => {
            problems.push(format!("{name}: {e}"));
            None
        }
    };
    let value = match toml_edit::de::from_document::<T>(doc) {
        Ok(v) => v,
        Err(e) => {
            problems.push(format!("{name}: {} (using the defaults)", one_line(&e.to_string())));
            T::default()
        }
    };
    Read { value, problems, migrated }
}

fn one_line(s: &str) -> String {
    s.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" · ")
}

/// Read `config.toml`, writing the default one first if there's none, and saving a migrated one back.
pub fn load_config(path: &Path) -> Read<Config> {
    load(path, "config.toml", DEFAULT_CONFIG, CONFIG_MIGRATIONS, CONFIG_SCHEMA)
}

/// Read a file the same way (`keys.toml` too).
pub fn load<T: Default + for<'de> Deserialize<'de>>(path: &Path, name: &str, default: &str, migrations: &[Migration], target: i64) -> Read<T> {
    if !path.exists() {
        let _ = write_atomic(path, default);
    }
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => default.to_string(),
    };
    let r = parse::<T>(name, &text, migrations, target);
    if let Some(m) = &r.migrated {
        let _ = write_atomic(path, m);
    }
    r
}

/// Write via a temporary file and a rename, so readers never see half a file.
pub fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = write_tmp(path, text)?;
    std::fs::rename(tmp, path)
}

/// Write `text` to the temporary file next to `path`; the rename is the caller's.
fn write_tmp(path: &Path, text: &str) -> std::io::Result<PathBuf> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text)?;
    Ok(tmp)
}

/// Write-and-rename, unless `path`'s stamp changed from `before` while the temporary file was written: an editor
/// saved in that window, so the caller must re-read instead of replacing the save. False means nothing was written.
fn write_atomic_checked(path: &Path, text: &str, before: Option<(SystemTime, u64)>) -> std::io::Result<bool> {
    let tmp = write_tmp(path, text)?;
    if stamp(path) != before {
        let _ = std::fs::remove_file(&tmp);
        return Ok(false);
    }
    std::fs::rename(&tmp, path)?;
    Ok(true)
}

/// Set `dotted` (`visuals.gpu`) in the document, creating tables as needed; the rest of the file is untouched.
pub fn set_value(doc: &mut DocumentMut, dotted: &str, v: Value) {
    let mut parts: Vec<&str> = dotted.split('.').collect();
    let last = parts.pop().expect("a key");
    let mut table = doc.as_table_mut();
    for p in parts {
        // a section written inline (`ui = { … }`) becomes a table with its values; anything else in its place (a
        // stray `ui = 5`) gives way to one
        if !table.get(p).is_some_and(Item::is_table) {
            let section = match table.remove(p) {
                Some(Item::Value(toml_edit::Value::InlineTable(t))) => t.into_table(),
                _ => toml_edit::Table::new(),
            };
            table.insert(p, Item::Table(section));
        }
        let Some(next) = table.get_mut(p).and_then(Item::as_table_mut) else { return };
        table = next;
    }
    // keep the value's own decoration (an end-of-line comment) when replacing it
    match table.get_mut(last).and_then(Item::as_value_mut) {
        Some(old) => {
            let decor = old.decor().clone();
            *old = v;
            *old.decor_mut() = decor;
        }
        None => {
            table.insert(last, toml_edit::value(v));
        }
    }
}

/// For migrations: rename a key in place, keeping its spot, its comments and its value. False if it wasn't there.
pub fn rename(table: &mut toml_edit::Table, from: &str, to: &str) -> bool {
    if !table.contains_key(from) || table.contains_key(to) {
        return false;
    }
    let names: Vec<String> = table.iter().map(|(k, _)| k.to_string()).collect();
    let entries: Vec<(toml_edit::Key, Item)> = names.iter().filter_map(|k| table.remove_entry(k)).collect();
    for (k, item) in entries {
        let k = if k.get() == from { toml_edit::Key::new(to).with_leaf_decor(k.leaf_decor().clone()) } else { k };
        table.insert_formatted(&k, item);
    }
    true
}

/// Change one setting in a file the way a Settings screen would: comments and layout stay.
pub fn set(path: &Path, default: &str, dotted: &str, v: Value) -> std::io::Result<()> {
    set_watched(path, default, dotted, v, |_| {})
}

/// `set`, but `after_read` runs between the read and the rename: a test plays the user's editor saving in that
/// window. The change then lands on the saved file instead of replacing it (the `Watched` stamp is `(mtime, size)`).
fn set_watched(path: &Path, default: &str, dotted: &str, v: Value, mut after_read: impl FnMut(&Path)) -> std::io::Result<()> {
    for _ in 0..3 {
        let before = stamp(path);
        let text = std::fs::read_to_string(path).unwrap_or_else(|_| default.to_string());
        let mut doc = text.parse::<DocumentMut>().map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        set_value(&mut doc, dotted, v.clone());
        after_read(path);
        if write_atomic_checked(path, &doc.to_string(), before)? {
            return Ok(());
        }
    }
    // saves landed in every window: don't guess which one to overwrite
    Err(std::io::Error::other("config.toml kept changing while it was being written"))
}

/// `keys.toml` as written: a preset and bindings by action name (checked against the actions by the Client).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Keys {
    pub preset: String,
    pub bindings: BTreeMap<String, OneOrMany>,
}

impl Default for Keys {
    fn default() -> Self {
        Keys { preset: "standard".into(), bindings: BTreeMap::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    pub fn keys(&self) -> Vec<String> {
        match self {
            OneOrMany::One(k) => vec![k.clone()],
            OneOrMany::Many(v) => v.clone(),
        }
    }
}

pub fn load_keys(path: &Path) -> Read<Keys> {
    load(path, "keys.toml", DEFAULT_KEYS, &[], KEYS_SCHEMA)
}

/// How a setting is edited on the Settings page (#57).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    /// on or off
    Toggle,
    /// one of these strings
    Choice(&'static [&'static str]),
    /// one of a list the Client knows (designs, themes, saved layouts); "" is "the design's own"
    Named,
    /// a number in `min..=max`, stepped by `step`; `whole`: written as an integer
    Number { min: f64, max: f64, step: f64, whole: bool },
}

/// Every setting the Settings page shows, in the file's order, and how it's edited.
pub const SETTINGS: &[(&str, Kind)] = &[
    ("ui.design", Kind::Named),
    ("ui.theme", Kind::Named),
    ("ui.transparent", Kind::Toggle),
    ("ui.layout", Kind::Named),
    ("ui.tint_art", Kind::Toggle),
    ("notifications.track_change", Kind::Toggle),
    ("visuals.gpu", Kind::Choice(&["low-power", "high-performance"])),
    ("visuals.unfocused_fps", Kind::Number { min: 1.0, max: 30.0, step: 1.0, whole: true }),
    ("idle.enabled", Kind::Toggle),
    ("idle.minutes", Kind::Number { min: 0.5, max: 120.0, step: 0.5, whole: false }),
    ("engine.kind", Kind::Choice(&["auto", "soloist", "spotify-app", "librespot"])),
    ("engine.cache_gb", Kind::Number { min: 0.0, max: 500.0, step: 1.0, whole: false }),
];

/// One row of the Settings page: a setting as the file has it (or its default, if the file leaves it out).
#[derive(Debug, Clone)]
pub struct Entry {
    /// `visuals.gpu`
    pub key: &'static str,
    pub kind: Kind,
    pub value: Value,
    /// the comment above it in the file (the default file's, if the user's has none), without the `#`s
    pub description: String,
}

impl Entry {
    /// `visuals`
    pub fn section(&self) -> &'static str {
        self.key.split_once('.').map_or("", |(s, _)| s)
    }
    /// `gpu`
    pub fn name(&self) -> &'static str {
        self.key.split_once('.').map_or(self.key, |(_, n)| n)
    }
}

/// The comment lines just above `dotted` in a document, joined into one line.
fn comment_above(doc: &DocumentMut, dotted: &str) -> Option<String> {
    let (table, key) = dotted.split_once('.')?;
    let k = doc.get(table)?.as_table()?.key(key)?;
    let raw = k.leaf_decor().prefix()?.as_str()?;
    // the comment block right above the key (a blank line ends it)
    let mut lines: Vec<&str> = vec![];
    for l in raw.lines().rev() {
        match l.trim().strip_prefix('#') {
            Some(c) => lines.push(c.trim()),
            None if lines.is_empty() => continue,
            None => break,
        }
    }
    lines.reverse();
    let s = lines.join(" ");
    (!s.is_empty()).then_some(s)
}

fn value_at(doc: &DocumentMut, dotted: &str) -> Option<Value> {
    let (table, key) = dotted.split_once('.')?;
    doc.get(table)?.as_table()?.get(key)?.as_value().cloned()
}

/// The Settings page's rows for `config.toml`'s text: each known setting, its value and description.
pub fn entries(text: &str) -> Vec<Entry> {
    let default: DocumentMut = DEFAULT_CONFIG.parse().expect("the default config parses");
    let doc: DocumentMut = text.parse().unwrap_or_else(|_| default.clone());
    SETTINGS
        .iter()
        .map(|&(key, kind)| {
            let mut value = value_at(&doc, key).or_else(|| value_at(&default, key)).expect("every setting is in the default file");
            value.decor_mut().clear();
            let description = comment_above(&doc, key).or_else(|| comment_above(&default, key)).unwrap_or_default();
            Entry { key, kind, value, description }
        })
        .collect()
}

/// Notices when a file was saved (polled: once a second is plenty for a human editing a file).
#[derive(Debug)]
pub struct Watched {
    pub path: PathBuf,
    stamp: Option<(SystemTime, u64)>,
}

impl Watched {
    pub fn new(path: PathBuf) -> Watched {
        let stamp = stamp(&path);
        Watched { path, stamp }
    }

    /// True once per change since the last call.
    pub fn changed(&mut self) -> bool {
        let now = stamp(&self.path);
        if now != self.stamp {
            self.stamp = now;
            return true;
        }
        false
    }
}

fn stamp(p: &Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(p).ok()?;
    Some((m.modified().ok()?, m.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_come_with_their_values_and_the_comments_above_them() {
        let e = entries(DEFAULT_CONFIG);
        assert_eq!(e.len(), SETTINGS.len());
        let gpu = e.iter().find(|e| e.key == "visuals.gpu").unwrap();
        assert_eq!((gpu.section(), gpu.name(), gpu.value.as_str()), ("visuals", "gpu", Some("low-power")));
        assert!(gpu.description.starts_with("which graphics chip draws the black hole"), "{}", gpu.description);
        // two comment lines become one; the first key of a table has its comment too
        let idle = e.iter().find(|e| e.key == "idle.enabled").unwrap();
        assert!(idle.description.contains("drifts over the screen; any key wakes it (Shift+I"), "{}", idle.description);
        // the user's own comment wins; a setting left out of the file shows its default and the default's comment
        let mine = "schema = 2\n[ui]\n# my words\ntransparent = true # tail\n";
        let e = entries(mine);
        let t = e.iter().find(|e| e.key == "ui.transparent").unwrap();
        assert_eq!((t.description.as_str(), t.value.as_bool()), ("my words", Some(true)));
        let fps = e.iter().find(|e| e.key == "visuals.unfocused_fps").unwrap();
        assert_eq!(fps.value.as_integer(), Some(30));
        assert!(fps.description.starts_with("frames per second"));
        // every setting maps onto Config: the schema and the struct agree
        for (key, _) in SETTINGS {
            assert!(value_at(&DEFAULT_CONFIG.parse().unwrap(), key).is_some(), "{key} in the default file");
        }
    }

    #[test]
    fn a_setting_is_set_whatever_shape_its_section_has() {
        // (#71) an inline table is valid TOML; a stray value in a section's place isn't, but mustn't crash the Client
        let mut doc: DocumentMut = "[visuals]\ngpu = \"low-power\"\nui = { design = \"nebula\", transparent = true }\n".parse().unwrap();
        let mut doc2: DocumentMut = "idle = 5\n".parse().unwrap();
        set_value(&mut doc, "visuals.ui.design", Value::from("cockpit"));
        set_value(&mut doc2, "idle.minutes", Value::from(7));
        let text = doc.to_string();
        assert!(text.contains("cockpit") && text.contains("transparent = true"), "{text}");
        assert_eq!(value_at(&doc2.to_string().parse().unwrap(), "idle.minutes").and_then(|v| v.as_integer()), Some(7));
    }

    #[test]
    fn the_default_files_parse_to_the_defaults() {
        let r = parse::<Config>("config.toml", DEFAULT_CONFIG, CONFIG_MIGRATIONS, CONFIG_SCHEMA);
        assert_eq!((r.value, r.problems.len(), r.migrated), (Config::default(), 0, None));
        let k = parse::<Keys>("keys.toml", DEFAULT_KEYS, &[], KEYS_SCHEMA);
        assert_eq!((k.value, k.problems.len()), (Keys::default(), 0));
    }

    /// A pretend history: schema 1 called the GPU setting `visuals.power` with "low"/"high"; schema 2 renamed it.
    fn v1_to_v2(doc: &mut DocumentMut) {
        let Some(visuals) = doc.get_mut("visuals").and_then(Item::as_table_mut) else { return };
        if !rename(visuals, "power", "gpu") {
            return;
        }
        let new = if visuals["gpu"].as_str() == Some("high") { "high-performance" } else { "low-power" };
        set_value(doc, "visuals.gpu", Value::from(new));
    }

    #[test]
    fn a_v1_config_migrates_to_v2_keeping_comments() {
        let v1 = "# my settings\nschema = 1\n\n[visuals]\n# the dGPU, please\npower = \"high\"  # it's fast\nunfocused_fps = 5 # tuned\n";
        let r = parse::<Config>("config.toml", v1, &[v1_to_v2], 2);
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert_eq!((r.value.visuals.gpu, r.value.visuals.unfocused_fps), (GpuPower::HighPerformance, 5));
        let text = r.migrated.expect("migrated");
        assert_eq!(text, "# my settings\nschema = 2\n\n[visuals]\n# the dGPU, please\ngpu = \"high-performance\"  # it's fast\nunfocused_fps = 5 # tuned\n");
        assert!(!text.contains("power ="));
        // running it again changes nothing
        let again = parse::<Config>("config.toml", &text, &[v1_to_v2], 2);
        assert!(again.migrated.is_none());
    }

    /// `config.toml` as schema 1 wrote it.
    const DEFAULT_CONFIG_V1: &str = r#"# Singularity settings. Changes apply as soon as you save this file.
# (Don't change `schema`: Singularity uses it to update this file when settings change between versions.)
schema = 1

[ui]
# the design: "singularity", "event-horizon", "classic", "cockpit" or "terminal" (Ctrl+K → Design shows them)
design = "singularity"
# colours: empty for the design's own, else a built-in theme or a file in themes/ (Ctrl+K → Theme, or t)
theme = ""
# let the terminal's own background (and its transparency) show instead of black
transparent = false
# a layout you saved (Ctrl+K → "Save layout as…"), by file name in layouts/; empty for the design's own
layout = ""

[notifications]
# a desktop notification when a new track starts
track_change = false

[visuals]
# which graphics chip draws the black hole: "low-power" (built-in graphics) or "high-performance" (dedicated card)
gpu = "low-power"
# frames per second while the terminal isn't focused (1–30; the Visuals keep moving, slower)
unfocused_fps = 10

[idle]
# after this many minutes without input the black hole leaves its Pane and drifts over the screen; any key wakes it
# (Shift+I switches it off and on)
enabled = true
minutes = 3
"#;

    #[test]
    fn a_schema_1_config_gains_tint_art_with_its_comment() {
        let r = parse::<Config>("config.toml", DEFAULT_CONFIG_V1, CONFIG_MIGRATIONS, CONFIG_SCHEMA);
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert_eq!(r.migrated.as_deref(), Some(DEFAULT_CONFIG), "an untouched old file becomes today's default");
        // a user's own choices and comments stay; a [ui] without the key gets it, switched on
        let mine = "schema = 1\n[ui]\ndesign = \"cockpit\" # mine\n";
        let r = parse::<Config>("config.toml", mine, CONFIG_MIGRATIONS, CONFIG_SCHEMA);
        assert!(r.value.ui.tint_art && r.value.ui.design == "cockpit");
        assert!(r.migrated.unwrap().contains("design = \"cockpit\" # mine\n# covers"));
        let off = parse::<Config>("config.toml", "schema = 5\n[ui]\ntint_art = false\n", CONFIG_MIGRATIONS, CONFIG_SCHEMA);
        assert!(!off.value.ui.tint_art && off.migrated.is_none());
    }

    #[test]
    fn a_schema_2_config_gains_the_engine_section() {
        let mine = "schema = 2\n\n[idle]\n# never\nenabled = false\n";
        let r = parse::<Config>("config.toml", mine, CONFIG_MIGRATIONS, CONFIG_SCHEMA);
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        let text = r.migrated.unwrap();
        assert!(text.starts_with("schema = 5\n\n[idle]\n# never\nenabled = false\n\n[engine]\n# what plays the music"), "{text}");
        assert_eq!((r.value.engine.kind, r.value.idle.enabled), (EngineKind::Auto, false));
        // an [engine] of the user's own is left as it is
        let theirs = parse::<Config>("config.toml", "schema = 2\n[engine]\nkind = \"soloist\"\n", CONFIG_MIGRATIONS, CONFIG_SCHEMA);
        assert_eq!(theirs.value.engine.kind, EngineKind::Soloist);
        assert!(theirs.migrated.unwrap().starts_with("schema = 5\n[engine]\nkind = \"soloist\"\n# disk space"));
        assert_eq!(EngineKind::Auto.resolved(), if cfg!(target_os = "macos") { EngineKind::Librespot } else { EngineKind::Soloist });
    }

    #[test]
    fn a_schema_3_config_gains_the_cache_size_keeping_its_engine() {
        let mine = "schema = 3\n\n[engine]\n# mine\nkind = \"librespot\"\n";
        let r = parse::<Config>("config.toml", mine, CONFIG_MIGRATIONS, CONFIG_SCHEMA);
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        let text = r.migrated.unwrap();
        assert!(text.starts_with("schema = 5\n\n[engine]\n# mine\nkind = \"librespot\"\n# disk space for Soloist's cache"), "{text}");
        assert!(text.ends_with("cache_gb = 5\n"), "{text}");
        assert_eq!((r.value.engine.kind, r.value.engine.cache_gb), (EngineKind::Librespot, 5.0));
        // the Settings page finds it, with its comment
        let e = entries(&text).into_iter().find(|e| e.key == "engine.cache_gb").unwrap();
        assert!(e.description.contains("0 = no limit"), "{}", e.description);
    }

    #[test]
    fn an_untouched_unfocused_rate_of_10_becomes_30_and_a_chosen_one_stays() {
        let old = "schema = 4\n\n[visuals]\n# frames per second while the terminal isn't focused (1–30; the Visuals keep moving, slower)\nunfocused_fps = 10 # hm\n";
        let r = parse::<Config>("config.toml", old, CONFIG_MIGRATIONS, CONFIG_SCHEMA);
        assert_eq!(r.value.visuals.unfocused_fps, 30);
        assert_eq!(r.migrated.as_deref(), Some("schema = 5\n\n[visuals]\n# frames per second while the terminal isn't focused (1–30; lower saves a little CPU, but looks choppier)\nunfocused_fps = 30 # hm\n"));
        let mine = parse::<Config>("config.toml", "schema = 4\n[visuals]\nunfocused_fps = 12\n", CONFIG_MIGRATIONS, CONFIG_SCHEMA);
        assert_eq!(mine.value.visuals.unfocused_fps, 12);
    }

    #[test]
    fn the_cache_size_becomes_soloists_megabytes() {
        let mb = |gb| EngineChoice { cache_gb: gb, ..Default::default() }.soloist_cache_mb();
        assert_eq!((mb(5.0), mb(0.0), mb(0.05), mb(1.5), mb(-3.0)), (5120, 0, 100, 1536, 5120), "default, no limit, Soloist's minimum, a half, nonsense");
    }

    #[test]
    fn bad_values_fall_back_and_are_reported_not_fatal() {
        let r = parse::<Config>("config.toml", "schema = 1\n[visuals]\ngpu = \"quantum\"\n", CONFIG_MIGRATIONS, CONFIG_SCHEMA);
        assert_eq!(r.value, Config::default());
        assert!(r.problems[0].contains("quantum"), "{:?}", r.problems);
        let r = parse::<Config>("config.toml", "this is = = not toml", CONFIG_MIGRATIONS, CONFIG_SCHEMA);
        assert_eq!((r.value, r.problems.len()), (Config::default(), 1));
        let r = parse::<Config>("config.toml", "schema = 9\n", CONFIG_MIGRATIONS, CONFIG_SCHEMA);
        assert!(r.problems[0].contains("newer Singularity"));
    }

    #[test]
    fn a_save_from_an_editor_during_a_setting_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.toml");
        std::fs::write(&p, DEFAULT_CONFIG).unwrap();
        // the editor's save lands between our read and our rename, once
        let mut saves = 0;
        set_watched(&p, DEFAULT_CONFIG, "notifications.track_change", Value::from(true), |path| {
            if saves == 0 {
                saves += 1;
                let text = std::fs::read_to_string(path).unwrap().replace("unfocused_fps = 30", "unfocused_fps = 15 # saved by the editor");
                std::fs::write(path, text).unwrap();
            }
        })
        .unwrap();
        assert_eq!(saves, 1, "the editor saved once");
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("track_change = true"), "our change landed:\n{text}");
        assert!(text.contains("unfocused_fps = 15 # saved by the editor"), "the editor's change survived:\n{text}");
        assert!(text.contains("# a desktop notification when a new track starts"), "comments stayed:\n{text}");
    }

    #[test]
    fn a_file_that_keeps_changing_is_refused_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.toml");
        std::fs::write(&p, DEFAULT_CONFIG).unwrap();
        let mut saves = 0;
        let err = set_watched(&p, DEFAULT_CONFIG, "notifications.track_change", Value::from(true), |path| {
            saves += 1;
            let text = std::fs::read_to_string(path).unwrap().replace("unfocused_fps = 30", &format!("unfocused_fps = 30 # save {saves}"));
            std::fs::write(path, text).unwrap();
        })
        .unwrap_err();
        assert!(err.to_string().contains("kept changing"), "{err}");
        assert!(saves >= 2, "it re-read and tried again");
    }

    #[test]
    fn setting_a_value_keeps_the_rest_of_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.toml");
        let mine = DEFAULT_CONFIG.replace("track_change = false", "track_change = false  # off for now");
        std::fs::write(&p, &mine).unwrap();
        set(&p, DEFAULT_CONFIG, "notifications.track_change", Value::from(true)).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert_eq!(text, mine.replace("track_change = false  # off for now", "track_change = true  # off for now"));
        set(&p, DEFAULT_CONFIG, "idle.minutes", Value::from(5)).unwrap();
        let r = load_config(&p);
        assert!(r.value.notifications.track_change && r.problems.is_empty());
        assert_eq!(r.value.idle.minutes, 5.0);
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("# (Shift+I switches it off and on)\nenabled = true\nminutes = 5\n"), "{text}");
        // a table that isn't there yet is added at the end
        set(&p, DEFAULT_CONFIG, "later.answer", Value::from(42)).unwrap();
        assert!(std::fs::read_to_string(&p).unwrap().ends_with("[later]\nanswer = 42\n"));
    }

    #[test]
    fn a_missing_file_is_written_and_saves_are_noticed() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub/config.toml");
        let r = load_config(&p);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), DEFAULT_CONFIG);
        assert_eq!(r.value, Config::default());
        let mut w = Watched::new(p.clone());
        assert!(!w.changed());
        write_atomic(&p, &DEFAULT_CONFIG.replace("unfocused_fps = 30", "unfocused_fps = 20")).unwrap();
        assert!(w.changed() && !w.changed());
        assert_eq!(load_config(&p).value.visuals.unfocused_fps, 20);
    }

    #[test]
    fn keys_take_one_key_or_a_list() {
        let k = parse::<Keys>("keys.toml", "preset = \"vim\"\n[bindings]\nlike = \"f\"\nnext = [\"n\", \"Ctrl+Right\"]\n", &[], KEYS_SCHEMA).value;
        assert_eq!(k.preset, "vim");
        assert_eq!(k.bindings["like"].keys(), vec!["f"]);
        assert_eq!(k.bindings["next"].keys(), vec!["n", "Ctrl+Right"]);
    }
}
