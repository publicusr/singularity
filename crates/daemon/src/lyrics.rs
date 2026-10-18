//! Lyrics (#14): Spotify's own (spclient `color-lyrics`, through the Catalogue session), else LRCLIB
//! (<https://lrclib.net>, free and open; only artist, title, album and duration are sent).

use serde_json::Value;
use singularity_proto::{LyricLine, Lyrics};

/// Spotify's answer → lines (times come as strings; `UNSYNCED` lyrics have zeros).
pub fn parse_spotify(uri: &str, v: &Value) -> Option<Lyrics> {
    let l = &v["lyrics"];
    let synced = l["syncType"].as_str() == Some("LINE_SYNCED");
    let lines: Vec<LyricLine> = l["lines"]
        .as_array()?
        .iter()
        .map(|x| LyricLine { start_ms: x["startTimeMs"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0), text: x["words"].as_str().unwrap_or_default().trim().to_string() })
        .collect();
    let source = l["providerDisplayName"].as_str().filter(|s| !s.is_empty()).unwrap_or("Spotify").to_string();
    (!lines.is_empty()).then(|| Lyrics { uri: uri.into(), synced, lines, source, ..Default::default() })
}

/// LRC (`[mm:ss.xx] words`, several stamps per line allowed) → lines in time order.
pub fn parse_lrc(text: &str) -> Vec<LyricLine> {
    let mut out = vec![];
    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut stamps = vec![];
        while let Some(r) = rest.strip_prefix('[') {
            let Some(end) = r.find(']') else { break };
            let tag = &r[..end];
            rest = r[end + 1..].trim_start();
            let mut p = tag.split(':');
            let (Some(m), Some(s)) = (p.next().and_then(|m| m.parse::<u64>().ok()), p.next().and_then(|s| s.parse::<f64>().ok())) else { continue };
            stamps.push(m * 60_000 + (s * 1000.0).round() as u64);
        }
        for t in stamps {
            out.push(LyricLine { start_ms: t, text: rest.to_string() });
        }
    }
    out.sort_by_key(|l| l.start_ms);
    out
}

/// LRCLIB's answer → lyrics, synced if it has them (an instrumental: no lines, and it says so).
pub fn parse_lrclib(uri: &str, v: &Value) -> Option<Lyrics> {
    if v["instrumental"].as_bool() == Some(true) {
        return Some(Lyrics { uri: uri.into(), source: "LRCLIB".into(), missing: "an instrumental, says LRCLIB".into(), ..Default::default() });
    }
    if let Some(s) = v["syncedLyrics"].as_str().filter(|s| !s.trim().is_empty()) {
        let lines = parse_lrc(s);
        if !lines.is_empty() {
            return Some(Lyrics { uri: uri.into(), synced: true, lines, source: "LRCLIB".into(), ..Default::default() });
        }
    }
    let plain = v["plainLyrics"].as_str()?;
    let lines: Vec<LyricLine> = plain.lines().map(|t| LyricLine { start_ms: 0, text: t.trim().to_string() }).collect();
    (!lines.is_empty()).then(|| Lyrics { uri: uri.into(), synced: false, lines, source: "LRCLIB".into(), ..Default::default() })
}

/// Ask LRCLIB (one request, no retry loop): `Ok(None)` when it has nothing, `Err` when it couldn't be asked.
pub async fn lrclib(uri: &str, artist: &str, title: &str, album: Option<&str>, duration_ms: u64) -> Result<Option<Lyrics>, String> {
    let http = reqwest::Client::builder()
        .user_agent(concat!("Singularity/", env!("CARGO_PKG_VERSION"), " (https://github.com/publicusr/singularity)"))
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let mut q = vec![("artist_name", artist.to_string()), ("track_name", title.to_string()), ("duration", (duration_ms / 1000).to_string())];
    if let Some(a) = album {
        q.push(("album_name", a.to_string()));
    }
    let resp = http.get("https://lrclib.net/api/get").query(&q).send().await.map_err(|e| e.to_string())?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let v: Value = serde_json::from_slice(&resp.bytes().await.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(parse_lrclib(uri, &v))
}

/// How one source answered.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outcome {
    Found,
    /// it answered: nothing for this track
    Nothing,
    /// it couldn't be asked (offline, an error, no session)
    Failed,
}

/// What the Pane says when neither source has lines, and whether that's worth remembering (only if both answered).
pub fn missing(spotify: Outcome, lrclib: Outcome) -> (String, bool) {
    use Outcome::*;
    match (spotify, lrclib) {
        (Failed, Failed) => ("couldn't reach Spotify or LRCLIB for lyrics".into(), false),
        (Failed, _) => ("no lyrics on LRCLIB; Spotify couldn't be asked".into(), false),
        (_, Failed) => ("no lyrics on Spotify; LRCLIB couldn't be asked".into(), false),
        _ => ("no lyrics on Spotify or LRCLIB".into(), true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spotify_lines_and_sync() {
        let v = serde_json::json!({"lyrics": {"syncType": "LINE_SYNCED", "providerDisplayName": "Musixmatch", "lines": [
            {"startTimeMs": "24590", "words": "First light ", "endTimeMs": "0"}, {"startTimeMs": "29000", "words": "♪", "endTimeMs": "0"}]}});
        let l = parse_spotify("u", &v).unwrap();
        assert!(l.synced);
        assert_eq!((l.lines[0].start_ms, l.lines[0].text.as_str(), l.source.as_str()), (24_590, "First light", "Musixmatch"));
        assert!(parse_spotify("u", &serde_json::json!({"lyrics": {"lines": []}})).is_none());
    }

    #[test]
    fn lrc_stamps_including_repeats() {
        let l = parse_lrc("[ar: x]\n[00:30.64] Like a phoenix\n[01:02.5][02:10.00] Chorus line\n[00:05.00]\n");
        assert_eq!(l.iter().map(|x| x.start_ms).collect::<Vec<_>>(), [5_000, 30_640, 62_500, 130_000]);
        assert_eq!(l[2].text, "Chorus line");
        assert_eq!(l[0].text, "", "an empty stamped line is a pause");
    }

    #[test]
    fn lrclib_prefers_synced_and_skips_instrumentals() {
        let v = serde_json::json!({"syncedLyrics": "[00:01.00] a\n[00:02.00] b", "plainLyrics": "a\nb"});
        assert!(parse_lrclib("u", &v).unwrap().synced);
        let v = serde_json::json!({"syncedLyrics": null, "plainLyrics": "a\nb"});
        assert!(!parse_lrclib("u", &v).unwrap().synced);
        let i = parse_lrclib("u", &serde_json::json!({"instrumental": true})).unwrap();
        assert!(i.lines.is_empty() && i.missing.contains("instrumental"));
    }

    #[test]
    fn missing_says_which_source_failed_and_only_definite_answers_are_kept() {
        use Outcome::*;
        assert_eq!(missing(Nothing, Nothing), ("no lyrics on Spotify or LRCLIB".into(), true));
        let (why, keep) = missing(Failed, Nothing);
        assert!(why.contains("Spotify couldn't be asked") && !keep);
        assert!(!missing(Nothing, Failed).1 && !missing(Failed, Failed).1);
    }
}
