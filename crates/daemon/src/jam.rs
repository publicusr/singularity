//! Jam (D64): Spotify's shared listening session, through the Catalogue's session and the `social-connect` API.
//! Facts and the live check are in `docs/research/2026-10-jam.md`: the Catalogue session may start a Jam hosted on
//! Singularity's own device (verified 2026-10-30), the dealer pushes `social-connect/v2/session_update` on every
//! change, and the cluster carries no Jam id, so the Jam is always read from `social-connect`.
//!
//! The answers are read as loose JSON (snake or camel case): Spotify adds fields and reasons the proto doesn't know.

use std::collections::HashMap;

use librespot_core::session::Session;
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue};
use serde_json::Value;
use singularity_proto::{Jam, JamMember};

/// The dealer topic that carries every change to the account's Jam.
pub const UPDATE_TOPIC: &str = "social-connect/v2/session_update";
const CURRENT: &str = "/social-connect/v2/sessions/current";
const INVITE_BASE: &str = "https://open.spotify.com/socialsession";

/// The account's current Jam; `None` when there is none (404) or only a dormant record of an ended one.
/// `devices` maps Connect device ids to names; `engine_id` is Singularity's own device.
pub async fn current(session: &Session, devices: &HashMap<String, String>, engine_id: Option<&str>) -> anyhow::Result<Option<Jam>> {
    match get(session, reqwest::Method::GET, CURRENT).await {
        Ok(v) => Ok(parse(&v, devices, engine_id)),
        Err(e) if status(&e) == Some(404) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Start a Jam hosted on `device_id` (or return the one already running).
pub async fn start(session: &Session, device_id: &str, devices: &HashMap<String, String>) -> anyhow::Result<Option<Jam>> {
    let v = get(session, reqwest::Method::GET, &start_path(device_id)).await?;
    Ok(parse(&v, devices, Some(device_id)))
}

/// End the Jam for everyone (owner only). The v2 path answered in the live check; v3 is the fallback.
pub async fn end(session: &Session, session_id: &str) -> anyhow::Result<()> {
    let mut last = None;
    for path in end_paths(session_id, session.device_id()) {
        match get(session, reqwest::Method::DELETE, &path).await {
            Ok(_) => return Ok(()),
            Err(e) if matches!(status(&e), Some(404 | 405)) => last = Some(e),
            Err(e) => return Err(e.into()),
        }
    }
    Err(last.map(Into::into).unwrap_or_else(|| anyhow::anyhow!("no end path answered")))
}

async fn get(session: &Session, method: reqwest::Method, path: &str) -> Result<Value, librespot_core::Error> {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    let bytes = session.spclient().request(&method, path, Some(headers), None).await?;
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes).map_err(|e| librespot_core::Error::internal(format!("Jam answer is not JSON: {e}")))
}

fn status(e: &librespot_core::Error) -> Option<u16> {
    use librespot_core::http_client::HttpClientError;
    e.error.downcast_ref::<HttpClientError>().map(|HttpClientError::StatusCode(code)| code.as_u16())
}

fn start_path(device_id: &str) -> String {
    format!("/social-connect/v2/sessions/current_or_new?local_device_id={device_id}&activate=true&type=REMOTE")
}

fn end_paths(session_id: &str, local_device_id: &str) -> [String; 2] {
    [
        format!("/social-connect/v2/sessions/{session_id}?local_device_id={local_device_id}"),
        format!("/social-connect/v3/sessions/{session_id}?local_device_id={local_device_id}"),
    ]
}

/// A `social-connect` session (bare, or under `session` as in `session_update`) as a [`Jam`]; `None` unless active.
pub fn parse(v: &Value, devices: &HashMap<String, String>, engine_id: Option<&str>) -> Option<Jam> {
    let s = v.get("session").filter(|s| s.is_object()).unwrap_or(v);
    if !flag(s, "active", "active") {
        return None;
    }
    let session_id = text(s, "session_id", "sessionId")?.to_string();
    let owner_id = text(s, "session_owner_id", "sessionOwnerId");
    let host_id = text(s, "host_active_device_id", "hostActiveDeviceId");
    let members = field(s, "session_members", "sessionMembers")
        .and_then(Value::as_array)
        .map(|ms| {
            ms.iter()
                .map(|m| {
                    let id = text(m, "id", "id").unwrap_or_default().to_string();
                    let username = text(m, "username", "username");
                    // some clients send the user id, some the username (research §2)
                    let owner = owner_id.is_some_and(|o| o == id || Some(o) == username);
                    let name = text(m, "display_name", "displayName").or(username).unwrap_or("someone").to_string();
                    JamMember { id, name, owner, listening: flag(m, "is_listening", "isListening") }
                })
                .collect()
        })
        .unwrap_or_default();
    let owner = flag(s, "is_session_owner", "isSessionOwner");
    Some(Jam {
        session_id,
        owner,
        host: host_id.and_then(|h| devices.get(h).cloned()),
        host_is_engine: host_id.is_some() && host_id == engine_id,
        invite: if owner { invite(s) } else { None },
        queue_only: flag(s, "queue_only_mode", "queueOnlyMode"),
        members,
    })
}

/// The public invite link; the token comes from `join_session_token`, else the last part of the URI or URL.
fn invite(s: &Value) -> Option<String> {
    let token = text(s, "join_session_token", "joinSessionToken")
        .or_else(|| text(s, "join_session_uri", "joinSessionUri").and_then(|u| u.rsplit(':').next()))
        .or_else(|| text(s, "join_session_url", "joinSessionUrl").and_then(|u| u.rsplit('/').next()).and_then(|l| l.split('?').next()))
        .filter(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric()))?;
    Some(format!("{INVITE_BASE}/{token}"))
}

