//! When each word is sung, for the Karaoke Widget. Exact where the lyrics say (`LyricLine::words`: LRCLIB's
//! word-synced Lyricsfile, enhanced LRC), estimated otherwise: a line's words share the time until the next line by
//! their syllables, each syllable at most `MAX_SYLLABLE_MS`, so a line before a long instrumental doesn't stretch its
//! last words over the solo (the ball rests on the last word instead, then hops on).
//!
//! Then which line is on show at a moment (`at`): the next line takes the stage a little before it is sung (after
//! the line before has been sung out), and a long gap shows a countdown of dots before the line after it.

use singularity_proto::LyricLine;

/// How long a syllable may last when estimated (a slow ballad's held notes are longer, but a guess that long reads
/// as wrong more often than right).
pub const MAX_SYLLABLE_MS: u64 = 550;
/// The part of a line's room an estimate may use: the rest is the breath before the next line.
const SUNG_SHARE: f64 = 0.88;
/// An estimated line's words start this long after its stamp (at most a tenth of its room).
const LEAD_MS: u64 = 80;
/// A gap at least this long between two lines is an instrumental: dots count the next line in.
pub const GAP_MS: u64 = 6_000;
/// The countdown before a line after a gap (one dot a second).
pub const COUNTDOWN_MS: u64 = 4_000;
/// The next line comes on stage this long before it is sung (once the one before is done).
pub const PREROLL_MS: u64 = 1_000;
/// A sung-out line stays this long before a gap's own view takes over.
pub const LINGER_MS: u64 = 1_500;
/// The longest hop of the ball: between words further apart than this, it rests, then hops.
pub const HOP_MS: u64 = 650;

/// A timed piece of a line: a word, or a syllable when the source times syllables.
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub text: String,
    pub start: u64,
    /// when its fill is complete (after `start`, at most the next piece's start)
    pub end: u64,
    /// sung straight after the piece before, no space between (a syllable of the same word, or CJK)
    pub glued: bool,
}

/// A line with its words timed.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// its stamp
    pub start: u64,
    /// when its last word is sung out
    pub end: u64,
    pub words: Vec<Word>,
    /// the words' times came with the lyrics (else they're estimated)
    pub exact: bool,
    /// the original line, for seeking to it
    pub index: usize,
}

impl Line {
    /// When the first word is sung.
    pub fn first(&self) -> u64 {
        self.words.first().map_or(self.start, |w| w.start)
    }
}

/// Syllables in a word, roughly: groups of vowels (a silent final `e` in English doesn't count), at least one.
/// Scripts without vowel letters (CJK, kana) count a syllable per character.
pub fn syllables(word: &str) -> u32 {
    let letters: Vec<char> = word.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect();
    if letters.is_empty() {
        return 1;
    }
    if letters.iter().any(|c| unicode_width::UnicodeWidthChar::width(*c).unwrap_or(0) >= 2) {
        return letters.len() as u32;
    }
    let vowel = |c: char| "aeiouyàáâãäåæèéêëìíîïòóôõöøùúûüýÿąęėįųœ".contains(c);
    let mut groups = 0;
    let mut prev = false;
    for &c in &letters {
        let v = vowel(c);
        if v && !prev {
            groups += 1;
        }
        prev = v;
    }
    // "home", "time": the final e is silent; "the", "be", "little" keep it
    let n = letters.len();
    if groups > 1 && n > 2 && letters[n - 1] == 'e' && !vowel(letters[n - 2]) && !(letters[n - 2] == 'l' && !vowel(letters[n - 3])) {
        groups -= 1;
    }
    groups.max(1)
}

