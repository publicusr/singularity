//! Dev tool: quiet, reverted live checks of the playlist writes (batch 3) on a scratch playlist this tool creates and
//! deletes itself, with the saved Catalogue login. Prints only yes/no and timings, never the library.
//!
//!     cargo run -p singularityd --example playlist_probe -- --create-delete
//!     cargo run -p singularityd --example playlist_probe -- --details
//!     cargo run -p singularityd --example playlist_probe -- --add
//!     cargo run -p singularityd --example playlist_probe -- --remove
//!     cargo run -p singularityd --example playlist_probe -- --move
//!     cargo run -p singularityd --example playlist_probe -- --cover some.jpg
//!
//! Spotify's "delete" takes the playlist out of the library (as the web player does); the emptied playlist stays
//! behind on Spotify's side, unlisted.

use librespot_core::{cache::Cache, config::SessionConfig, session::Session};
use serde_json::Value;
use singularityd::catalogue::Dirs;
use singularityd::playlists;
use singularityd::queries::QueryRegistry;

const SCRATCH: &str = "singularity scratch (safe to delete)";
/// A well-known public track to add to the scratch playlist.
const TRACK: &str = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";

/// One Pathfinder operation (the probe has no 412 refresh: run it with a fresh registry).
async fn op(session: &Session, registry: &mut QueryRegistry, name: &str, vars: Value) -> anyhow::Result<Value> {
    let hash = registry.hash(name).await?;
    Ok(session.spclient().pathfinder_query(name, &hash, vars).await?)
}

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

/// The playlist's name and description as its page says (fetchPlaylist, first page).
async fn read_details(session: &Session, registry: &mut QueryRegistry, uri: &str) -> anyhow::Result<(String, String)> {
    let (op, vars) = singularityd::library::tracks_query(uri, 0).unwrap();
    let hash = registry.hash(op).await?;
    let v = session.spclient().pathfinder_query(op, &hash, vars).await?;
    let name = v["data"]["playlistV2"]["name"].as_str().unwrap_or_default().to_string();
    Ok((name, singularityd::library::parse_header(uri, &v).map(|h| h.description).unwrap_or_default()))
}

/// The playlist's rows' uids, in order (first page).
async fn row_uids(session: &Session, registry: &mut QueryRegistry, uri: &str) -> anyhow::Result<Vec<String>> {
    let (name, vars) = singularityd::library::tracks_query(uri, 0).unwrap();
    let (rows, _) = singularityd::library::parse_tracks(uri, &op(session, registry, name, vars).await?);
    Ok(rows.into_iter().filter_map(|r| r.uid).collect())
}

/// The cover the playlist's page shows, and the library's for it.
async fn covers_of(session: &Session, registry: &mut QueryRegistry, uri: &str) -> anyhow::Result<(Option<String>, Option<String>)> {
    let (name, vars) = singularityd::library::tracks_query(uri, 0).unwrap();
    let page = singularityd::library::parse_header(uri, &op(session, registry, name, vars).await?).and_then(|h| h.cover_url);
    let mut vars = singularityd::library::library_vars(0, None);
    vars["textFilter"] = Value::from(SCRATCH);
    let v = op(session, registry, "libraryV3", vars).await?;
    let lib = singularityd::library::parse_library(&v, 0, &session.username()).items.into_iter().find(|i| i.uri == uri).and_then(|i| i.image_url);
    Ok((page, lib))
}

