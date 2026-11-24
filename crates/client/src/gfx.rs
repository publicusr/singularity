//! The two render tiers (spec D5), for anything made of pixels (cover art now, Visuals later):
//! - *cell*: half-block characters, two vertical pixels per cell in truecolor; works anywhere;
//! - *pixel*: the kitty graphics protocol (kitty, Ghostty, WezTerm), at the terminal's full resolution.
//!
//! Pixel-tier images are transmitted once and then only *placed*; moving or hiding one costs a few bytes.
//! Frames are queued while ratatui draws and written after it flushes ([`Kitty::flush`]).

use image::RgbImage;
use image::imageops::{self, FilterType};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use std::collections::HashMap;
use std::io::Write;

/// Crop `img` to the aspect ratio of `w`×`h` (centred) and resize it to exactly that size.
pub fn fit(img: &RgbImage, w: u32, h: u32) -> RgbImage {
    let (w, h) = (w.max(1), h.max(1));
    let (iw, ih) = img.dimensions();
    let (cw, ch) = if iw as u64 * h as u64 > ih as u64 * w as u64 {
        ((ih as u64 * w as u64 / h as u64) as u32, ih)
    } else {
        (iw, (iw as u64 * h as u64 / w as u64) as u32)
    };
    let crop = imageops::crop_imm(img, (iw - cw) / 2, (ih - ch) / 2, cw.max(1), ch.max(1)).to_image();
    // Lanczos keeps covers sharp at full resolution; Triangle averages well for the tiny cell-tier versions
    let filter = if w * 4 < cw { FilterType::Triangle } else { FilterType::Lanczos3 };
    imageops::resize(&crop, w, h, filter)
}

/// Cell tier: draw an image of `rect.width` × `rect.height * 2` pixels with '▀' (fg = upper pixel, bg = lower).
pub fn blit_halfblocks(img: &RgbImage, buf: &mut Buffer, rect: Rect) {
    blit_halfblocks_from(img, buf, rect, 0);
}

/// `blit_halfblocks`, starting `skip` cell rows into the image (the part of a cover scrolled off the top).
pub fn blit_halfblocks_from(img: &RgbImage, buf: &mut Buffer, rect: Rect, skip: u16) {
    let rgb = |x: u32, y: u32| {
        let p = img.get_pixel(x, y).0;
        Color::Rgb(p[0], p[1], p[2])
    };
    for cy in 0..rect.height as u32 {
        let iy = cy + skip as u32;
        for cx in 0..rect.width as u32 {
            if cx >= img.width() || iy * 2 + 1 >= img.height() {
                continue;
            }
            if let Some(cell) = buf.cell_mut((rect.x + cx as u16, rect.y + cy as u16)) {
                cell.set_char('▀').set_fg(rgb(cx, iy * 2)).set_bg(rgb(cx, iy * 2 + 1));
            }
        }
    }
}

/// The part of an image to show, in its pixels: x, y, width, height (a cover partly scrolled out of its Pane).
pub type Crop = [u32; 4];

/// `r` without the cells of `hole`: up to four rectangles (above, below, left and right of it).
fn minus(r: Rect, hole: Rect) -> Vec<Rect> {
    let i = r.intersection(hole);
    if i.is_empty() {
        return vec![r];
    }
    let parts = [
        Rect::new(r.x, r.y, r.width, i.y - r.y),
        Rect::new(r.x, i.bottom(), r.width, r.bottom() - i.bottom()),
        Rect::new(r.x, i.y, i.x - r.x, i.height),
        Rect::new(i.right(), i.y, r.right() - i.right(), i.height),
    ];
    parts.into_iter().filter(|p| !p.is_empty()).collect()
}

/// The crop that shows piece `p` of an image placed over `whole` (showing `crop` of its `dims` pixels).
fn piece_crop(whole: Rect, crop: Option<Crop>, (iw, ih): (u32, u32), p: Rect) -> Crop {
    let [sx, sy, sw, sh] = crop.unwrap_or([0, 0, iw, ih]);
    let along = |start: u32, len: u32, cells: u16, from: u16, to: u16| {
        let at = |c: u16| start + (c as u64 * len as u64 / cells.max(1) as u64) as u32;
        (at(from), at(to).saturating_sub(at(from)).max(1))
    };
    let (x, w) = along(sx, sw, whole.width, p.x - whole.x, p.right() - whole.x);
    let (y, h) = along(sy, sh, whole.height, p.y - whole.y, p.bottom() - whole.y);
    [x, y, w, h]
}