/// Every sung line of a song, timed. Empty lines (instrumental marks) aren't lines here, but the line before one
/// ends by it. `song_ms` (0 if unknown) bounds the last line. Times only ever go forward.
pub fn timed(lines: &[LyricLine], song_ms: u64) -> Vec<Line> {
    let mut out: Vec<Line> = vec![];
    let mut floor = 0u64;
    for (i, l) in lines.iter().enumerate() {
        if l.text.trim().is_empty() && l.words.is_empty() {
            continue;
        }
        let start = l.start_ms.max(floor);
        // the room until whatever comes next (another line or a mark); the last line until the song ends
        let next = lines[i + 1..].iter().map(|n| n.start_ms).find(|&s| s > start);
        let room_end = next.unwrap_or_else(|| if song_ms > start { song_ms } else { start.saturating_add(8_000) });
        let line = match exact(l, start, room_end) {
            Some(words) => Line { start, end: words.last().map_or(start, |w| w.end), words, exact: true, index: i },
            None => estimate(&l.text, start, room_end, i),
        };
        floor = line.end.max(start);
        out.push(line);
    }
    out
}

/// The line's own word times, cleaned up: inside [start, room_end], never going back, each word ending by the next.
fn exact(l: &LyricLine, start: u64, room_end: u64) -> Option<Vec<Word>> {
    if l.words.is_empty() {
        return None;
    }
    let mut words: Vec<Word> = vec![];
    let mut at = start;
    let mut space_before = true;
    for w in &l.words {
        let t = w.start_ms.clamp(at, room_end);
        let text = w.text.trim();
        if text.is_empty() {
            space_before = true;
            continue;
        }
        let glued = !space_before && !w.text.starts_with(char::is_whitespace) && !words.is_empty();
        words.push(Word { text: text.to_string(), start: t, end: w.end_ms.unwrap_or(0), glued });
        space_before = w.text.ends_with(char::is_whitespace);
        at = t;
    }
    if words.is_empty() {
        return None;
    }
    let n = words.len();
    for k in 0..n {
        let next = if k + 1 < n { words[k + 1].start } else { room_end };
        let said = words[k].end;
        words[k].end = if said > words[k].start && said <= next {
            said
        } else if k + 1 < n {
            next
        } else {
            // the last word without an end of its own: as long as an estimate would make it
            words[k].start.saturating_add(syllables(&words[k].text) as u64 * MAX_SYLLABLE_MS).min(next)
        };
    }
    Some(words)
}

/// A line's words spread over its room by their syllables (see the module notes).
fn estimate(text: &str, start: u64, room_end: u64, index: usize) -> Line {
    let pieces: Vec<&str> = text.split_whitespace().collect();
    let syl: Vec<u64> = pieces.iter().map(|w| syllables(w) as u64).collect();
    let total: u64 = syl.iter().sum::<u64>().max(1);
    let room = room_end.saturating_sub(start);
    let lead = LEAD_MS.min(room / 10);
    let per = ((room - lead) as f64 * SUNG_SHARE / total as f64).min(MAX_SYLLABLE_MS as f64);
    let mut at = (start + lead) as f64;
    let words = pieces
        .iter()
        .zip(&syl)
        .map(|(w, s)| {
            let st = at;
            at += per * *s as f64;
            Word { text: w.to_string(), start: st.round() as u64, end: (at.round() as u64).min(room_end), glued: false }
        })
        .collect::<Vec<_>>();
    Line { start, end: words.last().map_or(start, |w| w.end), words, exact: false, index }
}

/// How far `w` is sung at `pos`: 0 before it starts, 1 once it ends.
pub fn fill(w: &Word, pos: u64) -> f32 {
    if pos >= w.end || w.end <= w.start {
        return if pos >= w.start { 1.0 } else { 0.0 };
    }
    (pos.saturating_sub(w.start) as f32 / (w.end - w.start) as f32).clamp(0.0, 1.0)
}

/// What the Karaoke Widget shows at a moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Stage {
    /// line `i` is on stage; `count` dots count it in (a gap before it), 0 when none
    Line { i: usize, count: u8 },
    /// an instrumental: line `next` is `in_ms` away (None: the song's lines are over)
    Gap { next: Option<usize>, in_ms: u64 },
}

/// Whether line `i` comes after a gap (or opens the song).
pub fn after_gap(lines: &[Line], i: usize) -> bool {
    i == 0 || lines[i].first().saturating_sub(lines[i - 1].end) >= GAP_MS
}