fn field<'a>(v: &'a Value, snake: &str, camel: &str) -> Option<&'a Value> {
    v.get(snake).or_else(|| v.get(camel))
}

fn text<'a>(v: &'a Value, snake: &str, camel: &str) -> Option<&'a str> {
    field(v, snake, camel).and_then(Value::as_str)
}

fn flag(v: &Value, snake: &str, camel: &str) -> bool {
    field(v, snake, camel).and_then(Value::as_bool).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn devices() -> HashMap<String, String> {
        HashMap::from([("dev-solo".to_string(), "Singularity".to_string()), ("dev-phone".to_string(), "Pixel".to_string())])
    }

    /// The shape the live start answered (2026-10-30), with made-up values.
    fn started() -> Value {
        json!({
            "session_id": "0123456789abcdef0123456789abcdef",
            "active": true,
            "is_session_owner": true,
            "session_owner_id": "owner-user",
            "host_active_device_id": "dev-solo",
            "queue_only_mode": false,
            "join_session_token": "AbC123xyz",
            "join_session_uri": "spotify:socialsession:AbC123xyz",
            "user_capabilities": {"invite": true, "manage_participant_settings": true, "remove_participants": true},
            "session_members": [
                {"id": "member-1", "username": "owner-user", "display_name": "Owner", "is_listening": true, "is_controlling": true},
                {"id": "member-2", "username": "guest-user", "display_name": "Guest", "is_listening": true, "is_controlling": false},
            ],
        })
    }

    #[test]
    fn a_started_jam_names_its_host_link_and_members() {
        let jam = parse(&started(), &devices(), Some("dev-solo")).expect("active");
        assert_eq!(jam.session_id, "0123456789abcdef0123456789abcdef");
        assert!(jam.owner && jam.host_is_engine && !jam.queue_only);
        assert_eq!(jam.host.as_deref(), Some("Singularity"));
        assert_eq!(jam.invite.as_deref(), Some("https://open.spotify.com/socialsession/AbC123xyz"));
        assert_eq!(jam.members.iter().map(|m| (m.name.as_str(), m.owner)).collect::<Vec<_>>(), [("Owner", true), ("Guest", false)], "the owner is found by username too");
    }

    #[test]
    fn a_dormant_record_is_no_jam() {
        let mut v = started();
        v.as_object_mut().unwrap().remove("active");
        assert_eq!(parse(&v, &devices(), None), None, "`active` missing: an ended Jam's record");
        v["active"] = json!(false);
        assert_eq!(parse(&v, &devices(), None), None);
    }

    #[test]
    fn a_session_update_and_camel_case_are_read_too() {
        let update = json!({"reason": "USER_JOINED", "session": {
            "sessionId": "abc", "active": true, "isSessionOwner": false, "hostActiveDeviceId": "dev-phone",
            "joinSessionToken": "Tok1", "queueOnlyMode": true,
            "sessionMembers": [{"id": "m", "displayName": "Friend", "isListening": false}],
        }});
        let jam = parse(&update, &devices(), Some("dev-solo")).expect("active");
        assert_eq!(jam.host.as_deref(), Some("Pixel"));
        assert!(!jam.host_is_engine && jam.queue_only && !jam.owner);
        assert_eq!(jam.invite, None, "a guest gets no invite to pass on");
        assert_eq!(jam.members[0].name, "Friend");
    }

    #[test]
    fn the_invite_token_falls_back_and_refuses_odd_characters() {
        let s = json!({"join_session_url": "hm://social-connect/v2/sessions/join/Tok9?x=1"});
        assert_eq!(invite(&s).as_deref(), Some("https://open.spotify.com/socialsession/Tok9"));
        assert_eq!(invite(&json!({"join_session_token": "a/../b"})), None);
        assert_eq!(invite(&json!({})), None);
    }

    #[test]
    fn paths_are_the_ones_the_live_check_used() {
        assert_eq!(start_path("dev"), "/social-connect/v2/sessions/current_or_new?local_device_id=dev&activate=true&type=REMOTE");
        assert_eq!(end_paths("s", "c")[0], "/social-connect/v2/sessions/s?local_device_id=c");
    }
}
