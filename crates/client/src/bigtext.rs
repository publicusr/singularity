//! Big text on the pixel tier: a string drawn with a real font (Noto Sans, bundled; `assets/fonts`) into RGBA pixels,
//! sent as a kitty image and placed on cells like a cover. The lyrics and Karaoke Widgets use it to fill a big Pane
//! (a terminal draws every cell's text at one size; Ghostty has no text-sizing protocol to ask for more).
//!
//! An image is keyed by everything that shows in it (`key`), so the terminal is sent each one once and steady frames
//! only place it again. Glyphs are rasterized with `fontdue` and kept per size (`GLYPHS`), so a new line costs a blit
//! of glyphs already drawn. A string with a character the fonts don't have is drawn as terminal text instead
//! (`covers`).

use crate::gfx::ImageKey;
use crate::theme::Rgb;
use fontdue::{Font, FontSettings, Metrics};
use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::OnceLock;

static REGULAR: &[u8] = include_bytes!("../assets/fonts/NotoSans-Regular.ttf");
static BOLD: &[u8] = include_bytes!("../assets/fonts/NotoSans-Bold.ttf");
/// ♩♪♫♬♭♮♯ (an empty lyrics line shows a ♪)
static NOTES: &[u8] = include_bytes!("../assets/fonts/NotoSansMath-Notes.ttf");

/// Noto Sans's ascent and descent, in ems: a line's height without extra leading.
pub const ASCENT: f32 = 1.069;
pub const LINE: f32 = 1.069 + 0.293;

struct Fonts {
    regular: Font,
    bold: Font,
    notes: Font,
}

fn fonts() -> &'static Fonts {
    static F: OnceLock<Fonts> = OnceLock::new();
    F.get_or_init(|| {
        let load = |b: &'static [u8]| Font::from_bytes(b, FontSettings::default()).expect("bundled font");
        Fonts { regular: load(REGULAR), bold: load(BOLD), notes: load(NOTES) }
    })
}

/// The font that draws `c` (the notes come from their own).
fn font_for(c: char, bold: bool) -> Option<&'static Font> {
    let f = fonts();
    let main = if bold { &f.bold } else { &f.regular };
    if main.lookup_glyph_index(c) != 0 {
        Some(main)
    } else if f.notes.lookup_glyph_index(c) != 0 {
        Some(&f.notes)
    } else {
        None
    }
}

/// Whether the fonts draw every character of `s` (spaces and invisible marks aside).
pub fn covers(s: &str) -> bool {
    s.chars().all(|c| c.is_whitespace() || c.is_control() || c == '\u{200b}' || c == '\u{fe0f}' || font_for(c, false).is_some())
}

/// How far `c` moves the pen at `px` pixels to the em.
pub fn advance(c: char, px: f32, bold: bool) -> f32 {
    font_for(c, bold).map_or(0.0, |f| f.metrics(c, px).advance_width)
}

/// `s`'s width at `px`.
pub fn width(s: &str, px: f32, bold: bool) -> f32 {
    s.chars().map(|c| advance(c, px, bold)).sum()
}

/// The width of the longer of `lines` (nine in ten are no wider, or the longest when it is only a little wider; at
/// most `stretch` times the middle one's), in ems of bold big text: big text that wide spans the Pane, longer lines
/// wrap.
pub fn typical<'a>(lines: impl Iterator<Item = &'a str>, stretch: f32) -> f32 {
    let mut w: Vec<f32> = lines.filter(|l| !l.trim().is_empty()).map(|l| width(l.trim(), 1.0, true)).collect();
    if w.is_empty() {
        return 1.0;
    }
    w.sort_by(f32::total_cmp);
    let (median, p90, max) = (w[w.len() / 2], w[(w.len() * 9 / 10).min(w.len() - 1)], w[w.len() - 1]);
    // a song with many long lines still gets text its middling lines fill most of the Pane with
    (if max <= p90 * 1.15 { max } else { p90 }).min(median * stretch)
}

