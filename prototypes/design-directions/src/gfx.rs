//! Pixel framebuffers and the two render tiers:
//! - cell tier: half-block characters (2 vertical pixels per cell, truecolor)
//! - pixel tier: kitty graphics protocol (kitty, ghostty, wezterm)

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use std::io::Write;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rgb(pub f32, pub f32, pub f32);

impl Rgb {
    pub const BLACK: Rgb = Rgb(0.0, 0.0, 0.0);
    pub fn hex(h: u32) -> Rgb {
        Rgb(
            ((h >> 16) & 0xff) as f32 / 255.0,
            ((h >> 8) & 0xff) as f32 / 255.0,
            (h & 0xff) as f32 / 255.0,
        )
    }
    pub fn scale(self, k: f32) -> Rgb {
        Rgb(self.0 * k, self.1 * k, self.2 * k)
    }
    pub fn add(self, o: Rgb) -> Rgb {
        Rgb(self.0 + o.0, self.1 + o.1, self.2 + o.2)
    }
    pub fn lerp(self, o: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        Rgb(
            self.0 + (o.0 - self.0) * t,
            self.1 + (o.1 - self.1) * t,
            self.2 + (o.2 - self.2) * t,
        )
    }
    pub fn to_u8(self) -> [u8; 3] {
        // soft tonemap so bright highlights roll off instead of clipping
        let tm = |v: f32| {
            let v = v.max(0.0);
            let v = v / (1.0 + 0.15 * v);
            (v.min(1.0).powf(1.0 / 1.1) * 255.0) as u8
        };
        [tm(self.0), tm(self.1), tm(self.2)]
    }
    pub fn color(self) -> Color {
        let [r, g, b] = self.to_u8();
        Color::Rgb(r, g, b)
    }
}

/// Sample a multi-stop colour ramp at t in [0,1].
pub fn ramp(stops: &[Rgb], t: f32) -> Rgb {
    if stops.is_empty() {
        return Rgb::BLACK;
    }
    let t = t.clamp(0.0, 1.0) * (stops.len() - 1) as f32;
    let i = (t as usize).min(stops.len() - 1);
    let j = (i + 1).min(stops.len() - 1);
    stops[i].lerp(stops[j], t - i as f32)
}

pub fn hsv(h: f32, s: f32, v: f32) -> Rgb {
    let h = h.rem_euclid(1.0) * 6.0;
    let c = v * s;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let (r, g, b) = match h as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    Rgb(r + m, g + m, b + m)
}

pub fn hash2(x: i32, y: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(374761393) ^ (y as u32).wrapping_mul(668265263);
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    ((h ^ (h >> 16)) & 0xffffff) as f32 / 16777215.0
}

pub fn hash1(x: u32) -> f32 {
    hash2(x as i32, 7919)
}

/// Smooth value noise in [0,1].
pub fn noise2(x: f32, y: f32) -> f32 {
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let (xf, yf) = (x - x.floor(), y - y.floor());
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (u, v) = (s(xf), s(yf));
    let a = hash2(xi, yi);
    let b = hash2(xi + 1, yi);
    let c = hash2(xi, yi + 1);
    let d = hash2(xi + 1, yi + 1);
    a + (b - a) * u + (c - a) * v + (a - b - c + d) * u * v
}

pub fn fbm(x: f32, y: f32, oct: u32) -> f32 {
    let (mut sum, mut amp, mut f, mut norm) = (0.0, 0.5, 1.0, 0.0);
    for _ in 0..oct {
        sum += noise2(x * f, y * f) * amp;
        norm += amp;
        amp *= 0.5;
        f *= 2.03;
    }
    sum / norm
}

pub struct Fb {
    pub w: usize,
    pub h: usize,
    pub px: Vec<Rgb>,
    /// empty = opaque; otherwise one coverage value per pixel (overlays)
    pub alpha: Vec<f32>,
    /// exact straight-alpha RGBA8 from the GPU; when present it is sent to the terminal as-is
    pub raw: Vec<u8>,
}

