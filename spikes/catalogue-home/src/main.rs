//! Phase 1 spike (b): log in to Spotify's private API and fetch the real Home feed.
//! Throwaway code. Personal data (names of your playlists/mixes) is printed to the terminal only;
//! FINDINGS.md records structure (shelf counts, item types), never your content.
//!
//! Login: OAuth device flow with Spotify's desktop client id (as librespot/spotatui do). You approve
//! a code in the browser; no password is ever typed into this program, and nothing listens on a port.
//! Reusable credentials are cached in ./state (git-ignored) so later runs skip the login.

use librespot_core::{authentication::Credentials, cache::Cache, config::SessionConfig, session::Session};
use librespot_oauth::DeviceAuthClientBuilder;
use serde_json::json;
use std::io::Write;
use std::path::PathBuf;

/// Spotify's desktop client id (librespot's KEYMASTER_CLIENT_ID).
const CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";
const SCOPES: [&str; 6] = [
    "streaming",
    "user-read-playback-state",
    "user-modify-playback-state",
    "user-read-currently-playing",
    "user-library-read",
    "user-read-private",
];
/// Persisted query hash for `home`, read from the web player bundle on 2026-10-17 (web-player.0079e2e5.js).
/// A retired hash answers HTTP 412; the real app must refresh these automatically.
const HOME_HASH: &str = "5fb7da7a03ee2776c0670856609cb85a189a3cecc36cc160b91cde79063b9cf1";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn say(s: &str) {
    println!("\x1b[38;5;180m▸\x1b[0m {s}");
}

fn findings(line: &str) {
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(root().join("FINDINGS.md")).unwrap();
    let _ = writeln!(f, "{line}");
}

fn tz() -> String {
    std::process::Command::new("timedatectl")
        .args(["show", "-p", "Timezone", "--value"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "UTC".into())
}

#[tokio::main]
async fn main() {
    println!("\n\x1b[1mSingularity · Phase 1 spike: private-API Home feed\x1b[0m\n");
    let state = root().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let cache = Cache::new(Some(&state), None, None, None).expect("cache");
    let credentials = match cache.credentials() {
        Some(c) => {
            say("Using the cached login from a previous run.");
            c
        }
        None => {
            let client = DeviceAuthClientBuilder::new(CLIENT_ID, SCOPES.to_vec()).build().expect("oauth client");
            let auth = match client.request_device_code_async().await {
                Ok(a) => a,
                Err(e) => return say(&format!("Couldn't start the login: {e}")),
            };
            say("Log in to Spotify in your browser:");
            println!("\n      {}\n      code: \x1b[1m{}\x1b[0m\n", auth.url(), auth.user_code());
            let _ = std::process::Command::new("xdg-open").arg(auth.url()).spawn();
            say("Waiting for you to approve…");
            match client.poll_for_token_async(&auth).await {
                Ok(t) => Credentials::with_access_token(t.access_token),
                Err(e) => return say(&format!("Login failed: {e}")),
            }
        }
    };

    let session = Session::new(SessionConfig::default(), Some(cache));
    let t0 = std::time::Instant::now();
    if let Err(e) = session.connect(credentials, true).await {
        say(&format!("Session failed to connect: {e}"));
        findings(&format!("\n## Run\n\n- ❌ Session connect failed: `{e}`"));
        return;
    }
    // country and account type arrive in packets shortly after login
    let mut product = None;
    for _ in 0..30 {
        product = session.get_user_attribute("type");
        if product.is_some() && !session.country().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let product = product.unwrap_or_else(|| "?".into());
    say(&format!("Connected in {:?} · country {} · account type {product}", t0.elapsed(), session.country()));
    findings(&format!("\n## Run\n\n- ✅ Device-code login + librespot session (fork 0.8.6): connected in {:?}, account type `{product}`", t0.elapsed()));

    let vars = json!({
        "includeHomeChapterVideoCards": false,
        "homeEndUserIntegration": "INTEGRATION_WEB_PLAYER",
        "timeZone": tz(),
        "sp_t": "",
        "facet": "",
        "sectionItemsLimit": 10,
        "includeEpisodeContentRatingsV2": false,
    });
    let t1 = std::time::Instant::now();
    let home = match session.spclient().pathfinder_query("home", HOME_HASH, vars).await {
        Ok(v) => v,
        Err(e) => {
            say(&format!("Home query failed: {e}"));
            findings(&format!("- ❌ `home` query failed: `{e}`"));
            return;
        }
    };
    let elapsed = t1.elapsed();
    std::fs::write(state.join("home.json"), serde_json::to_string_pretty(&home).unwrap()).ok();
    let sections = home["data"]["home"]["sectionContainer"]["sections"]["items"].as_array().cloned().unwrap_or_default();
    if sections.is_empty() {
        say("The query answered, but with no sections. Raw answer saved to state/home.json.");
        findings(&format!("- ⚠️ `home` answered in {elapsed:?} but no sections found (shape changed?). Top-level keys: {:?}", home["data"].as_object().map(|o| o.keys().cloned().collect::<Vec<_>>())));
        if let Some(errs) = home["errors"].as_array() {
            for e in errs.iter().take(3) {
                println!("    error: {}", e["message"]);
            }
        }
        return;
    }
    say(&format!("Your Home feed ({} shelves, {elapsed:?}):\n", sections.len()));
    let mut types = std::collections::BTreeMap::<String, usize>::new();
    let mut shelf_kinds = vec![];
    for s in &sections {
        let title = s["data"]["title"]["transformedLabel"]
            .as_str()
            .or_else(|| s["data"]["title"]["text"].as_str())
            .unwrap_or("(untitled shelf)");
        let kind = s["data"]["__typename"].as_str().unwrap_or("?");
        let items = s["sectionItems"]["items"].as_array().cloned().unwrap_or_default();
        println!("  \x1b[1m{title}\x1b[0m  \x1b[2m({kind}, {} items)\x1b[0m", items.len());
        let names: Vec<String> = items
            .iter()
            .take(6)
            .map(|it| {
                let d = &it["content"]["data"];
                let ty = d["__typename"].as_str().unwrap_or("?").to_string();
                *types.entry(ty.clone()).or_default() += 1;
                let name = d["name"].as_str().or_else(|| d["profile"]["name"].as_str()).unwrap_or("?");
                format!("{name} \x1b[2m[{ty}]\x1b[0m")
            })
            .collect();
        println!("    {}", names.join("  ·  "));
        shelf_kinds.push(format!("{kind}×{}", items.len()));
    }
    findings(&format!("- ✅ `home` query (hash `{}…`) answered in {elapsed:?}: {} shelves", &HOME_HASH[..8], sections.len()));
    findings(&format!("- Shelf shapes: {}", shelf_kinds.join(", ")));
    findings(&format!("- Item types seen: {:?}", types));
    println!();
    say("Raw answer saved to state/home.json (git-ignored). FINDINGS.md updated (structure only, no names).");
}
