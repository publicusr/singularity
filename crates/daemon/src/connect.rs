//! ConnectRemote (spec D12): Spotify Connect commands Soloist's own API lacks, sent through the Catalogue's librespot
//! session. First user: starting a context at track N (verified 2026-10-17: Soloist's `play` ignores every offset
//! parameter, a Connect `play` with `skip_to.track_index` works for albums, playlists and Liked Songs).
//!
//! The session registers as a *hidden* Connect device (never shown in device pickers) so it may send commands; it
//! unregisters when dropped.

use futures_util::StreamExt;
use librespot_core::session::Session;
use librespot_protocol::connect::{Capabilities, Cluster, Device, DeviceInfo, MemberType, PutStateReason, PutStateRequest};
use protobuf::Message;
use serde_json::json;
use std::time::Duration;

pub struct ConnectRemote {
    session: Session,
    /// device ids looked up so far, by name
    ids: std::collections::HashMap<String, String>,
}

impl ConnectRemote {
    /// Open the dealer connection and register the hidden controller.
    pub async fn start(session: Session) -> anyhow::Result<ConnectRemote> {
        let mut conn = session.dealer().listen_for("hm://pusher/v1/connections/", |m| {
            m.headers.get("Spotify-Connection-Id").cloned().ok_or_else(|| librespot_core::Error::internal("no Spotify-Connection-Id"))
        })?;
        session.dealer().start().await?;
        let id = tokio::time::timeout(Duration::from_secs(10), conn.next()).await?.ok_or_else(|| anyhow::anyhow!("dealer closed"))??;
        session.set_connection_id(&id);
        let remote = ConnectRemote { session, ids: Default::default() };
        remote.devices().await?;
        Ok(remote)
    }

    /// Announce the hidden controller; the answer is the account's Connect cluster.
    async fn devices(&self) -> anyhow::Result<Cluster> {
        let mut caps = Capabilities::new();
        caps.hidden = true;
        caps.is_observable = true;
        caps.gaia_eq_connect_id = true;
        let mut info = DeviceInfo::new();
        info.name = "Singularity remote".into();
        info.device_id = self.session.device_id().to_string();
        info.capabilities = Some(caps).into();
        let mut dev = Device::new();
        dev.device_info = Some(info).into();
        let mut req = PutStateRequest::new();
        req.member_type = MemberType::CONNECT_STATE.into();
        req.put_state_reason = PutStateReason::NEW_DEVICE.into();
        req.device = Some(dev).into();
        let bytes = self.session.spclient().put_connect_state_request(&req).await?;
        Ok(Cluster::parse_from_bytes(&bytes)?)
    }

    /// The Connect id of the device called `name` (Soloist registers as [`crate::soloist::DEVICE_NAME`]).
    pub async fn device_id(&mut self, name: &str) -> anyhow::Result<String> {
        if let Some(id) = self.ids.get(name) {
            return Ok(id.clone());
        }
        let cluster = self.devices().await?;
        let id = cluster.device.iter().find(|(_, d)| d.name == name).map(|(id, _)| id.clone()).ok_or_else(|| anyhow::anyhow!("Connect device {name:?} not found"))?;
        self.ids.insert(name.into(), id.clone());
        Ok(id)
    }

    /// Start `context` at its `index`-th track on device `to`.
    pub async fn play_from(&self, to: &str, context: &str, index: u32, track_uri: Option<&str>) -> anyhow::Result<()> {
        let mut skip = json!({ "track_index": index });
        if let Some(t) = track_uri {
            skip["track_uri"] = json!(t);
        }
        let cmd = json!({
            "endpoint": "play",
            "context": { "uri": context, "url": format!("context://{context}"), "metadata": {} },
            "play_origin": { "feature_identifier": "singularity", "feature_version": crate::VERSION },
            "options": { "skip_to": skip, "license": "premium", "player_options_override": {} },
        });
        self.session.spclient().send_player_command(to, cmd).await?;
        Ok(())
    }
}

