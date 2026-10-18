//! Editing your own playlists (batch 3, #51–#56). Create and delete (#51): a new playlist is named in the naming
//! popup and opened once Spotify answers; a delete asks twice, leaves the sidebar at once and comes back with the
//! Daemon's fresh library if Spotify refused. Edit details (#52): a name and a description, shown everywhere at once
//! and put back if Spotify refuses.

use crate::app::{App, MenuAct, Naming, Page, Target};
use crate::theme::Theme;
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use singularity_proto::{Command, LibraryItem};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// A second "Delete playlist" within this long deletes it.
const CONFIRM: Duration = Duration::from_secs(10);
/// Spotify's limits (the web player's form).
const NAME_MAX: usize = 100;
const DESCRIPTION_MAX: usize = 300;
/// Lines of description the form shows (the end of a long one stays in view).
const DESCRIPTION_LINES: usize = 3;

/// The "Edit details" form.
#[derive(Clone, Debug, PartialEq)]
pub struct Details {
    pub uri: String,
    pub name: String,
    pub description: String,
    /// typing into the description (else the name)
    pub on_description: bool,
    /// the description has arrived (until then saving could clear it by mistake)
    pub known: bool,
}

#[derive(Default)]
pub struct Playlists {
    /// the name of the playlist we asked for, opened when it arrives
    creating: Option<String>,
    /// "Delete playlist" was chosen once for this URI
    confirm: Option<(String, Instant)>,
    pub details: Option<Details>,
    /// details shown before Spotify answered: what to put back if it refuses
    was: HashMap<String, (String, String)>,
}

impl App {
    /// The library entry for `uri`, if it's a playlist of yours.
    pub fn own_playlist(&self, uri: &str) -> Option<&LibraryItem> {
        self.library.as_ref()?.items.iter().find(|i| i.uri == uri && i.own)
    }

