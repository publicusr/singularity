//! Editing your own playlists (batch 3, #51–#56). Create and delete (#51): a new playlist is named in the naming
//! popup and opened once Spotify answers; a delete asks twice, leaves the sidebar at once and comes back with the
//! Daemon's fresh library if Spotify refused. Edit details (#52): a name and a description, shown everywhere at once
//! and put back if Spotify refuses. Add to playlist (#53): a picker over your own playlists, filtered as you type,
//! "New playlist" on top; a playlist that has the song already asks first. Remove (#54): one row, by its uid, gone at
//! once and put back where it was if Spotify refuses. Reorder (#55): drag a row (or Alt+↑↓) in the playlist's own
//! order; the preview follows the pointer, the drop sends one move anchored to a neighbour's uid.

use crate::app::{App, Hit, MenuAct, Naming, Page, Target};
use crate::theme::Theme;
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use singularity_proto::{AddOutcome, Command, LibraryItem};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// A second "Delete playlist" within this long deletes it.
const CONFIRM: Duration = Duration::from_secs(10);
/// Spotify's limits (the web player's form).
const NAME_MAX: usize = 100;
const DESCRIPTION_MAX: usize = 300;
/// Lines of description the form shows (the end of a long one stays in view).
const DESCRIPTION_LINES: usize = 3;

/// The "Add to playlist" picker: what's being added, and the filter typed so far.
#[derive(Clone, Debug, PartialEq)]
pub struct Adding {
    pub uris: Vec<String>,
    /// the song's name (the picker's title)
    pub label: String,
    pub filter: String,
    pub sel: usize,
}

/// A playlist row being dragged: from and to, as view positions (the view is in the playlist's own order).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RowDrag {
    pub from: usize,
    pub to: usize,
}

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
    pub adding: Option<Adding>,
    /// songs for the playlist being named in the naming popup ("New playlist" in the picker)
    with: Vec<String>,
    /// playlists (by name) this Client asked to add to: their answers toast here
    asked: Vec<String>,
    /// Spotify said this playlist has these already: picking it again within 10 s adds them anyway
    dup: Option<(String, Vec<String>, Instant)>,
    /// rows taken out before Spotify answered, by (playlist, uid): where they were
    removing: HashMap<(String, String), (usize, singularity_proto::TrackRow)>,
    /// the row pressed on (a drag from it reorders), and the drag under way
    pub press: Option<usize>,
    pub row_drag: Option<RowDrag>,
    /// rows moved before Spotify answered, by (playlist, uid): where they were, and how many moves are unanswered
    moving: HashMap<(String, String), (usize, usize)>,
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
        self.playlists.with.clear();
        self.naming = Some(Naming::playlist());
    }

    /// Enter in the naming popup for a new playlist.
    pub fn create_playlist(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return self.say("A playlist needs a name");
        }
        let uris = std::mem::take(&mut self.playlists.with);
        // made for a song: it stays where you are and says when the song's in it; else it opens
        if uris.is_empty() {
            self.playlists.creating = Some(name.to_string());
        } else {
            self.playlists.asked.push(name.to_string());
        }
        self.send(Command::CreatePlaylist { name: name.into(), uris });
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

impl App {
    /// "Add to playlist…": open the picker for these songs.
    pub fn ask_add(&mut self, uris: Vec<String>, label: &str) {
        if uris.iter().any(|u| !u.starts_with("spotify:track:") && !u.starts_with("spotify:episode:")) {
            return self.say("Only songs and episodes go into playlists");
        }
        self.playlists.adding = Some(Adding { uris, label: label.into(), filter: String::new(), sel: 0 });
    }

    /// The picker's rows: "New playlist" first, then your playlists whose names have the filter in them.
    pub fn adding_rows(&self) -> Vec<(Option<String>, String)> {
        let Some(a) = &self.playlists.adding else { return vec![] };
        let f = a.filter.trim().to_lowercase();
        let new = if f.is_empty() { "+ New playlist…".to_string() } else { format!("+ New playlist “{}”", a.filter.trim()) };
        let mut rows = vec![(None, new)];
        let own = self.library.iter().flat_map(|l| &l.items).filter(|i| i.own && i.kind == "playlist");
        rows.extend(own.filter(|i| i.name.to_lowercase().contains(&f)).map(|i| (Some(i.uri.clone()), i.name.clone())));
        rows
    }

