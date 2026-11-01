//! Playlist editing (batch 3, #51–#56): the web player's own writes, as JSON. Bodies are built here (pure, tested);
//! `post` sends one, once (a retried write could add tracks twice).
//!
//! Shapes read from the web player's bundle on 2026-10-18 (`docs/research/2026-10-playlists-and-podcasts.md`):
//! protobuf messages in their JSON form (enums by name, empty fields left out).

use librespot_core::session::Session;
use serde_json::{Value, json};

/// The web player's playlist host.
pub const HOST: &str = "https://spclient.wg.spotify.com/playlist/v2";

fn delta(ops: Vec<Value>) -> Value {
    json!({ "ops": ops, "info": { "source": { "client": "WEBPLAYER" } } })
}

/// List changes: one delta, no base revision (none of these writes needs one).
fn changes(ops: Vec<Value>) -> Value {
    json!({ "deltas": [delta(ops)] })
}

/// `POST /playlist`: a new playlist with this name (answered by `{uri}`; it isn't in the library until `rootlist_add`).
/// The body is the bare op list: unlike a delta it has no `info` (an unknown field is a 400).
pub fn create(name: &str) -> Value {
    json!({ "ops": [{ "kind": "UPDATE_LIST_ATTRIBUTES", "updateListAttributes": { "newAttributes": { "values": { "name": name } } } }] })
}

/// `POST /user/{username}/rootlist/changes`: put a playlist at the top of the library.
pub fn rootlist_add(uri: &str, now_ms: u64) -> Value {
    changes(vec![json!({ "kind": "ADD", "add": { "addFirst": true, "items": [{ "uri": uri, "attributes": { "timestamp": now_ms.to_string() } }] } })])
}

/// `POST /user/{username}/rootlist/changes`: take a playlist out of the library (Spotify's "delete").
pub fn rootlist_remove(uri: &str) -> Value {
    changes(vec![json!({ "kind": "REM", "rem": { "itemsAsKey": true, "items": [{ "uri": uri }] } })])
}

/// `POST /playlist/{id}/changes`: a new name and description; an empty description is cleared (`noValue`).
pub fn details(name: &str, description: &str) -> Value {
    let mut attrs = json!({ "values": { "name": name } });
    if description.is_empty() {
        attrs["noValue"] = json!(["LIST_DESCRIPTION"]);
    } else {
        attrs["values"]["description"] = Value::from(description);
    }
    changes(vec![json!({ "kind": "UPDATE_LIST_ATTRIBUTES", "updateListAttributes": { "newAttributes": attrs } })])
}

/// `/playlist/{id}/changes` for `spotify:playlist:{id}`.
pub fn changes_path(uri: &str) -> Option<String> {
    playlist_path(uri, "changes")
}

/// `/playlist/{id}/{what}` for `spotify:playlist:{id}` (ids are base62: anything else is refused).
pub fn playlist_path(uri: &str, what: &str) -> Option<String> {
    let id = uri.strip_prefix("spotify:playlist:")?;
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric())).then(|| format!("/playlist/{id}/{what}"))
}

/// The cover upload host (#56): the JPEG goes here first and comes back as an upload token.
pub const UPLOAD: &str = "https://image-upload.spotify.com/v4/playlist";

/// `POST /playlist/{id}/changes`: the registered picture as the cover, or (None) Spotify's mosaic back.
pub fn picture(picture: Option<&str>) -> Value {
    let attrs = match picture {
        Some(p) => json!({ "values": { "picture": p } }),
        None => json!({ "values": {}, "noValue": ["LIST_PICTURE"] }),
    };
    changes(vec![json!({ "kind": "UPDATE_LIST_ATTRIBUTES", "updateListAttributes": { "newAttributes": attrs } })])
}

/// Standard base64 (with or without padding); None if it isn't.
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let bytes: Vec<u8> = s.trim_end_matches('=').bytes().map(val).collect::<Option<_>>()?;
    if bytes.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, &b)| n | (b as u32) << (18 - 6 * i));
        out.extend(n.to_be_bytes()[1..chunk.len()].iter());
    }
    Some(out)
}

/// Send the JPEG to the upload host, once; its upload token.
pub async fn upload(session: &Session, jpeg: Vec<u8>) -> anyhow::Result<String> {
    let token = session.login5().auth_token().await?;
    // as the web player does: none of the usual headers, just who and what
    let resp = reqwest::Client::new()
        .post(UPLOAD)
        .header("authorization", format!("{} {}", token.token_type, token.access_token))
        .header("content-type", "image/jpeg")
        .body(jpeg)
        .send()
        .await?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        tracing::debug!("image upload: {status}: {}", text.chars().take(300).collect::<String>());
        anyhow::bail!("Spotify said {status}");
    }
    let v: Value = serde_json::from_str(&text)?;
    v["uploadToken"].as_str().map(String::from).ok_or_else(|| anyhow::anyhow!("no upload token"))
}

