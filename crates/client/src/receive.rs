//! The one way Daemon events enter the Client (D82): `main` and tests alike call [`App::receive`], so what an event
//! does is tested where it happens. Nothing here touches the terminal or starts a program: what the main loop does
//! for an event (opening the pairing page) is left for it in a field, like the clipboard.

use crate::app::{App, Link};
use crate::conn::ConnEvent;
use singularity_proto::{DaemonMsg, EngineStatus, Event};

impl App {
    /// A Daemon event, or a change in the connection.
    pub fn receive(&mut self, ev: ConnEvent) {
        // demo mode (D90): what this Client may show, before anything reads it
        let ev = match (ev, self.demo.as_mut()) {
            (ConnEvent::Msg(DaemonMsg::Event(e)), Some(d)) => ConnEvent::Msg(DaemonMsg::Event(d.scrub(e))),
            (ev, _) => ev,
        };
        match ev {
            ConnEvent::Connecting => {}
            ConnEvent::Updating => self.say("Updating singularityd…"),
            ConnEvent::Disconnected => self.link = Link::Connecting,
            ConnEvent::Connected { daemon_version, engine, soloist_in_vm, demo, account } => {
                // demo mode: the saved login's name is redacted from the start (errors come before the login is
                // Ready); a Daemon that didn't confirm demo mode would show the real stats, so none show
                if let Some(d) = self.demo.as_mut() {
                    d.redactor.learn(&account, "user");
                    d.confirmed = demo;
                    if !demo {
                        self.listening = None;
                        self.stats.clear();
                    }
                }
                self.link = Link::Connected { version: daemon_version };
                self.engine_kind = engine;
                self.soloist_in_vm = soloist_in_vm;
                self.on_connected();
            }
            ConnEvent::Incompatible(m) => {
                self.say(m.clone());
                self.link = Link::Incompatible(m);
            }
            ConnEvent::Msg(DaemonMsg::Event(Event::Engine(s))) => {
                self.setup.engine_was_ready |= matches!(s, EngineStatus::Ready { .. });
                self.engine = Some(s);
            }
            ConnEvent::Msg(DaemonMsg::Event(Event::Playback(p))) => self.playback_event(p),
            ConnEvent::Msg(DaemonMsg::Event(Event::Error { message })) => self.say(message),
            ConnEvent::Msg(DaemonMsg::Event(Event::Catalogue(c))) => {
                if let singularity_proto::CatalogueStatus::LoginPending { url, .. } = &c
                    && !matches!(self.catalogue, Some(singularity_proto::CatalogueStatus::LoginPending { .. }))
                {
                    self.login_hidden = false;
                    // the main loop opens the pairing page in the browser; not in demo mode, where it would show the
                    // account's name and picture on the recording (the link and code still show)
                    if self.demo.is_none() {
                        self.open_url = Some(url.clone());
                    }
                }
                self.catalogue = Some(c);
            }
            // demo mode: Home waits for the library, the Demo set it's built from
            ConnEvent::Msg(DaemonMsg::Event(Event::Home(_))) if self.demo.as_ref().is_some_and(|d| !d.knows_library()) => {}
            ConnEvent::Msg(DaemonMsg::Event(Event::Home(h))) => {
                // the selection stays on its Card (#23)
                if let Some(old) = &self.home {
                    self.home_view.refeed(old, &h);
                }
                self.home = Some(h);
            }
            ConnEvent::Msg(DaemonMsg::Event(Event::Library(l))) => {
                self.library = Some(l);
            }
            ConnEvent::Msg(DaemonMsg::Event(Event::PlaylistCreated { uri, name })) => {
                // demo mode: a playlist this Client made in this session stays in the library
                if self.asked_to_create(&name)
                    && let Some(d) = self.demo.as_mut()
                {
                    d.created(&uri);
                    if let Some(l) = d.library_again() {
                        self.library = Some(l);
                    }
                }
                self.playlist_created(&uri, &name)
            }
            ConnEvent::Msg(DaemonMsg::Event(Event::AddedToPlaylist { playlist, name, uris, outcome })) => self.added_to_playlist(&playlist, &name, uris, outcome),
            ConnEvent::Msg(DaemonMsg::Event(Event::PlaylistCoverSet { playlist, ok })) => self.cover_set(&playlist, ok),
            ConnEvent::Msg(DaemonMsg::Event(Event::MovedInPlaylist { playlist, uids, ok })) => self.moved_in_playlist(&playlist, &uids, ok),
            ConnEvent::Msg(DaemonMsg::Event(Event::RemovedFromPlaylist { playlist, uids, ok })) => self.removed_from_playlist(&playlist, &uids, ok),
            ConnEvent::Msg(DaemonMsg::Event(Event::PlaylistEdited { uri, name, description, ok })) => self.playlist_edited(&uri, &name, &description, ok),
            ConnEvent::Msg(DaemonMsg::Event(Event::Tracks(p))) => self.track_page(p),
            ConnEvent::Msg(DaemonMsg::Event(Event::Spectrum(f))) => {
                // the setup's audio check passes once real sound comes through
                if f.level > 20 {
                    self.setup.heard = true;
                }
                self.meters.push(f);
            }
            ConnEvent::Msg(DaemonMsg::Event(Event::Stream(s))) => self.stream = Some(s),
            ConnEvent::Msg(DaemonMsg::Event(Event::Delivery(d))) => self.delivery_event(d),
            ConnEvent::Msg(DaemonMsg::Event(Event::Health(h))) => self.health = Some(h),
            ConnEvent::Msg(DaemonMsg::Event(Event::Artist(p))) => self.artist_page(p),
            ConnEvent::Msg(DaemonMsg::Event(Event::EpisodeShow { episode, show })) => self.episode_show(episode, show),
            ConnEvent::Msg(DaemonMsg::Event(Event::Devices { devices })) => self.devices = devices,
            ConnEvent::Msg(DaemonMsg::Event(Event::Jam { jam })) => self.jam = jam,
            ConnEvent::Msg(DaemonMsg::Event(Event::Search(r))) => {
                self.search.receive(r);
            }
            ConnEvent::Msg(DaemonMsg::Event(Event::Lyrics(l))) => self.lyrics_event(l),
            ConnEvent::Msg(DaemonMsg::Event(Event::Notice { message })) => self.say(message),
            ConnEvent::Msg(DaemonMsg::Event(Event::Saved { entries })) => self.saved_event(entries),
            // demo mode with a Daemon that didn't confirm it: these would be the real log's
            ConnEvent::Msg(DaemonMsg::Event(Event::Listening(_) | Event::Stats(_))) if self.demo.as_ref().is_some_and(|d| !d.confirmed) => {}
            ConnEvent::Msg(DaemonMsg::Event(Event::Listening(s))) => {
                self.listening = Some(s);
                // a Play was logged (or old plays got their links): stats on screen ask again
                self.refresh_stats();
            }
            ConnEvent::Msg(DaemonMsg::Event(Event::Stats(s))) => {
                self.stats.insert(s.period, s);
            }
            ConnEvent::Msg(_) => {}
        }
        self.demo_follow_up();
    }

