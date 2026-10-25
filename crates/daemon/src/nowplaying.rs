//! The OS's media integration on macOS (#68), MPRIS's counterpart: the Now Playing entry (Control Centre, the menu
//! bar), media keys and other remote commands, and track-change notifications.
//!
//! - Only for an Engine that plays from the Daemon (librespot): the Spotify app publishes its own Now Playing entry,
//!   and a second one would fight it for the media keys.
//! - Apple delivers remote commands on the main thread's run loop, so on macOS the Daemon runs tokio on another
//!   thread and gives the main thread to [`main_thread`]; commands reach the Hub as a Client's would
//!   (`crate::transport`).
//! - Which track changes are news is shared with Linux ([`TrackNews`]); macOS notifications go through `osascript`
//!   (a process without an app bundle can't post its own), without the cover.

use singularity_proto::{PlayStatus, PlaybackState};

/// What the Daemon tells the OS.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// a notification on each new track (`notifications.track_change`, changed live)
    pub notify: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// Decides when a track change is worth a notification: once per new track, when it starts playing, and not for the
/// track that was already playing when the Daemon started.
#[derive(Default)]
pub struct TrackNews {
    last_uri: Option<String>,
}

/// A notification: the track's name, and "artists — album".
#[derive(Debug, Clone, PartialEq)]
pub struct News {
    pub title: String,
    pub body: String,
    pub cover_url: Option<String>,
}

impl TrackNews {
    /// The notification for `s`, if it's news (and notifications are `on`).
    pub fn news(&mut self, s: &PlaybackState, on: bool) -> Option<News> {
        let i = s.item.as_ref().filter(|_| s.status == PlayStatus::Playing)?;
        if self.last_uri.as_deref() == Some(i.uri.as_str()) {
            return None;
        }
        let first = self.last_uri.is_none();
        self.last_uri = Some(i.uri.clone());
        if !on || (first && s.position_at(singularity_proto::now_ms()) > 5000) {
            return None; // off, or the Daemon just started into a playing track: not news
        }
        let body = match &i.album {
            Some(a) if !a.is_empty() => format!("{} — {a}", i.artists.join(", ")),
            _ => i.artists.join(", "),
        };
        Some(News { title: i.name.clone(), body, cover_url: i.cover_url.clone() })
    }
}

#[cfg(target_os = "macos")]
pub use mac::{main_thread, run};

