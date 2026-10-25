//! Custom playlist covers (#56): a picture file made into what Spotify takes (a square JPEG of at most 256 KB), and
//! the popup that previews it before the upload.

use crate::app::App;
use crate::theme::Theme;
use crate::ui::text;
use image::RgbImage;
use image::imageops::FilterType;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use singularity_proto::Command;

/// Spotify's limit for a playlist cover.
pub const MAX_BYTES: usize = 256 * 1024;
/// The largest side we send (Spotify shows covers at 300–640 px).
const SIDE: u32 = 640;
/// Files bigger than this aren't pictures anyone means to upload.
const MAX_FILE: u64 = 50 * 1024 * 1024;

/// A picture ready to upload: the JPEG, and how it looks (for the preview).
#[derive(Clone)]
pub struct Prepared {
    pub jpeg: Vec<u8>,
    pub image: RgbImage,
}

/// The "Change cover" popup.
#[derive(Clone)]
pub struct CoverForm {
    pub playlist: String,
    pub name: String,
    pub path: String,
    /// the path's picture, made ready (it previews; Enter again uploads)
    pub ready: Option<Prepared>,
    pub error: Option<String>,
}

/// A typed or dropped path as a file path: quotes, `file://` (percent-encoded), `\ ` and `~/` undone.
pub fn clean_path(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')).or_else(|| s.strip_prefix('"').and_then(|s| s.strip_suffix('"'))).unwrap_or(s);
    let s = match s.strip_prefix("file://") {
        Some(rest) => {
            // file://host/path: the path starts at the first slash
            let rest = &rest[rest.find('/').unwrap_or(0)..];
            let b = rest.as_bytes();
            let mut out = vec![];
            let mut i = 0;
            while i < b.len() {
                match (b[i], b.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok())) {
                    (b'%', Some(v)) => {
                        out.push(v);
                        i += 3;
                    }
                    (c, _) => {
                        out.push(c);
                        i += 1;
                    }
                }
            }
            String::from_utf8_lossy(&out).into_owned()
        }
        None => s.replace("\\ ", " "),
    };
    match s.strip_prefix("~/") {
        Some(rest) => std::env::var("HOME").map(|h| format!("{h}/{rest}")).unwrap_or(s),
        None => s,
    }
}

/// A picture (PNG, JPEG or WebP) cropped square from the centre, at most 640 px, as a JPEG of at most 256 KB:
/// the quality steps down until it fits, then the size.
pub fn prepare(bytes: &[u8]) -> Result<Prepared, String> {
    // a dropped file gets the same limits as a downloaded cover: a header can claim more pixels than there is memory
    let img = crate::covers::decode(bytes).ok_or_else(|| "Not a picture this can read (PNG, JPEG or WebP), or far too large".to_string())?;
    let (w, h) = img.dimensions();
    let side = w.min(h);
    if side == 0 {
        return Err("The picture is empty".into());
    }
    let square = image::imageops::crop_imm(&img, (w - side) / 2, (h - side) / 2, side, side).to_image();
    for px in [SIDE, 480, 320] {
        let px = px.min(side);
        let scaled = if px == side { square.clone() } else { image::imageops::resize(&square, px, px, FilterType::Lanczos3) };
        for quality in [90u8, 80, 70, 60, 50, 40] {
            let mut jpeg = vec![];
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, quality).encode_image(&scaled).map_err(|e| e.to_string())?;
            if jpeg.len() <= MAX_BYTES {
                return Ok(Prepared { jpeg, image: scaled });
            }
        }
    }
    Err("Couldn't make it small enough for Spotify (256 KB)".into())
}

/// Read and prepare the file at `path`.
fn load(path: &str) -> Result<Prepared, String> {
    let meta = std::fs::metadata(path).map_err(|_| format!("No file at {path}"))?;
    if !meta.is_file() || meta.len() > MAX_FILE {
        return Err("That isn't a picture file (or it's over 50 MB)".into());
    }
    prepare(&std::fs::read(path).map_err(|e| e.to_string())?)
}