/// What a pixel-tier image is: the source and the exact pixel size it was rendered at.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ImageKey {
    pub source: String,
    pub w: u32,
    pub h: u32,
}

struct Stored {
    id: u32,
    last_used: u64,
    /// RGBA text (`place_rgba`): over cell backgrounds always, and kept to a smaller budget off screen
    text: bool,
}

/// Pixels waiting to be sent: RGB (covers) or RGBA (text over the Pane's own background).
enum Upload {
    Rgb(RgbImage),
    Rgba(u32, u32, Vec<u8>),
}

/// An animated image queued for this frame: its id, where it goes, and (when it's new) its size and pixels.
type FrameImage = (u32, Rect, Option<(u32, u32, std::sync::Arc<Vec<u8>>)>);

/// The pixel tier's bookkeeping: which images the terminal holds, and where they are placed.
pub struct Kitty {
    /// pass pixels through POSIX shared memory (local kitty only), else inline (zlib + base64)
    shm: bool,
    next_id: u32,
    frame: u64,
    stored: HashMap<ImageKey, Stored>,
    /// placements on screen: (image id, placement id) → cell rect and source crop
    placed: HashMap<(u32, u32), (Rect, Option<Crop>)>,
    /// this frame's placements and the images still to transmit
    queue: Vec<(u32, u32, Rect, Option<Crop>)>,
    uploads: Vec<(u32, Upload)>,
    /// image ids of RGBA text (`place_rgba`)
    text: std::collections::HashSet<u32>,
    /// animated images (Visuals) this frame: a new picture, or None to keep the one on screen
    frames: Vec<FrameImage>,
    /// per Visual: which of its two image ids is on screen, and where
    animated: HashMap<u32, (u32, Rect)>,
    shm_pending: Vec<(std::time::Instant, std::ffi::CString)>,
    /// numbers the shared-memory segments: each needs a name of its own, or a later one would be refused (O_EXCL)
    /// or unlinked by the sweep before kitty read it
    shm_seq: u64,
    /// place images under cell backgrounds (kitty does this; the cells under them need the default background)
    pub below_backgrounds: bool,
    /// Visuals that must stay over backgrounds (the idle takeover shows through the whole UI)
    over_backgrounds: std::collections::HashSet<u32>,
    /// Visuals drawn one layer under still images, so a still placed over them (glow's cover) is always on top
    under_stills: std::collections::HashSet<u32>,
    /// cells this frame's images leave free, so small things drawn there show whole (a dragged Widget's chip and its
    /// target's label: D87): an image over them is placed as the pieces around them instead of stepping down a tier
    pub holes: Vec<Rect>,
    /// every image's pixel size, for the crops of its pieces
    dims: HashMap<u32, (u32, u32)>,
    /// per animated Visual: the pieces it was last placed as
    animated_pieces: HashMap<u32, Vec<Rect>>,
}

/// kitty's layer under cell backgrounds: an image there shows only through cells with the default background, so
/// whatever is drawn over it with a background of its own (a popup) hides it (#41).
const BELOW_BACKGROUNDS: i32 = -1_073_741_825;

/// Images kept in the terminal before the least recently used are freed.
const MAX_STORED: usize = 300;
/// Text images (`place_rgba`) kept off screen before the least recently used are freed: they are big, and a song's
/// lines rarely come back once sung (a seek renders them again).
const MAX_TEXT: usize = 48;
/// Image ids below this are reserved for Visuals.
const FIRST_ID: u32 = 1000;