/// `s` wrapped into rows: the first at most `max` wide, the rest `max - indent` (the indent not included), by words; a
/// word too wide for a row is broken. `measure` gives a string's width.
pub fn wrap(s: &str, max: f32, indent: f32, measure: impl Fn(&str) -> f32) -> Vec<String> {
    let mut rows = vec![String::new()];
    let room = |n: usize| if n == 1 { max } else { (max - indent).max(1.0) };
    let space = measure(" ");
    let mut cur_w = 0.0;
    for word in s.split_whitespace() {
        let ww = measure(word);
        let need = if rows.last().unwrap().is_empty() { ww } else { cur_w + space + ww };
        if need <= room(rows.len()) {
            let cur = rows.last_mut().unwrap();
            if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(word);
            cur_w = need;
            continue;
        }
        if !rows.last().unwrap().is_empty() {
            rows.push(String::new());
            cur_w = 0.0;
        }
        if ww <= room(rows.len()) {
            rows.last_mut().unwrap().push_str(word);
            cur_w = ww;
            continue;
        }
        // a word wider than a row is broken across rows
        let mut tmp = [0u8; 4];
        for c in word.chars() {
            let cw = measure(c.encode_utf8(&mut tmp));
            if !rows.last().unwrap().is_empty() && cur_w + cw > room(rows.len()) {
                rows.push(String::new());
                cur_w = 0.0;
            }
            rows.last_mut().unwrap().push(c);
            cur_w += cw;
        }
    }
    rows
}

/// A run of text in one colour and size, `x` pixels from the image's left, on the image's baseline.
#[derive(Debug, Clone, PartialEq)]
pub struct Span<'a> {
    pub x: f32,
    pub text: &'a str,
    pub rgb: Rgb,
    pub px: f32,
    pub bold: bool,
}

/// An image of `spans`, `w`×`h` pixels, the baseline `baseline` pixels from its top: the key it is stored under.
pub fn key(spans: &[Span], w: u32, h: u32, baseline: f32) -> ImageKey {
    let mut s = std::collections::hash_map::DefaultHasher::new();
    baseline.to_bits().hash(&mut s);
    for sp in spans {
        (sp.x.to_bits(), sp.text, sp.rgb.0, sp.rgb.1, sp.rgb.2, sp.px.to_bits(), sp.bold).hash(&mut s);
    }
    ImageKey { source: format!("text:{:016x}", s.finish()), w, h }
}

type Glyph = Rc<(Metrics, Vec<u8>)>;