/// `addToPlaylist` (Pathfinder mutation): `uris` at the end of `playlist`.
pub fn add_vars(playlist: &str, uris: &[String]) -> Value {
    json!({ "playlistUri": playlist, "playlistItemUris": uris, "newPosition": { "moveType": "BOTTOM_OF_PLAYLIST", "fromUid": null }, "interactionId": null })
}

/// `removeFromPlaylist` (Pathfinder mutation): these rows, by uid.
pub fn remove_vars(playlist: &str, uids: &[String]) -> Value {
    json!({ "playlistUri": playlist, "uids": uids })
}

/// `moveItemsInPlaylist` (Pathfinder mutation): rows to just before `before`, else just after `after`, else last.
pub fn move_vars(playlist: &str, uids: &[String], before: Option<&str>, after: Option<&str>) -> Value {
    let to = match (before, after) {
        (Some(u), _) => json!({ "moveType": "BEFORE_UID", "fromUid": u }),
        (None, Some(u)) => json!({ "moveType": "AFTER_UID", "fromUid": u }),
        (None, None) => json!({ "moveType": "BOTTOM_OF_PLAYLIST", "fromUid": null }),
    };
    json!({ "playlistUri": playlist, "uids": uids, "newPosition": to })
}

/// Rows moved as `move_vars` says (the fake Catalogue, and the Client's own preview).
pub fn apply_move<T: Clone>(rows: &mut Vec<T>, uid_of: impl Fn(&T) -> Option<&str>, uids: &[String], before: Option<&str>, after: Option<&str>) {
    let moving: Vec<T> = rows.iter().filter(|r| uid_of(r).is_some_and(|u| uids.iter().any(|x| x == u))).cloned().collect();
    rows.retain(|r| !uid_of(r).is_some_and(|u| uids.iter().any(|x| x == u)));
    let at = match (before, after) {
        (Some(b), _) => rows.iter().position(|r| uid_of(r) == Some(b)),
        (None, Some(a)) => rows.iter().position(|r| uid_of(r) == Some(a)).map(|i| i + 1),
        (None, None) => None,
    }
    .unwrap_or(rows.len());
    rows.splice(at..at, moving);
}

/// `editablePlaylists` (Pathfinder query), filtered by a name, asking whether each already has `uris`.
pub fn editable_vars(filter: &str, uris: &[String]) -> Value {
    json!({ "offset": 0, "limit": 50, "textFilter": filter, "folderUri": null, "uris": uris })
}

/// From an `editablePlaylists` answer: whether `playlist` already has the tracks asked about (None: not listed).
pub fn curates(v: &Value, playlist: &str) -> Option<bool> {
    let items = v["data"]["me"]["editablePlaylists"]["items"].as_array()?;
    items.iter().find(|i| i["item"]["data"]["uri"] == playlist).map(|i| i["curates"].as_bool().unwrap_or(false))
}

/// A GraphQL answer can be a 200 carrying `errors`: their first message.
pub fn graphql_error(v: &Value) -> Option<String> {
    let e = v["errors"].as_array().filter(|e| !e.is_empty())?;
    Some(e[0]["message"].as_str().unwrap_or("an error").to_string())
}

pub fn rootlist_path(username: &str) -> String {
    let user: String = username.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    format!("/user/{user}/rootlist/changes")
}