impl Kitty {
    pub fn new(shm: bool) -> Kitty {
        Kitty {
            shm,
            next_id: FIRST_ID,
            frame: 0,
            stored: HashMap::new(),
            placed: HashMap::new(),
            queue: vec![],
            uploads: vec![],
            text: Default::default(),
            frames: vec![],
            animated: HashMap::new(),
            shm_pending: vec![],
            shm_seq: 0,
            below_backgrounds: false,
            over_backgrounds: Default::default(),
            under_stills: Default::default(),
            holes: vec![],
            dims: HashMap::new(),
            animated_pieces: HashMap::new(),
        }
    }

    /// Keep Visual `id` over cell backgrounds, under text (as everything was before #41).
    pub fn keep_over_backgrounds(&mut self, id: u32) {
        self.over_backgrounds.insert(id);
    }

    /// Keep Visual `id` one layer under still images (covers), so one placed over it always shows on top.
    pub fn keep_under_stills(&mut self, id: u32) {
        self.under_stills.insert(id);
    }

    /// The z-index for image `id` (an animated Visual's two ids map back to its own).
    fn z(&self, id: u32) -> i32 {
        // text sits over the cells' backgrounds (a band under the current line shows through), under their text
        if self.text.contains(&id) {
            return -1;
        }
        let visual = if id < FIRST_ID { id / 2 } else { id };
        let z = if self.below_backgrounds && !self.over_backgrounds.contains(&visual) { BELOW_BACKGROUNDS } else { -1 };
        if id < FIRST_ID && self.under_stills.contains(&visual) { z - 1 } else { z }
    }

    /// Place the image `key` over `rect` this frame; `render` supplies its pixels if the terminal doesn't have them yet.
    /// Showing only `crop` of the image, when given (scaled into `rect`).
    pub fn place_cropped(&mut self, key: ImageKey, rect: Rect, crop: Option<Crop>, render: impl FnOnce() -> RgbImage) {
        self.place(key, rect, crop, false, || Upload::Rgb(render()));
    }

    /// `place_cropped` for RGBA pixels (`render` gives width, height and pixels): text, placed over the cells'
    /// backgrounds (in kitty too) and under their text. The caller keeps popups off it (`App::pixel_text_for`).
    pub fn place_rgba(&mut self, key: ImageKey, rect: Rect, crop: Option<Crop>, render: impl FnOnce() -> (u32, u32, Vec<u8>)) {
        self.place(key, rect, crop, true, || {
            let (w, h, px) = render();
            Upload::Rgba(w, h, px)
        });
    }

    /// Whether the terminal already holds image `key` (it would be placed without rendering).
    pub fn holds(&self, key: &ImageKey) -> bool {
        self.stored.contains_key(key)
    }

    fn place(&mut self, key: ImageKey, rect: Rect, crop: Option<Crop>, text: bool, render: impl FnOnce() -> Upload) {
        let frame = self.frame;
        let id = match self.stored.get_mut(&key) {
            Some(s) => {
                s.last_used = frame;
                s.id
            }
            None => {
                let id = self.next_id;
                self.next_id = self.next_id.wrapping_add(1).max(FIRST_ID);
                let up = render();
                self.dims.insert(id, match &up {
                    Upload::Rgb(img) => img.dimensions(),
                    Upload::Rgba(w, h, _) => (*w, *h),
                });
                self.uploads.push((id, up));
                self.stored.insert(key, Stored { id, last_used: frame, text });
                if text {
                    self.text.insert(id);
                } else {
                    self.text.remove(&id);
                }
                id
            }
        };
        // the same image may be on screen more than once; each copy gets its own placement id
        let pid = 1 + self.queue.iter().filter(|(i, ..)| *i == id).count() as u32;
        self.queue.push((id, pid, rect, crop));
    }

    /// Show a new frame of Visual `id` (< 500) over `rect`: RGBA8, `w`×`h` pixels.
    pub fn frame(&mut self, id: u32, rect: Rect, w: u32, h: u32, rgba: std::sync::Arc<Vec<u8>>) {
        debug_assert!(id < FIRST_ID / 2);
        self.frames.push((id, rect, Some((w, h, rgba))));
    }

    /// Keep Visual `id`'s current frame on screen; false if there is none at `rect` (send a new one instead).
    pub fn keep(&mut self, id: u32, rect: Rect) -> bool {
        let ok = self.animated.get(&id).is_some_and(|(_, r)| *r == rect);
        if ok {
            self.frames.push((id, rect, None));
        }
        ok
    }