impl App {
    /// "Change cover…" on one of your playlists.
    pub fn ask_cover(&mut self, uri: &str) {
        let Some(name) = self.own_playlist(uri).map(|i| i.name.clone()) else {
            return self.say("Only your own playlists can get a new cover");
        };
        self.cover_form = Some(CoverForm { playlist: uri.into(), name, path: String::new(), ready: None, error: None });
    }

    pub fn cover_key(&mut self, k: KeyEvent) {
        let Some(f) = self.cover_form.as_mut() else { return };
        match k.code {
            KeyCode::Esc => self.cover_form = None,
            KeyCode::Enter if f.ready.is_some() => self.upload_cover(),
            KeyCode::Enter => self.load_cover(),
            KeyCode::Backspace => {
                f.path.pop();
                f.ready = None;
            }
            KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                f.path.clear();
                f.ready = None;
            }
            KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) && f.path.len() < 4096 => {
                f.path.push(c);
                f.ready = None;
            }
            _ => {}
        }
    }

    /// A dropped file arrives as a pasted path: it replaces what's typed and previews at once.
    pub fn cover_paste(&mut self, s: &str) {
        if let Some(f) = self.cover_form.as_mut() {
            f.path = s.chars().filter(|c| !c.is_control()).collect();
            self.load_cover();
        }
    }

    fn load_cover(&mut self) {
        let Some(f) = self.cover_form.as_mut() else { return };
        f.path = clean_path(&f.path);
        if f.path.is_empty() {
            f.error = Some("Type a path, or drop a picture file here".into());
            return;
        }
        match load(&f.path) {
            Ok(p) => {
                self.covers.insert(&preview_key(&f.playlist), p.image.clone());
                f.ready = Some(p);
                f.error = None;
            }
            Err(e) => {
                f.ready = None;
                f.error = Some(e);
            }
        }
    }

    /// Enter on a preview: the new cover shows at once, then goes to Spotify.
    fn upload_cover(&mut self) {
        let Some(f) = self.cover_form.take() else { return };
        let Some(p) = f.ready else { return };
        self.show_cover(&f.playlist, Some(preview_key(&f.playlist)));
        self.send(Command::SetPlaylistCover { playlist: f.playlist, jpeg: Some(crate::gfx::base64(&p.jpeg)) });
        self.say(format!("Uploading the cover of {}…", f.name));
    }

    /// "Remove cover": Spotify's mosaic comes back.
    pub fn remove_cover(&mut self, uri: &str) {
        let Some(name) = self.own_playlist(uri).map(|i| i.name.clone()) else { return };
        self.show_cover(uri, None);
        self.send(Command::SetPlaylistCover { playlist: uri.into(), jpeg: None });
        self.say(format!("Putting back Spotify's cover for {name}…"));
    }

    /// Show `url` as the playlist's cover (sidebar and header), remembering what to put back.
    fn show_cover(&mut self, uri: &str, url: Option<String>) {
        let lib = self.library.as_ref().and_then(|l| l.items.iter().find(|i| i.uri == uri)).map(|i| i.image_url.clone()).unwrap_or_default();
        let header = self.lists.get(uri).and_then(|l| l.header.as_ref()).and_then(|h| h.cover_url.clone());
        self.covers_was.entry(uri.into()).or_insert((lib, header));
        if let Some(i) = self.library.as_mut().and_then(|l| l.items.iter_mut().find(|i| i.uri == uri)) {
            i.image_url = url.clone();
        }
        if let Some(h) = self.lists.get_mut(uri).and_then(|l| l.header.as_mut()) {
            h.cover_url = url;
        }
    }

    /// `Event::PlaylistCoverSet`: the real cover follows (fresh library, the page fetched again); a refusal puts the
    /// old one back.
    pub fn cover_set(&mut self, uri: &str, ok: bool) {
        let was = self.covers_was.remove(uri);
        if ok && was.is_some() {
            self.say("Cover changed");
        }
        if ok {
            if self.view.as_ref().is_some_and(|v| v.uri == uri) {
                self.send(Command::GetTracks { uri: uri.into() });
            } else {
                self.lists.remove(uri);
            }
        } else if let Some((lib, header)) = was {
            if let Some(i) = self.library.as_mut().and_then(|l| l.items.iter_mut().find(|i| i.uri == uri)) {
                i.image_url = lib;
            }
            if let Some(h) = self.lists.get_mut(uri).and_then(|l| l.header.as_mut()) {
                h.cover_url = header;
            }
        }
    }
}