/// A scratch playlist in the library; its URI.
async fn scratch(session: &Session, root: &str) -> anyhow::Result<String> {
    let made = playlists::post(session, "/playlist", &playlists::create(SCRATCH)).await?;
    let uri = made["uri"].as_str().ok_or_else(|| anyhow::anyhow!("no uri"))?.to_string();
    playlists::post(session, root, &playlists::rootlist_add(&uri, singularity_proto::now_ms())).await?;
    Ok(uri)
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
        "--details" => {
            let uri = scratch(&session, &root).await?;
            let renamed = format!("{SCRATCH} renamed");
            let checked = async {
                let path = playlists::changes_path(&uri).unwrap();
                let t = std::time::Instant::now();
                playlists::post(&session, &path, &playlists::details(&renamed, "probe & <b>words</b>")).await?;
                println!("rename + describe: accepted ({:?})", t.elapsed());
                let (name, desc) = read_details(&session, &mut registry, &uri).await?;
                println!("read back: name matches {}, description matches {} ({desc:?})", name == renamed, desc == "probe & <b>words</b>");
                playlists::post(&session, &path, &playlists::details(&renamed, "")).await?;
                let (_, desc) = read_details(&session, &mut registry, &uri).await?;
                println!("emptied: description gone {}", desc.is_empty());
                let (there, _) = listed(&session, &mut registry, &uri).await?;
                println!("library lists it (by the new name's filter): {there}");
                anyhow::Ok(())
            };
            let res = checked.await;
            // whatever happened, the scratch playlist goes
            playlists::post(&session, &root, &playlists::rootlist_remove(&uri)).await?;
            let (there, _) = listed(&session, &mut registry, &uri).await?;
            println!("after delete, in the library: {there}");
            res?;
        }
        "--add" => {
            let uri = scratch(&session, &root).await?;
            let uris = vec![TRACK.to_string()];
            let checked = async {
                let v = op(&session, &mut registry, "editablePlaylists", playlists::editable_vars(SCRATCH, &uris)).await?;
                println!("editablePlaylists: lists the scratch {}, has the track {:?}", playlists::curates(&v, &uri).is_some(), playlists::curates(&v, &uri));
                let t = std::time::Instant::now();
                let v = op(&session, &mut registry, "addToPlaylist", playlists::add_vars(&uri, &uris)).await?;
                println!("addToPlaylist: errors {:?}, top-level keys {:?} ({:?})", playlists::graphql_error(&v), v["data"].as_object().map(|o| o.keys().collect::<Vec<_>>()), t.elapsed());
                let (op_name, vars) = singularityd::library::tracks_query(&uri, 0).unwrap();
                let page = op(&session, &mut registry, op_name, vars).await?;
                let (rows, total) = singularityd::library::parse_tracks(&uri, &page);
                let uid = page["data"]["playlistV2"]["content"]["items"][0]["uid"].is_string();
                println!("read back: total {total}, it's the track {}, row has a uid {uid}", rows.first().is_some_and(|r| r.uri == TRACK));
                let v = op(&session, &mut registry, "editablePlaylists", playlists::editable_vars(SCRATCH, &uris)).await?;
                println!("editablePlaylists now: has the track {:?}", playlists::curates(&v, &uri));
                anyhow::Ok(())
            };
            let res = checked.await;
            playlists::post(&session, &root, &playlists::rootlist_remove(&uri)).await?;
            let (there, _) = listed(&session, &mut registry, &uri).await?;
            println!("after delete, in the library: {there}");
            res?;
        }
        "--remove" => {
            let uri = scratch(&session, &root).await?;
            let uris = vec![TRACK.to_string()];
            let checked = async {
                // the same song twice: removing one row leaves the other
                for _ in 0..2 {
                    let v = op(&session, &mut registry, "addToPlaylist", playlists::add_vars(&uri, &uris)).await?;
                    anyhow::ensure!(playlists::graphql_error(&v).is_none(), "add refused");
                }
                let (op_name, vars) = singularityd::library::tracks_query(&uri, 0).unwrap();
                let (rows, _) = singularityd::library::parse_tracks(&uri, &op(&session, &mut registry, op_name, vars.clone()).await?);
                let uids: Vec<String> = rows.iter().filter_map(|r| r.uid.clone()).collect();
                println!("two rows of one song: {}, distinct uids: {}", rows.len() == 2, uids.len() == 2 && uids[0] != uids[1]);
                let t = std::time::Instant::now();
                let v = op(&session, &mut registry, "removeFromPlaylist", playlists::remove_vars(&uri, &uids[..1])).await?;
                println!("removeFromPlaylist: errors {:?}, keys {:?} ({:?})", playlists::graphql_error(&v), v["data"].as_object().map(|o| o.keys().collect::<Vec<_>>()), t.elapsed());
                let (rows, total) = singularityd::library::parse_tracks(&uri, &op(&session, &mut registry, op_name, vars).await?);
                println!("read back: total {total}, the other copy stayed {}", rows.len() == 1 && rows[0].uid.as_deref() == uids.get(1).map(String::as_str));
                anyhow::Ok(())
            };
            let res = checked.await;
            playlists::post(&session, &root, &playlists::rootlist_remove(&uri)).await?;
            let (there, _) = listed(&session, &mut registry, &uri).await?;
            println!("after delete, in the library: {there}");
            res?;
        }
        "--move" => {
            let uri = scratch(&session, &root).await?;
            let uris = vec![TRACK.to_string()];
            let checked = async {
                // three rows told apart by uid
                for _ in 0..3 {
                    let v = op(&session, &mut registry, "addToPlaylist", playlists::add_vars(&uri, &uris)).await?;
                    anyhow::ensure!(playlists::graphql_error(&v).is_none(), "add refused");
                }
                let u = row_uids(&session, &mut registry, &uri).await?;
                anyhow::ensure!(u.len() == 3, "expected 3 rows");
                let t = std::time::Instant::now();
                let v = op(&session, &mut registry, "moveItemsInPlaylist", playlists::move_vars(&uri, &u[..1], None, Some(&u[2]))).await?;
                println!("move after the last: errors {:?}, keys {:?} ({:?})", playlists::graphql_error(&v), v["data"].as_object().map(|o| o.keys().collect::<Vec<_>>()), t.elapsed());
                let mut want = u.clone();
                playlists::apply_move(&mut want, |x| Some(x.as_str()), &u[..1], None, Some(&u[2]));
                let got = row_uids(&session, &mut registry, &uri).await?;
                println!("read back in the expected order: {}", got == want);
                let v = op(&session, &mut registry, "moveItemsInPlaylist", playlists::move_vars(&uri, &got[2..], Some(&got[0]), None)).await?;
                playlists::apply_move(&mut want, |x| Some(x.as_str()), &got[2..], Some(&got[0]), None);
                println!("move before the first: errors {:?}, then as expected: {}", playlists::graphql_error(&v), row_uids(&session, &mut registry, &uri).await? == want);
                anyhow::Ok(())
            };
            let res = checked.await;
            playlists::post(&session, &root, &playlists::rootlist_remove(&uri)).await?;
            let (there, _) = listed(&session, &mut registry, &uri).await?;
            println!("after delete, in the library: {there}");
            res?;
        }
        "--cover" => {
            let jpeg = std::fs::read(std::env::args().nth(2).ok_or_else(|| anyhow::anyhow!("--cover <file.jpg>"))?)?;
            let uri = scratch(&session, &root).await?;
            let checked = async {
                let host = |u: &Option<String>| u.as_ref().map(|u| u.split('/').take(3).collect::<Vec<_>>().join("/"));
                let (p0, l0) = covers_of(&session, &mut registry, &uri).await?;
                println!("before: page cover {:?}, library cover {:?}", host(&p0), host(&l0));
                let t = std::time::Instant::now();
                let token = playlists::upload(&session, jpeg).await?;
                println!("upload: a token came back ({:?})", t.elapsed());
                let reg = playlists::post(&session, &playlists::playlist_path(&uri, "register-image").unwrap(), &serde_json::json!({ "uploadToken": token })).await?;
                let pic = reg["picture"].as_str().ok_or_else(|| anyhow::anyhow!("no picture: keys {:?}", reg.as_object().map(|o| o.keys().collect::<Vec<_>>())))?.to_string();
                println!("register-image: picture is a string of {} chars", pic.len());
                playlists::post(&session, &playlists::changes_path(&uri).unwrap(), &playlists::picture(Some(&pic))).await?;
                let (p1, l1) = covers_of(&session, &mut registry, &uri).await?;
                println!("set: page cover {:?} (changed {}), library cover {:?} (changed {}), same url both {}", host(&p1), p1 != p0, host(&l1), l1 != l0, p1 == l1);
                playlists::post(&session, &playlists::changes_path(&uri).unwrap(), &playlists::picture(None)).await?;
                let (p2, l2) = covers_of(&session, &mut registry, &uri).await?;
                println!("removed: page cover back as before {}, library {}", p2 == p0, l2 == l0);
                anyhow::Ok(())
            };
            let res = checked.await;
            playlists::post(&session, &root, &playlists::rootlist_remove(&uri)).await?;
            let (there, _) = listed(&session, &mut registry, &uri).await?;
            println!("after delete, in the library: {there}");
            res?;
        }
        _ => anyhow::bail!("modes: --create-delete, --details, --add, --remove, --move, --cover <jpg>"),
    }
    Ok(())
}
