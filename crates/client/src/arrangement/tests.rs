use super::*;
use crate::arrange::history::{CAP, History};
use crate::layout::{Axis, MIN_H, MIN_W, Size, Widget as W};
use Side::*;

const AREA: Rect = Rect { x: 0, y: 1, width: 160, height: 43 };

fn id(a: &Arrangement, w: W) -> PaneId {
    a.pane_of(w).unwrap_or_else(|| panic!("no Pane shows {w}"))
}

fn rects(a: &Arrangement) -> Vec<(PaneId, Rect)> {
    a.compute(AREA).panes.into_iter().map(|(p, r)| (p.id, r)).collect()
}

#[test]
fn a_swap_trades_widgets_and_the_panes_stay_put() {
    let a = Arrangement::from(Node::classic());
    let (home, lyrics) = (id(&a, W::Home), id(&a, W::Lyrics));
    let b = a.swap(home, lyrics).unwrap();
    assert_eq!((b.widget_in(home), b.widget_in(lyrics)), (Some(W::Lyrics), Some(W::Home)));
    assert_eq!(rects(&b), rects(&a), "every outline where it was");
    assert_eq!(outline(b.plan()), outline(a.plan()));
    assert_eq!(a.widget_in(home), Some(W::Home), "the old one is untouched: an edit is a new value");
    assert_eq!(a.swap(home, home), Err(Refused::SamePane));
    assert_eq!(a.swap(home, PaneId(999)), Err(Refused::NoPane));
}

#[test]
fn a_swap_with_an_empty_pane_fills_it_and_leaves_the_other_empty() {
    let a = Arrangement::from(Node::classic());
    let q = id(&a, W::Queue);
    let a = a.split(q, Right, None, AREA).unwrap();
    let empty = a.panes().into_iter().find(|p| p.widget.is_none()).unwrap().id;
    let b = a.swap(q, empty).unwrap();
    assert_eq!((b.widget_in(q), b.widget_in(empty)), (None, Some(W::Queue)));
    assert_eq!(outline(b.plan()), outline(a.plan()));
    // two empty Panes: nothing to move
    let c = b.split(q, Bottom, None, AREA).unwrap();
    let other = c.panes().into_iter().find(|p| p.widget.is_none() && p.id != q).unwrap().id;
    assert_eq!(c.swap(q, other), Err(Refused::NothingToMove));
}

#[test]
fn every_swap_is_allowed_even_below_a_widgets_smallest_size() {
    // the gravity well (it needs 6 rows and its own shape) into Singularity's small loudness Pane
    let a = Arrangement::from(Node::singularity());
    let small = Rect::new(0, 1, 100, 25);
    assert!(a.fits(small));
    let b = a.swap(id(&a, W::Well), id(&a, W::Loudness)).unwrap();
    assert!(b.fits(small), "the floor plan is as valid as it was");
    assert_eq!(outline(b.plan()), outline(a.plan()));
}

#[test]
fn fill_puts_a_widget_not_on_screen_into_an_empty_pane() {
    let a = Arrangement::from(Node::classic());
    let a = a.split(id(&a, W::Queue), Bottom, None, AREA).unwrap();
    let empty = a.panes().into_iter().find(|p| p.widget.is_none()).unwrap().id;
    let b = a.fill(empty, W::Fft).unwrap();
    assert_eq!(b.widget_in(empty), Some(W::Fft));
    assert_eq!(outline(b.plan()), outline(a.plan()));
    assert_eq!(a.fill(empty, W::Home), Err(Refused::OnScreen(W::Home)), "each Widget at most once");
    assert_eq!(b.fill(empty, W::Rain), Err(Refused::NotEmpty(W::Fft)));
    assert_eq!(a.fill(PaneId(999), W::Rain), Err(Refused::NoPane));
}

#[test]
fn replace_shows_another_widget_and_home_is_never_replaced() {
    let a = Arrangement::from(Node::classic());
    let q = id(&a, W::Queue);
    let b = a.replace(q, W::Marquee).unwrap();
    assert_eq!(b.widget_in(q), Some(W::Marquee));
    assert!(!b.has(W::Queue), "the replaced Widget leaves the screen");
    assert_eq!(outline(b.plan()), outline(a.plan()));
    assert_eq!(a.replace(q, W::Lyrics), Err(Refused::OnScreen(W::Lyrics)));
    assert_eq!(a.replace(id(&a, W::Home), W::Rain), Err(Refused::Required(W::Home)));
}

