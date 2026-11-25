//! The rule against the lookups it replaced (#97): each old one is copied here, reading a made-up environment, and
//! every folder must come out the same for every mix of `XDG_*` set, unset or empty, with `HOME` set. With `HOME`
//! unset the old ones fell back to a temp folder (or failed); now the app stops.

use super::*;
use std::collections::HashMap;

type Env = HashMap<&'static str, OsString>;

fn lookup(env: &Env) -> impl Fn(&str) -> Option<OsString> + '_ {
    |k| env.get(k).cloned()
}

const TMP: &str = "/made-up-tmp";

/// The lookups before App folders, one per call site, as they read the environment.
struct Old<'a>(&'a Env);

impl Old<'_> {
    fn var(&self, k: &str) -> Option<OsString> {
        self.0.get(k).cloned()
    }
    fn home_or_tmp(&self) -> PathBuf {
        self.var("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(TMP))
    }
    /// config's `dir()`, the Client's `start_daemon` log folder and `SoloistEngine::from_env` (one shape)
    fn xdg_then_home_then_tmp(&self, var: &str, under: &str) -> PathBuf {
        self.var(var).map(PathBuf::from).or_else(|| self.var("HOME").map(|h| PathBuf::from(h).join(under))).unwrap_or_else(|| PathBuf::from(TMP))
    }
    /// the Client's `cache_dir`, `recent_file`, the Catalogue's `Dirs`, `Store::default_dir`, the secrets' fallback
    /// folder (the other shape)
    fn xdg_else_home(&self, var: &str, under: &str) -> PathBuf {
        self.var(var).map(PathBuf::from).unwrap_or_else(|| self.home_or_tmp().join(under))
    }
    fn config_dir(&self) -> PathBuf {
        self.xdg_then_home_then_tmp("XDG_CONFIG_HOME", ".config").join("singularity")
    }
    fn daemon_log_dir(&self) -> PathBuf {
        self.xdg_then_home_then_tmp("XDG_STATE_HOME", ".local/state").join("singularity")
    }
    fn soloist_data(&self) -> PathBuf {
        self.xdg_then_home_then_tmp("XDG_DATA_HOME", ".local/share").join("singularity")
    }
    fn client_cache(&self) -> PathBuf {
        self.xdg_else_home("XDG_CACHE_HOME", ".cache").join("singularity")
    }
    fn recent_searches(&self) -> PathBuf {
        self.xdg_else_home("XDG_STATE_HOME", ".local/state").join("singularity/recent-searches.json")
    }
    fn catalogue_credentials(&self) -> PathBuf {
        self.xdg_else_home("XDG_DATA_HOME", ".local/share").join("singularity/catalogue")
    }
    fn catalogue_queries(&self) -> PathBuf {
        self.xdg_else_home("XDG_CACHE_HOME", ".cache").join("singularity/pathfinder-queries.json")
    }
    fn store(&self) -> PathBuf {
        self.xdg_else_home("XDG_DATA_HOME", ".local/share").join("singularity")
    }
    fn secrets(&self) -> PathBuf {
        self.xdg_else_home("XDG_CONFIG_HOME", ".config").join("singularity/secrets")
    }
    /// MPRIS's cover cache: no `HOME`, no cache
    fn mpris_cache(&self) -> Option<PathBuf> {
        let home = self.var("HOME").map(PathBuf::from)?;
        Some(self.var("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".cache")).join("singularity"))
    }
    /// proto's `socket_path` folder
    fn socket_dir(&self, host_uid: &str) -> PathBuf {
        let uid = self.var("UID").and_then(|v| v.into_string().ok()).unwrap_or_else(|| host_uid.into());
        self.var("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(TMP).join(format!("singularity-{uid}"))).join("singularity")
    }
}

const XDG: [&str; 4] = ["XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"];

/// Every mix of the four `XDG_*` variables unset, set (absolute, with a trailing slash, relative) or set empty, with
/// `home` as `HOME`.
fn every_mix(home: Option<&str>) -> Vec<Env> {
    const KINDS: usize = 5;
    let mut out = vec![];
    for n in 0..KINDS.pow(XDG.len() as u32) {
        let mut env = Env::new();
        if let Some(h) = home {
            env.insert("HOME", h.into());
        }
        for (i, var) in XDG.iter().enumerate() {
            match n / KINDS.pow(i as u32) % KINDS {
                0 => {}
                1 => drop(env.insert(var, format!("/x/{var}").into())),
                2 => drop(env.insert(var, format!("/x/{var}/").into())),
                3 => drop(env.insert(var, format!("rel/{var}").into())),
                _ => drop(env.insert(var, "".into())),
            }
        }
        out.push(env);
    }
    out
}

#[test]
fn every_folder_is_where_it_was_with_home_set() {
    for home in ["/home/someone", "/home/someone/", "rel-home", ""] {
        for env in every_mix(Some(home)) {
            let old = Old(&env);
            let new = AppFolders::from_lookup(lookup(&env)).unwrap();
            let why = format!("{env:?}");
            assert_eq!(new.config, old.config_dir(), "config, {why}");
            assert_eq!(new.config.join("secrets"), old.secrets(), "secrets, {why}");
            assert_eq!(new.data, old.soloist_data(), "Soloist's data, {why}");
            assert_eq!(new.data, old.store(), "the store, {why}");
            assert_eq!(new.data.join("catalogue"), old.catalogue_credentials(), "Catalogue login, {why}");
            assert_eq!(new.state, old.daemon_log_dir(), "the Daemon's log, {why}");
            assert_eq!(new.state.join("recent-searches.json"), old.recent_searches(), "recent searches, {why}");
            assert_eq!(new.cache, old.client_cache(), "covers, {why}");
            assert_eq!(new.cache.join("pathfinder-queries.json"), old.catalogue_queries(), "Catalogue queries, {why}");
            assert_eq!(Some(new.cache.clone()), old.mpris_cache(), "MPRIS covers, {why}");
        }
    }
}

#[test]
fn without_home_the_app_stops_unless_every_folder_is_named() {
    for env in every_mix(None) {
        let all_named = XDG.iter().all(|v| env.contains_key(v));
        match AppFolders::from_lookup(lookup(&env)) {
            Ok(new) => {
                assert!(all_named, "{env:?} has a folder that needs HOME");
                // where every folder is named, nothing needed HOME and nothing moved
                let old = Old(&env);
                assert_eq!((new.config, new.data, new.state, new.cache), (old.config_dir(), old.store(), old.daemon_log_dir(), old.client_cache()));
            }
            Err(e) => {
                assert!(!all_named, "{env:?} names every folder");
                assert_eq!(e, NoHome);
            }
        }
    }
    // today's three behaviours without HOME, all gone: a temp folder, a temp folder without `.config`, an error
    let env = Env::new();
    assert_eq!(Old(&env).config_dir(), Path::new(TMP).join("singularity"));
    assert_eq!(Old(&env).store(), Path::new(TMP).join(".local/share/singularity"));
    assert_eq!(Old(&env).mpris_cache(), None);
    assert_eq!(AppFolders::from_lookup(lookup(&env)), Err(NoHome));
}

#[test]
fn the_message_without_home_says_what_to_set() {
    let m = NoHome.to_string();
    assert!(m.starts_with("HOME isn't set"), "{m}");
    for v in ["HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"] {
        assert!(m.contains(v), "{m} names {v}");
    }
}

#[test]
fn the_socket_folder_keeps_its_own_rule() {
    let host_uid = if crate::host::MACOS { "user".to_string() } else { crate::host::uid().to_string() };
    for runtime in [None, Some("/run/user/7"), Some("")] {
        for uid in [None, Some("42")] {
            // HOME never matters to it
            for home in [None, Some("/home/someone")] {
                let mut env = Env::new();
                for (k, v) in [("XDG_RUNTIME_DIR", runtime), ("UID", uid), ("HOME", home)] {
                    if let Some(v) = v {
                        env.insert(k, v.into());
                    }
                }
                assert_eq!(socket_dir_from(lookup(&env), Path::new(TMP)), Old(&env).socket_dir(&host_uid), "{env:?}");
            }
        }
    }
    let env: Env = [("UID", "42".into())].into();
    assert_eq!(socket_dir_from(lookup(&env), Path::new(TMP)), Path::new(TMP).join("singularity-42/singularity"));
}

#[test]
fn downloads_are_where_they_were() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let config = dir.path().join("config");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&config).unwrap();
    let env = |pairs: &[(&'static str, &Path)]| -> Env { pairs.iter().map(|(k, v)| (*k, v.as_os_str().to_owned())).collect() };
    let at = |e: &Env| AppFolders::from_lookup(lookup(e)).unwrap().downloads();

    let plain = env(&[("HOME", &home)]);
    // no Downloads folder: home
    assert_eq!(at(&plain), Ok(home.clone()));
    std::fs::create_dir(home.join("Downloads")).unwrap();
    assert_eq!(at(&plain), Ok(home.join("Downloads")));
    // user-dirs.dirs under XDG_CONFIG_HOME, `$HOME` expanded
    std::fs::write(config.join("user-dirs.dirs"), "# comment\nXDG_DOWNLOAD_DIR=\"$HOME/Fetched\"\n").unwrap();
    let with_config = env(&[("HOME", &home), ("XDG_CONFIG_HOME", &config)]);
    assert_eq!(at(&with_config), Ok(home.join("Fetched")));
    // XDG_DOWNLOAD_DIR first
    let mut named = with_config.clone();
    named.insert("XDG_DOWNLOAD_DIR", "\"$HOME/Elsewhere\"".into());
    assert_eq!(at(&named), Ok(home.join("Elsewhere")));
    named.insert("XDG_DOWNLOAD_DIR", "/somewhere/plain".into());
    assert_eq!(at(&named), Ok(PathBuf::from("/somewhere/plain")));

    // without HOME, only a folder that doesn't need it
    let mut no_home: Env = XDG.iter().map(|v| (*v, config.as_os_str().to_owned())).collect();
    assert_eq!(at(&no_home), Err(NoDownloads), "user-dirs.dirs says $HOME/Fetched");
    std::fs::remove_file(config.join("user-dirs.dirs")).unwrap();
    assert_eq!(at(&no_home), Err(NoDownloads), "nothing names it");
    assert_eq!(NoDownloads.to_string(), "no Downloads folder: set HOME or XDG_DOWNLOAD_DIR");
    no_home.insert("XDG_DOWNLOAD_DIR", "/somewhere/plain".into());
    assert_eq!(at(&no_home), Ok(PathBuf::from("/somewhere/plain")));
}
