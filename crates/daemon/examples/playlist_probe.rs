//! Dev tool: quiet, reverted live checks of the playlist writes (batch 3) on a scratch playlist this tool creates and
//! deletes itself, with the saved Catalogue login. Prints only yes/no and timings, never the library.
//!
//!     cargo run -p singularityd --example playlist_probe -- --create-delete
//!
//! Spotify's "delete" takes the playlist out of the library (as the web player does); the emptied playlist stays
//! behind on Spotify's side, unlisted.

use librespot_core::{cache::Cache, config::SessionConfig, session::Session};
use serde_json::Value;
use singularityd::catalogue::Dirs;
use singularityd::playlists;
use singularityd::queries::QueryRegistry;

const SCRATCH: &str = "singularity scratch (safe to delete)";

/// Whether the library lists `uri`, and whether it's marked ours (filtered by the scratch name: one small page).
async fn listed(session: &Session, registry: &mut QueryRegistry, uri: &str) -> anyhow::Result<(bool, bool)> {
    let mut vars = singularityd::library::library_vars(0, None);
    vars["textFilter"] = Value::from(SCRATCH);
    let hash = registry.hash("libraryV3").await?;
    let v = session.spclient().pathfinder_query("libraryV3", &hash, vars).await?;
    let page = singularityd::library::parse_library(&v, 0, &session.username());
    let it = page.items.iter().find(|i| i.uri == uri);
    Ok((it.is_some(), it.is_some_and(|i| i.own)))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "singularityd=debug".into())).with_writer(std::io::stderr).init();
    let mode = std::env::args().nth(1).unwrap_or_default();
    let dirs = Dirs::from_env();
    let cache = Cache::new(Some(&dirs.credentials), None, None, None)?;
    let creds = singularityd::catalogue::saved_credentials(&dirs).ok_or_else(|| anyhow::anyhow!("no saved login; log in from the Client first"))?;
    let session = Session::new(SessionConfig::default(), Some(cache));
    session.connect(creds, true).await?;
    let mut registry = QueryRegistry::new(dirs.queries.clone());
    let root = playlists::rootlist_path(&session.username());
    match mode.as_str() {
        "--create-delete" => {
            let t = std::time::Instant::now();
            let made = playlists::post(&session, "/playlist", &playlists::create(SCRATCH)).await?;
            let uri = made["uri"].as_str().ok_or_else(|| anyhow::anyhow!("no uri in {:?}", made.as_object().map(|o| o.keys().collect::<Vec<_>>())))?.to_string();
            println!("create: a playlist URI came back ({:?})", t.elapsed());
            playlists::post(&session, &root, &playlists::rootlist_add(&uri, singularity_proto::now_ms())).await?;
            let (there, own) = listed(&session, &mut registry, &uri).await?;
            println!("in the library: {there}, marked ours: {own}");
            playlists::post(&session, &root, &playlists::rootlist_remove(&uri)).await?;
            let (there, _) = listed(&session, &mut registry, &uri).await?;
            println!("after delete, in the library: {there}");
        }
        _ => anyhow::bail!("modes: --create-delete"),
    }
    Ok(())
}