#[test]
fn a_line_moved_past_its_limit_stops_at_the_furthest_fit() {
    let a = Arrangement::from(Node::classic());
    let d = a.compute(AREA).dividers[0].clone();
    let b = a.move_line(&d, 0, AREA);
    assert_eq!(b.compute(AREA).rect_of(W::Library).unwrap().width, MIN_W, "at the minimum, not short of it");
    assert!(b.fits(AREA));
    let c = a.move_line(&d, 40, AREA);
    assert_eq!(c.compute(AREA).rect_of(W::Library).unwrap().width, 40);
    // the gravity well's Pane goes down to the Pane minimum like any other (#48: what a Pane holds asks for nothing)
    let col = Node::split(Axis::Vertical, Size::Cells(10), Node::pane(W::Well), Node::pane(W::Queue));
    let a = Arrangement::from(Node::split(Axis::Horizontal, Size::Ratio(0.2), col, Node::pane(W::Home)));
    let d = a.compute(AREA).dividers.iter().find(|d| d.axis == Axis::Vertical).unwrap().clone();
    let b = a.move_line(&d, 0, AREA);
    assert_eq!(b.compute(AREA).rect_of(W::Well).unwrap().height, MIN_H);
}

#[test]
fn split_close_and_steps_keep_the_floor_plan_valid_or_say_why() {
    let a = Arrangement::from(Node::classic());
    assert_eq!(a.split(id(&a, W::Queue), Left, Some(W::Home), AREA), Err(Refused::OnScreen(W::Home)));
    let tiny = Rect::new(0, 1, 60, 15);
    let mut b = Arrangement::from(Node::classic());
    let mut refused = None;
    for _ in 0..10 {
        match b.split(id(&b, W::Library), Bottom, None, tiny) {
            Ok(n) => b = n,
            Err(e) => {
                refused = Some(e);
                break;
            }
        }
    }
    assert_eq!(refused, Some(Refused::NoRoom), "split until there is no room");
    assert!(b.fits(tiny));
    // D87: only Home is required; the library, the queue and lyrics close and can be replaced
    assert_eq!(a.close(id(&a, W::Home), AREA), Err(Refused::Required(W::Home)));
    assert_eq!(a.replace(id(&a, W::Home), W::Fft), Err(Refused::Required(W::Home)));
    for w in [W::Library, W::Queue, W::Lyrics] {
        assert!(a.close(id(&a, w), AREA).is_ok_and(|b| !b.has(w)), "{w} closes");
        assert!(a.replace(id(&a, w), W::Starfield).is_ok_and(|b| !b.has(w) && b.has(W::Starfield)), "{w} is replaced");
    }
    let one = Arrangement::from(Node::pane(W::Fft));
    assert_eq!(one.close(PaneId(1), AREA), Err(Refused::LastPane));
    assert_eq!(a.step_line(PaneId(999), true, AREA), Err(Refused::NoPane));
    assert!(a.step_line(id(&a, W::Library), true, AREA).is_ok());
    assert_eq!(Refused::Required(W::Home).why(|w| w.name().to_string()), "home can be moved, but never removed");
}

// ── the property test ─────────────────────────────────────────────────

/// A small seeded generator (xorshift64*), so a failing sequence can be replayed from its seed.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Widgets an edit may bring in (on screen or not: an edit must refuse the ones that are).
const POOL: [W; 18] = [W::Well, W::Hole, W::Fft, W::Loudness, W::Levels, W::Scope, W::Rain, W::Starfield, W::Stream, W::Top, W::Listening, W::Today, W::Marquee, W::Glow, W::Karaoke, W::Queue, W::Lyrics, W::Home];

/// What an Arrangement must always be, whatever got it there.
fn assert_sound(a: &Arrangement, body: Rect, file: bool, ctx: &str) {
    assert!(a.fits(body), "{ctx}: the floor plan isn't valid at {body:?}\n{a:?}");
    let names = a.widgets();
    let mut u = names.clone();
    u.sort();
    u.dedup();
    assert_eq!(u.len(), names.len(), "{ctx}: a Widget twice: {names:?}");
    assert!(names.contains(&W::Home), "{ctx}: Home is gone");
    let c = a.compute(body);
    let cells: u32 = c.panes.iter().map(|(_, r)| r.width as u32 * r.height as u32).sum();
    assert_eq!(cells, body.width as u32 * body.height as u32, "{ctx}: the Panes don't tile the screen");
    for (i, (p, r)) in c.panes.iter().enumerate() {
        assert!(r.width >= MIN_W && r.height >= MIN_H, "{ctx}: {p:?} is {r:?}");
        assert!(c.panes.iter().skip(i + 1).all(|(_, o)| r.intersection(*o).is_empty()), "{ctx}: overlap");
    }
    let doc = singularity_config::toml_edit::ser::to_document(a.plan()).unwrap();
    let back: Node = singularity_config::toml_edit::de::from_str(&doc.to_string()).unwrap();
    assert_eq!(&back, a.plan(), "{ctx}: TOML changed it");
    if file {
        let dir = tempfile::tempdir().unwrap();
        let saved = crate::layouts::save(dir.path(), "p", "singularity", "singularity", a.plan()).unwrap();
        assert_eq!(crate::layouts::load_dir(dir.path()).0, vec![saved], "{ctx}: a saved file changed it");
    }
}

