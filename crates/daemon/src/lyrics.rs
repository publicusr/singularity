//! Lyrics (#14): Spotify's own (spclient `color-lyrics`, through the Catalogue session), else LRCLIB
//! (<https://lrclib.net>, free and open; only artist, title, album and duration are sent).

use futures_util::future::BoxFuture;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use singularity_proto::{LyricLine, LyricWord, Lyrics};

/// Spotify's answer → lines (times come as strings; `UNSYNCED` lyrics have zeros).
pub fn parse_spotify(uri: &str, v: &Value) -> Option<Lyrics> {
    let l = &v["lyrics"];
    let synced = l["syncType"].as_str() == Some("LINE_SYNCED");
    let lines: Vec<LyricLine> = l["lines"]
        .as_array()?
        .iter()
        .map(|x| LyricLine { start_ms: x["startTimeMs"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0), text: x["words"].as_str().unwrap_or_default().trim().to_string(), ..Default::default() })
        .collect();
    let source = l["providerDisplayName"].as_str().filter(|s| !s.is_empty()).unwrap_or("Spotify").to_string();
    (!lines.is_empty()).then(|| Lyrics { uri: uri.into(), synced, lines, source, ..Default::default() })
}

/// An LRC time stamp's inside (`mm:ss.xx`) in ms. Third-party text: `inf`, negative and enormous seconds must not
/// panic a counting arithmetic (debug) or wrap to an arbitrary time (release). A non-finite or negative stamp is no
/// stamp; an overflowing one saturates at `u64::MAX`, which keeps its line last in the sort.
fn stamp(tag: &str) -> Option<u64> {
    let mut p = tag.split(':');
    let (m, s) = (p.next()?.parse::<u64>().ok()?, p.next()?.parse::<f64>().ok()?);
    if !s.is_finite() || s < 0.0 {
        return None;
    }
    Some(m.saturating_mul(60_000).saturating_add((s * 1000.0).round() as u64))
}

/// A line's words from enhanced LRC (A2: `<mm:ss.xx>` before each word, a last one where the line's singing ends)
/// and the text without the tags. No word tags: no words. A `<` that doesn't open a stamp is text.
pub fn parse_enhanced(text: &str) -> (Vec<LyricWord>, String) {
    let mut words: Vec<LyricWord> = vec![];
    let mut lead = String::new();
    let mut rest = text;
    let mut tagged = false;
    while !rest.is_empty() {
        let at = rest.find('<').unwrap_or(rest.len());
        let (seg, after) = rest.split_at(at);
        match words.last_mut() {
            Some(w) => w.text.push_str(seg),
            None => lead.push_str(seg),
        }
        let Some(r) = after.strip_prefix('<') else { break };
        match r.find('>').and_then(|end| stamp(&r[..end]).map(|t| (t, end))) {
            Some((t, end)) => {
                tagged = true;
                // a tag closes the word before it (the last one marks where the line's singing ends)
                if let Some(w) = words.last_mut()
                    && t >= w.start_ms
                {
                    w.end_ms = Some(t);
                }
                words.push(LyricWord { start_ms: t, end_ms: None, text: String::new() });
                rest = &r[end + 1..];
            }
            None => {
                match words.last_mut() {
                    Some(w) => w.text.push('<'),
                    None => lead.push('<'),
                }
                rest = r;
            }
        }
    }
    if !tagged {
        return (vec![], text.trim().to_string());
    }
    // text before the first tag is sung with it; a tag with nothing after it only ends the word before
    if let Some(first) = words.first_mut() {
        first.text.insert_str(0, &lead);
    }
    let mut kept: Vec<LyricWord> = Vec::with_capacity(words.len());
    for w in words {
        match kept.last_mut() {
            // spacing between two tags belongs to the word before
            Some(prev) if w.text.trim().is_empty() => prev.text.push_str(&w.text),
            _ if w.text.trim().is_empty() => {}
            _ => kept.push(w),
        }
    }
    let mut words = kept;
    // a word ends where the next one starts unless it said otherwise; only a real pause keeps its own end
    for i in 0..words.len().saturating_sub(1) {
        if words[i].end_ms == Some(words[i + 1].start_ms) {
            words[i].end_ms = None;
        }
    }
    let plain: String = words.iter().map(|w| w.text.as_str()).collect();
    (words, tidy(&plain))
}

/// LRC (`[mm:ss.xx] words`, several stamps per line allowed; enhanced LRC's word stamps too) → lines in time order.
pub fn parse_lrc(text: &str) -> Vec<LyricLine> {
    let mut out = vec![];
    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut stamps = vec![];
        while let Some(r) = rest.strip_prefix('[') {
            let Some(end) = r.find(']') else { break };
            let tag = &r[..end];
            rest = r[end + 1..].trim_start();
            stamps.extend(stamp(tag));
        }
        let (words, text) = parse_enhanced(rest);
        let first = stamps.first().copied().unwrap_or(0);
        for t in stamps {
            // a repeated line (`[01:02][02:10] chorus`) sings its words as far after each stamp as after the first
            let shift = |ms: u64| if t >= first { ms.saturating_add(t - first) } else { ms.saturating_sub(first - t) };
            let words = words.iter().map(|w| LyricWord { start_ms: shift(w.start_ms), end_ms: w.end_ms.map(shift), text: w.text.clone() }).collect();
            out.push(LyricLine { start_ms: t, text: text.clone(), words });
        }
    }
    out.sort_by_key(|l| l.start_ms);
    out
}

