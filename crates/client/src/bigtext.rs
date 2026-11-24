//! Big text on the pixel tier: a string drawn with a real font into RGBA pixels, sent as a kitty image and placed on
//! cells like a cover. The lyrics and Karaoke Widgets use it to fill a big Pane (a terminal draws every cell's text at
//! one size; Ghostty has no text-sizing protocol to ask for more).
//!
//! The font is the terminal's own (`termfont.rs`: the same lettering as the cells, only bigger); characters it lacks,
//! or no terminal font found, come from a bundled Noto Sans (`assets/fonts`).
//!
//! An image is keyed by everything that shows in it (`key`), so the terminal is sent each one once and steady frames
//! only place it again. Glyphs are rasterized with `fontdue` and kept per size (`GLYPHS`), so a new line costs a blit
//! of glyphs already drawn. A string with a character none of the fonts have is drawn as terminal text instead
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

struct Fonts {
    /// the terminal's regular and bold faces
    term: Option<(Font, Font)>,
    regular: Font,
    bold: Font,
    notes: Font,
    /// the main font's ascent and its ascent + descent, in ems
    ascent: f32,
    line: f32,
}

fn fonts() -> &'static Fonts {
    static F: OnceLock<Fonts> = OnceLock::new();
    F.get_or_init(|| {
        let load = |b: &[u8], index: u32| Font::from_bytes(b, FontSettings { collection_index: index, ..FontSettings::default() });
        let bundled = |b: &'static [u8]| load(b, 0).expect("bundled font");
        // tests measure with the bundled font only, so they don't depend on the PC's fonts
        let term = if cfg!(test) { None } else { crate::termfont::find() }
            .and_then(|t| Some((load(&t.regular.data, t.regular.index).ok()?, load(&t.bold.data, t.bold.index).ok()?)));
        let regular = bundled(REGULAR);
        let main = term.as_ref().map_or(&regular, |t| &t.0);
        // Noto Sans's ascent and descent when a font doesn't say
        let (ascent, line) = main.horizontal_line_metrics(1.0).map_or((1.069, 1.069 + 0.293), |m| (m.ascent, m.ascent - m.descent));
        Fonts { term, regular, bold: bundled(BOLD), notes: bundled(NOTES), ascent, line }
    })
}

/// Find the terminal's font now, off the drawing thread, so the first big text doesn't wait for the font scan.
pub fn warm() {
    std::thread::spawn(|| {
        fonts();
    });
}

/// The main font's ascent, in ems: the baseline's depth in a line.
pub fn ascent() -> f32 {
    fonts().ascent
}

/// A line's height in ems (ascent + descent, no extra leading).
pub fn line() -> f32 {
    fonts().line
}

/// The font that draws `c`: the terminal's, else the bundled one, else the notes'.
fn font_for(c: char, bold: bool) -> Option<&'static Font> {
    let f = fonts();
    let term = f.term.as_ref().map(|t| if bold { &t.1 } else { &t.0 });
    let main = if bold { &f.bold } else { &f.regular };
    [term, Some(main), Some(&f.notes)].into_iter().flatten().find(|font| font.lookup_glyph_index(c) != 0)
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

/// A band of colour across an image from pixel row `.1` to `.2`, under its text (the lyrics' current line).
pub type Band = Option<(Rgb, u32, u32)>;

/// An image of `spans`, `w`×`h` pixels, the baseline `baseline` pixels from its top: the key it is stored under.
pub fn key(spans: &[Span], w: u32, h: u32, baseline: f32) -> ImageKey {
    key_on(spans, w, h, baseline, None)
}

/// `key` for an image of `spans` on `band`.
pub fn key_on(spans: &[Span], w: u32, h: u32, baseline: f32, band: Band) -> ImageKey {
    let mut s = std::collections::hash_map::DefaultHasher::new();
    baseline.to_bits().hash(&mut s);
    band.map(|(c, y0, y1)| (c.0, c.1, c.2, y0, y1)).hash(&mut s);
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
    render_on(spans, w, h, baseline, None)
}

/// `render` on `band`: its rows opaque in its colour, the text blended over them.
pub fn render_on(spans: &[Span], w: u32, h: u32, baseline: f32, band: Band) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut px = vec![0u8; w * h * 4];
    if let Some((c, y0, y1)) = band {
        for p in px[(y0 as usize).min(h) * w * 4..(y1 as usize).min(h) * w * 4].chunks_mut(4) {
            p.copy_from_slice(&[c.0, c.1, c.2, 255]);
        }
    }
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
                    if px[i + 3] == 255 {
                        // over the band (or full ink): blended
                        let mix = |o: u8, n: u8| ((o as u32 * (255 - a as u32) + n as u32 * a as u32) / 255) as u8;
                        px[i] = mix(px[i], sp.rgb.0);
                        px[i + 1] = mix(px[i + 1], sp.rgb.1);
                        px[i + 2] = mix(px[i + 2], sp.rgb.2);
                    } else if a > px[i + 3] {
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

    #[test]
    fn a_band_lies_under_the_text_across_its_rows() {
        let spans = [Span { x: 2.0, text: "Hi", rgb: Rgb(250, 250, 250), px: 30.0, bold: false }];
        let (w, h, band) = (60u32, 50u32, Some((Rgb(40, 30, 20), 5, 45)));
        let px = render_on(&spans, w, h, 32.0, band);
        let at = |x: u32, y: u32| &px[((y * w + x) * 4) as usize..][..4];
        assert_eq!(at(59, 5), [40, 30, 20, 255], "the band, to the image's edge");
        assert_eq!(at(0, 44), [40, 30, 20, 255]);
        assert_eq!(at(30, 4)[3], 0, "above it, clear");
        assert_eq!(at(30, 45)[3], 0, "below it, clear");
        // the ink is blended over the band: opaque, between the band and the text's colour
        let ink: Vec<&[u8]> = px.chunks(4).filter(|p| p[0] > 40).collect();
        assert!(ink.len() > 50 && ink.iter().all(|p| p[3] == 255 && p[0] >= p[2]), "{}", ink.len());
        assert_ne!(key_on(&spans, w, h, 32.0, band), key(&spans, w, h, 32.0), "a band, another image");
    }
}