    /// Write this frame's uploads, placements and removals. Call after ratatui has drawn.
    pub fn flush(&mut self, out: &mut impl Write) -> std::io::Result<()> {
        for (id, up) in std::mem::take(&mut self.uploads) {
            match up {
                Upload::Rgb(img) => self.transmit(out, id, img.dimensions(), img.as_raw(), 24, None)?,
                Upload::Rgba(w, h, px) => self.transmit(out, id, (w, h), &px, 32, None)?,
            }
        }
        let mut shown = HashMap::new();
        for (id, rect, px) in std::mem::take(&mut self.frames) {
            let old = self.animated.remove(&id);
            let Some((w, h, rgba)) = px else {
                if let Some((real, r)) = old {
                    // the same picture: placed again only if the holes over it moved
                    let pieces = self.pieces(real, r);
                    if self.animated_pieces.get(&id) != Some(&pieces) {
                        write!(out, "\x1b_Ga=d,d=i,i={real},q=2\x1b\\")?;
                        self.put_pieces(out, real, r, &pieces)?;
                        self.animated_pieces.insert(id, pieces);
                    }
                    shown.insert(id, (real, r));
                }
                continue;
            };
            // double-buffered: the new frame goes up under the other id first, then the old one is freed, so there
            // is never a moment without a picture (re-sending under the same id makes kitty drop it first: flicker)
            let real = match old {
                Some((r, _)) if r == id * 2 => id * 2 + 1,
                _ => id * 2,
            };
            self.dims.insert(real, (w, h));
            let pieces = self.pieces(real, rect);
            if pieces == [rect] {
                self.transmit(out, real, (w, h), &rgba, 32, Some(rect))?;
            } else {
                self.transmit(out, real, (w, h), &rgba, 32, None)?;
                self.put_pieces(out, real, rect, &pieces)?;
            }
            if let Some((r, _)) = old {
                write!(out, "\x1b_Ga=d,d=I,i={r},q=2\x1b\\")?;
                self.dims.remove(&r);
            }
            self.animated_pieces.insert(id, pieces);
            shown.insert(id, (real, rect));
        }
        for (r, _) in self.animated.values() {
            write!(out, "\x1b_Ga=d,d=I,i={r},q=2\x1b\\")?;
        }
        self.animated_pieces.retain(|id, _| shown.contains_key(id));
        self.animated = shown;
        let mut now: HashMap<(u32, u32), (Rect, Option<Crop>)> = HashMap::new();
        let queue: Vec<(u32, u32, Rect, Option<Crop>)> = std::mem::take(&mut self.queue).into_iter().flat_map(|(id, pid, r, crop)| {
            let pieces = self.pieces(id, r);
            let whole = pieces == [r];
            let dims = self.dims.get(&id).copied();
            pieces.into_iter().enumerate().map(move |(k, p)| if whole { (id, pid, r, crop) } else { (id, pid | ((k as u32 + 1) << 16), p, dims.map(|d| piece_crop(r, crop, d, p))) }).collect::<Vec<_>>()
        }).collect();
        for (id, pid, r, crop) in queue {
            if self.placed.get(&(id, pid)) != Some(&(r, crop)) {
                let src = crop.map(|[x, y, w, h]| format!("x={x},y={y},w={w},h={h},")).unwrap_or_default();
                // C=1: don't move the cursor; z: under text (and, in kitty, under cell backgrounds)
                write!(out, "\x1b[{};{}H\x1b_Ga=p,i={id},p={pid},{src}c={},r={},C=1,z={},q=2\x1b\\", r.y + 1, r.x + 1, r.width, r.height, self.z(id))?;
            }
            now.insert((id, pid), (r, crop));
        }
        for (id, pid) in self.placed.keys().filter(|k| !now.contains_key(k)) {
            write!(out, "\x1b_Ga=d,d=i,i={id},p={pid},q=2\x1b\\")?;
        }
        self.placed = now;
        self.evict(out)?;
        self.frame += 1;
        Ok(())
    }

