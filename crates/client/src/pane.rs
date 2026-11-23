//! Panes and Widgets, kept apart (D87). A **Pane** is a place in the floor plan (a leaf of the split tree in `layout`)
//! with an identity of its own, [`PaneId`], that stays while the tree around it changes; it holds a **Widget** or
//! nothing. A [`Widget`] is what a Pane shows. Focus, hit boxes and the tree's edits name Panes; titles, per-Widget
//! state (scroll, Visual settings) and the Widget pickers name Widgets, so that state follows a Widget that moves.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Declares `Widget`, `Widget::ALL` and the names saved Designs use, from one list.
macro_rules! widgets {
    ($($(#[$doc:meta])* $v:ident = $name:literal,)*) => {
        /// What a Pane shows.
        ///
        /// Adding a Widget? Add it here; the compiler then asks for its arm in `App::pane_wheel` (what the wheel does
        /// over it), in `ui::slot_info` (its title) and in the wheel test's fixture (something that overflows it).
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
        pub enum Widget {
            $($(#[$doc])* $v,)*
        }

        impl Widget {
            #[cfg_attr(not(test), allow(dead_code))]
            pub const ALL: &[Widget] = &[$(Widget::$v),*];

            /// The name a saved Design uses for it (and its dim label in the Widget list).
            pub fn name(self) -> &'static str {
                match self {
                    $(Widget::$v => $name,)*
                }
            }

            /// The Widget a saved Design names (a file may name one this build doesn't have).
            pub fn of(name: &str) -> Option<Widget> {
                match name {
                    $($name => Some(Widget::$v),)*
                    _ => None,
                }
            }
        }
    };
}

widgets! {
    Library = "library",
    /// the main Pane: what the wheel does depends on its page (`App::page_wheel`)
    Home = "home",
    Queue = "queue",
    Lyrics = "lyrics",
    Karaoke = "karaoke",
    Stream = "stream",
    NowPlaying = "nowplaying",
    Top = "top",
    Listening = "listening",
    Today = "today",
    TopWeek = "topweek",
    Marquee = "marquee",
    Glow = "glow",
    Fft = "fft",
    Loudness = "loudness",
    Levels = "levels",
    Scope = "scope",
    Rain = "rain",
    Starfield = "starfield",
    Well = "well",
    Hole = "hole",
}

impl std::fmt::Display for Widget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl Serialize for Widget {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for Widget {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Widget, D::Error> {
        let name = String::deserialize(d)?;
        Widget::of(&name).ok_or_else(|| serde::de::Error::custom(format!("no Widget is called {name:?}")))
    }
}

/// A Widget a saved Design names, or None if this build hasn't got it (that Pane then loads empty).
pub(crate) fn lenient<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Widget>, D::Error> {
    Ok(Option::<String>::deserialize(d)?.as_deref().and_then(Widget::of))
}

/// A Pane's identity: unique within its tree, kept while the tree around it changes (and in the saved Design).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PaneId(pub u32);

/// A place in the floor plan: its identity and the Widget it holds, if any.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Pane {
    /// (0 in a hand-written file without one: the tree gives it a fresh one)
    #[serde(default = "PaneId::unset")]
    pub id: PaneId,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub widget: Option<Widget>,
}

impl PaneId {
    /// No identity yet: `Node::fix_ids` gives it one.
    pub const UNSET: PaneId = PaneId(0);
    fn unset() -> PaneId {
        PaneId::UNSET
    }
}

impl Pane {
    /// Whether it holds `w`.
    pub fn holds(&self, w: Widget) -> bool {
        self.widget == Some(w)
    }
}
