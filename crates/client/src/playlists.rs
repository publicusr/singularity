//! Editing your own playlists (batch 3, #51–#56). Create and delete (#51): a new playlist is named in the naming
//! popup and opened once Spotify answers; a delete asks twice, leaves the sidebar at once and comes back with the
//! Daemon's fresh library if Spotify refused.

use crate::app::{App, MenuAct, Naming, Page, Target};
use singularity_proto::{Command, LibraryItem};
use std::time::{Duration, Instant};

/// A second "Delete playlist" within this long deletes it.
const CONFIRM: Duration = Duration::from_secs(10);

#[derive(Default)]
pub struct Playlists {
    /// the name of the playlist we asked for, opened when it arrives
    creating: Option<String>,
    /// "Delete playlist" was chosen once for this URI
    confirm: Option<(String, Instant)>,
}

impl App {
    /// The library entry for `uri`, if it's a playlist of yours.
    pub fn own_playlist(&self, uri: &str) -> Option<&LibraryItem> {
        self.library.as_ref()?.items.iter().find(|i| i.uri == uri && i.own)
    }

    /// Menu entries for a playlist: Delete on your own.
    pub fn playlist_entries(&self, uri: &str) -> Vec<(String, MenuAct)> {
        match self.own_playlist(uri) {
            Some(_) => vec![("Delete playlist…".into(), MenuAct::DeletePlaylist)],
            None => vec![],
        }
    }

    pub fn ask_new_playlist(&mut self) {
        self.naming = Some(Naming::playlist());
    }

    /// Enter in the naming popup for a new playlist.
    pub fn create_playlist(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return self.say("A playlist needs a name");
        }
        self.playlists.creating = Some(name.to_string());
        self.send(Command::CreatePlaylist { name: name.into() });
        self.say(format!("Creating {name}…"));
    }

    /// `Event::PlaylistCreated`: the one we asked for opens (another Client's just shows in the library).
    pub fn playlist_created(&mut self, uri: &str, name: &str) {
        if self.playlists.creating.as_deref() != Some(name) {
            return;
        }
        self.playlists.creating = None;
        if let Some(lib) = self.library.as_mut()
            && !lib.items.iter().any(|i| i.uri == uri)
        {
            let at = lib.items.iter().position(|i| i.kind != "liked").unwrap_or(lib.items.len());
            lib.items.insert(at, LibraryItem { uri: uri.into(), name: name.into(), subtitle: "Playlist".into(), kind: "playlist".into(), image_url: None, depth: 0, own: true });
        }
        self.open(uri, name, "Playlist", "playlist");
        self.say(format!("Created {name}"));
    }

    /// "Delete playlist": the first time asks, a second within 10 s deletes.
    pub fn delete_playlist(&mut self, uri: &str) {
        let Some(name) = self.own_playlist(uri).map(|i| i.name.clone()) else {
            return self.say("Only your own playlists can be deleted");
        };
        let confirmed = self.playlists.confirm.take().is_some_and(|(u, t)| u == uri && t.elapsed() < CONFIRM);
        if !confirmed {
            self.playlists.confirm = Some((uri.to_string(), Instant::now()));
            return self.say(format!("Delete “{name}”? Choose Delete again within 10 s"));
        }
        if let Some(lib) = self.library.as_mut() {
            lib.items.retain(|i| i.uri != uri);
        }
        self.sidebar.sel = self.sidebar.sel.min(crate::sidebar::len(self).saturating_sub(1));
        if matches!(self.current_page(), Page::List { uri: open, .. } if open == uri) {
            self.navigate(Page::Home);
        }
        self.send(Command::DeletePlaylist { uri: uri.into() });
        self.say(format!("Deleted {name}"));
    }

    /// The palette's "Delete playlist": the open one, else the one selected in the sidebar.
    pub fn delete_this_playlist(&mut self) {
        let open = match self.current_page() {
            Page::List { uri, .. } => Some(uri),
            _ => None,
        };
        let side = self.sidebar.sel.checked_sub(crate::sidebar::FIXED).and_then(|i| self.library.as_ref()?.items.get(i)).map(|i| i.uri.clone());
        match open.filter(|u| self.own_playlist(u).is_some()).or(side.filter(|u| self.own_playlist(u).is_some())) {
            Some(uri) => self.delete_playlist(&uri),
            None => self.say("Open or select one of your playlists to delete it"),
        }
    }

    /// Right-click (or the Menu key) on a sidebar row.
    pub fn open_side_menu(&mut self, row: usize, at: Option<(u16, u16)>) {
        self.sidebar.sel = row;
        let Some(it) = row.checked_sub(crate::sidebar::FIXED).and_then(|i| self.library.as_ref()?.items.get(i)).cloned() else { return };
        let mut items = vec![];
        if it.kind != "folder" {
            items.push(("Play".to_string(), MenuAct::Play));
        }
        items.push(("New playlist…".into(), MenuAct::NewPlaylist));
        items.extend(self.playlist_entries(&it.uri));
        if it.kind != "folder" {
            items.push(("Copy link".into(), MenuAct::CopyLink));
        }
        let at = at.or_else(|| self.hit_rect(&crate::app::Hit::Side(row)).map(|r| (r.x + 2, r.y + 1))).unwrap_or((0, 0));
        self.menu = Some(crate::app::Menu { at, target: Target::Library { uri: it.uri, name: it.name }, items, selected: 0 });
    }
}