    /// Image `id` over `rect` as the pieces this frame's holes leave of it (just `rect` where none is over it, or the
    /// image sits under cell backgrounds, which hide it under anything drawn there anyway).
    fn pieces(&self, id: u32, rect: Rect) -> Vec<Rect> {
        if self.z(id) <= BELOW_BACKGROUNDS || !self.dims.contains_key(&id) {
            return vec![rect];
        }
        let mut out = vec![rect];
        for h in &self.holes {
            out = out.into_iter().flat_map(|r| minus(r, *h)).collect();
        }
        out
    }

    /// Place image `id` over `rect` as `pieces` (each a crop of it).
    fn put_pieces(&self, out: &mut impl Write, id: u32, rect: Rect, pieces: &[Rect]) -> std::io::Result<()> {
        let Some(d) = self.dims.get(&id).copied() else { return Ok(()) };
        for (k, p) in pieces.iter().enumerate() {
            let [x, y, w, h] = piece_crop(rect, None, d, *p);
            write!(out, "\x1b[{};{}H\x1b_Ga=p,i={id},p={},x={x},y={y},w={w},h={h},c={},r={},C=1,z={},q=2\x1b\\", p.y + 1, p.x + 1, k + 1, p.width, p.height, self.z(id))?;
        }
        Ok(())
    }

    /// Forget everything (after a resize, which clears the screen, and on exit).
    pub fn reset(&mut self, out: &mut impl Write) -> std::io::Result<()> {
        self.stored.clear();
        self.text.clear();
        self.placed.clear();
        self.queue.clear();
        self.uploads.clear();
        self.frames.clear();
        self.animated.clear();
        self.animated_pieces.clear();
        self.dims.clear();
        write!(out, "\x1b_Ga=d,d=A,q=2\x1b\\")?;
        for (_, name) in self.shm_pending.drain(..) {
            unsafe { libc::shm_unlink(name.as_ptr()) };
        }
        Ok(())
    }

    fn evict(&mut self, out: &mut impl Write) -> std::io::Result<()> {
        let on_screen: std::collections::HashSet<u32> = self.placed.keys().map(|(i, _)| *i).collect();
        // text off screen first, to its own budget
        let texts = self.stored.values().filter(|s| s.text && !on_screen.contains(&s.id)).count();
        if texts > MAX_TEXT {
            let mut old: Vec<(u64, ImageKey)> = self.stored.iter().filter(|(_, s)| s.text && !on_screen.contains(&s.id)).map(|(k, s)| (s.last_used, k.clone())).collect();
            old.sort_by_key(|(t, _)| *t);
            for (_, k) in old.into_iter().take(texts - MAX_TEXT) {
                self.free(out, &k)?;
            }
        }
        if self.stored.len() <= MAX_STORED {
            return Ok(());
        }
        let mut old: Vec<(u64, ImageKey)> = self.stored.iter().filter(|(_, s)| !on_screen.contains(&s.id)).map(|(k, s)| (s.last_used, k.clone())).collect();
        old.sort_by_key(|(t, _)| *t);
        for (_, k) in old.into_iter().take(self.stored.len() - MAX_STORED) {
            self.free(out, &k)?;
        }
        Ok(())
    }

    fn free(&mut self, out: &mut impl Write, k: &ImageKey) -> std::io::Result<()> {
        if let Some(s) = self.stored.remove(k) {
            self.text.remove(&s.id);
            self.dims.remove(&s.id);
            write!(out, "\x1b_Ga=d,d=I,i={},q=2\x1b\\", s.id)?;
        }
        Ok(())
    }