    pub fn adding_key(&mut self, k: KeyEvent) {
        let Some(a) = self.playlists.adding.as_mut() else { return };
        match k.code {
            KeyCode::Esc => self.playlists.adding = None,
            KeyCode::Up => self.adding_move(-1),
            KeyCode::Down => self.adding_move(1),
            KeyCode::Enter => {
                let sel = a.sel;
                self.add_pick(sel);
            }
            KeyCode::Backspace => {
                a.filter.pop();
                a.sel = 0;
            }
            KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) && a.filter.chars().count() < NAME_MAX => {
                a.filter.push(c);
                a.sel = 0;
            }
            _ => {}
        }
    }

    pub fn adding_move(&mut self, d: i32) {
        let n = self.adding_rows().len();
        if let Some(a) = self.playlists.adding.as_mut() {
            a.sel = (a.sel as i64 + d as i64).clamp(0, n.saturating_sub(1) as i64) as usize;
        }
    }

    /// Picker row `i`: a new playlist (named by the filter, else in the naming popup) or one of yours.
    pub fn add_pick(&mut self, i: usize) {
        let Some((uri, name)) = self.adding_rows().get(i).cloned() else { return };
        let Some(a) = self.playlists.adding.take() else { return };
        let Some(uri) = uri else {
            self.playlists.with = a.uris;
            if a.filter.trim().is_empty() {
                self.naming = Some(Naming::playlist());
            } else {
                self.create_playlist(&a.filter);
            }
            return;
        };
        let anyway = self.playlists.dup.take().is_some_and(|(u, uris, t)| u == uri && uris == a.uris && t.elapsed() < CONFIRM);
        self.playlists.asked.push(name.clone());
        self.send(Command::AddToPlaylist { playlist: uri, name: name.clone(), uris: a.uris, anyway });
        self.say(format!("Adding to {name}…"));
    }

    /// `Event::AddedToPlaylist`: an open view of the playlist fetches its new end; the Client that asked is told.
    pub fn added_to_playlist(&mut self, playlist: &str, name: &str, uris: Vec<String>, outcome: AddOutcome) {
        let mine = self.playlists.asked.iter().position(|n| n == name).map(|i| self.playlists.asked.remove(i)).is_some();
        match outcome {
            AddOutcome::Added => {
                if self.view.as_ref().is_some_and(|v| v.uri == playlist) {
                    self.send(Command::GetTracks { uri: playlist.into() });
                } else {
                    self.lists.remove(playlist);
                }
                if mine {
                    self.say(format!("Added to {name}"));
                }
            }
            AddOutcome::AlreadyThere if mine => {
                self.playlists.dup = Some((playlist.into(), uris, Instant::now()));
                self.say(format!("Already in {name}. Add it there again within 10 s to have it twice"));
            }
            _ => {}
        }
    }
}

/// As tall as its rows, up to 18.
impl App {
    /// The open list is one of your playlists, so its rows can be removed.
    pub fn rows_removable(&self) -> bool {
        self.view.as_ref().is_some_and(|v| self.own_playlist(&v.uri).is_some())
    }

    /// Take the row at view position `vi` out of the open playlist.
    pub fn remove_row(&mut self, vi: usize) {
        let Some(playlist) = self.view.as_ref().map(|v| v.uri.clone()).filter(|u| self.own_playlist(u).is_some()) else {
            return self.say("Only your own playlists can lose songs");
        };
        let order = self.view_order();
        let Some(row) = order.get(vi).and_then(|&i| self.lists.get(&playlist)?.rows.get(i)).cloned() else { return };
        let Some(uid) = row.uid.clone() else {
            return self.say("Spotify didn't say which row this is; open the playlist again");
        };
        let Some(taken) = self.lists.get_mut(&playlist).and_then(|l| l.take_uid(&uid)) else { return };
        self.playlists.removing.insert((playlist.clone(), uid.clone()), taken);
        let len = self.view_order().len();
        if let Some(v) = self.view.as_mut() {
            v.sel = v.sel.min(len.saturating_sub(1));
        }
        self.send(Command::RemoveFromPlaylist { playlist, uids: vec![uid] });
        self.say(format!("Removed {}", row.name));
    }

