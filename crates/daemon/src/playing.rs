//! Now playing (D80): what is playing and on which device, the same for every Client.
//!
//! The hub feeds it the Engine's state, another device's state (or that it's gone), the track names the Catalogue
//! found or couldn't find, and a resync; each returns what the hub sends on. It holds no lock and never waits, so
//! the hub keeps it behind one lock and an update can't be lost between reading the state and writing it back.
//! Also here: where a play/pause button goes (the other device while one plays, else the Engine), and what's known
//! about a track for a lyrics search.

use crate::catalogue::{MAX_DESCRIBED, trim_any};
use singularity_proto::{Command, Item, PlayStatus, PlaybackState};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub struct NowPlaying {
    /// what Clients see: the other device's playback while one plays, else the Engine's
    shown: PlaybackState,
    engine: PlaybackState,
    remote: Option<PlaybackState>,
    /// track names the Catalogue filled in, for queue entries an Engine sent bare; and what's being asked for
    described: HashMap<String, Item>,
    describing: HashSet<String>,
}

/// What an input changed: the playback to send to Clients, and tracks to ask the Catalogue to name.
#[derive(Debug, Default, PartialEq)]
pub struct Changed {
    pub show: Option<PlaybackState>,
    pub ask: Vec<String>,
}

/// Where a Client's command goes.
#[derive(Debug, PartialEq)]
pub enum Route {
    /// over Connect, to the other device that's playing
    Connect(Command),
    /// to this computer's Engine
    Engine(Command),
}

impl NowPlaying {
    /// What Clients see now.
    pub fn shown(&self) -> &PlaybackState {
        &self.shown
    }

    /// Is another device the one playing (and followed)?
    pub fn elsewhere(&self) -> bool {
        self.remote.is_some()
    }

    /// The Engine's state. While another device plays it's kept, not shown.
    pub fn engine(&mut self, mut state: PlaybackState) -> Changed {
        let ask = self.fill(&mut state);
        self.engine = state.clone();
        if self.remote.is_some() {
            return Changed { show: None, ask };
        }
        self.shown = state.clone();
        Changed { show: Some(state), ask }
    }

    /// Another device's state, or `None` when no other device plays (any more).
    pub fn remote(&mut self, state: Option<PlaybackState>) -> Changed {
        let Some(mut state) = state else { return self.gone() };
        let ask = self.fill(&mut state);
        self.remote = Some(state.clone());
        self.shown = state.clone();
        Changed { show: Some(state), ask }
    }

    /// No other device is ours to follow any more (it stopped, or we logged out): the bar goes back to this
    /// computer's Engine. Nothing to show when none was followed.
    pub fn gone(&mut self) -> Changed {
        if self.remote.take().is_none() {
            return Changed::default();
        }
        // (named with what came meanwhile: the Engine's rows don't wait for its next update)
        let mut engine = std::mem::take(&mut self.engine);
        let ask = self.fill(&mut engine);
        self.engine = engine;
        self.shown = self.engine.clone();
        Changed { show: Some(self.shown.clone()), ask }
    }

    /// Names the Catalogue found: shown at once.
    pub fn described(&mut self, items: Vec<Item>) -> Changed {
        for i in items {
            self.describing.remove(&i.uri);
            trim_any(&mut self.described, MAX_DESCRIBED);
            self.described.insert(i.uri.clone(), i);
        }
        // what's on screen, and the Engine's state kept behind another device's
        let (mut shown, mut engine) = (std::mem::take(&mut self.shown), std::mem::take(&mut self.engine));
        let mut ask = self.fill(&mut shown);
        ask.extend(self.fill(&mut engine));
        (self.shown, self.engine) = (shown, engine);
        if let Some(r) = self.remote.as_mut() {
            *r = self.shown.clone();
        }
        Changed { show: Some(self.shown.clone()), ask }
    }

    /// Names that couldn't be found: a later state asks again (A8; the Catalogue keeps them from being asked too often).
    pub fn describe_failed(&mut self, uris: Vec<String>) -> Changed {
        for uri in uris {
            self.describing.remove(&uri);
        }
        Changed::default()
    }