/// Send one write to the playlist host. A non-2xx answer is an error carrying Spotify's status.
pub async fn post(session: &Session, path: &str, body: &Value) -> anyhow::Result<Value> {
    let token = session.login5().auth_token().await?;
    let mut req = reqwest::Client::new()
        .post(format!("{HOST}{path}"))
        .header("authorization", format!("{} {}", token.token_type, token.access_token))
        .header("accept", "application/json")
        .header("content-type", "application/json;charset=UTF-8")
        .body(body.to_string());
    if let Ok(t) = crate::spotify(session.spclient().client_token()).await {
        req = req.header("client-token", t);
    }
    let resp = req.send().await?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        tracing::debug!("{path}: {status}: {}", text.chars().take(300).collect::<String>());
        anyhow::bail!("Spotify said {status}");
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies_are_the_web_players() {
        assert_eq!(
            create("Road trip"),
            json!({"ops":[{"kind":"UPDATE_LIST_ATTRIBUTES","updateListAttributes":{"newAttributes":{"values":{"name":"Road trip"}}}}]})
        );
        let add = rootlist_add("spotify:playlist:x", 1_700_000_000_000);
        assert_eq!(add["deltas"][0]["ops"][0]["add"], json!({"addFirst":true,"items":[{"uri":"spotify:playlist:x","attributes":{"timestamp":"1700000000000"}}]}));
        assert!(add.get("baseRevision").is_none());
        assert_eq!(rootlist_remove("spotify:playlist:x")["deltas"][0]["ops"][0], json!({"kind":"REM","rem":{"itemsAsKey":true,"items":[{"uri":"spotify:playlist:x"}]}}));
        assert_eq!(rootlist_path("a b@c"), "/user/a%20b%40c/rootlist/changes");
        assert_eq!(
            details("Road trip", "Songs for the car")["deltas"][0]["ops"][0],
            json!({"kind":"UPDATE_LIST_ATTRIBUTES","updateListAttributes":{"newAttributes":{"values":{"name":"Road trip","description":"Songs for the car"}}}})
        );
        assert_eq!(details("Road trip", "")["deltas"][0]["ops"][0]["updateListAttributes"]["newAttributes"], json!({"values":{"name":"Road trip"},"noValue":["LIST_DESCRIPTION"]}));
        assert_eq!(changes_path("spotify:playlist:37i9dQZF1"), Some("/playlist/37i9dQZF1/changes".into()));
        assert_eq!(changes_path("spotify:album:x"), None);
        assert_eq!(changes_path("spotify:playlist:../x"), None);
        let uris = vec!["spotify:track:a".to_string()];
        assert_eq!(
            add_vars("spotify:playlist:p", &uris),
            json!({"playlistUri":"spotify:playlist:p","playlistItemUris":["spotify:track:a"],"newPosition":{"moveType":"BOTTOM_OF_PLAYLIST","fromUid":null},"interactionId":null})
        );
        assert_eq!(editable_vars("Mix", &uris)["uris"], json!(["spotify:track:a"]));
        assert_eq!(remove_vars("spotify:playlist:p", &["u1".into()]), json!({"playlistUri":"spotify:playlist:p","uids":["u1"]}));
        let uids = vec!["u1".to_string()];
        assert_eq!(move_vars("p", &uids, Some("u5"), None)["newPosition"], json!({"moveType":"BEFORE_UID","fromUid":"u5"}));
        assert_eq!(move_vars("p", &uids, None, Some("u5"))["newPosition"], json!({"moveType":"AFTER_UID","fromUid":"u5"}));
        assert_eq!(move_vars("p", &uids, None, None)["newPosition"], json!({"moveType":"BOTTOM_OF_PLAYLIST","fromUid":null}));
        let mut rows = vec!["a", "b", "c", "d"];
        apply_move(&mut rows, |r| Some(*r), &["a".into()], Some("d"), None);
        assert_eq!(rows, vec!["b", "c", "a", "d"]);
        apply_move(&mut rows, |r| Some(*r), &["b".into()], None, Some("d"));
        assert_eq!(rows, vec!["c", "a", "d", "b"]);
    }

    #[test]
    fn covers_are_set_or_put_back_and_base64_decodes() {
        assert_eq!(picture(Some("YWJj"))["deltas"][0]["ops"][0]["updateListAttributes"]["newAttributes"], json!({"values":{"picture":"YWJj"}}));
        assert_eq!(picture(None)["deltas"][0]["ops"][0]["updateListAttributes"]["newAttributes"], json!({"values":{},"noValue":["LIST_PICTURE"]}));
        assert_eq!(playlist_path("spotify:playlist:abc", "register-image"), Some("/playlist/abc/register-image".into()));
        assert_eq!(playlist_path("spotify:playlist:", "x"), None);
        for s in ["", "a", "ab", "abc", "abcd", "\u{0}\u{ff}\u{80} binary"] {
            let enc = {
                const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
                let b = s.as_bytes();
                let mut o = String::new();
                for c in b.chunks(3) {
                    let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
                    for i in 0..4 {
                        o.push(if i <= c.len() { T[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
                    }
                }
                o
            };
            assert_eq!(base64_decode(&enc).as_deref(), Some(s.as_bytes()), "{enc}");
        }
        assert_eq!(base64_decode("a?b="), None);
    }

    #[test]
    fn editable_playlists_say_what_they_already_have() {
        // the web player's own reading of it: {items:[{item:{__typename, data:{__typename, uri, name}}, pinned, curates}]}
        let v = json!({"data":{"me":{"editablePlaylists":{"totalCount":3,"items":[
            {"item":{"__typename":"LibraryFolderResponseWrapper","data":{"__typename":"Folder","uri":"spotify:user:u:folder:f","name":"F"}},"pinned":false},
            {"item":{"__typename":"PlaylistResponseWrapper","data":{"__typename":"Playlist","uri":"spotify:playlist:has","name":"Has"}},"pinned":false,"curates":true},
            {"item":{"__typename":"PlaylistResponseWrapper","data":{"__typename":"Playlist","uri":"spotify:playlist:not","name":"Not"}},"pinned":false,"curates":false}]}}}});
        assert_eq!(curates(&v, "spotify:playlist:has"), Some(true));
        assert_eq!(curates(&v, "spotify:playlist:not"), Some(false));
        assert_eq!(curates(&v, "spotify:playlist:elsewhere"), None);
        assert_eq!(graphql_error(&json!({"errors":[{"message":"nope"}],"data":null})), Some("nope".into()));
        assert_eq!(graphql_error(&json!({"data":{}})), None);
    }
}