/// Random 60-edit sequences of every kind of edit (swaps, fills, replacements, line moves and steps, splits with and
/// without a Widget, closes, undo and redo) on every built-in Design at terminal sizes from 60x20 to 300x90. Every
/// edit either keeps the Arrangement sound or is refused for the reason it should be; a contents edit never touches
/// the floor plan and a swap never loses or adds a Widget; undo and redo walk through exactly the states committed.
#[test]
fn random_edit_sequences_keep_every_arrangement_sound() {
    let sizes = [(60u16, 20u16), (80, 24), (100, 30), (120, 40), (160, 48), (200, 60), (300, 90)];
    let (mut sequences, mut committed, mut refused, mut skipped, mut undone, mut swaps) = (0, 0, 0, 0, 0, 0);
    for design in crate::design::DESIGNS {
        let base = Arrangement::from((design.layout)());
        for &(w, h) in &sizes {
            let body = Rect::new(0, 1, w, h - 5);
            for seed in 1..=40u64 {
                if !base.fits(body) {
                    skipped += 1;
                    continue;
                }
                sequences += 1;
                let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ ((w as u64) << 32 | h as u64));
                let mut now = base.clone();
                let mut hist: History<Arrangement> = Default::default();
                let (mut back, mut fwd): (Vec<Arrangement>, Vec<Arrangement>) = (vec![], vec![]);
                for step in 0..60 {
                    let ctx = format!("{} {w}x{h} seed {seed} step {step}", design.id);
                    let panes = now.panes();
                    let pick = |rng: &mut Rng| panes[rng.below(panes.len())].id;
                    let roll = rng.below(12);
                    if roll >= 10 {
                        let undo = roll == 10;
                        let ok = |a: &Arrangement| a.fits(body);
                        let got = if undo { hist.undo(now.clone(), ok) } else { hist.redo(now.clone(), ok) };
                        let want = if undo { back.pop() } else { fwd.pop() };
                        assert_eq!(got, want, "{ctx}: {}", if undo { "undo" } else { "redo" });
                        if let Some(a) = got {
                            if undo { fwd.push(now.clone()) } else { back.push(now.clone()) }
                            assert_sound(&a, body, false, &ctx);
                            now = a;
                            undone += 1;
                        }
                        continue;
                    }
                    let w_new = POOL[rng.below(POOL.len())];
                    let (contents, got) = match roll {
                        0..=2 => {
                            let (x, y) = (pick(&mut rng), pick(&mut rng));
                            let got = now.swap(x, y);
                            match &got {
                                Ok(n) => {
                                    let (mut a, mut b) = (now.widgets(), n.widgets());
                                    a.sort();
                                    b.sort();
                                    assert_eq!(a, b, "{ctx}: a swap changed what's on screen");
                                    assert_eq!((n.widget_in(x), n.widget_in(y)), (now.widget_in(y), now.widget_in(x)), "{ctx}: not swapped");
                                    swaps += 1;
                                }
                                Err(e) => assert!(x == y && *e == Refused::SamePane || now.widget_in(x).is_none() && now.widget_in(y).is_none() && *e == Refused::NothingToMove, "{ctx}: swap refused: {e:?}"),
                            }
                            (true, got)
                        }
                        3 => {
                            let p = pick(&mut rng);
                            let got = now.fill(p, w_new);
                            if let Err(e) = got {
                                assert!(matches!(e, Refused::NotEmpty(_)) && now.widget_in(p).is_some() || e == Refused::OnScreen(w_new) && now.has(w_new), "{ctx}: fill refused: {e:?}");
                            }
                            (true, got)
                        }
                        4 => {
                            let p = pick(&mut rng);
                            let got = now.replace(p, w_new);
                            match got {
                                Ok(ref n) => assert!(n.widget_in(p) == Some(w_new) && !now.has(w_new), "{ctx}"),
                                Err(e) => assert!(e == Refused::OnScreen(w_new) && now.has(w_new) || e == Refused::Required(W::Home) && now.widget_in(p) == Some(W::Home), "{ctx}: replace refused: {e:?}"),
                            }
                            (true, got)
                        }
                        5 | 6 => {
                            let ds = now.compute(body).dividers;
                            if ds.is_empty() {
                                continue;
                            }
                            let d = ds[rng.below(ds.len())].clone();
                            (false, Ok(now.move_line(&d, rng.below(w as usize + 20) as u16, body)))
                        }
                        7 => (false, now.step_line(pick(&mut rng), rng.below(2) == 0, body)),
                        8 => {
                            let side = [Left, Right, Top, Bottom][rng.below(4)];
                            let what = if rng.below(2) == 0 { None } else { Some(w_new) };
                            let got = now.split(pick(&mut rng), side, what, body);
                            if let Err(e) = got {
                                assert!(e == Refused::NoRoom || what.is_some_and(|x| e == Refused::OnScreen(x) && now.has(x)), "{ctx}: split refused: {e:?}");
                            }
                            (false, got)
                        }
                        _ => {
                            let p = pick(&mut rng);
                            let got = now.close(p, body);
                            if let Err(e) = got {
                                let core = required(now.widget_in(p));
                                assert!(core && matches!(e, Refused::Required(_)) || !core && matches!(e, Refused::NoRoom | Refused::LastPane), "{ctx}: close refused: {e:?}");
                            }
                            (false, got)
                        }
                    };
                    match got {
                        Ok(n) if n != now => {
                            if contents {
                                assert_eq!(outline(n.plan()), outline(now.plan()), "{ctx}: a contents edit moved the floor plan");
                                // and where the Panes are drawn: no Widget (the gravity well included) reshapes them
                                let drawn = |a: &Arrangement| a.compute(body).panes.into_iter().map(|(p, r)| (p.id, r)).collect::<Vec<_>>();
                                assert_eq!(drawn(&n), drawn(&now), "{ctx}: a contents edit moved a Pane on screen");
                            }
                            assert_sound(&n, body, committed % 6 == 0, &ctx);
                            hist.push(now.clone());
                            back.push(now.clone());
                            if back.len() > CAP {
                                back.remove(0);
                            }
                            fwd.clear();
                            now = n;
                            committed += 1;
                        }
                        Ok(_) => {}
                        Err(_) => refused += 1,
                    }
                }
                // and it still lays out, without a panic or an overlap, at any other size
                for (w2, h2) in [(40, 10), (90, 30), (250, 80)] {
                    let c = now.compute(Rect::new(0, 1, w2, h2));
                    let cells: u32 = c.panes.iter().map(|(_, r)| r.width as u32 * r.height as u32).sum();
                    assert_eq!(cells, w2 as u32 * h2 as u32, "{} {w}x{h} seed {seed} shown at {w2}x{h2}", design.id);
                }
            }
        }
    }
    eprintln!("{sequences} sequences ({skipped} skipped: the Design itself doesn't fit), {committed} edits committed ({swaps} swaps), {refused} refused, {undone} undos and redos");
    assert!(sequences >= 1000 && committed >= 12_000 && swaps >= 3_000 && undone >= 3_000, "{sequences} {committed} {swaps} {undone}");
}