thread_local! {
    /// Rasterized glyphs by (character, size, bold). Cleared when it grows past a few thousand (sizes change only with
    /// the Pane, so that is rare).
    static GLYPHS: RefCell<HashMap<(char, u32, bool), Glyph>> = RefCell::new(HashMap::new());
    static RASTERIZED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many glyphs have been rasterized so far on this thread (the cache's misses; for tests).
#[cfg(test)]
pub fn rasterized() -> u64 {
    RASTERIZED.with(|r| r.get())
}

fn glyph(c: char, px: f32, bold: bool) -> Option<Glyph> {
    let font = font_for(c, bold)?;
    GLYPHS.with(|g| {
        let mut g = g.borrow_mut();
        if g.len() > 4096 {
            g.clear();
        }
        Some(
            g.entry((c, px.to_bits(), bold))
                .or_insert_with(|| {
                    RASTERIZED.with(|r| r.set(r.get() + 1));
                    Rc::new(font.rasterize(c, px))
                })
                .clone(),
        )
    })
}

/// Coverage to alpha: a touch fuller than linear, so thin strokes hold up on a dark background.
fn alpha(cov: u8) -> u8 {
    static LUT: OnceLock<[u8; 256]> = OnceLock::new();
    LUT.get_or_init(|| std::array::from_fn(|i| ((i as f32 / 255.0).powf(0.8) * 255.0).round() as u8))[cov as usize]
}

/// Draw `spans` into a transparent `w`×`h` RGBA image, on a baseline `baseline` pixels from the top.
pub fn render(spans: &[Span], w: u32, h: u32, baseline: f32) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut px = vec![0u8; w * h * 4];
    for sp in spans {
        let mut pen = sp.x;
        for c in sp.text.chars() {
            let Some(g) = glyph(c, sp.px, sp.bold) else { continue };
            let (m, cov) = (&g.0, &g.1);
            // the bitmap's top-left: `ymin` is its bottom's height over the baseline
            let gx = (pen + m.xmin as f32).round() as i64;
            let gy = (baseline - m.ymin as f32 - m.height as f32).round() as i64;
            for row in 0..m.height {
                let y = gy + row as i64;
                if y < 0 || y >= h as i64 {
                    continue;
                }
                for col in 0..m.width {
                    let x = gx + col as i64;
                    let a = cov[row * m.width + col];
                    if a == 0 || x < 0 || x >= w as i64 {
                        continue;
                    }
                    let a = alpha(a);
                    let i = (y as usize * w + x as usize) * 4;
                    if a > px[i + 3] {
                        px[i..i + 4].copy_from_slice(&[sp.rgb.0, sp.rgb.1, sp.rgb.2, a]);
                    }
                }
            }
            pen += m.advance_width;
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fonts_cover_latin_greek_cyrillic_and_the_note() {
        assert!(covers("Ünïcödé — “quotes” … żółw, ñandú, Ελλάδα, Москва ♪"));
        assert!(!covers("光が曲がる"), "no CJK: drawn as terminal text");
        assert!(!covers("emoji 🎤"));
    }

    #[test]
    fn widths_grow_with_size_and_text() {
        let a = width("hello world", 20.0, false);
        assert!(a > 0.0 && (width("hello world", 40.0, false) - 2.0 * a).abs() < 1.0);
        assert!(width("hello world", 20.0, true) > a, "bold is wider");
        assert!(width("hello world!", 20.0, false) > a);
    }

    #[test]
    fn wrapping_keeps_rows_inside_their_width() {
        let m = |s: &str| width(s, 30.0, false);
        let text = "Every line a slow orbit, and the long ones fold over onto the next row incomprehensibilities";
        for max in [40.0, 90.0, 200.0, 400.0, 2000.0] {
            let rows = wrap(text, max, 25.0, m);
            for (k, r) in rows.iter().enumerate() {
                let room = if k == 0 { max } else { max - 25.0 };
                assert!(m(r) <= room + 0.01 || r.chars().count() == 1, "{max}: {r:?}");
            }
            assert_eq!(rows.join(" ").split_whitespace().collect::<String>(), text.split_whitespace().collect::<String>(), "nothing lost at {max}");
        }
        assert_eq!(wrap("short", 1000.0, 10.0, m), ["short"]);
    }

    #[test]
    fn rendering_draws_inside_the_image_in_the_colour_given() {
        let spans = [Span { x: 2.0, text: "Hi ♪", rgb: Rgb(200, 100, 50), px: 30.0, bold: true }];
        let (w, h) = (80, 44);
        let px = render(&spans, w, h, 32.0);
        assert_eq!(px.len(), (w * h * 4) as usize);
        let lit: Vec<&[u8]> = px.chunks(4).filter(|p| p[3] > 0).collect();
        assert!(lit.len() > 100, "ink");
        assert!(lit.iter().all(|p| p[..3] == [200, 100, 50]));
        // the same picture, the same key; anything else, another
        assert_eq!(key(&spans, w, h, 32.0), key(&spans, w, h, 32.0));
        let mut other = spans.clone();
        other[0].rgb = Rgb(1, 2, 3);
        assert_ne!(key(&spans, w, h, 32.0), key(&other, w, h, 32.0));
        // glyphs are rasterized once per size
        let before = rasterized();
        let _ = render(&spans, w, h, 32.0);
        assert_eq!(rasterized(), before);
    }
}