#[cfg(test)]
mod tests {
    use crate::app::{Action, App, MenuAct};
    use singularity_proto::{Command, Library, LibraryItem};

    fn item(uri: &str, name: &str, own: bool) -> LibraryItem {
        LibraryItem { uri: uri.into(), name: name.into(), subtitle: String::new(), kind: "playlist".into(), image_url: None, depth: 0, own }
    }

    fn app() -> (App, tokio::sync::mpsc::Receiver<Command>) {
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let mut app = App::new(Some(tx));
        app.link = crate::app::Link::Connected { version: "test".into() };
        app.library = Some(Library { items: vec![LibraryItem { kind: "liked".into(), ..item("spotify:collection", "Liked Songs", false) }, item("spotify:playlist:mine", "Mine", true), item("spotify:playlist:theirs", "Theirs", false)] });
        (app, rx)
    }

    fn sent(rx: &mut tokio::sync::mpsc::Receiver<Command>) -> Vec<Command> {
        std::iter::from_fn(|| rx.try_recv().ok()).filter(|c| !matches!(c, Command::GetTracks { .. } | Command::CheckSaved { .. })).collect()
    }

    fn menu_labels(app: &App) -> Vec<String> {
        app.menu.as_ref().unwrap().items.iter().map(|(l, _)| l.clone()).collect()
    }

    #[test]
    fn a_new_playlist_is_named_created_and_opened() {
        let (mut app, mut rx) = app();
        app.run(Action::NewPlaylist);
        assert!(app.naming.is_some());
        app.paste("   ");
        app.key(ratatui::crossterm::event::KeyEvent::from(ratatui::crossterm::event::KeyCode::Enter));
        assert!(sent(&mut rx).is_empty(), "no empty names");
        app.run(Action::NewPlaylist);
        for c in "Road trip".chars() {
            app.key(ratatui::crossterm::event::KeyEvent::from(ratatui::crossterm::event::KeyCode::Char(c)));
        }
        app.key(ratatui::crossterm::event::KeyEvent::from(ratatui::crossterm::event::KeyCode::Enter));
        assert_eq!(sent(&mut rx), vec![Command::CreatePlaylist { name: "Road trip".into() }]);
        // another Client's playlist doesn't open here
        app.playlist_created("spotify:playlist:other", "Something else");
        assert_eq!(app.current_page(), crate::app::Page::Home);
        app.playlist_created("spotify:playlist:new", "Road trip");
        assert!(matches!(app.current_page(), crate::app::Page::List { uri, .. } if uri == "spotify:playlist:new"));
        let lib = &app.library.as_ref().unwrap().items;
        assert_eq!((lib[1].uri.as_str(), lib[1].own), ("spotify:playlist:new", true), "at the top, under Liked Songs");
    }

    #[test]
    fn only_your_own_playlists_can_be_deleted_and_it_asks_twice() {
        let (mut app, mut rx) = app();
        // the sidebar menu: Delete only on your own
        app.open_side_menu(crate::sidebar::FIXED + 2, None);
        assert!(!menu_labels(&app).iter().any(|l| l.starts_with("Delete")), "{:?}", menu_labels(&app));
        app.menu = None;
        app.open_side_menu(crate::sidebar::FIXED + 1, None);
        let labels = menu_labels(&app);
        assert!(labels.contains(&"New playlist…".to_string()) && labels.contains(&"Delete playlist…".to_string()), "{labels:?}");
        let del = app.menu.as_ref().unwrap().items.iter().position(|(_, a)| *a == MenuAct::DeletePlaylist).unwrap();
        app.menu_run(del);
        assert!(sent(&mut rx).is_empty(), "the first time only asks");
        assert!(app.toast.as_ref().unwrap().0.contains("again"));
        app.open("spotify:playlist:mine", "Mine", "", "playlist");
        app.open_side_menu(crate::sidebar::FIXED + 1, None);
        app.menu_run(del);
        assert_eq!(sent(&mut rx), vec![Command::DeletePlaylist { uri: "spotify:playlist:mine".into() }]);
        assert!(app.own_playlist("spotify:playlist:mine").is_none(), "gone from the sidebar at once");
        assert_eq!(app.current_page(), crate::app::Page::Home, "its page closes");
        // the palette can't delete someone else's
        app.open("spotify:playlist:theirs", "Theirs", "", "playlist");
        app.run(Action::DeletePlaylist);
        app.run(Action::DeletePlaylist);
        assert!(sent(&mut rx).is_empty());
    }
}