    /// Demo mode (Q155–Q157): Home and the player again when what decides them came (the library, a Demo playlist's
    /// songs, an artist's picture), and what Home still needs fetched.
    fn demo_follow_up(&mut self) {
        let Some(up) = self.demo.as_mut().map(|d| d.follow_up()) else { return };
        if let Some(h) = up.home {
            // the selection stays on its Card (#23)
            if let Some(old) = &self.home {
                self.home_view.refeed(old, &h);
            }
            self.home = Some(h);
        }
        if let Some(p) = up.playback {
            self.playback_event(p);
        }
        for c in up.fetch {
            self.send(c);
        }
    }
}

#[cfg(test)]
impl App {
    /// A Daemon event, as `receive` gets it (tests).
    pub fn hear(&mut self, e: Event) {
        self.receive(ConnEvent::Msg(DaemonMsg::Event(e)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularity_proto::{CatalogueStatus, Command, EngineKind};

    fn event(e: Event) -> ConnEvent {
        ConnEvent::Msg(DaemonMsg::Event(e))
    }

    #[test]
    fn connecting_and_losing_the_daemon_shows_on_the_link() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let mut app = App::new(Some(tx));
        app.receive(ConnEvent::Connected { daemon_version: "1.2.3".into(), engine: EngineKind::Librespot, soloist_in_vm: false, demo: false, account: String::new() });
        assert_eq!(app.link, Link::Connected { version: "1.2.3".into() });
        assert_eq!(app.engine_kind, EngineKind::Librespot);
        assert!(!app.soloist_in_vm);
        app.receive(ConnEvent::Connected { daemon_version: "1.2.3".into(), engine: EngineKind::Soloist, soloist_in_vm: true, demo: false, account: String::new() });
        assert!(app.soloist_in_vm, "the Daemon's word on the Soloist VM (Q94)");
        assert!(std::iter::from_fn(|| rx.try_recv().ok()).any(|c| c == Command::GetListening), "connected: asks for the listening log");
        app.receive(ConnEvent::Disconnected);
        assert_eq!(app.link, Link::Connecting);
    }

    /// The pairing page opens once per login, by the main loop: never from inside `receive`, so a test can't open a
    /// browser.
    #[test]
    fn a_pending_login_asks_the_main_loop_to_open_its_page_once() {
        let mut app = App::new(None);
        let pending = || event(Event::Catalogue(CatalogueStatus::LoginPending { url: "https://example.invalid/pair".into(), code: "ABCD".into() }));
        app.receive(pending());
        assert_eq!(app.open_url.take().as_deref(), Some("https://example.invalid/pair"));
        app.receive(pending());
        assert_eq!(app.open_url, None, "still the same login: not opened again");
        assert!(matches!(app.catalogue, Some(CatalogueStatus::LoginPending { .. })));
    }

    #[test]
    fn an_engine_that_was_ready_once_is_remembered_by_the_setup() {
        let mut app = App::new(None);
        app.receive(event(Event::Engine(EngineStatus::Ready { device_name: "d".into(), active: true, version: "v".into(), expires_in_days: None })));
        app.receive(event(Event::Engine(EngineStatus::Failed { message: "gone".into() })));
        assert!(app.setup.engine_was_ready);
        assert_eq!(app.engine, Some(EngineStatus::Failed { message: "gone".into() }));
    }
}