impl Fb {
    pub fn new(w: usize, h: usize) -> Fb {
        Fb { w: w.max(1), h: h.max(1), px: vec![Rgb::BLACK; w.max(1) * h.max(1)], alpha: vec![], raw: vec![] }
    }
    pub fn get(&self, x: usize, y: usize) -> Rgb {
        self.px[y * self.w + x]
    }

    /// Cell tier: draw into the ratatui buffer with '▀' (fg = top pixel, bg = bottom pixel).
    /// The fb must be (rect.width) x (rect.height*2).
    pub fn blit_halfblocks(&self, buf: &mut Buffer, rect: Rect) {
        for cy in 0..rect.height as usize {
            for cx in 0..rect.width as usize {
                if cx >= self.w || cy * 2 + 1 >= self.h {
                    continue;
                }
                let top = self.get(cx, cy * 2);
                let bot = self.get(cx, cy * 2 + 1);
                let Some(cell) = buf.cell_mut((rect.x + cx as u16, rect.y + cy as u16)) else { continue };
                if self.alpha.is_empty() {
                    cell.set_char('▀').set_fg(top.color()).set_bg(bot.color());
                } else {
                    // overlay: only cover cells the disk/shadow actually reaches, blend onto black
                    let at = self.alpha[cy * 2 * self.w + cx];
                    let ab = self.alpha[(cy * 2 + 1) * self.w + cx];
                    if at.max(ab) < 0.18 {
                        continue;
                    }
                    cell.set_char('▀').set_fg(top.scale(at.min(1.0)).color()).set_bg(bot.scale(ab.min(1.0)).color());
                }
            }
        }
    }

    /// Nearest-neighbour integer upscale (pixel art stays crisp instead of being smeared by the terminal).
    pub fn upscale(&self, k: usize) -> Fb {
        let k = k.max(1);
        let mut o = Fb::new(self.w * k, self.h * k);
        let has_a = !self.alpha.is_empty();
        let has_raw = !self.raw.is_empty();
        if has_a {
            o.alpha = vec![0.0; o.w * o.h];
        }
        if has_raw {
            o.raw = vec![0; o.w * o.h * 4];
        }
        for y in 0..o.h {
            for x in 0..o.w {
                let (sx, sy) = (x / k, y / k);
                let si = sy * self.w + sx;
                let di = y * o.w + x;
                o.px[di] = self.px[si];
                if has_a {
                    o.alpha[di] = self.alpha[si];
                }
                if has_raw {
                    o.raw[di * 4..di * 4 + 4].copy_from_slice(&self.raw[si * 4..si * 4 + 4]);
                }
            }
        }
        o
    }

    fn bytes(&self) -> Vec<u8> {
        if !self.raw.is_empty() {
            return self.raw.clone();
        }
        let rgba = !self.alpha.is_empty();
        let mut out = Vec::with_capacity(self.w * self.h * if rgba { 4 } else { 3 });
        for (i, p) in self.px.iter().enumerate() {
            if rgba {
                let a = self.alpha[i].clamp(0.0, 1.0);
                // un-premultiply so kitty can blend
                out.extend_from_slice(&p.scale(1.0 / a.max(0.02)).to_u8());
                out.push((a * 255.0) as u8);
            } else {
                out.extend_from_slice(&p.to_u8());
            }
        }
        out
    }
}

