//! When each word is sung, for the Karaoke Widget. Exact where the lyrics say (`LyricLine::words`: LRCLIB's
//! word-synced Lyricsfile, enhanced LRC), estimated otherwise.
//!
//! An estimate sings the song at one pace: a singer's syllables come at about the song's tempo whatever the room
//! until the next line, and a line with room to spare holds its last word and rests (it doesn't slow down). So the
//! song's lines are fitted first (`pace`: a line's room ≈ syllables × pace + the hold and rest after them, a robust
//! straight-line fit over every line), then each line's words get that pace, scaled to the line's own room within
//! limits, a short pause at a comma, and the last word holds. Measured against modelled singing
//! (`tests::estimates_follow_modelled_singing`), word starts land a third closer than sharing the room out evenly
//! did (mean miss 450 → 300 ms, the worst tenth from 1.04 s → 0.69 s); a text can't say more: which syllables a
//! singer stretches is in the music alone.
//!
//! Then which line is on show at a moment (`at`): the next line takes the stage a little before it is sung (after
//! the line before has been sung out), and a long gap shows a countdown of dots before the line after it.

use singularity_proto::LyricLine;

/// How long the last word of an exactly timed line lasts per syllable when the source gives it no end.
pub const MAX_SYLLABLE_MS: u64 = 550;
/// The most of a line's room its estimated words may take: the rest is the breath before the next line.
const SUNG_SHARE: f64 = 0.95;
/// An estimated line's words start this long after its stamp (at most a tenth of its room): stamps tend to come a
/// moment before the voice.
const LEAD_MS: u64 = 80;
/// How much slower than the song's pace an estimated line may be sung when it has room to spare (it's rushed as much
/// as its room needs): beyond this the room is a held note and a rest, not slower singing.
const STRETCH: f64 = 1.15;
/// A comma (or `;`, `:`, a dash) in a line pauses for about a syllable.
const PAUSE_SYLLABLES: f64 = 1.0;
/// The last word of an estimated line holds at most this long past its syllables.
const MAX_HOLD_MS: f64 = 2_500.0;
/// The pace when a song gives nothing to fit (a line or two): ms a syllable, and the hold and rest after a line.
const FALLBACK_PACE: Pace = Pace { syllable: 300.0, tail: 900.0 };
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

/// The room line `i` has: until whatever comes next (another line or a mark); the last line until the song ends.
fn room_end(lines: &[LyricLine], i: usize, start: u64, song_ms: u64) -> u64 {
    let next = lines[i + 1..].iter().map(|n| n.start_ms).find(|&s| s > start);
    next.unwrap_or_else(|| if song_ms > start { song_ms } else { start.saturating_add(8_000) })
}

/// Whether a word ends on a pause mark (a comma and the like): the singer breathes there.
fn pauses(word: &str) -> bool {
    word.ends_with([',', ';', ':', '—', '–'])
}

/// The syllables a line's words take, its pauses included.
fn units(text: &str) -> f64 {
    let words: Vec<&str> = text.split_whitespace().collect();
    let syl: u32 = words.iter().map(|w| syllables(w)).sum();
    let breaths = words.iter().rev().skip(1).filter(|w| pauses(w)).count();
    syl.max(1) as f64 + breaths as f64 * PAUSE_SYLLABLES
}

/// How a song is sung, for estimating: ms a syllable, and how long a line's last word holds plus the rest after it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pace {
    pub syllable: f64,
    pub tail: f64,
}

