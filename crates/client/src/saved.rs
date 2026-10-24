//! Like/unlike and follow (#22): what's in the library, changed optimistically.
//!
//! A toggle shows at once and goes to the Daemon as `SetSaved`; `Event::Saved` is the truth to settle on (on a failure
//! it carries the old value back, so the UI rolls back). Liked Songs is patched in place, never reloaded.

use crate::app::{App, MenuAct};
use crate::tracks::LIKED;
use singularity_proto::{Command, SavedFlag, TrackRow};
use std::collections::HashMap;

#[derive(Default)]
pub struct Saved {
    /// what Spotify last said (or what we just asked for), by URI
    known: HashMap<String, bool>,
    /// toggles still waiting for their answer: answers to older ones are stale while newer ones are out
    pending: HashMap<String, u32>,
    /// rows taken out of Liked Songs, kept to put back on a rollback
    stash: HashMap<String, (usize, TrackRow)>,
    /// URIs asked about (so a menu's target is checked once)
    asked: std::collections::HashSet<String>,
    /// the playing track the ♥ was last checked for
    pub checked_now: Option<String>,
}

impl Saved {
    pub fn get(&self, uri: &str) -> Option<bool> {
        self.known.get(uri).copied()
    }
}

/// What kind of thing a URI is, for wording.
fn kind(uri: &str) -> &str {
    uri.split(':').nth(1).unwrap_or("")
}

/// The menu label that toggles `uri`, given whether it's saved.
pub fn label(uri: &str, saved: bool) -> &'static str {
    match (kind(uri), saved) {
        ("track", false) => "Save to Liked Songs",
        ("track", true) => "Remove from Liked Songs",
        ("artist" | "show", false) => "Follow",
        ("artist" | "show", true) => "Unfollow",
        (_, false) => "Save to Your Library",
        (_, true) => "Remove from Your Library",
    }
}

/// Things that can be liked, saved or followed.
pub fn savable(uri: &str) -> bool {
    matches!(kind(uri), "track" | "album" | "playlist" | "artist" | "show") && uri != LIKED
}

/// A track row's ♥ at (x, y): filled when liked, outlined when not, blank until Spotify has said (#49). Returns its
/// click target.
// A drawing call: its arguments are the row's cell and everything the heart shows.
#[allow(clippy::too_many_arguments)]
pub fn heart(buf: &mut ratatui::buffer::Buffer, x: u16, y: u16, uri: &str, name: &str, liked: Option<bool>, bg: ratatui::style::Color, th: &crate::theme::Theme) -> (ratatui::layout::Rect, crate::app::Hit) {
    let (ch, col) = match liked {
        Some(true) => ("♥", th.accent),
        Some(false) => ("♡", th.border),
        None => (" ", th.border),
    };
    crate::ui::text(buf, x, y, 1, ch, th.s(col).bg(bg));
    (ratatui::layout::Rect::new(x, y, 1, 1), crate::app::Hit::Save(singularity_proto::Link { name: name.into(), uri: uri.into() }))
}

