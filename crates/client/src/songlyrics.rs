//! The playing song's lyrics, as the Client holds them: what was asked of the Daemon, what arrived, and everything
//! the lyrics and Karaoke Widgets work out from them (wrapped rows, timed lines, sizes) and where their views are.
//!
//! An arrival (`receive`) starts all of that afresh, so nothing drawn from the lyrics before it can outlive them: the
//! draw functions get their rows and timed lines from here and never guess whether the lyrics changed. A Karaoke Pane
//! asks again with word timing (D67); that answer has the same song and lines as the one before, only the words'
//! times differ, and it is a new arrival like any other.

use crate::karaoke::timing::Line;
use singularity_proto::Lyrics;

#[derive(Default)]
pub struct SongLyrics {
    lyrics: Option<Lyrics>,
    /// the song lyrics were asked for, and whether with word timing (a Karaoke Pane was on screen)
    asked: Option<(String, bool)>,
    /// whether an answer for that song has come since
    answered: bool,
    /// the lyrics Widget's rows and view
    view: crate::lyrics::Wrapped,
    /// the Karaoke Widget's timed lines, sizes and view
    karaoke: crate::karaoke::State,
}

/// What the Karaoke Widget sings from: the lyrics, their timed lines, and the song's sizes for a Pane.
pub struct Sung<'a> {
    pub lyrics: &'a Lyrics,
    pub lines: &'a [Line],
    /// big text's pixels to the em (the pixel tier; None: the cells)
    pub px: Option<f32>,
    /// the pixel font's scale (0: plain text)
    pub scale: u16,
}

impl SongLyrics {
    /// Lyrics from the Daemon. Kept only for the song playing (an answer for one already skipped is dropped).
    pub fn receive(&mut self, l: Lyrics, playing: Option<&str>) {
        if playing != Some(l.uri.as_str()) {
            return;
        }
        self.answered |= self.asked.as_ref().is_some_and(|(u, _)| *u == l.uri);
        self.lyrics = Some(l);
        self.view = Default::default();
        self.karaoke.new_lyrics();
    }

    /// Whether to ask for the playing song's lyrics now, and with word timing: once a song, while a lyrics or Karaoke
    /// Pane is on screen; again with word timing when a Karaoke Pane comes on screen after a lyrics Pane asked.
    pub fn ask(&mut self, playing: &str, lyrics_shown: bool, karaoke_shown: bool) -> Option<bool> {
        let words = karaoke_shown;
        let done = matches!(&self.asked, Some((u, w)) if u == playing && (*w || !words));
        if done || !(words || lyrics_shown) {
            return None;
        }
        self.asked = Some((playing.to_string(), words));
        self.answered = false;
        Some(words)
    }

    /// The link to the Daemon came back: a request not answered yet may have been lost with it, so it is asked again.
    pub fn reconnected(&mut self) {
        if !self.answered {
            self.asked = None;
        }
    }

    /// The lyrics in hand if they are `uri`'s.
    pub fn for_song(&self, uri: &str) -> Option<&Lyrics> {
        self.lyrics.as_ref().filter(|l| l.uri == uri)
    }

    /// The lyrics in hand and the lyrics Widget's rows and view, for drawing.
    pub fn lyrics_view(&mut self) -> Option<(&Lyrics, &mut crate::lyrics::Wrapped)> {
        Some((self.lyrics.as_ref()?, &mut self.view))
    }

