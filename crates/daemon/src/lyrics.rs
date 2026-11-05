//! Lyrics (#14): Spotify's own (spclient `color-lyrics`, through the Catalogue session), else LRCLIB
//! (<https://lrclib.net>, free and open; only artist, title, album and duration are sent).

use serde_json::Value;
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
            let words: Vec<LyricWord> = l
                .get("words")
                .map(Yaml::seq)
                .unwrap_or_default()
                .iter()
                .filter_map(|w| Some(LyricWord { start_ms: w.get("start_ms")?.ms()?, end_ms: w.get("end_ms").and_then(Yaml::ms), text: w.get("text")?.str()?.to_string() }))
                .filter(|w| !w.text.trim().is_empty())
                .collect();
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
