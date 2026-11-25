//! OS media integration's macOS adapter (#68), MPRIS's counterpart: the Now Playing entry (Control Centre, the menu
//! bar), media keys and other remote commands, and track-change notifications.
//!
//! - Apple delivers remote commands on the main thread's run loop: [`main_thread`] is the main thread's part (the
//!   handover is `super::main_thread`); commands reach the Hub as a Client's would (`crate::transport`).
//! - macOS notifications go through `osascript` (a process without an app bundle can't post its own), without the
//!   cover. Which track changes are news is shared (`super::TrackNews`).

use super::News;

/// The name of its background task (the health view's row when it stops).
pub const NAME: &str = "Now Playing";

/// Now Playing is one entry for the whole system: the app that has it gets the media keys.
pub const ONE_ENTRY: bool = true;

/// No D-Bus: `SINGULARITY_DBUS=<address>` means nothing here (only `=off` does).
pub const ON_A_BUS: bool = false;

/// `osascript`'s arguments for a notification. The title and body are data from Spotify (a podcast's episode titles
/// come straight from its publisher's feed), so they go after `--`: a title like `-e…` would otherwise be read as
/// one more line of AppleScript (a `property` initialiser runs `do shell script` at compile time).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn osascript_args(n: &News) -> [&str; 9] {
    ["-e", "on run argv", "-e", "display notification (item 2 of argv) with title (item 1 of argv)", "-e", "end run", "--", &n.title, &n.body]
}

#[cfg(target_os = "macos")]
pub use mac::{main_thread, run};

#[cfg(target_os = "macos")]
mod mac {
    use crate::os_media::{News, Options, TrackNews};
    use crate::Hub;
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSDictionary, NSNumber, NSString};
    use objc2_media_player::*;
    use singularity_proto::{Command, Event, PlayStatus, PlaybackState};
    use std::ptr::NonNull;
    use std::sync::{Arc, OnceLock};

    /// A remote command and the transport command it stands for.
    type Simple = (Retained<MPRemoteCommand>, fn() -> Command);

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
        let state = hub.playback();
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
            let simple: [Simple; 6] = [
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

    /// A macOS notification through `osascript` (title and body passed as arguments after `--`, never as script text
    /// or options: `osascript_args`).
    async fn notify(n: News) {
        let r = crate::programs::tokio_command("/usr/bin/osascript")
            .args(super::osascript_args(&n))
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

    #[test]
    fn a_title_or_body_that_looks_like_an_option_stays_data_for_osascript() {
        let n = News { title: "-eproperty p : (do shell script \"touch /tmp/x\")".into(), body: "-e".into(), cover_url: None };
        let args = osascript_args(&n);
        let end = args.iter().position(|a| *a == "--").expect("options end before the data");
        assert_eq!(&args[end + 1..], [n.title.as_str(), n.body.as_str()]);
        assert_eq!(args.len(), end + 3, "nothing else after the data");
    }
}
