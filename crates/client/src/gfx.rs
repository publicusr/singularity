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
}

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
    uploads: Vec<(u32, RgbImage)>,
    /// animated images (Visuals) this frame: a new picture, or None to keep the one on screen
    frames: Vec<(u32, Rect, Option<(u32, u32, std::sync::Arc<Vec<u8>>)>)>,
    /// per Visual: which of its two image ids is on screen, and where
    animated: HashMap<u32, (u32, Rect)>,
    shm_pending: Vec<(std::time::Instant, std::ffi::CString)>,
    /// place images under cell backgrounds (kitty does this; the cells under them need the default background)
    pub below_backgrounds: bool,
    /// Visuals that must stay over backgrounds (the idle takeover shows through the whole UI)
    over_backgrounds: std::collections::HashSet<u32>,
}

/// kitty's layer under cell backgrounds: an image there shows only through cells with the default background, so
/// whatever is drawn over it with a background of its own (a popup) hides it (#41).
const BELOW_BACKGROUNDS: i32 = -1_073_741_825;

/// Images kept in the terminal before the least recently used are freed.
const MAX_STORED: usize = 300;
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
            frames: vec![],
            animated: HashMap::new(),
            shm_pending: vec![],
            below_backgrounds: false,
            over_backgrounds: Default::default(),
        }
    }

    /// Keep Visual `id` over cell backgrounds, under text (as everything was before #41).
    pub fn keep_over_backgrounds(&mut self, id: u32) {
        self.over_backgrounds.insert(id);
    }

    /// The z-index for image `id` (an animated Visual's two ids map back to its own).
    fn z(&self, id: u32) -> i32 {
        let visual = if id < FIRST_ID { id / 2 } else { id };
        if self.below_backgrounds && !self.over_backgrounds.contains(&visual) { BELOW_BACKGROUNDS } else { -1 }
    }

    /// Place the image `key` over `rect` this frame; `render` supplies its pixels if the terminal doesn't have them yet.
    /// Showing only `crop` of the image, when given (scaled into `rect`).
    pub fn place_cropped(&mut self, key: ImageKey, rect: Rect, crop: Option<Crop>, render: impl FnOnce() -> RgbImage) {
        let frame = self.frame;
        let id = match self.stored.get_mut(&key) {
            Some(s) => {
                s.last_used = frame;
                s.id
            }
            None => {
                let id = self.next_id;
                self.next_id = self.next_id.wrapping_add(1).max(FIRST_ID);
                self.uploads.push((id, render()));
                self.stored.insert(key, Stored { id, last_used: frame });
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
        for (id, img) in std::mem::take(&mut self.uploads) {
            let (w, h) = img.dimensions();
            self.transmit(out, id, (w, h), img.as_raw(), 24, None)?;
        }
        let mut shown = HashMap::new();
        for (id, rect, px) in std::mem::take(&mut self.frames) {
            let old = self.animated.remove(&id);
            let Some((w, h, rgba)) = px else {
                if let Some(o) = old {
                    shown.insert(id, o);
                }
                continue;
            };
            // double-buffered: the new frame goes up under the other id first, then the old one is freed, so there
            // is never a moment without a picture (re-sending under the same id makes kitty drop it first: flicker)
            let real = match old {
                Some((r, _)) if r == id * 2 => id * 2 + 1,
                _ => id * 2,
            };
            self.transmit(out, real, (w, h), &rgba, 32, Some(rect))?;
            if let Some((r, _)) = old {
                write!(out, "\x1b_Ga=d,d=I,i={r},q=2\x1b\\")?;
            }
            shown.insert(id, (real, rect));
        }
        for (r, _) in self.animated.values() {
            write!(out, "\x1b_Ga=d,d=I,i={r},q=2\x1b\\")?;
        }
        self.animated = shown;
        let mut now: HashMap<(u32, u32), (Rect, Option<Crop>)> = HashMap::new();
        for (id, pid, r, crop) in std::mem::take(&mut self.queue) {
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

    /// Forget everything (after a resize, which clears the screen, and on exit).
    pub fn reset(&mut self, out: &mut impl Write) -> std::io::Result<()> {
        self.stored.clear();
        self.placed.clear();
        self.queue.clear();
        self.uploads.clear();
        self.frames.clear();
        self.animated.clear();
        write!(out, "\x1b_Ga=d,d=A,q=2\x1b\\")?;
        for (_, name) in self.shm_pending.drain(..) {
            unsafe { libc::shm_unlink(name.as_ptr()) };
        }
        Ok(())
    }

    fn evict(&mut self, out: &mut impl Write) -> std::io::Result<()> {
        if self.stored.len() <= MAX_STORED {
            return Ok(());
        }
        let on_screen: std::collections::HashSet<u32> = self.placed.keys().map(|(i, _)| *i).collect();
        let mut old: Vec<(u64, ImageKey)> = self.stored.iter().filter(|(_, s)| !on_screen.contains(&s.id)).map(|(k, s)| (s.last_used, k.clone())).collect();
        old.sort_by_key(|(t, _)| *t);
        for (_, k) in old.into_iter().take(self.stored.len() - MAX_STORED) {
            if let Some(s) = self.stored.remove(&k) {
                write!(out, "\x1b_Ga=d,d=I,i={},q=2\x1b\\", s.id)?;
            }
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
        let name = format!("/sgl-{}-{}", std::process::id(), self.next_id);
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
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(b"fo"), "Zm8=");
    }
}
