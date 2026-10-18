//! Artist pages (#11): name and listeners, ▶ Play, popular tracks, then Shelves of releases, appearances and related
//! artists (drawn like Home's, so Cards open the same way).

use crate::app::{App, Hit};
use crate::home::HomeView;
use crate::theme::Theme;
use crate::ui::text;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use singularity_proto::{ArtistPage, HomeFeed};

pub struct ArtistView {
    pub uri: String,
    pub name: String,
    pub page: Option<ArtistPage>,
    /// the releases as a feed, for the Card renderer
    pub feed: HomeFeed,
    pub shelves: HomeView,
    pub top_sel: usize,
}

impl ArtistView {
    pub fn new(uri: &str, name: &str) -> ArtistView {
        ArtistView { uri: uri.into(), name: name.into(), page: None, feed: HomeFeed::default(), shelves: HomeView::default(), top_sel: 0 }
    }

    pub fn set_page(&mut self, p: ArtistPage) {
        self.name = p.name.clone();
        self.feed = HomeFeed { fetched_unix_ms: 0, shelves: p.shelves.clone() };
        self.page = Some(p);
    }
}

fn clip(s: &str, w: u16) -> String {
    let w = w as usize;
    if s.chars().count() <= w {
        return s.to_string();
    }
    let mut o: String = s.chars().take(w.saturating_sub(1)).collect();
    o.push('…');
    o
}

/// How many popular tracks fit: up to ten, leaving the Shelves at least half the Pane.
pub fn top_rows(height: u16, n: usize) -> usize {
    n.min(10).min((height as usize / 2).saturating_sub(4)).max(n.min(3))
}

pub fn draw(buf: &mut Buffer, r: Rect, app: &mut App, th: &Theme) {
    let Some(mut v) = app.artist.take() else { return };
    let (x, w) = (r.x + 1, r.width.saturating_sub(2));
    // (the name is the Pane's title)
    let Some(page) = v.page.clone() else {
        text(buf, x, r.y, w, "loading…", th.s(th.dim));
        app.artist = Some(v);
        return;
    };
    text(buf, x, r.y, 8, " ▶ Play ", th.chip(th.accent));
    app.hit(Rect::new(x, r.y, 8, 1), Hit::PlayArtist);
    // + Follow / ✓ Following
    let following = app.is_saved(&v.uri) == Some(true);
    let label = if following { " ✓ Following " } else { " + Follow " };
    let fw = label.chars().count() as u16;
    text(buf, x + 9, r.y, fw, label, if following { th.chip(th.accent2) } else { th.s(th.fg) });
    app.hit(Rect::new(x + 9, r.y, fw, 1), Hit::Save(singularity_proto::Link { name: v.name.clone(), uri: v.uri.clone() }));
    if let Some(n) = page.monthly_listeners {
        let lx = x + 10 + fw;
        text(buf, lx, r.y, w.saturating_sub(lx - x), &format!("{} monthly listeners", crate::tracks::grouped(n)), th.s(th.dim));
    }
    let mut y = r.y + 2;
    let n = top_rows(r.height, page.top_tracks.len());
    if n > 0 {
        text(buf, x, y, w, "Popular", th.s(th.accent).add_modifier(Modifier::BOLD));
        y += 1;
        let focused = app.focus == "home";
        let playing = app.playback.item.as_ref().map(|i| i.uri.clone());
        for (i, t) in page.top_tracks.iter().take(n).enumerate() {
            let selected = focused && i == v.top_sel;
            let bg = if selected { th.selection.color() } else { th.bg.color() };
            text(buf, x, y, w, &" ".repeat(w as usize), th.base().bg(bg));
            let now = playing.as_deref() == Some(t.uri.as_str());
            let num = if now { "  ♪".to_string() } else { format!("{:>3}", i + 1) };
            text(buf, x, y, 4, &num, th.s(if now { th.accent } else { th.dim }).bg(bg));
            let tw = (w.saturating_sub(12)) * 6 / 10;
            text(buf, x + 5, y, tw, &clip(&t.name, tw.saturating_sub(1)), th.s(if now { th.accent } else { th.fg }).bg(bg));
            let aw = w.saturating_sub(12 + tw);
            text(buf, x + 5 + tw, y, aw, &clip(&t.album, aw.saturating_sub(1)), th.s(th.dim).bg(bg));
            let d = t.duration_ms / 1000;
            text(buf, x + w - 6, y, 6, &format!("{:>2}:{:02}", d / 60, d % 60), th.s(th.dim).bg(bg));
            app.hit(Rect::new(x, y, w, 1), Hit::TopTrack(i));
            if let Some(au) = &t.album_uri {
                app.hit(Rect::new(x + 5 + tw, y, (t.album.chars().count() as u16).min(aw), 1), Hit::Link(singularity_proto::Link { name: t.album.clone(), uri: au.clone() }));
            }
            y += 1;
        }
        y += 1;
    }
    if y < r.bottom() {
        let feed = std::mem::take(&mut v.feed);
        let view = std::mem::take(&mut v.shelves);
        // the Card renderer reads the artist from the App, so put it back while it draws
        let uri = v.uri.clone();
        app.artist = Some(v);
        let (feed, view) = crate::home::draw_feed(buf, Rect::new(r.x, y, r.width, r.bottom() - y), app, th, feed, view);
        if let Some(a) = app.artist.as_mut().filter(|a| a.uri == uri) {
            a.feed = feed;
            a.shelves = view;
        }
        return;
    }
    app.artist = Some(v);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popular_tracks_leave_room_for_the_shelves() {
        assert_eq!(top_rows(60, 10), 10);
        assert_eq!(top_rows(20, 10), 6);
        assert_eq!(top_rows(10, 10), 3, "always a few");
        assert_eq!(top_rows(40, 2), 2);
    }
}
