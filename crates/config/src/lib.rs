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
use toml_edit::{DocumentMut, Item, Value};

/// The config schema this build writes and reads.
pub const SCHEMA: i64 = 1;

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
    pub notifications: Notifications,
    pub visuals: Visuals,
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
        Visuals { gpu: GpuPower::LowPower, unfocused_fps: 10 }
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
schema = 1

[notifications]
# a desktop notification when a new track starts
track_change = false

[visuals]
# which graphics chip draws the black hole: "low-power" (built-in graphics) or "high-performance" (dedicated card)
gpu = "low-power"
# frames per second while the terminal isn't focused (1–30; the Visuals keep moving, slower)
unfocused_fps = 10
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

/// `config.toml`'s migrations: entry `i` takes schema `i + 1` to `i + 2`. (None yet: schema 1 is the first.)
pub const CONFIG_MIGRATIONS: &[Migration] = &[];

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
    load(path, "config.toml", DEFAULT_CONFIG, CONFIG_MIGRATIONS, SCHEMA)
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
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, path)
}

/// Set `dotted` (`visuals.gpu`) in the document, creating tables as needed; the rest of the file is untouched.
pub fn set_value(doc: &mut DocumentMut, dotted: &str, v: Value) {
    let mut parts: Vec<&str> = dotted.split('.').collect();
    let last = parts.pop().expect("a key");
    let mut table = doc.as_table_mut();
    for p in parts {
        if !table.contains_key(p) {
            table.insert(p, toml_edit::table());
        }
        table = table[p].as_table_mut().expect("a table on the way");
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
    let text = std::fs::read_to_string(path).unwrap_or_else(|_| default.to_string());
    let mut doc = text.parse::<DocumentMut>().map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    set_value(&mut doc, dotted, v);
    write_atomic(path, &doc.to_string())
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
    load(path, "keys.toml", DEFAULT_KEYS, &[], SCHEMA)
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
    fn the_default_files_parse_to_the_defaults() {
        let r = parse::<Config>("config.toml", DEFAULT_CONFIG, CONFIG_MIGRATIONS, SCHEMA);
        assert_eq!((r.value, r.problems.len(), r.migrated), (Config::default(), 0, None));
        let k = parse::<Keys>("keys.toml", DEFAULT_KEYS, &[], SCHEMA);
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

    #[test]
    fn bad_values_fall_back_and_are_reported_not_fatal() {
        let r = parse::<Config>("config.toml", "schema = 1\n[visuals]\ngpu = \"quantum\"\n", CONFIG_MIGRATIONS, SCHEMA);
        assert_eq!(r.value, Config::default());
        assert!(r.problems[0].contains("quantum"), "{:?}", r.problems);
        let r = parse::<Config>("config.toml", "this is = = not toml", CONFIG_MIGRATIONS, SCHEMA);
        assert_eq!((r.value, r.problems.len()), (Config::default(), 1));
        let r = parse::<Config>("config.toml", "schema = 9\n", CONFIG_MIGRATIONS, SCHEMA);
        assert!(r.problems[0].contains("newer Singularity"));
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
        assert!(std::fs::read_to_string(&p).unwrap().ends_with("[idle]\nminutes = 5\n"));
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
        write_atomic(&p, &DEFAULT_CONFIG.replace("unfocused_fps = 10", "unfocused_fps = 20")).unwrap();
        assert!(w.changed() && !w.changed());
        assert_eq!(load_config(&p).value.visuals.unfocused_fps, 20);
    }

    #[test]
    fn keys_take_one_key_or_a_list() {
        let k = parse::<Keys>("keys.toml", "preset = \"vim\"\n[bindings]\nlike = \"f\"\nnext = [\"n\", \"Ctrl+Right\"]\n", &[], SCHEMA).value;
        assert_eq!(k.preset, "vim");
        assert_eq!(k.bindings["like"].keys(), vec!["f"]);
        assert_eq!(k.bindings["next"].keys(), vec!["n", "Ctrl+Right"]);
    }
}