/// `YYYY-MM-DD` for a Unix time (a new Liked Songs row's "added" date).
fn iso_date(unix_s: u64) -> String {
    // civil-from-days (Howard Hinnant)
    let z = (unix_s / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

impl App {
    /// Whether `uri` is liked/saved/followed, if known.
    pub fn is_saved(&self, uri: &str) -> Option<bool> {
        self.saved.get(uri)
    }

    /// Ask Spotify about these, once each (the current track, an open page, a menu's target).
    pub fn check_saved(&mut self, uris: Vec<String>) {
        let uris: Vec<String> = uris.into_iter().filter(|u| savable(u) && self.saved.asked.insert(u.clone())).collect();
        if !uris.is_empty() && matches!(self.link, crate::app::Link::Connected { .. }) {
            self.send(Command::CheckSaved { uris });
        }
    }

    /// Rows from the first one in view onwards (`visible` of them on screen): once a row on screen hasn't been asked
    /// about, ask about the next 50 in one go, so scrolling a long list sends a batch about once a screenful rather
    /// than one per row (#49).
    pub fn want_saved<'a>(&mut self, rows: impl Iterator<Item = &'a str>, visible: usize) {
        let rows: Vec<&str> = rows.take(50).collect();
        if rows.iter().take(visible).any(|u| savable(u) && !self.saved.asked.contains(*u)) {
            let batch: Vec<String> = rows.into_iter().filter(|u| savable(u) && !self.saved.asked.contains(*u)).map(String::from).collect();
            self.check_saved(batch);
        }
    }

    /// Ask again even if asked before (a page opened, a new track: it may have changed on another device).
    pub fn recheck_saved(&mut self, uri: &str) {
        self.saved.asked.remove(uri);
        self.check_saved(vec![uri.to_string()]);
    }

    /// Like/unlike, save/remove, follow/unfollow `uri`: shown at once, settled by `Event::Saved`.
    pub fn toggle_saved(&mut self, uri: &str, name: &str) {
        if !savable(uri) {
            return;
        }
        let now = !self.saved.get(uri).unwrap_or(false);
        self.saved.known.insert(uri.into(), now);
        *self.saved.pending.entry(uri.into()).or_default() += 1;
        self.send(Command::SetSaved { uri: uri.into(), saved: now });
        self.patch_liked(uri, now);
        let msg = match (kind(uri), now) {
            ("track", true) => "Added to Liked Songs".to_string(),
            ("track", false) => "Removed from Liked Songs".to_string(),
            ("artist" | "show", true) => format!("Following {name}"),
            ("artist" | "show", false) => format!("Unfollowed {name}"),
            (_, true) => "Saved to Your Library".to_string(),
            (_, false) => "Removed from Your Library".to_string(),
        };
        self.say(msg);
    }

    /// Like or unlike what's playing (the ♥ in the player bar, key `l`).
    pub fn toggle_current(&mut self) {
        match self.playback.item.as_ref().map(|i| (i.uri.clone(), i.name.clone())) {
            Some((uri, name)) if savable(&uri) => self.toggle_saved(&uri, &name),
            _ => self.say("Nothing playing to like"),
        }
    }

    /// Spotify's answer: settle on it, unless newer toggles of the same thing are still out.
    pub fn saved_event(&mut self, entries: Vec<SavedFlag>) {
        for SavedFlag { uri, saved } in entries {
            if let Some(n) = self.saved.pending.get_mut(&uri) {
                *n -= 1;
                if *n > 0 {
                    continue;
                }
                self.saved.pending.remove(&uri);
            }
            let was = self.saved.known.insert(uri.clone(), saved);
            if was != Some(saved) {
                // a rollback, or news from another device
                self.patch_liked(&uri, saved);
            }
            if saved {
                self.saved.stash.remove(&uri);
            }
            // an open menu about this shows the right wording
            let about = self.menu.as_ref().and_then(|m| self.target_track(&m.target)).map(|(u, ..)| u);
            if about.as_deref() == Some(uri.as_str())
                && let Some(item) = self.menu.as_mut().and_then(|m| m.items.iter_mut().find(|(_, a)| *a == MenuAct::Save))
            {
                item.0 = label(&uri, saved).into();
            }
        }
    }

    /// Everything in Liked Songs is liked (seeded as its pages arrive, unless a toggle is out).
    pub fn seed_liked(&mut self, rows: &[TrackRow]) {
        for r in rows {
            if !self.saved.pending.contains_key(&r.uri) {
                self.saved.known.insert(r.uri.clone(), true);
            }
        }
    }

    /// Keep the cached Liked Songs in step with a like (row on top) or an unlike (row out).
    fn patch_liked(&mut self, uri: &str, saved: bool) {
        if kind(uri) != "track" {
            return;
        }
        let Some(list) = self.lists.get_mut(LIKED) else { return };
        if !list.complete() {
            // pages are still arriving by offset: start over rather than shift them
            self.lists.remove(LIKED);
            if self.view.as_ref().is_some_and(|v| v.uri == LIKED) {
                self.send(Command::GetTracks { uri: LIKED.into() });
            }
            return;
        }
        if saved {
            let (at, row) = match self.saved.stash.remove(uri) {
                Some(s) => s,
                None => match self.find_row(uri) {
                    Some(mut r) => {
                        r.added_at = Some(iso_date(singularity_proto::now_ms() / 1000));
                        (0, r)
                    }
                    None => return,
                },
            };
            if let Some(list) = self.lists.get_mut(LIKED) {
                list.insert(at, row);
            }
        } else if let Some(removed) = list.remove(uri) {
            self.saved.stash.insert(uri.into(), removed);
        }
        // the sidebar's and the list's "N songs" count along
        let Some(n) = self.lists.get(LIKED).map(|l| l.total) else { return };
        let count = format!("{n} songs");
        if let Some(it) = self.library.as_mut().and_then(|l| l.items.iter_mut().find(|i| i.kind == "liked")) {
            it.subtitle = count.clone();
        }
        if let Some(v) = self.view.as_mut().filter(|v| v.uri == LIKED) {
            v.subtitle = count;
        }
    }

    /// A track's row from anywhere on screen (lists, search, the artist page, what's playing).
    fn find_row(&self, uri: &str) -> Option<TrackRow> {
        let lists = self.lists.values().flat_map(|l| l.rows.iter());
        let search = self.search.iter().filter_map(|s| s.results.as_ref()).flat_map(|r| r.tracks.iter());
        let artist = self.artist.iter().filter_map(|a| a.page.as_ref()).flat_map(|p| p.top_tracks.iter());
        if let Some(r) = lists.chain(search).chain(artist).find(|r| r.uri == uri) {
            return Some(r.clone());
        }
        let i = self.playback.item.as_ref().filter(|i| i.uri == uri)?;
        Some(TrackRow {
            uri: i.uri.clone(),
            name: i.name.clone(),
            artists: i.artists.clone(),
            album: i.album.clone().unwrap_or_default(),
            added_at: None,
            duration_ms: i.duration_ms,
            playable: true,
            artist_uris: i.artist_uris.clone(),
            album_uri: i.album_uri.clone(),
            uid: None,
            progress: None,
        })
    }

    /// The Save entry for a menu about `uri` (asks Spotify first if we don't know yet).
    pub fn save_entry(&mut self, uri: &str) -> Option<(String, MenuAct)> {
        if !savable(uri) {
            return None;
        }
        if self.saved.get(uri).is_none() {
            self.check_saved(vec![uri.to_string()]);
        }
        Some((label(uri, self.saved.get(uri).unwrap_or(false)).into(), MenuAct::Save))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_civil() {
        assert_eq!(iso_date(0), "1970-01-01");
        assert_eq!(iso_date(1_792_281_600), "2026-10-18");
        assert_eq!(iso_date(951_782_400), "2000-02-29");
    }

    #[test]
    fn wording_follows_the_kind() {
        assert_eq!(label("spotify:track:x", false), "Save to Liked Songs");
        assert_eq!(label("spotify:artist:x", true), "Unfollow");
        assert_eq!(label("spotify:playlist:x", true), "Remove from Your Library");
        assert!(!savable(LIKED) && savable("spotify:show:x") && !savable("spotify:episode:x"));
        assert_eq!(label("spotify:show:x", true), "Unfollow");
    }
}