/// One change to what plays next. Only user-queued tracks (`provider == "queue"`) can be removed.
#[derive(Debug, Clone, PartialEq)]
pub enum QueueEdit {
    PlayNext(String),
    Remove { uri: String, uid: Option<String> },
    Clear,
}

/// Apply `edit` to the player's `next_tracks` (as sent back in `set_queue`). False if there was nothing to change.
pub fn edit_queue(next: &mut Vec<serde_json::Value>, edit: &QueueEdit) -> bool {
    let queued = |t: &serde_json::Value| t["provider"] == "queue";
    match edit {
        QueueEdit::PlayNext(uri) => {
            next.insert(0, json!({ "uri": uri, "uid": "", "metadata": { "is_queued": "true" }, "provider": "queue" }));
            true
        }
        QueueEdit::Remove { uri, uid } => {
            let by_uid = uid.as_deref().filter(|u| !u.is_empty()).and_then(|u| next.iter().position(|t| queued(t) && t["uid"] == u));
            match by_uid.or_else(|| next.iter().position(|t| queued(t) && t["uri"] == uri.as_str())) {
                Some(i) => {
                    next.remove(i);
                    true
                }
                None => false,
            }
        }
        QueueEdit::Clear => {
            let before = next.len();
            next.retain(|t| !queued(t));
            next.len() != before
        }
    }
}

impl ConnectRemote {
    /// Change what plays next on device `to`, against the player's current queue revision.
    pub async fn edit_queue(&self, to: &str, edit: &QueueEdit) -> anyhow::Result<()> {
        let ps = self.devices().await?.player_state.unwrap_or_default();
        let as_json = |t: &librespot_protocol::player::ProvidedTrack| json!({ "uri": t.uri, "uid": t.uid, "metadata": t.metadata, "provider": t.provider });
        let mut next: Vec<serde_json::Value> = ps.next_tracks.iter().map(as_json).collect();
        anyhow::ensure!(edit_queue(&mut next, edit), "that track isn't in the queue any more");
        let prev: Vec<serde_json::Value> = ps.prev_tracks.iter().map(as_json).collect();
        let cmd = json!({ "endpoint": "set_queue", "next_tracks": next, "prev_tracks": prev, "queue_revision": ps.queue_revision });
        self.session.spclient().send_player_command(to, cmd).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q() -> Vec<serde_json::Value> {
        vec![
            json!({"uri": "spotify:track:a", "uid": "u1", "provider": "queue"}),
            json!({"uri": "spotify:track:a", "uid": "u2", "provider": "queue"}),
            json!({"uri": "spotify:track:c", "uid": "u3", "provider": "context"}),
            json!({"uri": "spotify:track:d", "uid": "u4", "provider": "autoplay"}),
        ]
    }

    #[test]
    fn queue_edits_touch_only_queued_tracks() {
        let mut n = q();
        assert!(edit_queue(&mut n, &QueueEdit::Remove { uri: "spotify:track:a".into(), uid: Some("u2".into()) }));
        assert_eq!(n.iter().map(|t| t["uid"].as_str().unwrap()).collect::<Vec<_>>(), ["u1", "u3", "u4"], "the right copy goes");
        assert!(!edit_queue(&mut n, &QueueEdit::Remove { uri: "spotify:track:c".into(), uid: None }), "context tracks can't be removed");
        assert!(edit_queue(&mut n, &QueueEdit::PlayNext("spotify:track:z".into())));
        assert_eq!((n[0]["uri"].as_str(), n[0]["provider"].as_str()), (Some("spotify:track:z"), Some("queue")));
        assert!(edit_queue(&mut n, &QueueEdit::Clear));
        assert!(n.iter().all(|t| t["provider"] != "queue") && n.len() == 2, "context and autoplay stay");
    }
}

impl Drop for ConnectRemote {
    fn drop(&mut self) {
        let session = self.session.clone();
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move {
                let _ = session.spclient().delete_connect_state_request().await;
            });
        }
    }
}
