//! Dev tool: list Connect devices, and optionally start a context at track N on one of them.
//!
//!     cargo run -p singularityd --example connect_probe                       # list devices
//!     cargo run -p singularityd --example connect_probe -- <device-name> <context-uri> <index>
//!
//! Registers a *hidden* controller for the duration (it never shows in device pickers) and removes it afterwards.

use futures_util::StreamExt;
use librespot_core::{cache::Cache, config::SessionConfig, session::Session};
use librespot_protocol::connect::{Capabilities, Cluster, Device, DeviceInfo, MemberType, PutStateReason, PutStateRequest};
use protobuf::Message;
use singularityd::catalogue::Dirs;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let dirs = Dirs::from_env();
    let cache = Cache::new(Some(&dirs.credentials), None, None, None)?;
    let creds = cache.credentials().ok_or_else(|| anyhow::anyhow!("no saved login"))?;
    let session = Session::new(SessionConfig::default(), Some(cache));
    session.connect(creds, true).await?;

    let mut conn = session.dealer().listen_for("hm://pusher/v1/connections/", |m| {
        m.headers.get("Spotify-Connection-Id").cloned().ok_or_else(|| librespot_core::Error::internal("no connection id"))
    })?;
    session.dealer().start().await?;
    let id = tokio::time::timeout(std::time::Duration::from_secs(10), conn.next()).await?.ok_or_else(|| anyhow::anyhow!("dealer closed"))??;
    session.set_connection_id(&id);

    let mut req = PutStateRequest::new();
    req.member_type = MemberType::CONNECT_STATE.into();
    req.put_state_reason = PutStateReason::NEW_DEVICE.into();
    let mut info = DeviceInfo::new();
    info.name = "Singularity remote".into();
    info.device_id = session.device_id().to_string();
    info.can_play = false;
    let mut caps = Capabilities::new();
    caps.hidden = true;
    caps.is_observable = true;
    caps.is_controllable = false;
    caps.can_be_player = false;
    caps.gaia_eq_connect_id = true;
    info.capabilities = Some(caps).into();
    let mut dev = Device::new();
    dev.device_info = Some(info).into();
    req.device = Some(dev).into();
    let bytes = session.spclient().put_connect_state_request(&req).await?;
    let cluster = Cluster::parse_from_bytes(&bytes)?;
    println!("active: {}", cluster.active_device_id);
    for (id, d) in &cluster.device {
        println!("{id}  {:?}  {:?}  hidden={}", d.name, d.device_type, d.capabilities.hidden);
    }

    if let (Some(name), Some(ctx), Some(idx)) = (args.get(1), args.get(2), args.get(3)) {
        let to = cluster.device.iter().find(|(_, d)| &d.name == name).map(|(id, _)| id.clone()).ok_or_else(|| anyhow::anyhow!("no device {name}"))?;
        let idx: u32 = idx.parse()?;
        let ctx = &if ctx == "liked" { format!("spotify:user:{}:collection", session.username()) } else { ctx.clone() };
        let cmd = serde_json::json!({
            "endpoint": "play",
            "context": { "uri": ctx, "url": format!("context://{ctx}"), "metadata": {} },
            "play_origin": { "feature_identifier": "singularity", "feature_version": env!("CARGO_PKG_VERSION") },
            "options": { "skip_to": { "track_index": idx }, "license": "premium", "player_options_override": {} },
        });
        let r = session.spclient().send_player_command(&to, cmd).await;
        println!("play {ctx} at {idx} on {to}: {:?}", r.map(|b| String::from_utf8_lossy(&b).to_string()));
    }
    let _ = session.spclient().delete_connect_state_request().await;
    session.dealer().close().await;
    Ok(())
}