/// D87: a split divides only the Pane split; every other Pane keeps its rectangle exactly (a split that could only
/// happen by squeezing a neighbour is refused). Every Pane × every side × every built-in Design × sizes.
#[test]
fn a_split_never_moves_any_other_pane() {
    let sizes = [(80u16, 24u16), (100, 30), (120, 40), (160, 48), (200, 60), (300, 80)];
    let (mut accepted, mut refused) = (0, 0);
    for design in crate::design::DESIGNS {
        let a = Arrangement::from((design.layout)());
        for &(w, h) in &sizes {
            let body = Rect::new(0, 1, w, h - 5);
            if !a.fits(body) {
                continue;
            }
            let before: Vec<(PaneId, Rect)> = a.compute(body).panes.into_iter().map(|(p, r)| (p.id, r)).collect();
            for p in a.panes() {
                for side in [Left, Right, Top, Bottom] {
                    let Ok(b) = a.split(p.id, side, None, body) else {
                        refused += 1;
                        continue;
                    };
                    accepted += 1;
                    let after = b.compute(body).panes;
                    for (id, r) in before.iter().filter(|(id, _)| *id != p.id) {
                        let now = after.iter().find(|(q, _)| q.id == *id).map(|(_, r)| *r);
                        assert_eq!(now, Some(*r), "{} {w}x{h}: splitting {:?} {side:?} moved {id:?}", design.id, p.widget);
                    }
                }
            }
        }
    }
    assert!(accepted > 300 && refused > 0, "{accepted} accepted, {refused} refused");
}