/// A frame queued for the pixel tier, written after ratatui has flushed.
pub struct KittyImage {
    pub id: u32,
    pub rect: Rect,
    pub fb: Fb,
    /// negative = under text glyphs (so labels can sit on top of visuals)
    pub z: i32,
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(data: &[u8]) -> String {
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

pub static USE_SHM: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Put pixels in a POSIX shared-memory object; kitty reads and unlinks it (t=s). No zlib, no base64.
fn shm_put(data: &[u8]) -> Option<String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let name = format!("/sgl-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed));
    let cname = std::ffi::CString::new(name.clone()).ok()?;
    unsafe {
        let fd = libc::shm_open(cname.as_ptr(), libc::O_CREAT | libc::O_RDWR | libc::O_EXCL, 0o600);
        if fd < 0 {
            return None;
        }
        if libc::ftruncate(fd, data.len() as libc::off_t) != 0 {
            libc::close(fd);
            libc::shm_unlink(cname.as_ptr());
            return None;
        }
        let p = libc::mmap(std::ptr::null_mut(), data.len(), libc::PROT_WRITE, libc::MAP_SHARED, fd, 0);
        libc::close(fd);
        if p == libc::MAP_FAILED {
            libc::shm_unlink(cname.as_ptr());
            return None;
        }
        std::ptr::copy_nonoverlapping(data.as_ptr(), p as *mut u8, data.len());
        libc::munmap(p, data.len());
    }
    // kitty unlinks segments once it has read them; sweep any it never picked up (e.g. inside tmux)
    // so nothing accumulates in /dev/shm
    let mut q = SHM_PENDING.lock().unwrap();
    let now = std::time::Instant::now();
    while q.front().is_some_and(|(t, _)| now.duration_since(*t).as_secs_f32() > 2.0) {
        let (_, old) = q.pop_front().unwrap();
        unsafe { libc::shm_unlink(old.as_ptr()) };
    }
    q.push_back((now, cname));
    Some(name)
}

static SHM_PENDING: std::sync::Mutex<std::collections::VecDeque<(std::time::Instant, std::ffi::CString)>> =
    std::sync::Mutex::new(std::collections::VecDeque::new());

/// Unlink every segment we created that may still exist (on exit).
pub fn shm_cleanup() {
    for (_, n) in SHM_PENDING.lock().unwrap().drain(..) {
        unsafe { libc::shm_unlink(n.as_ptr()) };
    }
}

pub fn write_kitty(out: &mut impl Write, img: &KittyImage) -> std::io::Result<()> {
    use flate2::{Compression, write::ZlibEncoder};
    let fmt = if img.fb.alpha.is_empty() && img.fb.raw.is_empty() { 24 } else { 32 };
    let data = img.fb.bytes();
    if USE_SHM.load(std::sync::atomic::Ordering::Relaxed) {
        if let Some(name) = shm_put(&data) {
            write!(out, "\x1b[{};{}H", img.rect.y + 1, img.rect.x + 1)?;
            return write!(
                out,
                "\x1b_Ga=T,t=s,f={},q=2,C=1,i={},p=1,s={},v={},S={},c={},r={},z={};{}\x1b\\",
                fmt, img.id, img.fb.w, img.fb.h, data.len(), img.rect.width, img.rect.height, img.z, base64(name.as_bytes())
            );
        }
    }
    let mut z = ZlibEncoder::new(Vec::new(), Compression::fast());
    z.write_all(&data)?;
    let payload = base64(&z.finish()?);
    write!(out, "\x1b[{};{}H", img.rect.y + 1, img.rect.x + 1)?;
    let chunks: Vec<&[u8]> = payload.as_bytes().chunks(4096).collect();
    for (i, chunk) in chunks.iter().enumerate() {
        let more = if i + 1 < chunks.len() { 1 } else { 0 };
        if i == 0 {
            write!(
                out,
                "\x1b_Ga=T,f={},o=z,q=2,C=1,i={},p=1,s={},v={},c={},r={},z={},m={};",
                fmt, img.id, img.fb.w, img.fb.h, img.rect.width, img.rect.height, img.z, more
            )?;
        } else {
            write!(out, "\x1b_Gm={};", more)?;
        }
        out.write_all(chunk)?;
        out.write_all(b"\x1b\\")?;
    }
    Ok(())
}

pub fn kitty_delete(out: &mut impl Write, id: u32) -> std::io::Result<()> {
    write!(out, "\x1b_Ga=d,d=I,i={},q=2\x1b\\", id)
}

pub fn kitty_delete_all(out: &mut impl Write) -> std::io::Result<()> {
    write!(out, "\x1b_Ga=d,d=A,q=2\x1b\\")
}

/// Fill a rect with a background colour (used under kitty images and as pane backgrounds).
pub fn fill(buf: &mut Buffer, rect: Rect, bg: Color) {
    for y in rect.top()..rect.bottom() {
        for x in rect.left()..rect.right() {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_char(' ').set_bg(bg);
            }
        }
    }
}
