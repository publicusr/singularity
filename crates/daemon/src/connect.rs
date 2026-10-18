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

/// Cluster updates as they're pushed (devices appearing, playback moving between them).
pub type ClusterStream = librespot_core::dealer::manager::BoxedStreamResult<librespot_protocol::connect::ClusterUpdate>;

pub struct ConnectRemote {
    session: Session,
    /// device ids looked up so far, by name
    ids: std::collections::HashMap<String, String>,
}

impl ConnectRemote {
    /// Open the dealer connection and register the hidden controller. The stream carries every cluster change.
    pub async fn start(session: Session) -> anyhow::Result<(ConnectRemote, Cluster, ClusterStream)> {
        let mut conn = session.dealer().listen_for("hm://pusher/v1/connections/", |m| {
            m.headers.get("Spotify-Connection-Id").cloned().ok_or_else(|| librespot_core::Error::internal("no Spotify-Connection-Id"))
        })?;
        let updates: ClusterStream = session.dealer().listen_for("hm://connect-state/v1/cluster", librespot_core::dealer::protocol::Message::from_raw::<librespot_protocol::connect::ClusterUpdate>)?;
        session.dealer().start().await?;
        let id = tokio::time::timeout(Duration::from_secs(10), conn.next()).await?.ok_or_else(|| anyhow::anyhow!("dealer closed"))??;
        session.set_connection_id(&id);
        let remote = ConnectRemote { session, ids: Default::default() };
        let cluster = remote.devices().await?;
        Ok((remote, cluster, updates))
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

    /// Forget looked-up device ids (after a failure: a device may have re-registered under a new one).
    pub fn forget_ids(&mut self) {
        self.ids.clear();
    }

    /// Move playback to `to`: start the same context there, at the same track and position, paused if it was.
    /// (Connect's `transfer` endpoint answers 500 to this session; `play` with `skip_to` is proven, and only one
    /// device plays at a time, so the old one stops by itself.)
    pub async fn transfer(&self, cluster: &Cluster, to: &str) -> anyhow::Result<()> {
        let ps = cluster.player_state.as_ref().ok_or_else(|| anyhow::anyhow!("nothing is playing"))?;
        let track = ps.track.as_ref().filter(|t| !t.uri.is_empty()).ok_or_else(|| anyhow::anyhow!("nothing is playing"))?;
        let mut pos = ps.position_as_of_timestamp.max(0) as u64;
        if ps.is_playing && !ps.is_paused && ps.timestamp > 0 {
            pos += singularity_proto::now_ms().saturating_sub(ps.timestamp as u64);
        }
        let r = crate::engine::Resume { context_uri: Some(ps.context_uri.clone()), track_uri: track.uri.clone(), position_ms: pos, paused: ps.is_paused || !ps.is_playing };
        self.resume(to, &r, &track.uid).await
    }

    /// Start a context on `to` at a track and position, paused or not (a transfer, or picking up after the Engine
    /// restarted).
    pub async fn resume(&self, to: &str, r: &crate::engine::Resume, track_uid: &str) -> anyhow::Result<()> {
        let context = r.context_uri.clone().filter(|c| !c.is_empty()).unwrap_or_else(|| r.track_uri.clone());
        let mut skip = json!({ "track_uri": r.track_uri });
        if !track_uid.is_empty() {
            skip["track_uid"] = json!(track_uid);
        }
        let (pos, paused) = (r.position_ms, r.paused);
        let cmd = json!({
            "endpoint": "play",
            "context": { "uri": context, "url": format!("context://{context}"), "metadata": {} },
            "play_origin": { "feature_identifier": "singularity", "feature_version": crate::VERSION },
            "options": {
                "skip_to": skip,
                "seek_to": pos,
                "initially_paused": paused,
                "license": "premium",
                "player_options_override": {},
            },
        });
        self.session.spclient().send_player_command(to, cmd).await?;
        Ok(())
    }

    /// Drive another device's player: what our transport buttons mean over Connect.
    pub async fn control(&self, to: &str, cmd: &singularity_proto::Command) -> anyhow::Result<()> {
        use singularity_proto::{Command as C, RepeatMode};
        let sp = self.session.spclient();
        let body = match cmd {
            C::SetVolume { percent } => {
                sp.set_remote_volume(to, (*percent as u32 * 65535 / 100) as u16).await?;
                return Ok(());
            }
            C::Play { uri: Some(u) } => json!({ "endpoint": "play", "context": { "uri": u, "url": format!("context://{u}"), "metadata": {} }, "options": { "license": "premium" } }),
            C::Play { uri: None } => json!({ "endpoint": "resume" }),
            C::Pause => json!({ "endpoint": "pause" }),
            C::Next => json!({ "endpoint": "skip_next" }),
            C::Previous => json!({ "endpoint": "skip_prev" }),
            C::Seek { position_ms } => json!({ "endpoint": "seek_to", "value": position_ms }),
            C::SetShuffle { enabled } => json!({ "endpoint": "set_shuffling_context", "value": enabled }),
            C::SetRepeat { mode } => match mode {
                RepeatMode::Track => json!({ "endpoint": "set_options", "repeating_track": true, "repeating_context": false }),
                RepeatMode::Context => json!({ "endpoint": "set_options", "repeating_track": false, "repeating_context": true }),
                RepeatMode::Off => json!({ "endpoint": "set_options", "repeating_track": false, "repeating_context": false }),
            },
            C::PlayFrom { context_uri, index, track_uri } => {
                return self.play_from(to, context_uri, *index, track_uri.as_deref()).await;
            }
            other => anyhow::bail!("{other:?} can't be sent to another device"),
        };
        sp.send_player_command(to, body).await?;
        Ok(())
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

fn device_kind(t: librespot_protocol::devices::DeviceType) -> &'static str {
    use librespot_protocol::devices::DeviceType as T;
    match t {
        T::COMPUTER => "computer",
        T::SMARTPHONE => "smartphone",
        T::TABLET => "tablet",
        T::SPEAKER => "speaker",
        T::TV => "tv",
        T::AVR | T::STB | T::AUDIO_DONGLE => "speaker",
        T::GAME_CONSOLE => "console",
        T::CAST_VIDEO | T::CAST_AUDIO => "cast",
        T::AUTOMOBILE => "car",
        T::SMARTWATCH => "watch",
        _ => "device",
    }
}

/// The cluster's devices, active first, then by name; hidden ones (like our controller) left out.
pub fn devices(cluster: &Cluster, engine_name: &str) -> Vec<singularity_proto::Device> {
    let mut out: Vec<singularity_proto::Device> = cluster
        .device
        .iter()
        .filter(|(_, d)| !d.capabilities.hidden)
        .map(|(id, d)| singularity_proto::Device {
            id: id.clone(),
            name: d.name.clone(),
            kind: device_kind(d.device_type.enum_value_or_default()).into(),
            active: *id == cluster.active_device_id,
            volume: (!d.capabilities.disable_volume).then(|| (d.volume as u64 * 100 / 65535) as u8),
            is_engine: d.name == engine_name,
        })
        .collect();
    out.sort_by(|a, b| b.active.cmp(&a.active).then(a.name.cmp(&b.name)));
    out
}

/// When another device is playing: its playback, as ours is shown. None when it's our Engine (or nothing).
pub fn remote_playback(cluster: &Cluster, engine_name: &str, now_ms: u64) -> Option<singularity_proto::PlaybackState> {
    use singularity_proto::{Item, PlayStatus, PlaybackState, RepeatMode};
    let active = cluster.device.get(&cluster.active_device_id)?;
    if active.name == engine_name {
        return None;
    }
    let ps = cluster.player_state.as_ref()?;
    let t = ps.track.as_ref()?;
    let md = |k: &str| t.metadata.get(k).cloned().filter(|v| !v.is_empty());
    let status = if !ps.is_playing { PlayStatus::Idle } else if ps.is_paused { PlayStatus::Paused } else if ps.is_buffering { PlayStatus::Buffering } else { PlayStatus::Playing };
    let opts = ps.options.as_ref();
    let repeat = match opts {
        Some(o) if o.repeating_track => RepeatMode::Track,
        Some(o) if o.repeating_context => RepeatMode::Context,
        _ => RepeatMode::Off,
    };
    // the position is anchored at the cluster's timestamp; move it to now if it's playing
    let mut pos = ps.position_as_of_timestamp.max(0) as u64;
    if status == PlayStatus::Playing && ps.timestamp > 0 {
        pos += now_ms.saturating_sub(ps.timestamp as u64);
    }
    Some(PlaybackState {
        status,
        item: Some(Item {
            uri: t.uri.clone(),
            name: md("title").unwrap_or_default(),
            artists: vec![], // filled in from the Catalogue (the cluster carries only the artist's URI)
            album: md("album_title"),
            cover_url: md("image_large_url").or_else(|| md("image_url")),
            duration_ms: ps.duration.max(0) as u64,
            explicit: false,
            artist_uris: md("artist_uri").into_iter().collect(),
            album_uri: md("album_uri"),
        }),
        context_uri: Some(ps.context_uri.clone()).filter(|c| !c.is_empty()),
        context_name: ps.context_metadata.get("context_description").cloned(),
        position_ms: pos,
        timestamp_ms: now_ms,
        speed: if status == PlayStatus::Playing { 1.0 } else { 0.0 },
        volume: (active.volume as u64 * 100 / 65535) as u8,
        shuffle: opts.is_some_and(|o| o.shuffling_context),
        repeat,
        queue: vec![],
    })
}

/// One change to what plays next. Only user-queued tracks (`provider == "queue"`) can be removed; those and the
/// context's ("Next from …") can be moved, each within its own kind. Autoplay is Spotify's to choose.
#[derive(Debug, Clone, PartialEq)]
pub enum QueueEdit {
    PlayNext(String),
    Remove { uri: String, uid: Option<String> },
    Clear,
    /// to place `to` among the tracks of its own kind (queued, or the context's); `into_queue`: a context track
    /// becomes a queued one at place `to` among the queued (#50)
    Move { uri: String, uid: Option<String>, to: usize, into_queue: bool },
}

/// Apply `edit` to the player's `next_tracks` (as sent back in `set_queue`). False if there was nothing to change.
pub fn edit_queue(next: &mut Vec<serde_json::Value>, edit: &QueueEdit) -> bool {
    let queued = |t: &serde_json::Value| t["provider"] == "queue";
    match edit {
        QueueEdit::PlayNext(uri) => {
            next.insert(0, json!({ "uri": uri, "uid": "", "metadata": { "is_queued": "true" }, "provider": "queue" }));
            true
        }
        QueueEdit::Remove { uri, uid } => match find_queued(next, uri, uid.as_deref()) {
            Some(i) => {
                next.remove(i);
                true
            }
            None => false,
        },
        QueueEdit::Move { uri, uid, to, into_queue: true } => {
            let Some(i) = find(next, "context", uri, uid.as_deref()) else { return false };
            next.remove(i);
            let places: Vec<usize> = (0..next.len()).filter(|&k| queued(&next[k])).collect();
            let at = places.get(*to).copied().unwrap_or_else(|| places.last().map_or(0, |&k| k + 1));
            // as Spotify's own queued entries look (its uid is Spotify's to give)
            next.insert(at, json!({ "uri": uri, "uid": "", "metadata": { "is_queued": "true" }, "provider": "queue" }));
            true
        }
        QueueEdit::Move { uri, uid, to, into_queue: false } => {
            let Some(i) = find_queued(next, uri, uid.as_deref()).or_else(|| find(next, "context", uri, uid.as_deref())) else { return false };
            let track = next.remove(i);
            let kind = track["provider"].clone();
            let places: Vec<usize> = (0..next.len()).filter(|&k| next[k]["provider"] == kind).collect();
            // before the `to`-th track of its kind, else after the last one
            let at = places.get(*to).copied().unwrap_or_else(|| places.last().map_or(0, |&k| k + 1));
            next.insert(at, track);
            at != i
        }
        QueueEdit::Clear => {
            let before = next.len();
            next.retain(|t| !queued(t));
            next.len() != before
        }
    }
}

/// A queued track by its uid, else its URI.
fn find_queued(next: &[serde_json::Value], uri: &str, uid: Option<&str>) -> Option<usize> {
    find(next, "queue", uri, uid)
}

/// A track from `provider` by its uid, else its URI.
fn find(next: &[serde_json::Value], provider: &str, uri: &str, uid: Option<&str>) -> Option<usize> {
    let mine = |t: &serde_json::Value| t["provider"] == provider;
    let by_uid = uid.filter(|u| !u.is_empty()).and_then(|u| next.iter().position(|t| mine(t) && t["uid"] == u));
    by_uid.or_else(|| next.iter().position(|t| mine(t) && t["uri"] == uri))
}

impl ConnectRemote {
    /// Change what plays next on device `to`, against the player's current queue revision.
    pub async fn edit_queue(&self, to: &str, edit: &QueueEdit) -> anyhow::Result<()> {
        let cluster = self.devices().await?;
        // Spotify's copy of the queue is the active player's; until Singularity has played since it started, that
        // copy isn't its queue, and an edit would be made against the wrong list
        if cluster.active_device_id != to {
            tracing::info!("queue edit: the active Connect device is {:?}, not Singularity", if cluster.active_device_id.is_empty() { "none" } else { "another" });
            anyhow::bail!("Spotify doesn't see Singularity as the player yet; press play once, then try again");
        }
        let ps = cluster.player_state.unwrap_or_default();
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

    fn cluster() -> Cluster {
        use librespot_protocol::connect::{Capabilities, DeviceInfo};
        let dev = |name: &str, hidden: bool, vol: u32| {
            let mut d = DeviceInfo::new();
            d.name = name.into();
            d.volume = vol;
            let mut c = Capabilities::new();
            c.hidden = hidden;
            d.capabilities = Some(c).into();
            d
        };
        let mut c = Cluster::new();
        c.device.insert("phone".into(), dev("Pixel", false, 65535));
        c.device.insert("solo".into(), dev("Singularity", false, 32767));
        c.device.insert("me".into(), dev("Singularity remote", true, 0));
        c.active_device_id = "phone".into();
        let mut ps = librespot_protocol::player::PlayerState::new();
        ps.is_playing = true;
        ps.position_as_of_timestamp = 10_000;
        ps.timestamp = 1_000_000;
        ps.duration = 200_000;
        let mut t = librespot_protocol::player::ProvidedTrack::new();
        t.uri = "spotify:track:x".into();
        t.metadata.insert("title".into(), "On the phone".into());
        t.metadata.insert("artist_uri".into(), "spotify:artist:a".into());
        ps.track = Some(t).into();
        c.player_state = Some(ps).into();
        c
    }

    #[test]
    fn devices_and_remote_playback_from_the_cluster() {
        let c = cluster();
        let d = devices(&c, "Singularity");
        assert_eq!(d.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["Pixel", "Singularity"], "active first, hidden left out");
        assert!(d[0].active && !d[1].active && d[1].is_engine);
        assert_eq!(d[1].volume, Some(49));
        let p = remote_playback(&c, "Singularity", 1_005_000).unwrap();
        assert_eq!(p.item.as_ref().unwrap().name, "On the phone");
        assert_eq!(p.position_ms, 15_000, "moved to now");
        assert_eq!(p.status, singularity_proto::PlayStatus::Playing);
        let mut ours = c.clone();
        ours.active_device_id = "solo".into();
        assert!(remote_playback(&ours, "Singularity", 0).is_none(), "our own Engine reports for itself");
    }

    #[test]
    fn queue_edits_touch_only_queued_tracks() {
        let mut n = q();
        assert!(edit_queue(&mut n, &QueueEdit::Remove { uri: "spotify:track:a".into(), uid: Some("u2".into()) }));
        assert_eq!(n.iter().map(|t| t["uid"].as_str().unwrap()).collect::<Vec<_>>(), ["u1", "u3", "u4"], "the right copy goes");
        assert!(!edit_queue(&mut n, &QueueEdit::Remove { uri: "spotify:track:c".into(), uid: None }), "context tracks can't be removed");
        assert!(edit_queue(&mut n, &QueueEdit::PlayNext("spotify:track:z".into())));
        assert_eq!((n[0]["uri"].as_str(), n[0]["provider"].as_str()), (Some("spotify:track:z"), Some("queue")));
        let mut m = q();
        m.insert(2, json!({"uri": "spotify:track:b", "uid": "u5", "provider": "queue"}));
        let mv = |uri: &str, uid: &str, to| QueueEdit::Move { uri: uri.into(), uid: Some(uid.into()), to, into_queue: false };
        assert!(edit_queue(&mut m, &mv("spotify:track:b", "u5", 0)));
        let uids = |m: &[serde_json::Value]| m.iter().map(|t| t["uid"].as_str().unwrap_or("").to_string()).collect::<Vec<_>>();
        assert_eq!(uids(&m), ["u5", "u1", "u2", "u3", "u4"]);
        assert!(edit_queue(&mut m, &mv("spotify:track:b", "u5", 9)), "past the end: last of the queued");
        assert_eq!(m[2]["uid"], "u5");
        assert_eq!(m[3]["provider"], "context", "context order untouched");
        // a context track moves among the context's: [u1 u2 u5 u3 u6 u4] → u6 first of the context
        m.insert(4, json!({"uri": "spotify:track:e", "uid": "u6", "provider": "context"}));
        assert!(edit_queue(&mut m, &mv("spotify:track:e", "u6", 0)));
        assert_eq!(uids(&m), ["u1", "u2", "u5", "u6", "u3", "u4"], "it stays after the queued tracks");
        assert!(!edit_queue(&mut m, &mv("spotify:track:d", "u4", 0)), "autoplay doesn't move");
        // a context track up into the queue (#50): it leaves the context and is queued at that place
        let up = QueueEdit::Move { uri: "spotify:track:c".into(), uid: Some("u3".into()), to: 1, into_queue: true };
        assert!(edit_queue(&mut m, &up));
        assert_eq!(m[1]["uri"], "spotify:track:c");
        assert_eq!((m[1]["provider"].as_str(), m.iter().filter(|t| t["uri"] == "spotify:track:c").count()), (Some("queue"), 1));
        assert_eq!(m.iter().filter(|t| t["provider"] == "context").count(), 1, "one context track left");
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