    /// Send pixels (`fmt` 24 = RGB, 32 = RGBA). With `place`, also show them there (a=T), else only store (a=t).
    fn transmit(&mut self, out: &mut impl Write, id: u32, (w, h): (u32, u32), data: &[u8], fmt: u8, place: Option<Rect>) -> std::io::Result<()> {
        let (action, placement) = match place {
            Some(r) => {
                write!(out, "\x1b[{};{}H", r.y + 1, r.x + 1)?;
                ("T", format!(",p=1,c={},r={},C=1,z={}", r.width, r.height, self.z(id)))
            }
            None => ("t", String::new()),
        };
        if self.shm
            && let Some(name) = self.shm_put(data)
        {
            return write!(out, "\x1b_Ga={action},t=s,f={fmt},i={id},s={w},v={h},S={}{placement},q=2;{}\x1b\\", data.len(), base64(name.as_bytes()));
        }
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        z.write_all(data)?;
        let payload = base64(&z.finish()?);
        let chunks: Vec<&[u8]> = payload.as_bytes().chunks(4096).collect();
        for (i, chunk) in chunks.iter().enumerate() {
            let more = u8::from(i + 1 < chunks.len());
            if i == 0 {
                write!(out, "\x1b_Ga={action},f={fmt},o=z,i={id},s={w},v={h}{placement},q=2,m={more};")?;
            } else {
                write!(out, "\x1b_Gm={more};")?;
            }
            out.write_all(chunk)?;
            out.write_all(b"\x1b\\")?;
        }
        Ok(())
    }

    /// Put pixels in a POSIX shared-memory object for kitty to read and unlink (`t=s`). Segments kitty never
    /// picked up (a terminal that ignores the request) are swept after 2 s, so nothing piles up in /dev/shm.
    fn shm_put(&mut self, data: &[u8]) -> Option<String> {
        let now = std::time::Instant::now();
        self.shm_pending.retain(|(t, name)| {
            let keep = now.duration_since(*t).as_secs() < 2;
            if !keep {
                unsafe { libc::shm_unlink(name.as_ptr()) };
            }
            keep
        });
        self.shm_seq += 1;
        let name = format!("/sgl-{}-{}", std::process::id(), self.shm_seq);
        let cname = std::ffi::CString::new(name.clone()).ok()?;
        // SAFETY: plain POSIX calls on a fresh, exclusively created segment; every failure path unlinks it
        unsafe {
            let fd = libc::shm_open(cname.as_ptr(), libc::O_CREAT | libc::O_RDWR | libc::O_EXCL, 0o600);
            if fd < 0 {
                return None;
            }
            let ok = libc::ftruncate(fd, data.len() as libc::off_t) == 0 && {
                let p = libc::mmap(std::ptr::null_mut(), data.len(), libc::PROT_WRITE, libc::MAP_SHARED, fd, 0);
                p != libc::MAP_FAILED && {
                    std::ptr::copy_nonoverlapping(data.as_ptr(), p as *mut u8, data.len());
                    libc::munmap(p, data.len());
                    true
                }
            };
            libc::close(fd);
            if !ok {
                libc::shm_unlink(cname.as_ptr());
                return None;
            }
        }
        self.shm_pending.push((now, cname));
        Some(name)
    }
}

impl Drop for Kitty {
    fn drop(&mut self) {
        for (_, name) in self.shm_pending.drain(..) {
            unsafe { libc::shm_unlink(name.as_ptr()) };
        }
    }
}