    /// `Event::RemovedFromPlaylist`: settled; or put back (ours); another Client's removal leaves here too.
    pub fn removed_from_playlist(&mut self, playlist: &str, uids: &[String], ok: bool) {
        for uid in uids {
            match (self.playlists.removing.remove(&(playlist.to_string(), uid.clone())), ok) {
                (Some((at, row)), false) => {
                    self.say(format!("Spotify didn't remove {}; it's back", row.name));
                    if let Some(l) = self.lists.get_mut(playlist) {
                        l.put_back(at, row);
                    }
                }
                (None, true) => {
                    if let Some(l) = self.lists.get_mut(playlist) {
                        l.take_uid(uid);
                    }
                }
                _ => {}
            }
        }
    }
}

impl App {
    /// Why the open list's rows can't be moved, if they can't.
    fn unmovable(&self) -> Option<&'static str> {
        let v = self.view.as_ref()?;
        if self.own_playlist(&v.uri).is_none() {
            Some("Only your own playlists can be reordered")
        } else if v.sort.is_some() || !v.filter.is_empty() {
            Some("Clear the sort and the filter to reorder: moves follow the playlist's own order")
        } else {
            None
        }
    }

    /// The pointer moved with the button held after pressing a row: start or follow a drag.
    pub fn drag_row_to(&mut self, y: u16) {
        let Some(from) = self.playlists.press else { return };
        // the row under the pointer (rows are drawn in the preview's order), clamped to the visible ones
        let rows: Vec<(u16, usize)> = self.hits.iter().filter_map(|(r, h)| if let Hit::Track(k) = h { Some((r.y, *k)) } else { None }).collect();
        let to = match rows.iter().find(|(ry, _)| *ry == y) {
            Some((_, k)) => *k,
            None if rows.first().is_some_and(|(ry, _)| y < *ry) => rows.iter().map(|(_, k)| *k).min().unwrap_or(from),
            None => rows.iter().map(|(_, k)| *k).max().unwrap_or(from),
        };
        // a drag starts once the pointer reaches another row (a shaky click isn't one)
        if self.playlists.row_drag.is_none() {
            if to == from {
                return;
            }
            if let Some(why) = self.unmovable() {
                self.playlists.press = None;
                return self.say(why);
            }
            self.playlists.row_drag = Some(RowDrag { from, to: from });
        }
        if let Some(d) = self.playlists.row_drag.as_mut() {
            d.to = to;
        }
        if let Some(v) = self.view.as_mut() {
            v.sel = to;
        }
    }

    /// The button let go: a drag lands.
    pub fn drop_row(&mut self) {
        self.playlists.press = None;
        if let Some(d) = self.playlists.row_drag.take()
            && d.to != d.from
        {
            self.move_row(d.from, d.to);
        }
    }

    /// Alt+↑↓ on the selected row.
    pub fn nudge_row(&mut self, sel: usize, d: i32) {
        if let Some(why) = self.unmovable() {
            return self.say(why);
        }
        let len = self.view_order().len();
        let to = sel as i64 + d as i64;
        if (0..len as i64).contains(&to) {
            self.move_row(sel, to as usize);
        }
    }

    /// Move row `from` to `to` (positions in the playlist's own order): at once here, then one move on Spotify,
    /// anchored to the row it lands before (or after, at the end).
    fn move_row(&mut self, from: usize, to: usize) {
        let Some(playlist) = self.view.as_ref().map(|v| v.uri.clone()) else { return };
        let Some(list) = self.lists.get_mut(&playlist) else { return };
        let Some(uid) = list.rows.get(from).and_then(|r| r.uid.clone()) else {
            return self.say("Spotify didn't say which row this is; open the playlist again");
        };
        let row = list.rows.remove(from);
        list.rows.insert(to.min(list.rows.len()), row);
        let before = list.rows.get(to + 1).and_then(|r| r.uid.clone());
        let after = if before.is_none() { to.checked_sub(1).and_then(|i| list.rows.get(i)).and_then(|r| r.uid.clone()) } else { None };
        self.playlists.moving.entry((playlist.clone(), uid.clone())).or_insert((from, 0)).1 += 1;
        if let Some(v) = self.view.as_mut() {
            v.sel = to;
        }
        self.send(Command::MoveInPlaylist { playlist, uids: vec![uid], before, after });
    }

    /// `Event::MovedInPlaylist`: settled; or moved back (ours); another Client's move: fetch the new order.
    pub fn moved_in_playlist(&mut self, playlist: &str, uids: &[String], ok: bool) {
        for uid in uids {
            let key = (playlist.to_string(), uid.clone());
            // the last answer for a row settles it (a refusal puts it back where the first move took it from)
            let mine = match self.playlists.moving.get_mut(&key) {
                Some((_, n)) if *n > 1 && ok => {
                    *n -= 1;
                    continue;
                }
                Some(_) => self.playlists.moving.remove(&key).map(|(was, _)| was),
                None => None,
            };
            match (mine, ok) {
                (Some(was), false) => {
                    if let Some(l) = self.lists.get_mut(playlist)
                        && let Some(i) = l.rows.iter().position(|r| r.uid.as_deref() == Some(uid.as_str()))
                    {
                        let row = l.rows.remove(i);
                        l.rows.insert(was.min(l.rows.len()), row);
                    }
                    self.say("Spotify didn't take the move; it's back where it was");
                }
                (None, true) => {
                    if self.view.as_ref().is_some_and(|v| v.uri == playlist) {
                        self.send(Command::GetTracks { uri: playlist.into() });
                    } else {
                        self.lists.remove(playlist);
                    }
                }
                _ => {}
            }
        }
    }
}