/// When line `i` takes the stage: a little before it is sung (longer after a gap, for the countdown), never before
/// the line before it is sung out.
pub fn on_stage(lines: &[Line], i: usize) -> u64 {
    let pre = if after_gap(lines, i) { COUNTDOWN_MS } else { PREROLL_MS };
    let early = lines[i].first().saturating_sub(pre).min(lines[i].start);
    if i == 0 { early } else { early.max(lines[i - 1].end).min(lines[i].start.max(lines[i - 1].end)) }
}

/// What's on show at `pos`.
pub fn at(lines: &[Line], pos: u64) -> Stage {
    if lines.is_empty() {
        return Stage::Gap { next: None, in_ms: 0 };
    }
    // the last line already on stage (switch times only go forward, so a binary search would do; songs are short)
    let shown = (0..lines.len()).rev().find(|&i| on_stage(lines, i) <= pos);
    let Some(i) = shown else {
        return Stage::Gap { next: Some(0), in_ms: lines[0].first().saturating_sub(pos) };
    };
    let l = &lines[i];
    // sung out, and a long way to the next line (or none): the gap's own view
    if pos >= l.end + LINGER_MS {
        match lines.get(i + 1) {
            None => return Stage::Gap { next: None, in_ms: 0 },
            Some(nx) if after_gap(lines, i + 1) => return Stage::Gap { next: Some(i + 1), in_ms: nx.first().saturating_sub(pos) },
            _ => {}
        }
    }
    let count = if after_gap(lines, i) && pos < l.first() { (l.first() - pos).div_ceil(1000).min(COUNTDOWN_MS / 1000) as u8 } else { 0 };
    Stage::Line { i, count }
}

/// Where the ball is over line `l` at `pos`: (word position, height). The position counts words, fractions between
/// them: -1 is the waiting spot left of the first word. Height 0 is resting on a word, 1 the top of the hop.
pub fn ball(l: &Line, pos: u64, shown_at: u64) -> (f32, f32) {
    let w = &l.words;
    if w.is_empty() {
        return (-1.0, 0.0);
    }
    // the word it is on or heading for: the last one started
    let k = w.partition_point(|x| x.start <= pos);
    if k == 0 {
        // waiting left of the line, then hopping onto the first word as it is sung
        let hop = HOP_MS.min(w[0].start.saturating_sub(shown_at)).max(1);
        let from = w[0].start.saturating_sub(hop);
        if pos < from {
            return (-1.0, 0.0);
        }
        let t = (pos - from) as f32 / hop as f32;
        return (-1.0 + t, arc(t, hop));
    }
    if k >= w.len() {
        return ((w.len() - 1) as f32, 0.0);
    }
    // between word k-1 and word k: rest, then hop to land as word k starts
    let gap = w[k].start - w[k - 1].start;
    let hop = HOP_MS.min(gap).max(1);
    let from = w[k].start - hop;
    if pos < from {
        return ((k - 1) as f32, 0.0);
    }
    let t = (pos - from) as f32 / hop as f32;
    ((k - 1) as f32 + t, arc(t, hop))
}