/// The biggest Lyricsfile read: a long song with every word timed is a few tens of kB.
const MAX_LYRICSFILE: usize = 512 * 1024;
/// Nesting and node limits for its YAML: a Lyricsfile is four levels deep.
const MAX_DEPTH: usize = 12;
const MAX_NODES: usize = 200_000;

/// A YAML tree, only as much of one as a Lyricsfile needs: no anchors or aliases (a few lines of aliases can expand
/// to gigabytes), no tags.
#[derive(Debug)]
enum Yaml {
    Scalar(String),
    Seq(Vec<Yaml>),
    Map(Vec<(String, Yaml)>),
}

impl Yaml {
    fn get(&self, key: &str) -> Option<&Yaml> {
        match self {
            Yaml::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    fn str(&self) -> Option<&str> {
        match self {
            Yaml::Scalar(s) => Some(s),
            _ => None,
        }
    }
    fn ms(&self) -> Option<u64> {
        self.str()?.trim().parse::<u64>().ok()
    }
    fn seq(&self) -> &[Yaml] {
        match self {
            Yaml::Seq(v) => v,
            _ => &[],
        }
    }
}

/// The first YAML document of `text`, read event by event without recursion (the parser's own loader recurses once
/// per level), refusing aliases and anything too deep or too big.
fn yaml(text: &str) -> Option<Yaml> {
    use yaml_rust2::parser::{Event, Parser};
    if text.len() > MAX_LYRICSFILE {
        return None;
    }
    // the open collections; a map's key waits beside it until its value is complete
    let mut stack: Vec<(Yaml, Option<String>)> = vec![];
    let mut parser = Parser::new_from_str(text);
    for _ in 0..MAX_NODES {
        let (ev, _) = parser.next_token().ok()?;
        let node = match ev {
            Event::StreamStart | Event::DocumentStart | Event::Nothing => continue,
            Event::StreamEnd | Event::DocumentEnd | Event::Alias(_) => return None,
            Event::Scalar(s, ..) => Yaml::Scalar(s),
            Event::SequenceStart(..) | Event::MappingStart(..) => {
                if stack.len() >= MAX_DEPTH {
                    return None;
                }
                stack.push((if matches!(ev, Event::SequenceStart(..)) { Yaml::Seq(vec![]) } else { Yaml::Map(vec![]) }, None));
                continue;
            }
            Event::SequenceEnd | Event::MappingEnd => stack.pop()?.0,
        };
        // a finished node goes into its parent, or is the document
        let Some((parent, key)) = stack.last_mut() else { return Some(node) };
        match parent {
            Yaml::Seq(v) => v.push(node),
            Yaml::Map(m) => match (key.take(), node) {
                (Some(k), node) => m.push((k, node)),
                (None, Yaml::Scalar(k)) => *key = Some(k),
                // a collection as a key is no Lyricsfile's
                (None, _) => return None,
            },
            Yaml::Scalar(_) => return None,
        }
    }
    None
}

/// Spaces collapsed, ends trimmed.
fn tidy(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// LRCLIB's Lyricsfile (YAML: `lines[]` with `text`, `start_ms`, optional `end_ms` and `words[]` of the same; times
/// in integer ms) → lines, if any of them has its words timed. Otherwise None: the LRC carries the same lines.
pub fn parse_lyricsfile(text: &str) -> Option<Vec<LyricLine>> {
    let doc = yaml(text)?;
    let mut lines: Vec<LyricLine> = doc
        .get("lines")?
        .seq()
        .iter()
        .filter_map(|l| {
            let start_ms = l.get("start_ms")?.ms()?;
            let mut words: Vec<LyricWord> = l
                .get("words")
                .map(Yaml::seq)
                .unwrap_or_default()
                .iter()
                .filter_map(|w| Some(LyricWord { start_ms: w.get("start_ms")?.ms()?, end_ms: w.get("end_ms").and_then(Yaml::ms), text: w.get("text")?.str()?.to_string() }))
                .filter(|w| !w.text.trim().is_empty())
                .collect();
            // LRCLIB's word times are starts only, its lines' ends are given: the last word is sung until the line ends
            if let (Some(last), Some(end)) = (words.last_mut(), l.get("end_ms").and_then(Yaml::ms))
                && last.end_ms.is_none()
                && end > last.start_ms
            {
                last.end_ms = Some(end);
            }
            let text = match l.get("text").and_then(Yaml::str) {
                Some(t) => tidy(t),
                None => tidy(&words.iter().map(|w| w.text.as_str()).collect::<String>()),
            };
            Some(LyricLine { start_ms, text, words })
        })
        .collect();
    if lines.iter().all(|l| l.words.is_empty()) {
        return None;
    }
    lines.sort_by_key(|l| l.start_ms);
    Some(lines)
}

/// LRCLIB's answer → lyrics, synced if it has them (an instrumental: no lines, and it says so).
pub fn parse_lrclib(uri: &str, v: &Value) -> Option<Lyrics> {
    if v["instrumental"].as_bool() == Some(true) {
        return Some(Lyrics { uri: uri.into(), source: "LRCLIB".into(), missing: "an instrumental, says LRCLIB".into(), ..Default::default() });
    }
    // word timing comes only in the Lyricsfile (`hasWordSync`): the LRC's lines, each word timed
    if v["hasWordSync"].as_bool() == Some(true)
        && let Some(lines) = v["lyricsfile"].as_str().and_then(parse_lyricsfile)
    {
        return Some(Lyrics { uri: uri.into(), synced: true, lines, source: "LRCLIB".into(), ..Default::default() });
    }
    if let Some(s) = v["syncedLyrics"].as_str().filter(|s| !s.trim().is_empty()) {
        let lines = parse_lrc(s);
        if !lines.is_empty() {
            return Some(Lyrics { uri: uri.into(), synced: true, lines, source: "LRCLIB".into(), ..Default::default() });
        }
    }
    let plain = v["plainLyrics"].as_str()?;
    let lines: Vec<LyricLine> = plain.lines().map(|t| LyricLine { start_ms: 0, text: t.trim().to_string(), ..Default::default() }).collect();
    (!lines.is_empty()).then(|| Lyrics { uri: uri.into(), synced: false, lines, source: "LRCLIB".into(), ..Default::default() })
}

/// LRCLIB's turn: held for a whole lookup (all its asks), so lookups go one at a time; when its last request went.
static LRCLIB_TURN: tokio::sync::Mutex<Option<tokio::time::Instant>> = tokio::sync::Mutex::const_new(None);
/// The least time between two requests to LRCLIB.
const LRCLIB_SPACING: Duration = Duration::from_millis(300);

/// LRCLIB asked us to wait (a 429 or 503 with `Retry-After`): no request goes to it before this (Unix ms), for any
/// track. Its docs: clients "must honor `Retry-After`", with jitter, or risk a temporary ban.
static LRCLIB_QUIET_UNTIL: AtomicU64 = AtomicU64::new(0);

/// How long LRCLIB asks us to wait after an answer with `status`: `Retry-After` in seconds (an HTTP date or none: a
/// minute after a 429, a second after a 503), plus `jitter` (0..1) of up to 30 % of it. None: no wait asked.
fn wait_asked(status: u16, retry_after: Option<&str>, jitter: f64) -> Option<Duration> {
    let default = match status {
        429 => 60,
        503 => 1,
        _ => return None,
    };
    let secs = retry_after.and_then(|r| r.trim().parse::<u64>().ok()).unwrap_or(default).min(3_600);
    Some(Duration::from_millis(secs * 1000 + (secs as f64 * 1000.0 * 0.3 * jitter.clamp(0.0, 1.0)) as u64))
}

/// Whether LRCLIB may be asked now; if not, the seconds still to wait.
fn quiet_left(now_ms: u64) -> Option<u64> {
    let until = LRCLIB_QUIET_UNTIL.load(Ordering::Relaxed);
    (now_ms < until).then(|| (until - now_ms).div_ceil(1000))
}

/// How much the length of a search's word-timed version may differ from the track's (LRCLIB's own `/api/get`
/// matches within 2 s too).
const SAME_LENGTH_MS: f64 = 2_000.0;

/// What an LRCLIB lookup is for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Want {
    /// lyrics, no Karaoke Widget on screen: `/api/get` alone
    Lines,
    /// lyrics for a Karaoke Widget: `/api/get`, then one search when its best match times only lines (the owner, Q183)
    Words,
    /// a word-timed version of a song whose `/api/get` answer timed only lines (asked without a Karaoke Widget): the
    /// search alone
    SearchOnly,
}

/// Ask LRCLIB: with the album, then without it if that finds nothing (`asks`); for a Karaoke Widget (`want`), one
/// search for a word-timed version of the same song when the best match times only lines. `Ok(None)` when it has
/// nothing (a search finding nothing word-timed), `Err` when it couldn't be asked, including while it has asked us to
/// wait.
pub async fn lrclib(uri: &str, artist: &str, title: &str, album: Option<&str>, duration_ms: u64, want: Want) -> Result<Option<Lyrics>, String> {
    let http = reqwest::Client::builder()
        .user_agent(concat!("Singularity/", env!("CARGO_PKG_VERSION"), " (https://github.com/publicusr/singularity)"))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let ask = move |path: &'static str, q: Vec<(&'static str, String)>| -> BoxFuture<'static, Result<Answer, String>> {
        let http = http.clone();
        Box::pin(async move {
            let resp = http.get(format!("https://lrclib.net{path}")).query(&q).send().await.map_err(|e| e.to_string())?;
            let retry_after = resp.headers().get(reqwest::header::RETRY_AFTER).and_then(|v| v.to_str().ok()).map(str::to_string);
            let status = resp.status().as_u16();
            let body = if resp.status().is_success() { resp.bytes().await.map_err(|e| e.to_string())?.to_vec() } else { vec![] };
            Ok(Answer { status, retry_after, body })
        })
    };
    lookup(&ask, uri, artist, title, album, duration_ms, want).await
}