    /// Menu entries for a playlist: Edit details and Delete on your own.
    pub fn playlist_entries(&self, uri: &str) -> Vec<(String, MenuAct)> {
        match self.own_playlist(uri) {
            Some(_) => vec![("Edit details…".into(), MenuAct::EditPlaylist), ("Delete playlist…".into(), MenuAct::DeletePlaylist)],
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

    /// What the palette acts on: the open playlist if it's yours, else the one selected in the sidebar.
    fn this_playlist(&self) -> Option<String> {
        let open = match self.current_page() {
            Page::List { uri, .. } => Some(uri),
            _ => None,
        };
        let side = self.sidebar.sel.checked_sub(crate::sidebar::FIXED).and_then(|i| self.library.as_ref()?.items.get(i)).map(|i| i.uri.clone());
        open.filter(|u| self.own_playlist(u).is_some()).or(side.filter(|u| self.own_playlist(u).is_some()))
    }

    pub fn delete_this_playlist(&mut self) {
        match self.this_playlist() {
            Some(uri) => self.delete_playlist(&uri),
            None => self.say("Open or select one of your playlists to delete it"),
        }
    }

    pub fn edit_this_playlist(&mut self) {
        match self.this_playlist() {
            Some(uri) => self.edit_details(&uri),
            None => self.say("Open or select one of your playlists to edit it"),
        }
    }

    /// The description is known once the playlist's first page has arrived (it carries the header).
    fn description_known(&self, uri: &str) -> bool {
        self.lists.contains_key(uri)
    }

    /// A track page arrived: the form waiting for this playlist's description gets it.
    pub fn details_arrived(&mut self, uri: &str) {
        if self.playlists.details.as_ref().is_some_and(|d| d.uri == uri && !d.known) {
            let description = self.description_of(uri);
            let d = self.playlists.details.as_mut().unwrap();
            d.description = format!("{description}{}", d.description);
            d.known = true;
        }
    }

    /// The playlist's description as last heard (its page's header), if its page has been open.
    fn description_of(&self, uri: &str) -> String {
        self.lists.get(uri).and_then(|l| l.header.as_ref()).map(|h| h.description.clone()).unwrap_or_default()
    }

    /// Open the "Edit details" form, filled in.
    pub fn edit_details(&mut self, uri: &str) {
        let Some(name) = self.own_playlist(uri).map(|i| i.name.clone()) else {
            return self.say("Only your own playlists can be edited");
        };
        // the description comes with the playlist's first page: fetch it if it hasn't been open
        let known = self.description_known(uri);
        if !known {
            self.send(Command::GetTracks { uri: uri.into() });
        }
        let description = self.description_of(uri);
        self.playlists.details = Some(Details { uri: uri.into(), name, description, on_description: false, known });
    }

    pub fn details_key(&mut self, k: KeyEvent) {
        let Some(d) = self.playlists.details.as_mut() else { return };
        let field = if d.on_description { &mut d.description } else { &mut d.name };
        let max = if d.on_description { DESCRIPTION_MAX } else { NAME_MAX };
        match k.code {
            KeyCode::Esc => self.playlists.details = None,
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down => d.on_description = !d.on_description,
            KeyCode::Enter => self.save_details(),
            KeyCode::Backspace if k.modifiers.contains(KeyModifiers::CONTROL) => {
                let keep = field.trim_end().rfind(' ').map_or(0, |i| i + 1);
                field.truncate(keep);
            }
            KeyCode::Backspace => {
                field.pop();
            }
            KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => field.clear(),
            KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) && field.chars().count() < max => field.push(c),
            _ => {}
        }
    }

    /// Pasted text goes into the field being typed (line breaks become spaces).
    pub fn details_paste(&mut self, s: &str) {
        let Some(d) = self.playlists.details.as_mut() else { return };
        let (field, max) = if d.on_description { (&mut d.description, DESCRIPTION_MAX) } else { (&mut d.name, NAME_MAX) };
        let room = max.saturating_sub(field.chars().count());
        field.extend(s.chars().map(|c| if c.is_control() { ' ' } else { c }).take(room));
    }

    /// Enter: show the new details at once and send them; nothing changed just closes.
    fn save_details(&mut self) {
        let Some(d) = self.playlists.details.clone() else { return };
        let (name, description) = (d.name.trim().to_string(), d.description.trim().to_string());
        if name.is_empty() {
            return self.say("A playlist needs a name");
        }
        if !d.known {
            return self.say("Still fetching the description…");
        }
        self.playlists.details = None;
        let Some(old) = self.own_playlist(&d.uri).map(|i| i.name.clone()) else { return };
        let old = (old, self.description_of(&d.uri));
        if old == (name.clone(), description.clone()) {
            return;
        }
        // the first one shown is what Spotify still has, if edits pile up
        self.playlists.was.entry(d.uri.clone()).or_insert(old);
        self.show_details(&d.uri, &name, &description);
        self.send(Command::EditPlaylist { uri: d.uri, name: name.clone(), description });
        self.say(format!("Saved {name}"));
    }

    /// A name and description everywhere they show: the sidebar, the page and its header, back/forward.
    fn show_details(&mut self, uri: &str, name: &str, description: &str) {
        if let Some(lib) = self.library.as_mut() {
            lib.items.iter_mut().filter(|i| i.uri == uri).for_each(|i| i.name = name.into());
        }
        if let Some(v) = self.view.as_mut().filter(|v| v.uri == uri) {
            v.title = name.into();
        }
        if let Some(v) = self.list_views.get_mut(uri) {
            v.title = name.into();
        }
        for p in self.back.iter_mut().chain(self.forward.iter_mut()) {
            if let Page::List { uri: u, title, .. } = p
                && u == uri
            {
                *title = name.into();
            }
        }
        if let Some(l) = self.lists.get_mut(uri) {
            l.header.get_or_insert_with(Default::default).description = description.into();
        }
    }

    /// `Event::PlaylistEdited`: settled, or put back (ours) / shown (another Client's).
    pub fn playlist_edited(&mut self, uri: &str, name: &str, description: &str, ok: bool) {
        let was = self.playlists.was.remove(uri);
        match (ok, was) {
            (true, _) => self.show_details(uri, name, description),
            (false, Some((n, d))) => {
                self.show_details(uri, &n, &d);
                self.say(format!("Spotify didn't take the new details; it's still {n}"));
            }
            (false, None) => {}
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

pub fn details_rect(area: Rect) -> Rect {
    crate::ui::centered(area, 64, 5 + DESCRIPTION_LINES as u16 + 1)
}

/// `s` in lines of at most `w` characters, broken at spaces where it can.
fn wrap(s: &str, w: usize) -> Vec<String> {
    let w = w.max(1);
    let (mut lines, mut cur) = (vec![], String::new());
    for word in s.split(' ') {
        let wl = word.chars().count();
        if !cur.is_empty() {
            if cur.chars().count() + 1 + wl <= w {
                cur.push(' ');
                cur.push_str(word);
                continue;
            }
            lines.push(std::mem::take(&mut cur));
        }
        let mut chars: Vec<char> = word.chars().collect();
        while chars.len() > w {
            lines.push(chars.drain(..w).collect());
        }
        cur = chars.into_iter().collect();
    }
    lines.push(cur);
    lines
}

/// The "Edit details" form: Name, Description (wrapped), and how to save.
pub fn draw_details(buf: &mut Buffer, area: Rect, app: &App, th: &Theme) {
    let Some(d) = &app.playlists.details else { return };
    let inner = crate::ui::popup(buf, details_rect(area), th, "Edit details");
    let (x, w) = (inner.x + 1, inner.width.saturating_sub(2));
    let label = |on: bool| th.s(if on { th.accent } else { th.dim });
    let cursor = |on: bool| if on { "▏" } else { "" };
    text(buf, x, inner.y, w, "Name", label(!d.on_description));
    let fw = w.saturating_sub(3) as usize;
    let shown: String = d.name.chars().skip(d.name.chars().count().saturating_sub(fw.saturating_sub(1))).collect();
    text(buf, x, inner.y + 1, w, &format!("› {shown}{}", cursor(!d.on_description)), th.s(th.fg).add_modifier(Modifier::BOLD));
    text(buf, x, inner.y + 2, w, if d.known { "Description" } else { "Description (fetching…)" }, label(d.on_description));
    let mut lines = wrap(&format!("{}{}", d.description, cursor(d.on_description)), fw);
    let skip = lines.len().saturating_sub(DESCRIPTION_LINES);
    lines.drain(..skip);
    for (i, l) in lines.iter().enumerate() {
        let lead = if i == 0 && skip == 0 { "› " } else { "  " };
        text(buf, x, inner.y + 3 + i as u16, w, &format!("{lead}{l}"), th.s(th.fg));
    }
    let hint = format!("Tab next field · Enter save · Esc cancel · {}/{DESCRIPTION_MAX}", d.description.chars().count());
    text(buf, x, inner.y + 3 + DESCRIPTION_LINES as u16, w, &hint, th.s(th.dim));
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

    fn key(app: &mut App, c: ratatui::crossterm::event::KeyCode) {
        app.key(ratatui::crossterm::event::KeyEvent::from(c));
    }

    fn typed(app: &mut App, s: &str) {
        s.chars().for_each(|c| key(app, ratatui::crossterm::event::KeyCode::Char(c)));
    }

    #[test]
    fn details_are_edited_shown_at_once_and_put_back_if_refused() {
        use ratatui::crossterm::event::KeyCode;
        use singularity_proto::{ListHeader, TrackPage};
        let (mut app, mut rx) = app();
        // others' playlists: nothing offered, nothing sent
        app.open_side_menu(crate::sidebar::FIXED + 2, None);
        assert!(!menu_labels(&app).contains(&"Edit details…".to_string()));
        app.menu = None;
        app.edit_details("spotify:playlist:theirs");
        assert!(app.playlists.details.is_none());
        app.open("spotify:playlist:mine", "Mine", "", "playlist");
        // until the first page brings the description, Enter can't clear it by mistake
        app.run(Action::EditPlaylist);
        assert!(!app.playlists.details.as_ref().unwrap().known);
        key(&mut app, KeyCode::Enter);
        assert!(sent(&mut rx).is_empty() && app.playlists.details.is_some());
        let header = ListHeader { subtitle: "Playlist · me".into(), description: "Old words".into(), ..Default::default() };
        app.track_page(TrackPage { uri: "spotify:playlist:mine".into(), offset: 0, total: 0, tracks: vec![], header: Some(header) });
        assert_eq!(app.playlists.details.as_ref().unwrap().description, "Old words");
        // an empty name is refused; the form stays
        app.playlists.details.as_mut().unwrap().name.clear();
        key(&mut app, KeyCode::Enter);
        assert!(sent(&mut rx).is_empty() && app.playlists.details.is_some());
        typed(&mut app, "Road trip");
        key(&mut app, KeyCode::Tab);
        app.playlists.details.as_mut().unwrap().description.clear();
        app.paste("Songs for\nthe car");
        key(&mut app, KeyCode::Enter);
        assert_eq!(sent(&mut rx), vec![Command::EditPlaylist { uri: "spotify:playlist:mine".into(), name: "Road trip".into(), description: "Songs for the car".into() }]);
        assert!(app.playlists.details.is_none());
        let shown = |app: &App| (app.own_playlist("spotify:playlist:mine").unwrap().name.clone(), app.current_page(), app.lists["spotify:playlist:mine"].header.as_ref().unwrap().description.clone());
        let (name, page, desc) = shown(&app);
        assert!(name == "Road trip" && desc == "Songs for the car" && matches!(page, crate::app::Page::List { title, .. } if title == "Road trip"), "shown at once");
        // Spotify refuses: all of it goes back
        app.playlist_edited("spotify:playlist:mine", "Road trip", "Songs for the car", false);
        let (name, page, desc) = shown(&app);
        assert!(name == "Mine" && desc == "Old words" && matches!(page, crate::app::Page::List { title, .. } if title == "Mine"), "{name} {desc}");
        // emptying the description clears it; nothing changed sends nothing
        app.edit_details("spotify:playlist:mine");
        key(&mut app, KeyCode::Down);
        app.playlists.details.as_mut().unwrap().description.clear();
        key(&mut app, KeyCode::Enter);
        assert_eq!(sent(&mut rx), vec![Command::EditPlaylist { uri: "spotify:playlist:mine".into(), name: "Mine".into(), description: String::new() }]);
        app.playlist_edited("spotify:playlist:mine", "Mine", "", true);
        app.edit_details("spotify:playlist:mine");
        key(&mut app, KeyCode::Enter);
        assert!(sent(&mut rx).is_empty());
    }

    #[test]
    fn a_description_wraps_at_spaces() {
        assert_eq!(super::wrap("songs for the car", 9), vec!["songs for", "the car"]);
        assert_eq!(super::wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(super::wrap("", 4), vec![""]);
    }
}