/// The song's pace from its lines' (room, syllables): a straight line room = syllables × pace + tail through them,
/// fitted by least squares twice, the second time without the lines far off the first fit. Lines before an
/// instrumental (far more room than the song's usual) don't count. A fit that makes no sense falls back on the
/// typical room per syllable.
pub fn pace(rooms: &[(f64, f64)]) -> Pace {
    let mut per: Vec<f64> = rooms.iter().filter(|(r, u)| *r > 0.0 && *u > 0.0).map(|(r, u)| r / u).collect();
    if per.len() < 4 {
        return FALLBACK_PACE;
    }
    per.sort_by(f64::total_cmp);
    let med = per[per.len() / 2];
    let mut pts: Vec<(f64, f64)> = rooms.iter().filter(|(r, u)| *u > 0.0 && r / u < med * 2.5).map(|(r, u)| (*u, *r)).collect();
    let (mut p, mut c) = (med, 0.0);
    for _ in 0..2 {
        let n = pts.len() as f64;
        if n < 3.0 {
            break;
        }
        let (mx, my) = (pts.iter().map(|q| q.0).sum::<f64>() / n, pts.iter().map(|q| q.1).sum::<f64>() / n);
        let vx: f64 = pts.iter().map(|q| (q.0 - mx).powi(2)).sum();
        p = if vx > 0.0 { pts.iter().map(|q| (q.0 - mx) * (q.1 - my)).sum::<f64>() / vx } else { my / mx };
        c = my - p * mx;
        let res: Vec<f64> = pts.iter().map(|q| (q.1 - (p * q.0 + c)).abs()).collect();
        let mut sorted = res.clone();
        sorted.sort_by(f64::total_cmp);
        let limit = 3.0 * sorted[sorted.len() / 2] + 1.0;
        pts = pts.into_iter().zip(res).filter(|(_, e)| *e <= limit).map(|(q, _)| q).collect();
    }
    if (60.0..1_200.0).contains(&p) && c >= 0.0 { Pace { syllable: p, tail: c } } else { Pace { syllable: med * 0.75, tail: med * 1.5 } }
}