/// One answer from LRCLIB: its status, its `Retry-After`, its body (a success's only).
struct Answer {
    status: u16,
    retry_after: Option<String>,
    body: Vec<u8>,
}

/// How a request goes to LRCLIB: an endpoint (`/api/get`, `/api/search`) and its query.
type Ask = dyn Fn(&'static str, Vec<(&'static str, String)>) -> BoxFuture<'static, Result<Answer, String>> + Sync;

/// `lrclib`, its requests sent through `ask`.
async fn lookup(ask: &Ask, uri: &str, artist: &str, title: &str, album: Option<&str>, duration_ms: u64, want: Want) -> Result<Option<Lyrics>, String> {
    // one lookup at a time, its requests spaced (LRCLIB's docs: sequential, 200–500 ms apart)
    let mut turn = LRCLIB_TURN.lock().await;
    let mut best = None;
    if want != Want::SearchOnly {
        let q = vec![("artist_name", artist.to_string()), ("track_name", title.to_string()), ("duration", (duration_ms / 1000).to_string())];
        let mut record = None;
        for q in asks(q, album) {
            if let Some(v) = send(ask, &mut turn, "/api/get", q).await? {
                record = Some(v);
                break;
            }
        }
        let Some(record) = record else { return Ok(None) };
        best = parse_lrclib(uri, &record);
        let lines_only = best.as_ref().is_some_and(|l| l.synced && l.lines.iter().all(|x| x.words.is_empty()));
        if !(want == Want::Words && lines_only) {
            return Ok(best);
        }
    }
    let q = vec![("track_name", title.to_string()), ("artist_name", artist.to_string())];
    match send(ask, &mut turn, "/api/search", q).await {
        Ok(Some(Value::Array(records))) => {
            if let Some(w) = word_timed(uri, &records, artist, title, duration_ms) {
                tracing::info!("lyrics for {uri}: LRCLIB's best match times lines only, a search found a word-timed version");
                return Ok(Some(w));
            }
        }
        Ok(_) => {}
        // (the best match stands)
        Err(e) => tracing::info!("lyrics for {uri}: LRCLIB search failed: {e}"),
    }
    Ok(best)
}

