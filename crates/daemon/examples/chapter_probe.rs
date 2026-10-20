//! Dev tool (#62): can librespot fetch an audiobook chapter? Silent: metadata, audio files, a decryption key and the
//! storage location; nothing is played, so no audiobook time is counted.
//!
//!     cargo run -p singularityd --example chapter_probe -- spotify:episode:<chapter id>

use librespot_core::{FileId, SpotifyId, SpotifyUri, cache::Cache, config::SessionConfig, session::Session};
use protobuf::Message;
use singularityd::catalogue::Dirs;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let uri = std::env::args().nth(1).expect("spotify:episode:<id>");
    let dirs = Dirs::from_env();
    let cache = Cache::new(Some(&dirs.credentials), None, None, None)?;
    let creds = singularityd::catalogue::saved_credentials(&dirs).ok_or_else(|| anyhow::anyhow!("no saved login; log in from the Client first"))?;
    let session = Session::new(SessionConfig::default(), Some(cache));
    session.connect(creds, true).await?;

    let parsed = SpotifyUri::from_uri(&uri)?;
    let bytes = session.spclient().get_episode_metadata(&parsed).await?;
    let ep = librespot_protocol::metadata::Episode::parse_from_bytes(&bytes)?;
    println!("is_audiobook_chapter: {}", ep.is_audiobook_chapter());
    println!("audio files: {}", ep.audio.len());
    for f in &ep.audio {
        println!("  {:?}", f.format());
    }
    println!("restrictions: {}", ep.restriction.len());
    let id = SpotifyId::from_base62(uri.rsplit(':').next().unwrap_or(&uri))?;
    let Some(file) = ep.audio.iter().find(|f| format!("{:?}", f.format()).starts_with("OGG_VORBIS")).or(ep.audio.first()) else {
        println!("no audio file: librespot can't play it");
        return Ok(());
    };
    let file = FileId::from(file);
    match session.audio_key().request(id, file).await {
        Ok(_) => println!("audio key: ok"),
        Err(e) => println!("audio key: refused ({e})"),
    }
    match session.spclient().get_audio_storage(&file).await {
        Ok(b) => println!("storage: ok ({} bytes)", b.len()),
        Err(e) => println!("storage: refused ({e})"),
    }
    Ok(())
}