/// The hop's height at `t` (0..1) of a hop lasting `hop_ms`: a parabola, lower for quick hops.
fn arc(t: f32, hop_ms: u64) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let peak = (hop_ms as f32 / 400.0).clamp(0.35, 1.0);
    4.0 * t * (1.0 - t) * peak
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularity_proto::LyricWord;

    fn line(start_ms: u64, text: &str) -> LyricLine {
        LyricLine { start_ms, text: text.into(), ..Default::default() }
    }

    #[test]
    fn syllables_are_counted_well_enough() {
        for (w, n) in [("the", 1), ("home", 1), ("time", 1), ("little", 2), ("every", 3), ("echo", 2), ("beautiful", 3), ("night", 1), ("chorus,", 2), ("I", 1), ("rhythm", 1), ("canción", 2), ("żółw", 1), ("♪", 1), ("光が", 2), ("2024", 1)] {
            assert_eq!(syllables(w), n, "{w}");
        }
    }

    #[test]
    fn estimates_share_the_room_by_syllables_and_keep_a_breath() {
        let l = timed(&[line(10_000, "every little thing"), line(14_000, "next")], 0);
        let w = &l[0].words;
        assert!(!l[0].exact);
        assert_eq!(w.len(), 3);
        // 3 + 2 + 1 syllables in 88 % of 3.92 s: 575 ms each, capped at 550
        assert_eq!(w[0].start, 10_080);
        assert_eq!(w[1].start - w[0].start, 3 * MAX_SYLLABLE_MS);
        assert!(l[0].end < 14_000, "a breath before the next line");
        // a fast line: no cap, the shares are by syllable
        let l = timed(&[line(0, "every little thing"), line(1_200, "next")], 0);
        let w = &l[0].words;
        let (a, b) = (w[1].start - w[0].start, w[2].start - w[1].start);
        assert!((a as f64 / b as f64 - 1.5).abs() < 0.05, "{a} vs {b}");
    }

    #[test]
    fn a_long_gap_doesnt_stretch_the_last_words() {
        // the next line is 40 s away: three words still take at most 550 ms a syllable
        let l = timed(&[line(5_000, "hold on now"), line(45_000, "after the solo")], 0);
        assert!(l[0].end <= 5_000 + LEAD_MS + 3 * MAX_SYLLABLE_MS, "{:?}", l[0]);
        // an empty line is a mark that ends the line before it, not a line
        let l = timed(&[line(5_000, "a b c d e f g h i j k l"), line(6_000, ""), line(9_000, "x")], 0);
        assert_eq!(l.len(), 2);
        assert!(l[0].end <= 6_000);
        assert_eq!(l[1].index, 2);
    }

    #[test]
    fn exact_times_are_used_and_cleaned() {
        let words = vec![
            LyricWord { start_ms: 900, end_ms: None, text: "Turn ".into() },
            LyricWord { start_ms: 1_400, end_ms: Some(1_600), text: "the ".into() },
            LyricWord { start_ms: 1_700, end_ms: None, text: "beau".into() },
            // out of order: held at the word before
            LyricWord { start_ms: 1_650, end_ms: None, text: "ti".into() },
            LyricWord { start_ms: 2_300, end_ms: Some(99_000), text: "ful".into() },
        ];
        let l = timed(&[LyricLine { start_ms: 1_000, text: "Turn the beautiful".into(), words }, line(4_000, "next")], 0);
        let w = &l[0].words;
        assert!(l[0].exact);
        assert_eq!(w.iter().map(|w| w.start).collect::<Vec<_>>(), [1_000, 1_400, 1_700, 1_700, 2_300], "inside the line, never back");
        assert_eq!(w.iter().map(|w| w.glued).collect::<Vec<_>>(), [false, false, false, true, true]);
        assert_eq!(w[1].end, 1_600, "its own end");
        assert_eq!(w[0].end, 1_400, "else the next word's start");
        assert_eq!(w[4].end, 2_300 + MAX_SYLLABLE_MS, "an end past the next line is no end: estimated");
    }

    #[test]
    fn times_only_go_forward_whatever_the_lyrics_say() {
        // overlapping, repeated and backwards stamps, words before their line
        let mut lines = vec![line(5_000, "one two three four five six seven"), line(5_000, "same time"), line(4_000, "backwards"), line(u64::MAX, "the end")];
        lines.push(LyricLine { start_ms: 30_000, text: "x y".into(), words: vec![LyricWord { start_ms: 1, end_ms: Some(0), text: "x ".into() }, LyricWord { start_ms: u64::MAX, end_ms: None, text: "y".into() }] });
        for song in [0, 60_000] {
            let l = timed(&lines, song);
            let mut last = 0;
            for line in &l {
                assert!(line.start >= last, "{l:#?}");
                for w in &line.words {
                    assert!(w.start >= last && w.end >= w.start, "{l:#?}");
                    last = w.start;
                }
                assert!(line.end >= line.start);
                last = last.max(line.end);
            }
        }
    }

    fn song() -> Vec<Line> {
        timed(&[line(3_000, "turn the lights down"), line(6_500, "sing it like the night"), line(10_000, ""), line(20_000, "after the gap")], 0)
    }

    #[test]
    fn the_stage_counts_in_after_a_gap_and_switches_early() {
        let l = song();
        // the intro: a gap until the countdown before the first line
        let first = l[0].first();
        assert_eq!(at(&l, 0), Stage::Line { i: 0, count: 4 });
        assert_eq!(at(&l, first - 2_500), Stage::Line { i: 0, count: 3 });
        assert_eq!(at(&l, first - 500), Stage::Line { i: 0, count: 1 });
        assert_eq!(at(&l, 4_000), Stage::Line { i: 0, count: 0 });
        // the next line comes on a moment early, once the first is sung out
        assert!(l[0].end < l[1].first() - PREROLL_MS);
        assert_eq!(at(&l, l[1].first() - PREROLL_MS - 1), Stage::Line { i: 0, count: 0 });
        assert_eq!(at(&l, l[1].first() - PREROLL_MS), Stage::Line { i: 1, count: 0 });
        // the mark at 10 s ends line 1; sung out plus a moment: the gap, then the countdown for "after the gap"
        assert!(l[1].end <= 10_000);
        assert!(matches!(at(&l, 12_000), Stage::Gap { next: Some(2), in_ms } if in_ms == l[2].first() - 12_000));
        assert_eq!(at(&l, l[2].first() - 2_500), Stage::Line { i: 2, count: 3 });
        assert_eq!(at(&l, 60_000), Stage::Gap { next: None, in_ms: 0 });
        assert_eq!(at(&[], 5), Stage::Gap { next: None, in_ms: 0 });
        // the stage only goes forward through the song
        let mut last = (0usize, false);
        for pos in (0..30_000).step_by(37) {
            if let Stage::Line { i, .. } = at(&l, pos) {
                assert!(i >= last.0, "{pos}");
                last = (i, true);
            }
        }
    }

    #[test]
    fn the_ball_waits_hops_and_lands_on_each_word_as_it_starts() {
        let l = &song()[0];
        let shown = on_stage(&song(), 0);
        let w = &l.words;
        assert_eq!(ball(l, shown, shown), (-1.0, 0.0), "waiting left of the line");
        assert_eq!(ball(l, w[0].start, shown), (0.0, 0.0), "lands on the first word as it starts");
        let (x, h) = ball(l, w[0].start - HOP_MS.min(w[0].start - shown) / 2, shown);
        assert!((x + 0.5).abs() < 0.01 && h > 0.3, "mid-hop: {x} {h}");
        for (k, wk) in w.iter().enumerate().skip(1) {
            assert_eq!(ball(l, wk.start, shown), (k as f32, 0.0));
            let (x, h) = ball(l, wk.start - 1, shown);
            assert!(x > (k - 1) as f32 && x < k as f32 && h < 0.1, "about to land: {x} {h}");
        }
        assert_eq!(ball(l, l.end + 5_000, shown), ((w.len() - 1) as f32, 0.0), "rests on the last word");
        // positions only go forward
        let mut last = -2.0;
        for pos in (shown..l.end).step_by(11) {
            let (x, h) = ball(l, pos, shown);
            assert!(x >= last && (0.0..=1.0).contains(&h), "{pos}: {x} {h}");
            last = x;
        }
    }

    #[test]
    fn fill_sweeps_each_word_once() {
        let w = Word { text: "x".into(), start: 1_000, end: 2_000, glued: false };
        assert_eq!((fill(&w, 999), fill(&w, 1_500), fill(&w, 2_000)), (0.0, 0.5, 1.0));
        let z = Word { end: 1_000, ..w };
        assert_eq!((fill(&z, 999), fill(&z, 1_000)), (0.0, 1.0));
    }
}