/// One request in a lookup's turn, `LRCLIB_SPACING` after the one before at least, none while LRCLIB has asked us to
/// wait: the answer's JSON, `None` on a 404.
async fn send(ask: &Ask, last: &mut Option<tokio::time::Instant>, path: &'static str, q: Vec<(&'static str, String)>) -> Result<Option<Value>, String> {
    // (checked after the wait for the turn: the lookup before may have been told to wait)
    if let Some(s) = quiet_left(singularity_proto::now_ms()) {
        return Err(format!("LRCLIB asked us to wait ({s} s left)"));
    }
    if let Some(at) = *last {
        tokio::time::sleep(LRCLIB_SPACING.saturating_sub(at.elapsed())).await;
    }
    *last = Some(tokio::time::Instant::now());
    let a = ask(path, q).await?;
    // (a jitter from the clock's nanoseconds: no randomness crate for this)
    let jitter = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos()) as f64 / 1e9;
    if let Some(wait) = wait_asked(a.status, a.retry_after.as_deref(), jitter) {
        LRCLIB_QUIET_UNTIL.fetch_max(singularity_proto::now_ms() + wait.as_millis() as u64, Ordering::Relaxed);
        return Err(format!("HTTP {}: LRCLIB asks us to wait {} s", a.status, wait.as_secs()));
    }
    match a.status {
        404 => Ok(None),
        200..=299 => serde_json::from_slice(&a.body).map(Some).map_err(|e| e.to_string()),
        s => Err(format!("HTTP {s}")),
    }
}

/// Of a search's `records`, the word-timed version of `artist`'s `title`, `duration_ms` long: the closest in length
/// within `SAME_LENGTH_MS` whose words parse.
fn word_timed(uri: &str, records: &[Value], artist: &str, title: &str, duration_ms: u64) -> Option<Lyrics> {
    word_timed_records(records, artist, title, duration_ms).into_iter().find_map(|r| parse_lrclib(uri, r).filter(|l| l.lines.iter().any(|x| !x.words.is_empty())))
}

/// The word-timed records of a search that are `artist`'s `title` (`same_song`), within `SAME_LENGTH_MS` of
/// `duration_ms` (no instrumentals), closest first.
fn word_timed_records<'a>(records: &'a [Value], artist: &str, title: &str, duration_ms: u64) -> Vec<&'a Value> {
    let mut near: Vec<(f64, &Value)> = records
        .iter()
        .filter(|r| r["hasWordSync"].as_bool() == Some(true) && r["instrumental"].as_bool() != Some(true))
        .filter(|r| same_song(r["artistName"].as_str().unwrap_or(""), r["trackName"].as_str().unwrap_or(""), artist, title))
        .filter_map(|r| Some(((r["duration"].as_f64()? * 1000.0 - duration_ms as f64).abs(), r)))
        .filter(|(d, _)| *d <= SAME_LENGTH_MS)
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    near.into_iter().map(|(_, r)| r).collect()
}