pub fn adding_rect(area: Rect, rows: usize) -> Rect {
    crate::ui::centered(area, 56, (rows as u16 + 4).clamp(7, 18))
}

/// The "Add to playlist" picker: the filter, "New playlist", your playlists.
pub fn draw_adding(buf: &mut Buffer, area: Rect, app: &mut App, th: &Theme) {
    let Some(a) = app.playlists.adding.clone() else { return };
    let rows = app.adding_rows();
    let title = format!("Add “{}” to", a.label);
    let inner = crate::ui::popup(buf, adding_rect(area, rows.len()), th, &title);
    let w = inner.width;
    let shown = if a.filter.is_empty() { "type to filter".to_string() } else { a.filter.clone() };
    text(buf, inner.x + 1, inner.y, w.saturating_sub(2), &format!("› {shown}▏"), th.s(if a.filter.is_empty() { th.dim } else { th.fg }).add_modifier(Modifier::BOLD));
    // the filter above, the hint below
    let room = inner.height.saturating_sub(2) as usize;
    // keep the selection in view
    let top = a.sel.saturating_sub(room.saturating_sub(1));
    for (n, (i, (uri, name))) in rows.iter().enumerate().skip(top).take(room).enumerate() {
        let y = inner.y + 1 + n as u16;
        let on = i == a.sel;
        let bg = if on { th.selection.color() } else { th.bg.color() };
        text(buf, inner.x, y, w, &" ".repeat(w as usize), th.base().bg(bg));
        text(buf, inner.x + 1, y, 2, if on { "›" } else { " " }, th.s(th.accent).bg(bg));
        let style = if uri.is_none() { th.s(th.accent) } else if on { th.title() } else { th.s(th.fg) };
        text(buf, inner.x + 3, y, w.saturating_sub(4), name, style.bg(bg));
        app.hit(Rect::new(inner.x, y, w, 1), crate::app::Hit::AddPick(i));
    }
    if rows.len() == 1 && !a.filter.is_empty() {
        text(buf, inner.x + 3, inner.y + 2, w.saturating_sub(4), "none of your playlists match", th.s(th.dim));
    }
    text(buf, inner.x + 1, inner.bottom().saturating_sub(1), w.saturating_sub(2), "type to filter · ↑↓ · Enter add · Esc close", th.s(th.dim));
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
        assert_eq!(sent(&mut rx), vec![Command::CreatePlaylist { name: "Road trip".into(), uris: vec![] }]);
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

    #[test]
    fn songs_are_added_from_a_filtered_picker_and_a_duplicate_asks_first() {
        use ratatui::crossterm::event::KeyCode;
        use singularity_proto::AddOutcome;
        let (mut app, mut rx) = app();
        app.library.as_mut().unwrap().items.push(item("spotify:playlist:road", "Road trip", true));
        // every track menu offers it
        app.open_row_menu(0, None);
        app.menu = None;
        app.ask_add(vec!["spotify:track:t".into()], "Song");
        let names = |app: &App| app.adding_rows().into_iter().map(|(_, n)| n).collect::<Vec<_>>();
        assert_eq!(names(&app), vec!["+ New playlist…", "Mine", "Road trip"], "only your own, New first");
        typed(&mut app, "ROAD");
        assert_eq!(names(&app), vec!["+ New playlist “ROAD”", "Road trip"]);
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Enter);
        let add = |anyway| Command::AddToPlaylist { playlist: "spotify:playlist:road".into(), name: "Road trip".into(), uris: vec!["spotify:track:t".into()], anyway };
        assert_eq!(sent(&mut rx), vec![add(false)]);
        assert!(app.playlists.adding.is_none());
        // it has it: asked, and the same pick again adds it anyway
        app.added_to_playlist("spotify:playlist:road", "Road trip", vec!["spotify:track:t".into()], AddOutcome::AlreadyThere);
        assert!(app.toast.as_ref().unwrap().0.contains("Already in Road trip"));
        app.ask_add(vec!["spotify:track:t".into()], "Song");
        app.add_pick(2);
        assert_eq!(sent(&mut rx), vec![add(true)]);
        // the open playlist fetches its new end; the toast says so
        app.open("spotify:playlist:road", "Road trip", "", "playlist");
        sent(&mut rx);
        app.added_to_playlist("spotify:playlist:road", "Road trip", vec!["spotify:track:t".into()], AddOutcome::Added);
        assert_eq!(app.toast.as_ref().unwrap().0, "Added to Road trip");
        assert!(std::iter::from_fn(|| rx.try_recv().ok()).any(|c| c == Command::GetTracks { uri: "spotify:playlist:road".into() }));
        // "New playlist" with a filter: made with the song in it, and it doesn't open
        app.ask_add(vec!["spotify:track:t".into()], "Song");
        typed(&mut app, "Fresh");
        key(&mut app, KeyCode::Enter);
        assert_eq!(sent(&mut rx), vec![Command::CreatePlaylist { name: "Fresh".into(), uris: vec!["spotify:track:t".into()] }]);
        app.playlist_created("spotify:playlist:fresh", "Fresh");
        assert!(matches!(app.current_page(), crate::app::Page::List { uri, .. } if uri == "spotify:playlist:road"));
        // without a filter, it's named first
        app.ask_add(vec!["spotify:track:u".into()], "Other");
        key(&mut app, KeyCode::Enter);
        assert!(app.naming.is_some());
        typed(&mut app, "Named");
        key(&mut app, KeyCode::Enter);
        assert_eq!(sent(&mut rx), vec![Command::CreatePlaylist { name: "Named".into(), uris: vec!["spotify:track:u".into()] }]);
        // albums don't go into playlists
        app.ask_add(vec!["spotify:album:a".into()], "Album");
        assert!(app.playlists.adding.is_none());
    }

    #[test]
    fn one_row_is_removed_by_uid_and_put_back_if_refused() {
        use ratatui::crossterm::event::KeyCode;
        use singularity_proto::{TrackPage, TrackRow};
        let (mut app, mut rx) = app();
        let row = |uri: &str, uid: &str| TrackRow { uri: uri.into(), name: uri.into(), uid: Some(uid.into()), ..Default::default() };
        // the same song twice
        let rows = vec![row("a", "u0"), row("dup", "u1"), row("b", "u2"), row("dup", "u3")];
        app.open("spotify:playlist:mine", "Mine", "", "playlist");
        app.track_page(TrackPage { uri: "spotify:playlist:mine".into(), offset: 0, total: 4, tracks: rows, header: None });
        sent(&mut rx);
        app.open_row_menu(1, None);
        let labels = menu_labels(&app);
        let at = labels.iter().position(|l| l == "Remove from this playlist").expect("offered on your own");
        app.menu_run(at);
        assert_eq!(sent(&mut rx), vec![Command::RemoveFromPlaylist { playlist: "spotify:playlist:mine".into(), uids: vec!["u1".into()] }]);
        let uids = |app: &App| app.lists["spotify:playlist:mine"].rows.iter().map(|r| r.uid.clone().unwrap()).collect::<Vec<_>>();
        assert_eq!(uids(&app), vec!["u0", "u2", "u3"], "one copy goes at once, the other stays");
        // refused: back where it was
        app.removed_from_playlist("spotify:playlist:mine", &["u1".into()], false);
        assert_eq!(uids(&app), vec!["u0", "u1", "u2", "u3"]);
        assert!(app.toast.as_ref().unwrap().0.contains("it's back"));
        // the Delete key on the selected row; Spotify agrees
        app.focus = "home".into();
        app.view.as_mut().unwrap().sel = 3;
        app.key(ratatui::crossterm::event::KeyEvent::from(KeyCode::Delete));
        assert_eq!(sent(&mut rx), vec![Command::RemoveFromPlaylist { playlist: "spotify:playlist:mine".into(), uids: vec!["u3".into()] }]);
        app.removed_from_playlist("spotify:playlist:mine", &["u3".into()], true);
        assert_eq!(uids(&app), vec!["u0", "u1", "u2"]);
        assert_eq!(app.view.as_ref().unwrap().sel, 2, "the selection stays on a row");
        // another Client's removal leaves here too
        app.removed_from_playlist("spotify:playlist:mine", &["u0".into()], true);
        assert_eq!(uids(&app), vec!["u1", "u2"]);
        // someone else's playlist: not offered, Delete does nothing
        app.open("spotify:playlist:theirs", "Theirs", "", "playlist");
        app.track_page(TrackPage { uri: "spotify:playlist:theirs".into(), offset: 0, total: 1, tracks: vec![row("c", "x0")], header: None });
        app.open_row_menu(0, None);
        assert!(!menu_labels(&app).iter().any(|l| l.starts_with("Remove")));
        app.menu = None;
        app.key(ratatui::crossterm::event::KeyEvent::from(KeyCode::Delete));
        assert!(!sent(&mut rx).iter().any(|c| matches!(c, Command::RemoveFromPlaylist { .. })));
    }

    #[test]
    fn rows_are_dragged_or_nudged_into_place_and_moved_back_if_refused() {
        use crate::app::Hit;
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use singularity_proto::{TrackPage, TrackRow};
        let (mut app, mut rx) = app();
        let row = |n: usize| TrackRow { uri: format!("t{n}"), name: format!("Song {n}"), uid: Some(format!("u{n}")), playable: true, ..Default::default() };
        app.open("spotify:playlist:mine", "Mine", "", "playlist");
        app.track_page(TrackPage { uri: "spotify:playlist:mine".into(), offset: 0, total: 5, tracks: (0..5).map(row).collect(), header: None });
        sent(&mut rx);
        let draw = |app: &mut App| {
            let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(140, 40)).unwrap();
            t.draw(|f| crate::ui::draw(f, app)).unwrap();
            t.backend().buffer().clone()
        };
        let uids = |app: &App| app.lists["spotify:playlist:mine"].rows.iter().map(|r| r.uid.clone().unwrap()).collect::<Vec<_>>();
        draw(&mut app);
        let y = |app: &App, k: usize| app.hit_rect(&Hit::Track(k)).unwrap().y;
        // press row 0, drag down onto row 3: the preview shows it there before the drop
        app.playlists.press = Some(0);
        let (y0, y3) = (y(&app, 0), y(&app, 3));
        app.drag_row_to(y0);
        assert!(app.playlists.row_drag.is_none(), "a shaky click isn't a drag");
        app.drag_row_to(y3);
        let screen = draw(&mut app);
        let line = |b: &ratatui::buffer::Buffer, yy: u16| (0..b.area.width).map(|x| b[(x, yy)].symbol().to_string()).collect::<String>();
        assert!(line(&screen, y3).contains("Song 0"), "{}", line(&screen, y3));
        assert!(sent(&mut rx).is_empty(), "nothing sent while dragging");
        app.drop_row();
        assert_eq!(sent(&mut rx), vec![Command::MoveInPlaylist { playlist: "spotify:playlist:mine".into(), uids: vec!["u0".into()], before: Some("u4".into()), after: None }]);
        assert_eq!(uids(&app), vec!["u1", "u2", "u3", "u0", "u4"]);
        // Alt+↓ on the last-but-one: to the end, anchored after the last
        app.focus = "home".into();
        app.view.as_mut().unwrap().sel = 3;
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::ALT));
        assert_eq!(sent(&mut rx), vec![Command::MoveInPlaylist { playlist: "spotify:playlist:mine".into(), uids: vec!["u0".into()], before: None, after: Some("u4".into()) }]);
        assert_eq!(uids(&app), vec!["u1", "u2", "u3", "u4", "u0"]);
        // the first answer only counts down; the refused second puts it back where the first move took it from
        app.moved_in_playlist("spotify:playlist:mine", &["u0".into()], true);
        assert!(sent(&mut rx).is_empty(), "no refetch for our own move");
        app.moved_in_playlist("spotify:playlist:mine", &["u0".into()], false);
        assert_eq!(uids(&app), vec!["u0", "u1", "u2", "u3", "u4"]);
        // a sorted view says why instead
        app.view.as_mut().unwrap().sort = Some((crate::tracks::Col::Title, false));
        app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::ALT));
        assert!(app.toast.as_ref().unwrap().0.contains("Clear the sort"));
        assert!(sent(&mut rx).is_empty());
    }

    #[test]
    fn the_wheel_moves_the_add_picker_not_the_list_behind() {
        use ratatui::crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
        let (mut app, _rx) = app();
        for n in 0..30 {
            app.library.as_mut().unwrap().items.push(item(&format!("spotify:playlist:p{n}"), &format!("List {n}"), true));
        }
        app.ask_add(vec!["spotify:track:t".into()], "Song");
        let wheel = |app: &mut App, kind, modifiers| app.pointer(MouseEvent { kind, column: 60, row: 10, modifiers }, 60.0, false);
        for _ in 0..3 {
            wheel(&mut app, MouseEventKind::ScrollDown, KeyModifiers::NONE);
        }
        assert_eq!(app.playlists.adding.as_ref().unwrap().sel, 3);
        wheel(&mut app, MouseEventKind::ScrollDown, KeyModifiers::SHIFT);
        assert_eq!(app.playlists.adding.as_ref().unwrap().sel, 13, "Shift: a page");
        wheel(&mut app, MouseEventKind::ScrollUp, KeyModifiers::NONE);
        assert_eq!(app.playlists.adding.as_ref().unwrap().sel, 12);
        for _ in 0..50 {
            wheel(&mut app, MouseEventKind::ScrollDown, KeyModifiers::NONE);
        }
        assert_eq!(app.playlists.adding.as_ref().unwrap().sel, app.adding_rows().len() - 1, "stops at the end");
    }
}
