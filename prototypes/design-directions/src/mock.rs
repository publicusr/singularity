//! Fake catalogue data. Artists, titles and lyrics are invented for the prototype.

pub struct Track {
    pub title: &'static str,
    pub artist: &'static str,
    pub album: &'static str,
    pub secs: u32,
    pub cover: u32,
}

pub const NOW: Track = Track { title: "Accretion Disk", artist: "Nova Drift", album: "Event Horizon", secs: 227, cover: 10 };

pub const QUEUE: &[Track] = &[
    Track { title: "Photon Sphere", artist: "Nova Drift", album: "Event Horizon", secs: 251, cover: 10 },
    Track { title: "Glass Harbor", artist: "Lumen Fields", album: "Tidewater", secs: 198, cover: 21 },
    Track { title: "Kessler Syndrome", artist: "Orbital Debris", album: "Low Earth", secs: 304, cover: 32 },
    Track { title: "Aphelion", artist: "Mira Vale", album: "Long Way Round", secs: 233, cover: 43 },
    Track { title: "Night Bus to Nowhere", artist: "Echo Valley Radio", album: "Night Bus", secs: 187, cover: 54 },
    Track { title: "Sodium Lights", artist: "The Vantablacks", album: "Sodium", secs: 215, cover: 65 },
    Track { title: "Slow Orbit", artist: "Lumen Fields", album: "Tidewater", secs: 276, cover: 21 },
    Track { title: "Redshift", artist: "Kepler & Co.", album: "Doppler", secs: 242, cover: 76 },
];

pub const LIBRARY: &[(&str, &str)] = &[
    ("♥", "Liked Songs"),
    ("◉", "Daily Mix 1"),
    ("◉", "Daily Mix 2"),
    ("✦", "Discover Weekly"),
    ("✦", "Release Radar"),
    ("♫", "night drive"),
    ("♫", "deep focus // no vocals"),
    ("♫", "gym //"),
    ("♫", "lofi rain"),
    ("♫", "sunday coffee"),
    ("◎", "Nova Drift"),
    ("◎", "Lumen Fields"),
    ("▣", "Event Horizon"),
    ("▣", "Tidewater"),
    ("◍", "The Deep Dive Pod"),
];

pub struct Shelf {
    pub title: &'static str,
    pub items: &'static [(&'static str, &'static str, u32)], // title, subtitle, cover seed
}

pub const HOME: &[Shelf] = &[
    Shelf {
        title: "Jump back in",
        items: &[
            ("night drive", "Playlist · you", 101),
            ("Event Horizon", "Album · Nova Drift", 10),
            ("Daily Mix 1", "Nova Drift, Mira Vale…", 103),
            ("deep focus", "Playlist · you", 104),
            ("Tidewater", "Album · Lumen Fields", 21),
            ("Low Earth", "Album · Orbital Debris", 32),
        ],
    },
    Shelf {
        title: "Made For You",
        items: &[
            ("Discover Weekly", "Your weekly mixtape", 201),
            ("Release Radar", "New from artists you follow", 202),
            ("Daily Mix 2", "Kepler & Co., The Vantablacks…", 203),
            ("On Repeat", "Songs you can't stop playing", 204),
            ("Time Capsule", "Songs from your past", 205),
            ("Synthwave Mix", "Neon, retro, outrun", 206),
        ],
    },
    Shelf {
        title: "Recently played",
        items: &[
            ("Sodium", "The Vantablacks", 65),
            ("Doppler", "Kepler & Co.", 76),
            ("lofi rain", "Playlist · you", 303),
            ("Night Bus", "Echo Valley Radio", 54),
            ("Long Way Round", "Mira Vale", 43),
            ("gym //", "Playlist · you", 306),
        ],
    },
];

/// (start second, line)
pub const LYRICS: &[(u32, &str)] = &[
    (0, "♪"),
    (8, "Falling slow through the amber light"),
    (13, "Every star bends to meet your eyes"),
    (18, "We were spinning at the edge of time"),
    (23, "Counting seconds that would never die"),
    (29, "♪"),
    (34, "Hold on, the horizon's calling"),
    (39, "Hold on, there's no use in stalling"),
    (44, "Light can't leave but I'm not afraid"),
    (49, "Pulled inside of the orbit we made"),
    (55, "♪"),
    (62, "Turn the dial past the morning static"),
    (67, "Every signal sounds a little erratic"),
    (72, "Gravity's a promise we can't keep"),
    (77, "Fold the universe and fall asleep"),
    (83, "Hold on, the horizon's calling"),
    (88, "Hold on, there's no use in stalling"),
    (93, "Light can't leave but I'm not afraid"),
    (98, "Pulled inside of the orbit we made"),
    (105, "♪"),
];

pub fn lyric_index(pos: f32) -> usize {
    let looped = pos % 110.0;
    LYRICS.iter().rposition(|(s, _)| *s as f32 <= looped).unwrap_or(0)
}

pub fn lyric_progress(pos: f32) -> f32 {
    let looped = pos % 110.0;
    let i = lyric_index(pos);
    let start = LYRICS[i].0 as f32;
    let end = LYRICS.get(i + 1).map(|l| l.0 as f32).unwrap_or(110.0);
    ((looped - start) / (end - start)).clamp(0.0, 1.0)
}

pub fn fmt_time(secs: f32) -> String {
    let s = secs.max(0.0) as u32;
    format!("{}:{:02}", s / 60, s % 60)
}