    /// A change the Clients showed early didn't happen: the last real state puts it back.
    pub fn resync(&self) -> Changed {
        Changed { show: Some(self.shown.clone()), ask: vec![] }
    }

    /// Where a Client's command goes: a play/pause button drives the other device while one plays.
    pub fn route(&self, cmd: Command) -> Route {
        match &self.remote {
            Some(r) if is_transport(&cmd) => Route::Connect(match cmd {
                // buffering is playing on its way: a "play" then would be a no-op
                Command::TogglePlay if matches!(r.status, PlayStatus::Playing | PlayStatus::Buffering) => Command::Pause,
                Command::TogglePlay => Command::Play { uri: None },
                other => other,
            }),
            _ => Route::Engine(cmd),
        }
    }

    /// What's known about a track on screen, to help a lyrics search find it: a name and an artist, else nothing (the
    /// Catalogue looks the track up; LRCLIB searched with no artist finds the wrong song or none).
    pub fn lyrics_hint(&self, uri: &str) -> Option<Item> {
        let s = &self.shown;
        s.item.iter().chain(s.queue.iter().map(|q| &q.item)).find(|i| i.uri == uri && !i.name.is_empty() && !i.artists.is_empty()).cloned()
    }

    /// Fill bare entries from the names known; returns the tracks still unknown and not yet asked for.
    fn fill(&mut self, state: &mut PlaybackState) -> Vec<String> {
        let mut missing = vec![];
        let bare = |i: &Item| (i.name.is_empty() || i.artists.is_empty()) && i.uri.starts_with("spotify:track:");
        for item in state.item.iter_mut().chain(state.queue.iter_mut().map(|q| &mut q.item)).filter(|i| bare(i)) {
            match self.described.get(&item.uri) {
                Some(i) => *item = i.clone(),
                // (A8/S8: the "being asked" set is bounded like the Catalogue's caches; past that, rows stay bare)
                None if self.describing.len() < MAX_DESCRIBED && self.describing.insert(item.uri.clone()) => missing.push(item.uri.clone()),
                None => {}
            }
        }
        missing
    }
}