/// Whether a search's record (`artist`, `title`) is the song asked for (`want_artist`, `want_title`): the same artist,
/// whatever the case (or the first one it credits: "A, B", "A & B"), and one title within the other once credits in
/// brackets ("(feat. …)", "(Prod. …)", "[with …]") are dropped, whatever the case.
fn same_song(artist: &str, title: &str, want_artist: &str, want_title: &str) -> bool {
    let first = artist.split([',', '&']).next().unwrap_or("").trim();
    let artist_ok = [artist.trim(), first].iter().any(|a| a.to_lowercase() == want_artist.trim().to_lowercase());
    let (a, b) = (bare_title(title), bare_title(want_title));
    artist_ok && !a.is_empty() && !b.is_empty() && (a.contains(&b) || b.contains(&a))
}

/// A title in lower case without bracketed credits ("(feat. X)", "(ft. X)", "(Prod. X)", "[with X]").
fn bare_title(t: &str) -> String {
    let mut out = String::new();
    let mut rest = t;
    while let Some(i) = rest.find(['(', '[']) {
        let close = if rest[i..].starts_with('(') { ')' } else { ']' };
        let inner = &rest[i + 1..];
        let end = inner.find(close).map_or(inner.len(), |e| e + 1);
        let word = inner.trim_start().to_lowercase();
        let credit = ["feat", "ft.", "ft ", "prod", "with "].iter().any(|c| word.starts_with(c));
        out.push_str(&rest[..i]);
        if !credit {
            out.push_str(&rest[i..i + 1 + end.min(inner.len())]);
        }
        rest = &inner[end.min(inner.len())..];
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// What to ask LRCLIB, in turn until one finds the song: with the album, then without it. LRCLIB finds nothing when it
/// doesn't know the album (a compilation, a playlist's single, a deluxe edition), even when it has the song.
fn asks(q: Vec<(&'static str, String)>, album: Option<&str>) -> Vec<Vec<(&'static str, String)>> {
    match album.filter(|a| !a.trim().is_empty()) {
        Some(a) => {
            let mut with = q.clone();
            with.push(("album_name", a.to_string()));
            vec![with, q]
        }
        None => vec![q],
    }
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
    fn enhanced_lrc_times_each_word_and_keeps_the_text_clean() {
        let l = parse_lrc("[00:12.00]<00:12.00>Turn <00:12.40>the <00:12.70>lights <00:13.20>down<00:14.00>\n[00:15.00] plain line\n");
        assert_eq!(l[0].text, "Turn the lights down");
        let w = &l[0].words;
        assert_eq!(w.iter().map(|w| (w.start_ms, w.text.as_str())).collect::<Vec<_>>(), [(12_000, "Turn "), (12_400, "the "), (12_700, "lights "), (13_200, "down")]);
        assert_eq!(w.iter().map(|w| w.end_ms).collect::<Vec<_>>(), [None, None, None, Some(14_000)], "the closing tag ends the last word");
        assert!(l[1].words.is_empty() && l[1].text == "plain line");
        // a pause between two tags keeps the word's own end and its space
        let (w, t) = parse_enhanced("<00:01.00>Hold<00:01.50> <00:03.00>on");
        assert_eq!(t, "Hold on");
        assert_eq!((w[0].end_ms, w[0].text.as_str(), w[1].start_ms), (Some(1_500), "Hold ", 3_000));
        // syllables of one word: no space between the pieces
        let (w, t) = parse_enhanced("<00:01.00>beau<00:01.20>ti<00:01.40>ful");
        assert_eq!((t.as_str(), w.len()), ("beautiful", 3));
        // a `<` that isn't a stamp is text, and hostile stamps are no stamps
        assert_eq!(parse_enhanced("I <3 you").1, "I <3 you");
        assert!(parse_enhanced("I <3 you").0.is_empty());
        let (w, _) = parse_enhanced("<inf:-1>a <99999999999999999999:00>b <00:01.00>c");
        assert_eq!(w.len(), 1, "{w:?}");
        // a repeated stamp shifts the words with it
        let l = parse_lrc("[00:10.00][01:10.00]<00:10.00>la <00:10.50>la\n");
        assert_eq!((l[1].start_ms, l[1].words[1].start_ms), (70_000, 70_500));
    }

    const LYRICSFILE: &str = "version: '1.0'
metadata:
  title: Invented
  artist: Nobody
  offset_ms: 0
lines:
- text: 'Turn the lights down'
  start_ms: 12000
  end_ms: 14000
  words:
  - text: 'Turn '
    start_ms: 12000
    end_ms: 12400
  - {text: 'the ', start_ms: 12400}
  - text: \"lights \"
    start_ms: 12700
  - text: down
    start_ms: 13200
    end_ms: 14000
- text: Sing it back
  start_ms: 15000
plain: |
  Turn the lights down
  Sing it back
";

    #[test]
    fn a_word_synced_lyricsfile_gives_lines_with_words() {
        let l = parse_lyricsfile(LYRICSFILE).unwrap();
        assert_eq!(l.len(), 2);
        assert_eq!((l[0].text.as_str(), l[0].words.len(), l[0].words[2].text.as_str(), l[0].words[3].end_ms), ("Turn the lights down", 4, "lights ", Some(14_000)));
        assert!(l[1].words.is_empty() && l[1].text == "Sing it back");
        // no words anywhere: the LRC says the same, so nothing here
        assert!(parse_lyricsfile("lines:\n- text: a\n  start_ms: 1\n").is_none());
        // through LRCLIB's answer, only when it says the words are timed
        let v = serde_json::json!({"hasWordSync": true, "lyricsfile": LYRICSFILE, "syncedLyrics": "[00:12.00] Turn the lights down"});
        let got = parse_lrclib("u", &v).unwrap();
        assert!(got.synced && got.lines[0].words.len() == 4);
        let v = serde_json::json!({"hasWordSync": false, "lyricsfile": LYRICSFILE, "syncedLyrics": "[00:12.00] Turn the lights down"});
        assert!(parse_lrclib("u", &v).unwrap().lines[0].words.is_empty());
    }

    /// A word-synced record as LRCLIB answers it today (2026-11, recorded from `/api/get`): `hasWordSync`, line-only
    /// `syncedLyrics` (its stamps a little off the Lyricsfile's), and a Lyricsfile whose words carry starts only and
    /// whose lines carry their ends. The structure, quoting and times are the recorded answer's, cut to two lines; the
    /// words and names are invented.
    const RECORDED: &str = include_str!("../tests/fixtures/lrclib-word-synced.json");

    #[test]
    fn todays_lrclib_answer_gives_every_word_its_time() {
        let v: Value = serde_json::from_str(RECORDED).unwrap();
        let l = parse_lrclib("u", &v).unwrap();
        assert!(l.synced && l.lines.len() == 2, "{l:?}");
        let w = &l.lines[0].words;
        assert_eq!(l.lines[0].text, "Paper boats drift past, the harbour wall");
        assert_eq!(w.iter().map(|w| (w.start_ms, w.text.as_str())).collect::<Vec<_>>(), [(8_120, "Paper "), (8_789, "boats "), (9_212, "drift "), (9_757, "past, "), (9_997, "the "), (10_663, "harbour "), (11_126, "wall")]);
        // only the starts are given; the last word is sung until its line ends
        assert_eq!(w.iter().map(|w| w.end_ms).collect::<Vec<_>>(), [None, None, None, None, None, None, Some(11_620)]);
        assert_eq!(l.lines[1].words.last().unwrap().end_ms, Some(14_936));
        // the line's stamp is the Lyricsfile's, not the LRC's
        assert_eq!(l.lines[0].start_ms, 8_120);
        // a line end before the last word's start says nothing about it
        let odd = parse_lyricsfile("lines:\n- text: a b\n  start_ms: 10\n  end_ms: 15\n  words:\n  - {text: 'a ', start_ms: 10}\n  - {text: b, start_ms: 20}\n").unwrap();
        assert_eq!(odd[0].words[1].end_ms, None);
    }

    #[test]
    fn an_album_lrclib_doesnt_know_is_left_out_on_a_second_ask() {
        let q = || vec![("artist_name", "A".to_string()), ("track_name", "T".to_string())];
        let a = asks(q(), Some("Hits 2017"));
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].last(), Some(&("album_name", "Hits 2017".to_string())));
        assert_eq!(a[1], q());
        assert_eq!(asks(q(), None), [q()]);
        assert_eq!(asks(q(), Some(" ")), [q()]);
    }

    #[test]
    fn lrclib_is_left_alone_as_long_as_it_asks() {
        let _quiet = QUIET.lock().unwrap_or_else(|e| e.into_inner());
        // Retry-After in seconds, plus up to 30 % jitter; an HTTP date or none: a minute (429), a second (503)
        assert_eq!(wait_asked(429, Some("12"), 0.0), Some(Duration::from_secs(12)));
        assert_eq!(wait_asked(429, Some(" 10 "), 1.0), Some(Duration::from_secs(13)));
        assert_eq!(wait_asked(429, Some("Wed, 21 Oct 2026 07:28:00 GMT"), 0.0), Some(Duration::from_secs(60)));
        assert_eq!(wait_asked(503, None, 0.0), Some(Duration::from_secs(1)));
        assert_eq!(wait_asked(429, Some("999999"), 0.0), Some(Duration::from_secs(3_600)), "an hour at most");
        assert_eq!(wait_asked(200, Some("5"), 0.0), None);
        assert_eq!(wait_asked(404, None, 0.0), None);
        // while it lasts, no track is asked about: the lookup fails at once without a request
        let now = singularity_proto::now_ms();
        LRCLIB_QUIET_UNTIL.store(now + 20_000, Ordering::Relaxed);
        assert_eq!(quiet_left(now), Some(20));
        assert_eq!(quiet_left(now + 19_500), Some(1));
        assert_eq!(quiet_left(now + 20_000), None);
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let got = rt.block_on(lrclib("u", "A", "T", Some("LP"), 200_000, Want::Words));
        assert!(got.as_ref().is_err_and(|e| e.contains("wait")), "{got:?}");
        LRCLIB_QUIET_UNTIL.store(0, Ordering::Relaxed);
    }

    /// Serializes the tests that touch LRCLIB's wait (`LRCLIB_QUIET_UNTIL`, process-wide).
    static QUIET: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// A search answer as LRCLIB gives it today (2026-11, recorded from `/api/search`): records of `/api/get`'s fields,
    /// lengths in seconds or none, word-timed and line-only versions of one song. Cut to five records of two lines; the
    /// names, words and lengths are invented: 3 line-only (no length), 4 word-timed 169.7 s, 5 line-only 243 s, 6
    /// word-timed 241.4 s, 7 word-timed 242.56 s.
    const SEARCH: &str = include_str!("../tests/fixtures/lrclib-search.json");

    /// A stand-in LRCLIB: `/api/get` answers `get` (None: a 404), `/api/search` answers `search` (Err: that status,
    /// with a `Retry-After` of 5 s); every request is written down in `asked`.
    fn stand_in(get: Option<Value>, search: Result<Value, u16>, asked: std::sync::Arc<std::sync::Mutex<Vec<String>>>) -> Box<Ask> {
        Box::new(move |path: &'static str, q: Vec<(&'static str, String)>| -> BoxFuture<'static, Result<Answer, String>> {
            asked.lock().unwrap().push(format!("{path}?{}", q.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&")));
            let ok = |v: &Value| Answer { status: 200, retry_after: None, body: serde_json::to_vec(v).unwrap() };
            let a = match (path, &get, &search) {
                ("/api/get", Some(v), _) => ok(v),
                ("/api/get", None, _) => Answer { status: 404, retry_after: None, body: vec![] },
                (_, _, Ok(v)) => ok(v),
                (_, _, Err(s)) => Answer { status: *s, retry_after: Some("5".into()), body: vec![] },
            };
            Box::pin(async move { Ok(a) })
        })
    }

    #[test]
    fn a_line_only_best_match_leads_to_one_search_for_a_word_timed_version() {
        let _quiet = QUIET.lock().unwrap_or_else(|e| e.into_inner());
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mut line_only: Value = serde_json::from_str(RECORDED).unwrap();
        line_only["hasWordSync"] = false.into();
        let search: Value = serde_json::from_str(SEARCH).unwrap();
        let run = |get: Option<Value>, search: Result<Value, u16>, want: Want| {
            let asked = std::sync::Arc::default();
            let ask = stand_in(get, search, std::sync::Arc::clone(&asked));
            let got = rt.block_on(lookup(&ask, "u", "Nikt", "Wymyślona piosenka", Some("Wymyślony album"), 243_000, want));
            (got, asked.lock().unwrap().clone())
        };
        let words = |l: &Lyrics| l.lines.iter().any(|x| !x.words.is_empty());
        // a Karaoke Widget on screen: the get, then one search for the same track and artist, which has a word-timed
        // version within 2 s of the track's length
        let (got, asked) = run(Some(line_only.clone()), Ok(search.clone()), Want::Words);
        assert_eq!(asked, ["/api/get?artist_name=Nikt&track_name=Wymyślona piosenka&duration=243&album_name=Wymyślony album", "/api/search?track_name=Wymyślona piosenka&artist_name=Nikt"]);
        let l = got.unwrap().unwrap();
        assert!(l.synced && l.source == "LRCLIB" && words(&l), "{l:?}");
        assert_eq!(l.lines[0].words.iter().map(|w| w.text.as_str()).collect::<String>(), "Lampy nad rzeką gasną powoli");
        // none on screen: no search, the best match as it is
        let (got, asked) = run(Some(line_only.clone()), Ok(search.clone()), Want::Lines);
        let l = got.unwrap().unwrap();
        assert!(asked.len() == 1 && !words(&l) && l.lines[0].text == "Paper boats drift past, the harbour wall", "{asked:?}");
        // a word-timed best match: no search
        let (got, asked) = run(Some(serde_json::from_str(RECORDED).unwrap()), Ok(search.clone()), Want::Words);
        assert!(asked.len() == 1 && words(&got.unwrap().unwrap()), "{asked:?}");
        // nothing word-timed of the same length, the search failing: the best match stands, after the one search
        let (got, asked) = run(Some(line_only.clone()), Ok(Value::Array(search.as_array().unwrap()[..3].to_vec())), Want::Words);
        assert!(asked.len() == 2 && !words(&got.unwrap().unwrap()), "{asked:?}");
        let (got, asked) = run(Some(line_only.clone()), Err(500), Want::Words);
        assert!(asked.len() == 2 && !words(&got.unwrap().unwrap()), "{asked:?}");
        // not found at all: the two gets (with the album and without), no search
        let (got, asked) = run(None, Ok(search.clone()), Want::Words);
        assert!(asked.len() == 2 && asked.iter().all(|a| a.starts_with("/api/get")) && got == Ok(None), "{asked:?}");
        // the search told to wait: the best match stands, and LRCLIB is left alone meanwhile, any track
        let (got, asked) = run(Some(line_only.clone()), Err(429), Want::Words);
        assert!(asked.len() == 2 && !words(&got.unwrap().unwrap()), "{asked:?}");
        let (got, asked) = run(Some(line_only.clone()), Ok(search.clone()), Want::Words);
        assert!(asked.is_empty() && got.is_err_and(|e| e.contains("wait")), "{asked:?}");
        LRCLIB_QUIET_UNTIL.store(0, Ordering::Relaxed);
        // a song whose get timed only lines, asked about again for a Karaoke Widget: the search alone, no get again
        let (got, asked) = run(Some(line_only.clone()), Ok(search), Want::SearchOnly);
        assert_eq!(asked, ["/api/search?track_name=Wymyślona piosenka&artist_name=Nikt"]);
        assert!(words(&got.unwrap().unwrap()));
        let (got, asked) = run(Some(line_only), Ok(Value::Array(vec![])), Want::SearchOnly);
        assert!(asked.len() == 1 && got == Ok(None), "{asked:?}");
    }

    #[test]
    fn the_search_takes_only_the_same_artists_same_song() {
        // the same artist whatever the case, or the first one a record credits
        assert!(same_song("nikt", "Wymyślona piosenka", "Nikt", "Wymyślona piosenka"));
        assert!(same_song("Nikt, Ktoś", "Wymyślona piosenka", "Nikt", "Wymyślona piosenka"));
        assert!(same_song("Nikt & Ktoś", "Wymyślona piosenka", "Nikt", "Wymyślona piosenka"));
        assert!(!same_song("Ktoś, Nikt", "Wymyślona piosenka", "Nikt", "Wymyślona piosenka"), "someone else's song");
        assert!(!same_song("Nikt Inny", "Wymyślona piosenka", "Nikt", "Wymyślona piosenka"));
        // one title within the other, credits in brackets dropped
        assert!(same_song("Nikt", "Wymyślona piosenka (Prod. Ktoś)", "Nikt", "Wymyślona piosenka"));
        assert!(same_song("Nikt", "Wymyślona Piosenka", "Nikt", "Wymyślona piosenka (feat. Ktoś)"));
        assert!(same_song("Nikt", "Wymyślona piosenka [with Ktoś]", "Nikt", "Wymyślona piosenka (ft. Ktoś)"));
        assert!(same_song("Nikt", "Wymyślona piosenka - Remastered", "Nikt", "Wymyślona piosenka"));
        assert!(!same_song("Nikt", "Inna piosenka", "Nikt", "Wymyślona piosenka"));
        assert_eq!(bare_title("  Fiji (Prod. Borucci) [feat. X] (Live)"), "fiji (live)");
        // in a search: someone else's record of the same length is no match
        let mut search: Vec<Value> = serde_json::from_str(SEARCH).unwrap();
        search[4]["artistName"] = "Ktoś".into();
        assert_eq!(word_timed_records(&search, "Nikt", "Wymyślona piosenka", 243_000).iter().map(|r| r["id"].as_u64().unwrap()).collect::<Vec<_>>(), [6]);
        search[3]["trackName"] = "Inna piosenka".into();
        assert!(word_timed_records(&search, "Nikt", "Wymyślona piosenka", 243_000).is_empty());
    }

    #[test]
    fn the_search_takes_the_closest_word_timed_version_of_the_same_length() {
        let search: Vec<Value> = serde_json::from_str(SEARCH).unwrap();
        let ids = |ms: u64| word_timed_records(&search, "Nikt", "Wymyślona piosenka", ms).iter().map(|r| r["id"].as_u64().unwrap()).collect::<Vec<_>>();
        assert_eq!(ids(243_000), [7, 6]);
        assert_eq!(ids(241_000), [6, 7]);
        assert_eq!(ids(170_500), [4]);
        assert_eq!(ids(250_000), [] as [u64; 0]);
        // an instrumental is no version to sing
        let mut inst = search.clone();
        inst[4]["instrumental"] = true.into();
        assert_eq!(word_timed_records(&inst, "Nikt", "Wymyślona piosenka", 243_000).iter().map(|r| r["id"].as_u64().unwrap()).collect::<Vec<_>>(), [6]);
    }

    #[test]
    fn hostile_yaml_is_refused_without_blowing_up() {
        // aliases (a billion laughs), deep nesting, garbage, too big
        let laughs = "a: &a [x, x, x, x, x, x, x, x, x]\nb: &b [*a, *a, *a, *a, *a, *a, *a, *a]\nc: &c [*b, *b, *b, *b, *b, *b, *b, *b]\nlines: [*c, *c]\n";
        assert!(parse_lyricsfile(laughs).is_none());
        assert!(parse_lyricsfile(&format!("lines: {}{}", "[".repeat(100_000), "]".repeat(100_000))).is_none());
        assert!(parse_lyricsfile(&"- ".repeat(50_000)).is_none());
        for junk in ["", ":", "lines: 5", "lines: [{start_ms: -1, text: a}]", "{[}", "lines:\n- words: [{text: a, start_ms: x}]\n  start_ms: 3"] {
            assert!(parse_lyricsfile(junk).is_none(), "{junk}");
        }
        assert!(parse_lyricsfile(&format!("{LYRICSFILE}#{}", "x".repeat(MAX_LYRICSFILE))).is_none());
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