#[cfg(target_os = "macos")]
mod mac {
    use super::{News, Options, TrackNews};
    use crate::Hub;
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSDictionary, NSNumber, NSString};
    use objc2_media_player::*;
    use singularity_proto::{Command, Event, PlayStatus, PlaybackState};
    use std::ptr::NonNull;
    use std::sync::{Arc, OnceLock};

    /// The Hub, and the runtime to send commands from, once the Daemon serves.
    static HUB: OnceLock<(Arc<Hub>, tokio::runtime::Handle)> = OnceLock::new();
    /// Set by the Daemon: register the remote commands (from the main thread) and keep its run loop.
    static WANTED: OnceLock<std::sync::mpsc::SyncSender<bool>> = OnceLock::new();

    /// The main thread's part: wait for the Daemon to say whether it wants Now Playing, register the remote commands if
    /// so, and run the main run loop (where Apple delivers them) until the process exits. Never returns.
    pub fn main_thread() -> ! {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let _ = WANTED.set(tx);
        if !rx.recv().unwrap_or(false) {
            // nothing to serve: the main thread just waits for the process to end
            loop {
                std::thread::park();
            }
        }
        register();
        use objc2_core_foundation::{CFRunLoop, CFRunLoopTimer, kCFRunLoopDefaultMode};
        // a run loop with nothing in it returns at once: a timer that fires once a day keeps it serving (costs nothing)
        unsafe extern "C-unwind" fn tick(_: *mut CFRunLoopTimer, _: *mut std::ffi::c_void) {}
        // SAFETY: a plain repeating timer with no context, added to this (the main) thread's run loop
        unsafe {
            let timer = CFRunLoopTimer::new(None, 0.0, 86_400.0, 0, 0, Some(tick), std::ptr::null_mut());
            if let Some(main) = CFRunLoop::main() {
                main.add_timer(timer.as_deref(), kCFRunLoopDefaultMode);
            }
        }
        loop {
            CFRunLoop::run();
        }
    }

    /// Serve Now Playing and notifications until the Daemon stops.
    pub async fn run(hub: Arc<Hub>, opts: Options) {
        let _ = HUB.set((hub.clone(), tokio::runtime::Handle::current()));
        if let Some(tx) = WANTED.get() {
            let _ = tx.send(true);
        }
        let mut events = hub.events.subscribe();
        let mut news = TrackNews::default();
        let state = hub.playback.lock().await.clone();
        show(&state);
        news.news(&state, false);
        tracing::info!("Now Playing: on");
        loop {
            match events.recv().await {
                Ok(Event::Playback(s)) => {
                    show(&s);
                    if let Some(n) = news.news(&s, opts.notify.load(std::sync::atomic::Ordering::Relaxed)) {
                        notify(n).await;
                    }
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    }

    /// The Daemon runs without Now Playing (the Spotify app as Engine, tests): let the main thread idle.
    pub fn not_wanted() {
        if let Some(tx) = WANTED.get() {
            let _ = tx.send(false);
        }
    }

    fn any<T: objc2::Message>(r: Retained<T>) -> Retained<AnyObject> {
        // SAFETY: every Objective-C object is an AnyObject
        unsafe { Retained::cast_unchecked(r) }
    }

    /// Put `s` in the Now Playing entry (the system interpolates the position from the rate).
    fn show(s: &PlaybackState) {
        // SAFETY: MPNowPlayingInfoCenter may be used from any thread; the keys are Apple's own constants
        unsafe {
            let center = MPNowPlayingInfoCenter::defaultCenter();
            let Some(i) = s.item.as_ref() else {
                center.setNowPlayingInfo(None);
                center.setPlaybackState(MPNowPlayingPlaybackState::Stopped);
                return;
            };
            let pos = s.position_at(singularity_proto::now_ms()) as f64 / 1000.0;
            let playing = s.status == PlayStatus::Playing;
            let keys: [&NSString; 6] = [MPMediaItemPropertyTitle, MPMediaItemPropertyArtist, MPMediaItemPropertyAlbumTitle, MPMediaItemPropertyPlaybackDuration, MPNowPlayingInfoPropertyElapsedPlaybackTime, MPNowPlayingInfoPropertyPlaybackRate];
            let values = [
                any(NSString::from_str(&i.name)),
                any(NSString::from_str(&i.artists.join(", "))),
                any(NSString::from_str(i.album.as_deref().unwrap_or(""))),
                any(NSNumber::numberWithDouble(i.duration_ms as f64 / 1000.0)),
                any(NSNumber::numberWithDouble(pos)),
                any(NSNumber::numberWithDouble(if playing { 1.0 } else { 0.0 })),
            ];
            let info = NSDictionary::from_retained_objects(&keys, &values);
            center.setNowPlayingInfo(Some(&info));
            center.setPlaybackState(match s.status {
                PlayStatus::Playing | PlayStatus::Buffering => MPNowPlayingPlaybackState::Playing,
                PlayStatus::Paused => MPNowPlayingPlaybackState::Paused,
                _ => MPNowPlayingPlaybackState::Stopped,
            });
        }
    }

    /// Hand `cmd` to the Hub, as a Client's transport command.
    fn send(cmd: Command) -> MPRemoteCommandHandlerStatus {
        let Some((hub, rt)) = HUB.get() else { return MPRemoteCommandHandlerStatus::CommandFailed };
        let hub = hub.clone();
        rt.spawn(async move { crate::transport(&hub, cmd).await });
        MPRemoteCommandHandlerStatus::Success
    }

    /// Media keys, Control Centre, headphones, the Touch Bar: each remote command → a transport command.
    fn register() {
        // SAFETY: called on the main thread before its run loop starts; the handlers live for the process
        unsafe {
            let c = MPRemoteCommandCenter::sharedCommandCenter();
            let simple: [(Retained<MPRemoteCommand>, fn() -> Command); 6] = [
                (c.playCommand(), || Command::Play { uri: None }),
                (c.pauseCommand(), || Command::Pause),
                (c.togglePlayPauseCommand(), || Command::TogglePlay),
                (c.stopCommand(), || Command::Pause),
                (c.nextTrackCommand(), || Command::Next),
                (c.previousTrackCommand(), || Command::Previous),
            ];
            for (command, make) in simple {
                command.setEnabled(true);
                let block = RcBlock::new(move |_: NonNull<MPRemoteCommandEvent>| send(make()));
                std::mem::forget(command.addTargetWithHandler(&block));
            }
            // dragging the position in Control Centre
            let seek = c.changePlaybackPositionCommand();
            seek.setEnabled(true);
            let block = RcBlock::new(|e: NonNull<MPRemoteCommandEvent>| {
                let e = e.cast::<MPChangePlaybackPositionCommandEvent>();
                let secs = e.as_ref().positionTime();
                send(Command::Seek { position_ms: (secs.max(0.0) * 1000.0) as u64 })
            });
            std::mem::forget(seek.addTargetWithHandler(&block));
        }
    }

    /// A macOS notification through `osascript` (title and body passed as arguments, never as script text).
    async fn notify(n: News) {
        let r = crate::programs::tokio_command("/usr/bin/osascript")
            .args(["-e", "on run argv", "-e", "display notification (item 2 of argv) with title (item 1 of argv)", "-e", "end run", &n.title, &n.body])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
        if let Err(e) = r {
            tracing::debug!("notification: {e}");
        }
    }
}

#[cfg(target_os = "macos")]
pub use mac::not_wanted;

#[cfg(test)]
mod tests {
    use super::*;
    use singularity_proto::Item;

    fn playing(uri: &str, pos: u64) -> PlaybackState {
        let now = singularity_proto::now_ms();
        PlaybackState { status: PlayStatus::Playing, item: Some(Item { uri: uri.into(), name: "Song".into(), artists: vec!["A".into(), "B".into()], album: Some("LP".into()), ..Default::default() }), position_ms: pos, timestamp_ms: now, speed: 1.0, ..Default::default() }
    }

    #[test]
    fn a_new_track_is_news_once_and_the_one_playing_at_start_is_not() {
        let mut n = TrackNews::default();
        assert_eq!(n.news(&playing("t:1", 60_000), true), None, "already playing when the Daemon started");
        let first = n.news(&playing("t:2", 0), true).unwrap();
        assert_eq!((first.title.as_str(), first.body.as_str()), ("Song", "A, B — LP"));
        assert_eq!(n.news(&playing("t:2", 1000), true), None, "the same track again");
        assert_eq!(n.news(&playing("t:3", 0), false), None, "notifications off");
        assert_eq!(n.news(&playing("t:3", 0), true), None, "seen while off: not news later");
        let mut paused = playing("t:4", 0);
        paused.status = PlayStatus::Paused;
        assert_eq!(n.news(&paused, true), None, "only once it plays");
        assert!(n.news(&playing("t:4", 0), true).is_some());
    }
}