/// Commands that drive whichever device is playing.
fn is_transport(c: &Command) -> bool {
    matches!(
        c,
        Command::Play { .. } | Command::Pause | Command::TogglePlay | Command::Next | Command::Previous | Command::Seek { .. } | Command::SetVolume { .. } | Command::SetShuffle { .. } | Command::SetRepeat { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularity_proto::QueueEntry;

    fn bare(uri: &str) -> Item {
        Item { uri: uri.into(), ..Default::default() }
    }

    fn named(uri: &str, name: &str) -> Item {
        Item { uri: uri.into(), name: name.into(), artists: vec!["Nova".into()], ..Default::default() }
    }

    fn playing(item: Item, status: PlayStatus) -> PlaybackState {
        PlaybackState { item: Some(item), status, ..Default::default() }
    }

    #[test]
    fn a_bare_track_is_asked_for_once_and_named_when_its_name_comes() {
        let mut now = NowPlaying::default();
        let t = "spotify:track:a";
        let first = now.engine(playing(bare(t), PlayStatus::Playing));
        assert_eq!(first.ask, [t]);
        assert_eq!(now.engine(playing(bare(t), PlayStatus::Playing)).ask, Vec::<String>::new(), "already asked");
        let shown = now.described(vec![named(t, "Song")]).show.unwrap();
        assert_eq!(shown.item.unwrap().name, "Song");
    }

    #[test]
    fn a_track_that_could_not_be_named_is_asked_for_again_later() {
        let mut now = NowPlaying::default();
        let t = "spotify:track:a";
        now.engine(playing(bare(t), PlayStatus::Playing));
        now.describe_failed(vec![t.into()]);
        assert_eq!(now.engine(playing(bare(t), PlayStatus::Playing)).ask, [t]);
    }

    #[test]
    fn the_engine_is_not_shown_while_another_device_plays() {
        let mut now = NowPlaying::default();
        let phone = playing(named("spotify:track:p", "Phone song"), PlayStatus::Playing);
        assert_eq!(now.remote(Some(phone.clone())).show, Some(phone.clone()));
        assert_eq!(now.engine(playing(named("spotify:track:e", "Ours"), PlayStatus::Paused)).show, None);
        assert_eq!(*now.shown(), phone);
        assert!(now.elsewhere());
    }

    #[test]
    fn when_the_other_device_goes_the_bar_shows_the_engine_again() {
        let mut now = NowPlaying::default();
        let ours = playing(named("spotify:track:e", "Ours"), PlayStatus::Paused);
        now.engine(ours.clone());
        now.remote(Some(playing(named("spotify:track:p", "Phone song"), PlayStatus::Playing)));
        assert_eq!(now.remote(None).show, Some(ours.clone()));
        assert_eq!(now.gone(), Changed::default(), "nothing to let go of twice");
        assert!(!now.elsewhere());
    }

    /// Q4: names that came while another device played used to reach only what was on screen, so going back to this
    /// computer showed the Engine's rows bare until its next update.
    #[test]
    fn names_that_came_while_another_device_played_are_kept_when_playback_comes_back() {
        let mut now = NowPlaying::default();
        let mine = "spotify:track:mine";
        let theirs = "spotify:track:theirs";
        let mut ours = playing(bare(mine), PlayStatus::Paused);
        ours.queue.push(QueueEntry { item: bare(theirs), source: "queue".into(), uid: "u".into() });
        now.engine(ours);
        now.remote(Some(playing(bare(theirs), PlayStatus::Playing)));
        now.described(vec![named(mine, "Mine"), named(theirs, "Theirs")]);
        let back = now.gone().show.unwrap();
        assert_eq!(back.item.unwrap().name, "Mine");
        assert_eq!(back.queue[0].item.name, "Theirs");
    }

    /// Review: a track marked as being asked about must be asked about, whichever input found it bare.
    #[test]
    fn a_bare_track_found_on_coming_back_is_asked_for() {
        let mut now = NowPlaying::default();
        let t = "spotify:track:mine";
        now.engine(playing(bare(t), PlayStatus::Paused));
        now.remote(Some(playing(named("spotify:track:p", "P"), PlayStatus::Playing)));
        now.describe_failed(vec![t.into()]);
        assert_eq!(now.gone().ask, [t], "asked again once the bar is back on it");
    }

    #[test]
    fn play_and_pause_drive_the_device_that_plays() {
        let mut now = NowPlaying::default();
        assert_eq!(now.route(Command::TogglePlay), Route::Engine(Command::TogglePlay));
        now.remote(Some(playing(named("spotify:track:p", "P"), PlayStatus::Buffering)));
        assert_eq!(now.route(Command::TogglePlay), Route::Connect(Command::Pause), "buffering is playing on its way");
        assert_eq!(now.route(Command::Next), Route::Connect(Command::Next));
        now.remote(Some(playing(named("spotify:track:p", "P"), PlayStatus::Paused)));
        assert_eq!(now.route(Command::TogglePlay), Route::Connect(Command::Play { uri: None }));
        assert_eq!(now.route(Command::GetState), Route::Engine(Command::GetState), "only the transport goes over Connect");
        now.remote(None);
        assert_eq!(now.route(Command::Next), Route::Engine(Command::Next));
    }

    #[test]
    fn a_lyrics_hint_needs_a_name_and_an_artist() {
        let mut now = NowPlaying::default();
        let mut half = named("spotify:track:a", "Song");
        half.artists.clear();
        let mut s = playing(named("spotify:track:b", "Other"), PlayStatus::Playing);
        s.queue.push(QueueEntry { item: half, source: "queue".into(), uid: "u".into() });
        // (the Catalogue hasn't named it yet)
        now.engine(s);
        assert_eq!(now.lyrics_hint("spotify:track:b").map(|i| i.name), Some("Other".into()));
        assert_eq!(now.lyrics_hint("spotify:track:a"), None, "no artist: the Catalogue looks it up instead");
        assert_eq!(now.lyrics_hint("spotify:track:none"), None);
    }
}