/// Every sung line of a song, timed. Empty lines (instrumental marks) aren't lines here, but the line before one
/// ends by it. `song_ms` (0 if unknown) bounds the last line. Times only ever go forward.
pub fn timed(lines: &[LyricLine], song_ms: u64) -> Vec<Line> {
    let sung = |l: &LyricLine| !(l.text.trim().is_empty() && l.words.is_empty());
    // the song's pace, from the lines without word times of their own
    let rooms: Vec<(f64, f64)> = (0..lines.len())
        .filter(|&i| sung(&lines[i]) && lines[i].words.is_empty())
        .map(|i| ((room_end(lines, i, lines[i].start_ms, song_ms) - lines[i].start_ms) as f64, units(&lines[i].text)))
        .collect();
    let pace = pace(&rooms);
    let mut out: Vec<Line> = vec![];
    let mut floor = 0u64;
    for (i, l) in lines.iter().enumerate() {
        if !sung(l) {
            continue;
        }
        let start = l.start_ms.max(floor);
        let room_end = room_end(lines, i, start, song_ms);
        let line = match exact(l, start, room_end) {
            Some(words) => Line { start, end: words.last().map_or(start, |w| w.end), words, exact: true, index: i },
            None => estimate(&l.text, start, room_end, i, pace),
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

/// A line's words at the song's pace, fitted to the line's room (see the module notes).
fn estimate(text: &str, start: u64, room_end: u64, index: usize, pace: Pace) -> Line {
    let pieces: Vec<&str> = text.split_whitespace().collect();
    let room = room_end.saturating_sub(start);
    let lead = LEAD_MS.min(room / 10);
    let usable = (room - lead) as f64;
    let n = units(text);
    // the line's share of the song's way of singing (its syllables and its tail), no slower than a little past the
    // song's pace, and inside the room whatever the fit says
    let per = (usable / (n + pace.tail / pace.syllable)).min(pace.syllable * STRETCH).min(usable * SUNG_SHARE / n);
    let first = (start + lead) as f64;
    let mut at = first;
    let last = pieces.len().saturating_sub(1);
    let mut words = Vec::with_capacity(pieces.len());
    for (k, w) in pieces.iter().enumerate() {
        let st = at;
        at += per * syllables(w) as f64;
        let mut end = at;
        if k == last {
            // the last word holds into the line's tail, as far as the room allows
            end = at + (usable * SUNG_SHARE - (at - first)).clamp(0.0, (pace.tail * 0.5).min(MAX_HOLD_MS));
        } else if pauses(w) {
            at += per * PAUSE_SYLLABLES;
        }
        words.push(Word { text: w.to_string(), start: st.round() as u64, end: (end.round() as u64).clamp(st.round() as u64, room_end), glued: false });
    }
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
    fn estimates_sing_at_the_songs_pace_and_hold_the_last_word() {
        // a verse at 250 ms a syllable with a second of hold and rest after each line: the fit finds the pace
        let verse = ["every little thing", "we carry over the river", "neon lights", "hold me, closer now", "run tonight under silver rain"];
        let mut lines = vec![];
        let mut at = 10_000u64;
        for _ in 0..4 {
            for t in verse {
                lines.push(line(at, t));
                at += (units(t) * 250.0) as u64 + 1_000;
            }
        }
        let rooms: Vec<(f64, f64)> = lines.windows(2).map(|w| ((w[1].start_ms - w[0].start_ms) as f64, units(&w[0].text))).collect();
        let p = pace(&rooms);
        assert!((p.syllable - 250.0).abs() < 5.0 && (p.tail - 1_000.0).abs() < 30.0, "{p:?}");
        let l = timed(&lines, at + 5_000);
        let w = &l[0].words;
        assert!(!l[0].exact);
        assert_eq!(w.len(), 3);
        assert_eq!(w[0].start, 10_080, "a moment after the stamp");
        // "every" is three syllables, "little" two: by syllables, at about the song's pace (a little under: the line
        // shares its room with its tail)
        let (a, b) = (w[1].start - w[0].start, w[2].start - w[1].start);
        assert!((a as f64 / b as f64 - 1.5).abs() < 0.05 && (460..=520).contains(&b), "{a} vs {b}");
        // the last word holds into the tail, with a breath before the next line
        assert!(w[2].end - w[2].start > 400 && l[0].end < l[1].start, "{:?}", l[0]);
        // a comma pauses: "hold me, closer now" has a syllable's rest after "me,"
        let h = &l[3].words;
        assert!(h[2].start - h[1].start > (h[1].start - h[0].start) * 3 / 2, "{h:?}");
    }

    #[test]
    fn a_long_gap_doesnt_stretch_the_last_words() {
        // the next line is 40 s away: the words keep a usual pace and the last one holds at most a little
        let l = timed(&[line(5_000, "hold on now"), line(45_000, "after the solo")], 0);
        let w = &l[0].words;
        assert!(w[2].start <= 5_000 + LEAD_MS + 2 * 400, "{:?}", l[0]);
        assert!(l[0].end <= w[2].start + 400 + MAX_HOLD_MS as u64, "{:?}", l[0]);
        // an empty line is a mark that ends the line before it, not a line
        let l = timed(&[line(5_000, "a b c d e f g h i j k l"), line(6_000, ""), line(9_000, "x")], 0);
        assert_eq!(l.len(), 2);
        assert!(l[0].end <= 6_000);
        assert_eq!(l[1].index, 2);
    }

    /// Invented songs sung the way people sing (`sing`), and how far estimated word starts are from the sung ones.
    mod singing {
        use super::*;

        /// A small deterministic random source (xorshift).
        pub struct Rng(u64);
        impl Rng {
            pub fn new(seed: u64) -> Rng {
                Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
            }
            pub fn next(&mut self) -> f64 {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 7;
                self.0 ^= self.0 << 17;
                (self.0 >> 11) as f64 / (1u64 << 53) as f64
            }
            pub fn int(&mut self, lo: u32, hi: u32) -> u32 {
                lo + (self.next() * (hi - lo + 1) as f64) as u32
            }
            pub fn gauss(&mut self, mean: f64, sd: f64) -> f64 {
                let (a, b) = (self.next().max(1e-12), self.next());
                mean + sd * (-2.0 * a.ln()).sqrt() * (std::f64::consts::TAU * b).cos()
            }
        }

        const WORDS: &str = "neon city lights we run tonight under silver rain and the river glows hold me closer never let go paper hearts falling slow electric dreams carry me home over oceans of the radio wire";

        /// A style of singing: tempo range, grid steps a beat, the steps a syllable takes (with their odds), the
        /// steps the last syllable holds, the rest after a line, and words a line.
        pub struct Style {
            pub name: &'static str,
            bpm: (f64, f64),
            sub: f64,
            steps: &'static [(u32, f64)],
            hold: (u32, u32),
            rest: (u32, u32),
            words: (u32, u32),
        }

        pub const STYLES: [Style; 4] = [
            Style { name: "ballad", bpm: (66.0, 80.0), sub: 2.0, steps: &[(1, 0.45), (2, 0.4), (3, 0.1), (4, 0.05)], hold: (3, 8), rest: (1, 4), words: (3, 7) },
            Style { name: "pop", bpm: (96.0, 124.0), sub: 2.0, steps: &[(1, 0.65), (2, 0.28), (3, 0.07)], hold: (2, 6), rest: (0, 3), words: (4, 9) },
            Style { name: "uptempo", bpm: (128.0, 150.0), sub: 2.0, steps: &[(1, 0.75), (2, 0.2), (3, 0.05)], hold: (2, 6), rest: (0, 4), words: (5, 10) },
            Style { name: "rap", bpm: (86.0, 98.0), sub: 4.0, steps: &[(1, 0.8), (2, 0.17), (3, 0.03)], hold: (1, 3), rest: (0, 2), words: (8, 14) },
        ];

        /// A sung song: its lyrics as the source stamps them (a line's stamp near its first word, often a little
        /// early), each line's sung word starts, and its length. Syllables fall on the beat's grid and mostly take
        /// one step, some two or more; the last one holds; a comma breathes; lines rest a while, sometimes for an
        /// instrumental; the syllable counter is wrong about how a word is sung one time in ten.
        pub fn sing(seed: u64, style: &Style) -> (Vec<LyricLine>, Vec<Vec<f64>>, u64) {
            let mut r = Rng::new(seed);
            let pool: Vec<&str> = WORDS.split(' ').collect();
            let beat = 60_000.0 / (style.bpm.0 + r.next() * (style.bpm.1 - style.bpm.0));
            let step = beat / style.sub;
            let phase = r.next() * beat;
            let mut t = phase + 4.0 * beat * r.int(2, 4) as f64;
            let (mut lines, mut truth) = (vec![], vec![]);
            for _ in 0..r.int(28, 44) {
                let n = r.int(style.words.0, style.words.1) as usize;
                let mut words: Vec<String> = (0..n).map(|_| pool[(r.next() * pool.len() as f64) as usize].to_string()).collect();
                if r.next() < 0.3 && n > 4 {
                    let k = r.int(1, n as u32 - 2) as usize;
                    words[k].push(',');
                }
                let mut starts = vec![];
                let mut at = t;
                for (wi, w) in words.iter().enumerate() {
                    starts.push(at);
                    let mut s = syllables(w) as i64;
                    if r.next() < 0.1 {
                        s = (s + if r.next() < 0.5 { -1 } else { 1 }).max(1);
                    }
                    for si in 0..s {
                        let held = wi == n - 1 && si == s - 1;
                        let k = if held {
                            r.int(style.hold.0, style.hold.1)
                        } else {
                            let (x, mut acc) = (r.next(), 0.0);
                            style.steps.iter().find(|(_, p)| {
                                acc += p;
                                x <= acc
                            }).map_or(1, |(k, _)| *k)
                        };
                        at += step * k as f64;
                    }
                    if w.ends_with(',') {
                        at += step * r.int(1, 2) as f64;
                    }
                }
                let stamp = (starts[0] + r.gauss(-60.0, 90.0)).max(0.0) as u64;
                lines.push(LyricLine { start_ms: stamp, text: words.join(" "), ..Default::default() });
                truth.push(starts);
                t = at + step * r.int(style.rest.0, style.rest.1) as f64;
                t = phase + ((t - phase) / step).ceil() * step;
                if r.next() < 0.08 {
                    t += 4.0 * beat * r.int(2, 4) as f64;
                }
            }
            (lines, truth, (t + 10_000.0) as u64)
        }

        /// The estimate before the song's pace was fitted (2026-11-05): 88 % of the room shared by syllables, at
        /// most 550 ms a syllable, 80 ms after the stamp. Kept to measure against.
        pub fn even_share(lines: &[LyricLine], song_ms: u64) -> Vec<Vec<f64>> {
            (0..lines.len())
                .map(|i| {
                    let start = lines[i].start_ms;
                    let room = (room_end(lines, i, start, song_ms) - start) as f64;
                    let syl: Vec<f64> = lines[i].text.split_whitespace().map(|w| syllables(w) as f64).collect();
                    let lead = 80f64.min(room / 10.0);
                    let per = ((room - lead) * 0.88 / syl.iter().sum::<f64>()).min(550.0);
                    let mut at = start as f64 + lead;
                    syl.iter()
                        .map(|s| {
                            let st = at;
                            at += per * s;
                            st
                        })
                        .collect()
                })
                .collect()
        }

        /// |estimated − sung| of every word start, sorted.
        pub fn misses(estimated: &[Vec<f64>], sung: &[Vec<f64>]) -> Vec<f64> {
            let mut out: Vec<f64> = estimated.iter().zip(sung).flat_map(|(e, s)| e.iter().zip(s).map(|(a, b)| (a - b).abs())).collect();
            out.sort_by(f64::total_cmp);
            out
        }
    }

    #[test]
    fn estimates_follow_modelled_singing() {
        use singing::*;
        let stats = |m: &[f64]| (m.iter().sum::<f64>() / m.len() as f64, m[m.len() / 2], m[m.len() * 9 / 10], m.iter().filter(|x| **x <= 150.0).count() as f64 / m.len() as f64);
        let (mut all_old, mut all_new) = (vec![], vec![]);
        for style in &STYLES {
            let (mut old, mut new) = (vec![], vec![]);
            for seed in 0..40 {
                let (lines, sung, song_ms) = sing(seed * 31 + style.name.len() as u64, style);
                old.extend(misses(&even_share(&lines, song_ms), &sung));
                let est: Vec<Vec<f64>> = timed(&lines, song_ms).iter().map(|l| l.words.iter().map(|w| w.start as f64).collect()).collect();
                new.extend(misses(&est, &sung));
            }
            old.sort_by(f64::total_cmp);
            new.sort_by(f64::total_cmp);
            let (o, n) = (stats(&old), stats(&new));
            eprintln!("{:8} before: mean {:4.0} median {:4.0} p90 {:5.0} ≤150 ms {:3.0} %   after: mean {:4.0} median {:4.0} p90 {:5.0} ≤150 ms {:3.0} %", style.name, o.0, o.1, o.2, o.3 * 100.0, n.0, n.1, n.2, n.3 * 100.0);
            // no style gets worse on the whole
            assert!(n.0 < o.0 * 1.05, "{}: {o:?} → {n:?}", style.name);
            all_old.extend(old);
            all_new.extend(new);
        }
        all_old.sort_by(f64::total_cmp);
        all_new.sort_by(f64::total_cmp);
        let (o, n) = (stats(&all_old), stats(&all_new));
        eprintln!("all      before: mean {:4.0} median {:4.0} p90 {:5.0} ≤150 ms {:3.0} %   after: mean {:4.0} median {:4.0} p90 {:5.0} ≤150 ms {:3.0} %", o.0, o.1, o.2, o.3 * 100.0, n.0, n.1, n.2, n.3 * 100.0);
        assert!(n.0 < o.0 * 0.8 && n.2 < o.2 * 0.8, "{o:?} → {n:?}");
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