    /// The Karaoke Widget's lyrics and timed lines for a song `duration` ms long, and the song's sizes for a `w`×`h`
    /// Pane (cells of `cpx` pixels on the pixel tier): each worked out once for these lyrics, Pane and cells.
    pub fn sung(&mut self, duration: u64, w: u16, h: u16, cpx: Option<(f64, f64)>) -> Option<Sung<'_>> {
        let lyrics = self.lyrics.as_ref()?;
        let (lines, px, scale) = self.karaoke.song(&lyrics.lines, duration, w, h, cpx);
        Some(Sung { lyrics, lines, px, scale })
    }

    /// The lyrics in hand, whichever song they are for.
    pub fn current(&self) -> Option<&Lyrics> {
        self.lyrics.as_ref()
    }

    /// The Karaoke Widget's view (the wheel's hold, whether the ball needs quick frames).
    pub fn karaoke(&self) -> &crate::karaoke::State {
        &self.karaoke
    }

    /// The Karaoke ball is moving on the pixel tier: quicker frames for a moment.
    pub fn ball_moving(&mut self) {
        self.karaoke.moving();
    }

    /// The wheel over the lyrics Widget, `rows` rows on: synced lyrics are held there for a while, unsynced ones stay
    /// where the reader takes them. False when there are no lyrics to move.
    pub fn wheel_lyrics(&mut self, rows: i32) -> bool {
        let Some(l) = self.lyrics.as_ref() else { return false };
        if l.synced {
            self.view.follow.by(rows);
        } else {
            self.view.scroll = (self.view.scroll as i64 + rows as i64).max(0) as usize;
        }
        true
    }

    /// The wheel over the Karaoke Widget: only unsynced lyrics scroll (synced, the line being sung is the point).
    /// False when there is nothing it moves.
    pub fn wheel_karaoke(&mut self, rows: i32) -> bool {
        if !self.lyrics.as_ref().is_some_and(|l| !l.synced && !l.lines.is_empty()) {
            return false;
        }
        self.karaoke.follow.by(rows);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularity_proto::{LyricLine, LyricWord};

    const URI: &str = "spotify:track:k";

    /// A song of three lines, line-synced, or with its first line's words timed too.
    fn lyrics(words: bool) -> Lyrics {
        let timed = vec![
            LyricWord { start_ms: 3_000, end_ms: None, text: "Turn ".into() },
            LyricWord { start_ms: 3_400, end_ms: None, text: "the ".into() },
            LyricWord { start_ms: 3_700, end_ms: None, text: "lights ".into() },
            LyricWord { start_ms: 4_600, end_ms: Some(5_400), text: "down".into() },
        ];
        let lines = vec![
            LyricLine { start_ms: 3_000, text: "Turn the lights down".into(), words: if words { timed } else { vec![] } },
            LyricLine { start_ms: 6_500, text: "Sing it like the night is young".into(), ..Default::default() },
            LyricLine { start_ms: 10_000, text: "Hold on".into(), ..Default::default() },
        ];
        Lyrics { uri: URI.into(), synced: true, lines, source: "LRCLIB".into(), ..Default::default() }
    }

    fn first_line(s: &mut SongLyrics) -> Line {
        s.sung(120_000, 60, 12, None).expect("lyrics in hand").lines[0].clone()
    }

    /// B11: a lyrics Pane asked without word timing, then a Karaoke Pane asked again with it. The second answer has
    /// the same song, line count and length; Karaoke must sing its exact word times, not keep the estimate.
    #[test]
    fn word_timing_that_arrives_later_replaces_the_estimate() {
        let mut s = SongLyrics::default();
        s.receive(lyrics(false), Some(URI));
        let estimated = first_line(&mut s);
        assert!(!estimated.exact);
        s.receive(lyrics(true), Some(URI));
        let exact = first_line(&mut s);
        assert!(exact.exact, "{exact:?}");
        assert_eq!(exact.words.iter().map(|w| w.start).collect::<Vec<_>>(), [3_000, 3_400, 3_700, 4_600]);
    }

    #[test]
    fn an_answer_for_another_song_is_dropped() {
        let mut s = SongLyrics::default();
        s.receive(lyrics(false), Some("spotify:track:other"));
        assert!(s.for_song(URI).is_none() && s.sung(120_000, 60, 12, None).is_none());
        s.receive(lyrics(false), None);
        assert!(s.for_song(URI).is_none());
        s.receive(lyrics(false), Some(URI));
        assert!(s.for_song(URI).is_some() && s.for_song("spotify:track:other").is_none());
    }

    #[test]
    fn asks_once_a_song_and_again_for_word_timing() {
        let mut s = SongLyrics::default();
        assert_eq!(s.ask(URI, false, false), None, "no lyrics Widget on screen: nothing to ask");
        assert_eq!(s.ask(URI, true, false), Some(false));
        assert_eq!(s.ask(URI, true, false), None);
        assert_eq!(s.ask(URI, true, true), Some(true), "a Karaoke Pane wants word timing");
        assert_eq!(s.ask(URI, true, false), None, "word timing already asked for");
        assert_eq!(s.ask("spotify:track:next", true, false), Some(false), "a new song");
    }

    #[test]
    fn a_reconnect_asks_again() {
        let mut s = SongLyrics::default();
        assert_eq!(s.ask(URI, false, true), Some(true));
        s.reconnected();
        assert_eq!(s.ask(URI, false, true), Some(true), "the answer may have been lost with the link");
    }

    #[test]
    fn a_reconnect_leaves_an_answered_request_alone() {
        let mut s = SongLyrics::default();
        assert_eq!(s.ask(URI, true, false), Some(false));
        s.receive(lyrics(false), Some(URI));
        s.reconnected();
        assert_eq!(s.ask(URI, true, false), None, "the lyrics are in hand: nothing to ask again");
        // word timing asked for and lost with the link: asked again
        assert_eq!(s.ask(URI, true, true), Some(true));
        s.reconnected();
        assert_eq!(s.ask(URI, true, true), Some(true));
    }

    #[test]
    fn an_arrival_starts_the_view_afresh() {
        let mut s = SongLyrics::default();
        let mut l = lyrics(false);
        l.synced = false;
        s.receive(l.clone(), Some(URI));
        assert!(s.wheel_lyrics(6));
        assert_eq!(s.lyrics_view().unwrap().1.scroll, 6);
        s.receive(l, Some(URI));
        assert_eq!(s.lyrics_view().unwrap().1.scroll, 0, "new lyrics: from the top");
    }
}