/// Where the preview lives in the cover cache (it's never fetched).
fn preview_key(playlist: &str) -> String {
    format!("local:cover:{playlist}")
}

pub fn rect(area: Rect) -> Rect {
    crate::ui::centered(area, 66, 17)
}

pub fn draw(buf: &mut Buffer, area: Rect, app: &mut App, th: &Theme) {
    let Some(f) = app.cover_form.clone() else { return };
    let inner = crate::ui::popup(buf, rect(area), th, &format!("Cover for {}", f.name));
    let (x, w) = (inner.x + 1, inner.width.saturating_sub(2));
    text(buf, x, inner.y, w, "Picture file (type a path, or drop one here)", th.s(th.dim));
    let pw = w.saturating_sub(3) as usize;
    let shown: String = f.path.chars().skip(f.path.chars().count().saturating_sub(pw.saturating_sub(1))).collect();
    text(buf, x, inner.y + 1, w, &format!("› {shown}▏"), th.s(th.fg).add_modifier(Modifier::BOLD));
    let pr = Rect::new(x, inner.y + 3, 22, 11);
    match (&f.ready, &f.error) {
        (Some(p), _) => {
            crate::home::cover(buf, pr, &preview_key(&f.playlist), "♪", app, th);
            let tx = x + 24;
            let tw = w.saturating_sub(24);
            text(buf, tx, inner.y + 4, tw, &format!("{}×{} · {} KB JPEG", p.image.width(), p.image.height(), p.jpeg.len().div_ceil(1024)), th.s(th.fg));
            text(buf, tx, inner.y + 5, tw, "cropped square from the centre", th.s(th.dim));
            text(buf, tx, inner.y + 7, tw, "Enter uploads it", th.s(th.accent));
        }
        (None, Some(e)) => {
            text(buf, x, inner.y + 3, w, e, th.s(th.accent2));
        }
        (None, None) => {
            text(buf, x, inner.y + 3, w, "PNG, JPEG or WebP: it's cropped square and made small enough for Spotify", th.s(th.dim));
        }
    }
    let hint = if f.ready.is_some() { "Enter upload · edit the path to pick another · Esc cancel" } else { "Enter preview · Esc cancel" };
    text(buf, x, inner.bottom().saturating_sub(1), w, hint, th.s(th.dim));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, noisy: bool) -> Vec<u8> {
        let img = RgbImage::from_fn(w, h, |x, y| {
            // the left third red, so the crop shows: the centre square has none of it when wide
            if x < w / 3 && w > h {
                image::Rgb([255, 0, 0])
            } else if noisy {
                let n = (x.wrapping_mul(2654435761) ^ y.wrapping_mul(40503)).wrapping_mul(2246822519);
                image::Rgb([(n >> 8) as u8, (n >> 16) as u8, (n >> 24) as u8])
            } else {
                image::Rgb([0, 0, 200])
            }
        });
        let mut out = std::io::Cursor::new(vec![]);
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn a_picture_is_cropped_square_from_the_centre_and_fits_in_256_kb() {
        let p = prepare(&png(900, 300, false)).unwrap();
        assert_eq!(p.image.dimensions(), (300, 300), "small pictures aren't blown up");
        assert!(p.jpeg.starts_with(&[0xFF, 0xD8]));
        // the centre: no red from the left third
        assert!(p.image.pixels().all(|px| px.0[0] < 60), "cropped from the centre");
        // big and noisy (hard to compress): scaled to 640, and the quality steps down until it fits
        let p = prepare(&png(2000, 1600, true)).unwrap();
        assert!(p.image.width() <= 640 && p.image.width() == p.image.height());
        assert!(p.jpeg.len() <= MAX_BYTES, "{} bytes", p.jpeg.len());
        assert!(prepare(b"not a picture").is_err());
    }

    #[test]
    fn a_dropped_picture_claiming_to_be_huge_is_refused_like_a_downloaded_one() {
        // a 12000×12000 header would allocate ~430 MB; the same 4096²/256 MB limits as a download stop it first
        assert!(prepare(&crate::covers::huge_png_header(12_000, 12_000)).is_err());
    }

    #[test]
    fn dropped_paths_are_cleaned() {
        assert_eq!(clean_path("  '/home/u/My Pics/a.png' "), "/home/u/My Pics/a.png");
        assert_eq!(clean_path("file:///home/u/My%20Pics/a.png"), "/home/u/My Pics/a.png");
        assert_eq!(clean_path("/home/u/My\\ Pics/a.png"), "/home/u/My Pics/a.png");
        assert_eq!(clean_path("\"/x/y.jpg\""), "/x/y.jpg");
        let home = std::env::var("HOME").unwrap();
        assert_eq!(clean_path("~/a.webp"), format!("{home}/a.webp"));
    }

    #[test]
    fn a_dropped_picture_previews_uploads_shows_at_once_and_goes_back_if_refused() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent};
        use singularity_proto::{Library, LibraryItem};
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let mut app = App::new(Some(tx));
        app.link = crate::app::Link::Connected { version: "test".into() };
        let item = |uri: &str, own| LibraryItem { uri: uri.into(), name: "P".into(), subtitle: String::new(), kind: "playlist".into(), image_url: Some("https://old".into()), depth: 0, own };
        app.library = Some(Library { items: vec![item("spotify:playlist:mine", true), item("spotify:playlist:theirs", false)] });
        let sent = |rx: &mut tokio::sync::mpsc::Receiver<Command>| std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
        app.ask_cover("spotify:playlist:theirs");
        assert!(app.cover_form.is_none(), "only your own");
        app.ask_cover("spotify:playlist:mine");
        // a missing file says so
        app.paste("/nowhere/at/all.png");
        assert!(app.cover_form.as_ref().unwrap().error.as_ref().unwrap().contains("No file"));
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("my pic.png");
        std::fs::write(&file, png(400, 500, false)).unwrap();
        // dropped as a file:// URI with an encoded space
        app.paste(&format!("file://{}", file.display().to_string().replace(' ', "%20")));
        let f = app.cover_form.as_ref().unwrap();
        assert!(f.ready.as_ref().is_some_and(|p| p.image.dimensions() == (400, 400)), "{:?}", f.error);
        assert!(sent(&mut rx).is_empty(), "nothing goes before Enter");
        app.key(KeyEvent::from(KeyCode::Enter));
        let cmds = sent(&mut rx);
        assert!(matches!(&cmds[..], [Command::SetPlaylistCover { playlist, jpeg: Some(j) }] if playlist == "spotify:playlist:mine" && j.starts_with("/9j/")), "{cmds:?}");
        let url = |app: &App| app.library.as_ref().unwrap().items[0].image_url.clone();
        assert_eq!(url(&app).as_deref(), Some("local:cover:spotify:playlist:mine"), "shown at once");
        app.cover_set("spotify:playlist:mine", false);
        assert_eq!(url(&app).as_deref(), Some("https://old"), "refused: the old one back");
        app.remove_cover("spotify:playlist:mine");
        assert_eq!(sent(&mut rx), vec![Command::SetPlaylistCover { playlist: "spotify:playlist:mine".into(), jpeg: None }]);
    }
}