pub fn base64(data: &[u8]) -> String {
    const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(B64[(n >> 18) as usize & 63] as char);
        s.push(B64[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> ImageKey {
        ImageKey { source: s.into(), w: 20, h: 20 }
    }

    fn flush(k: &mut Kitty) -> String {
        let mut out = vec![];
        k.flush(&mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn transmits_once_then_only_places_and_removes() {
        let mut k = Kitty::new(false);
        let r = Rect::new(2, 3, 4, 2);
        let mut renders = 0;
        k.place_cropped(key("a"), r, None, || {
            renders += 1;
            RgbImage::new(20, 20)
        });
        let first = flush(&mut k);
        assert!(first.contains("a=t,f=24,o=z,i=1000,s=20,v=20") && first.contains("a=p,i=1000,p=1,c=4,r=2"));
        assert!(first.contains("\x1b[4;3H"), "placed at the rect's cell (1-based)");
        // same place next frame: nothing to say
        k.place_cropped(key("a"), r, None, || unreachable!());
        assert_eq!(flush(&mut k), "");
        // moved: re-placed, not re-sent
        k.place_cropped(key("a"), Rect::new(2, 9, 4, 2), None, || unreachable!());
        let moved = flush(&mut k);
        assert!(moved.contains("a=p,i=1000,p=1") && !moved.contains("a=t"));
        // gone: placement deleted, image kept for later
        assert_eq!(flush(&mut k), "\x1b_Ga=d,d=i,i=1000,p=1,q=2\x1b\\");
        assert_eq!(renders, 1);
    }

    #[test]
    fn animated_frames_are_resent_and_removed_when_gone() {
        let mut k = Kitty::new(false);
        let r = Rect::new(1, 1, 3, 2);
        k.frame(1, r, 2, 2, vec![0; 16].into());
        let s = flush(&mut k);
        assert!(s.contains("\x1b[2;2H\x1b_Ga=T,f=32,o=z,i=2,s=2,v=2,p=1,c=3,r=2,C=1,z=-1"), "{s:?}");
        // the next frame goes up under the other id, and only then is the first freed
        k.frame(1, r, 2, 2, vec![0; 16].into());
        let s = flush(&mut k);
        let (up, free) = (s.find("a=T,f=32,o=z,i=3,").unwrap(), s.find("a=d,d=I,i=2,").unwrap());
        assert!(up < free, "{s:?}");
        // keeping the frame costs nothing on the wire
        assert!(k.keep(1, r) && flush(&mut k).is_empty());
        assert!(!k.keep(1, Rect::new(0, 0, 3, 2)), "moved: needs a new frame");
        assert_eq!(flush(&mut k), "\x1b_Ga=d,d=I,i=3,q=2\x1b\\", "a Visual that's gone is freed");
    }

    /// The placements in `out`: (column, row, columns, rows, crop x, y, w, h), 0-based cells.
    fn placed(out: &str) -> Vec<(u16, u16, u16, u16, [u32; 4])> {
        out.split("\x1b[")
            .filter_map(|s| {
                let (pos, rest) = s.split_once('H')?;
                let rest = rest.strip_prefix("\x1b_Ga=")?;
                let (y, x) = pos.split_once(';')?;
                let n = |k: &str| rest.split(',').find_map(|kv| kv.strip_prefix(k)?.parse::<u32>().ok());
                Some((x.parse::<u16>().ok()? - 1, y.parse::<u16>().ok()? - 1, n("c=")? as u16, n("r=")? as u16, [n("x=")?, n("y=")?, n("w=")?, n("h=")?]))
            })
            .collect()
    }

    /// D87: a dragged Widget's chip over an image. The image isn't stepped down a tier: it is placed as the pieces
    /// around the hole, each showing its own part of the picture, and whole again once the hole is gone.
    #[test]
    fn an_image_under_a_hole_is_placed_around_it() {
        let mut k = Kitty::new(false);
        let r = Rect::new(10, 5, 20, 10);
        k.place_cropped(key("a"), r, None, || RgbImage::new(20, 20));
        assert!(flush(&mut k).contains("p=1,c=20,r=10"));
        let hole = Rect::new(14, 8, 6, 1);
        k.holes = vec![hole];
        k.place_cropped(key("a"), r, None, || unreachable!());
        let out = flush(&mut k);
        let pieces = placed(&out);
        assert!(out.contains("a=d,d=i,i=1000,p=1,"), "the whole one goes: {out:?}");
        // the pieces cover the image's rect exactly, except the hole
        let cells: u32 = pieces.iter().map(|p| p.2 as u32 * p.3 as u32).sum();
        assert_eq!(cells, 20 * 10 - 6, "{pieces:?}");
        for (x, y, w, h, [cx, cy, cw, ch]) in &pieces {
            let p = Rect::new(*x, *y, *w, *h);
            assert!(p.intersection(hole).is_empty() && r.union(p) == r, "{p:?}");
            // 20 px over 20 columns, 20 px over 10 rows: a pixel a column, two a row
            assert_eq!([*cx, *cy, *cw, *ch], [(x - 10) as u32, (y - 5) as u32 * 2, *w as u32, *h as u32 * 2]);
        }
        // the hole moves on: placed again around the new one; gone: whole again
        k.holes = vec![];
        k.place_cropped(key("a"), r, None, || unreachable!());
        let out = flush(&mut k);
        assert!(out.contains("p=1,c=20,r=10") && placed(&out).is_empty() && out.matches("a=d,d=i").count() == 4, "{out:?}");
        // under cell backgrounds (kitty) the cells hide it anyway: never cut
        let mut k = Kitty::new(false);
        k.below_backgrounds = true;
        k.holes = vec![hole];
        k.place_cropped(key("a"), r, None, || RgbImage::new(20, 20));
        assert!(flush(&mut k).contains("p=1,c=20,r=10"));
        // an animated Visual too
        let mut k = Kitty::new(false);
        k.holes = vec![hole];
        k.frame(1, r, 20, 20, vec![0; 1600].into());
        let s = flush(&mut k);
        assert!(s.contains("a=t,f=32") && placed(&s).len() == 4, "{s:?}");
        k.holes = vec![];
        assert!(k.keep(1, r));
        let s = flush(&mut k);
        assert!(s.contains("a=d,d=i,i=2,") && placed(&s).len() == 1, "{s:?}");
    }

    #[test]
    fn the_same_image_twice_gets_two_placements() {
        let mut k = Kitty::new(false);
        k.place_cropped(key("a"), Rect::new(0, 0, 2, 1), None, || RgbImage::new(20, 20));
        k.place_cropped(key("a"), Rect::new(5, 0, 2, 1), None, || unreachable!());
        let s = flush(&mut k);
        assert!(s.contains("i=1000,p=1,") && s.contains("i=1000,p=2,"));
    }

    #[test]
    fn evicts_least_recently_used_images_off_screen() {
        let mut k = Kitty::new(false);
        for i in 0..MAX_STORED + 5 {
            k.place_cropped(key(&i.to_string()), Rect::new(0, 0, 1, 1), None, || RgbImage::new(1, 1));
            flush(&mut k);
        }
        assert_eq!(k.stored.len(), MAX_STORED);
        assert!(!k.stored.contains_key(&key("0")) && k.stored.contains_key(&key(&(MAX_STORED + 4).to_string())));
    }

    #[test]
    fn fit_crops_to_aspect_and_halfblocks_fill_the_rect() {
        let mut img = RgbImage::new(300, 200);
        for (x, _, p) in img.enumerate_pixels_mut() {
            *p = image::Rgb(if x < 150 { [200, 0, 0] } else { [0, 0, 200] });
        }
        let small = fit(&img, 4, 8);
        assert_eq!(small.dimensions(), (4, 8));
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 4));
        blit_halfblocks(&small, &mut buf, Rect::new(0, 0, 4, 4));
        assert_eq!(buf[(0, 0)].symbol(), "▀");
        assert!(matches!(buf[(0, 3)].fg, Color::Rgb(r, _, b) if r > b), "left half stays red");
        assert!(matches!(buf[(3, 3)].bg, Color::Rgb(r, _, b) if b > r), "right half stays blue");
    }

    #[test]
    fn every_shared_memory_upload_gets_a_segment_of_its_own() {
        // two covers and a Visual frame in one flush, nobody reading: each must still go through shared memory
        let mut k = Kitty::new(true);
        let r = Rect::new(0, 0, 4, 2);
        k.place_cropped(key("a"), r, None, || RgbImage::new(8, 8));
        k.place_cropped(key("b"), r, None, || RgbImage::new(8, 8));
        k.frame(1, r, 8, 8, std::sync::Arc::new(vec![0; 8 * 8 * 4]));
        let out = flush(&mut k);
        let names: std::collections::HashSet<&str> = out.split("t=s,").skip(1).map(|s| s.split(';').nth(1).unwrap().split('\x1b').next().unwrap()).collect();
        assert_eq!((out.matches("t=s,").count(), names.len()), (3, 3), "{out:?}");
        // a second frame later on: a new name too, so the sweep can never unlink one kitty hasn't read
        k.frame(1, r, 8, 8, std::sync::Arc::new(vec![0; 8 * 8 * 4]));
        let out = flush(&mut k);
        let name = out.split("t=s,").nth(1).unwrap().split(';').nth(1).unwrap().split('\x1b').next().unwrap();
        assert!(!names.contains(name));
    }

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(b"fo"), "Zm8=");
    }
}
