//! A pretend Catalogue serving a recorded Home feed (`--fake-catalogue <home.json>`): UI work, screenshots and the
//! protocol tests without a Spotify login. Only in builds with the `fake` feature (the tests turn it on).

use super::*;

/// The fake Catalogue: answers every command from `home` and made-up data, with the shapes the real one sends.
pub fn spawn_fake(home: HomeFeed) -> (mpsc::Sender<CatCmd>, mpsc::Receiver<CatEvent>) {
    let (cmd_tx, mut cmd_rx) = mpsc::channel(16);
    let (ev_tx, ev_rx) = mpsc::channel(64);
    let personal = personal();
    let home = if personal { personal_home(home) } else { home };
    tokio::spawn(async move {
        let ready = CatalogueStatus::Ready { username: if personal { personal::USERNAME.into() } else { "fake".into() }, country: "SE".into(), premium: true };
        let _ = ev_tx.send(CatEvent::Status(ready.clone())).await;
        // SINGULARITY_FAKE_DELAY_MS: Home and the library take this long, like Spotify's after a fresh start (#81)
        if let Some(ms) = std::env::var("SINGULARITY_FAKE_DELAY_MS").ok().and_then(|v| v.parse().ok()) {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }
        let _ = ev_tx.send(CatEvent::Home(home.clone())).await;
        let mut lib = if personal { personal_library(&home) } else { fake_library(&home) };
        let _ = ev_tx.send(CatEvent::Library(lib.clone())).await;
        // what's been liked/saved here; the rest follows `fake_saved`
        let mut saved: HashMap<String, bool> = HashMap::new();
        let mut created = 0;
        // tracks added to each fake playlist
        let mut added: HashMap<String, Vec<String>> = HashMap::new();
        // rows taken out of each, by uid
        let mut removed: HashMap<String, Vec<String>> = HashMap::new();
        // and moved, in order: (uids, before, after)
        type Moved = (Vec<String>, Option<String>, Option<String>);
        let mut moves: HashMap<String, Vec<Moved>> = HashMap::new();
        // a Jam started here (D64): hosted on the fake Engine, one fake guest
        let mut jam: Option<singularity_proto::Jam> = None;
        let _ = ev_tx.send(CatEvent::Jam(None)).await;
        while let Some(cmd) = cmd_rx.recv().await {
            let evs = match cmd {
                CatCmd::Home => vec![CatEvent::Home(home.clone())],
                CatCmd::Login => vec![CatEvent::Status(ready.clone())],
                CatCmd::Reconnect => vec![],
                CatCmd::Logout => vec![CatEvent::Status(CatalogueStatus::LoggedOut)],
                CatCmd::Library => vec![CatEvent::Library(lib.clone())],
                CatCmd::HomeAndLibrary => vec![CatEvent::Home(home.clone()), CatEvent::Library(lib.clone())],
                // a name with "fail" in it fails, as does deleting a playlist whose URI has it
                CatCmd::CreatePlaylist { name, .. } if name.contains("fail") => {
                    // the personal account fails as Spotify does: with a URL that names the account
                    let why = if personal { format!("Couldn't create the playlist: error sending request for url (https://spclient.example.invalid/playlist/v2/user/{}/rootlist/changes)", personal::USERNAME) } else { "Couldn't create the playlist (fake failure)".into() };
                    vec![CatEvent::Error(why), CatEvent::Library(lib.clone())]
                }
                CatCmd::CreatePlaylist { name, uris } => {
                    created += 1;
                    let uri = format!("spotify:playlist:fakenew{created}");
                    let at = lib.items.iter().position(|i| i.kind != "liked").unwrap_or(lib.items.len());
                    let me = if personal { personal::DISPLAY_NAME } else { "fake" };
                    lib.items.insert(at, singularity_proto::LibraryItem { uri: uri.clone(), name: name.clone(), subtitle: format!("Playlist · {me}"), kind: "playlist".into(), image_url: None, depth: 0, own: true, authors: vec![me.into()] });
                    let mut ev = vec![CatEvent::Created { uri: uri.clone(), name: name.clone() }];
                    if !uris.is_empty() {
                        added.entry(uri.clone()).or_default().extend(uris.iter().cloned());
                        ev.push(CatEvent::Added { playlist: uri, name, uris, outcome: singularity_proto::AddOutcome::Added });
                    }
                    ev.push(CatEvent::Library(lib.clone()));
                    ev
                }
                // the fake lists hold fake0…fake39 (new ones nothing) plus what was added; others' refuse
                CatCmd::AddToPlaylist { playlist, name, uris, anyway } => {
                    use singularity_proto::AddOutcome;
                    let extra = added.entry(playlist.clone()).or_default();
                    let has = |u: &String| extra.contains(u) || (!playlist.contains(":fakenew") && u.strip_prefix("spotify:track:fake").and_then(|n| n.parse::<u32>().ok()).is_some_and(|n| n < 40));
                    let outcome = if !lib.items.iter().any(|i| i.uri == playlist && i.own) || playlist.contains("fail") {
                        AddOutcome::Failed
                    } else if !anyway && uris.iter().any(has) {
                        AddOutcome::AlreadyThere
                    } else {
                        extra.extend(uris.iter().cloned());
                        AddOutcome::Added
                    };
                    let mut ev = vec![];
                    if outcome == AddOutcome::Failed {
                        ev.push(CatEvent::Error("Couldn't add to the playlist (fake failure)".into()));
                    }
                    ev.push(CatEvent::Added { playlist, name, uris, outcome });
                    ev
                }
                CatCmd::DeletePlaylist(uri) => {
                    let mut ev = vec![];
                    if uri.contains("fail") || !lib.items.iter().any(|i| i.uri == uri && i.own) {
                        ev.push(CatEvent::Error("Couldn't delete the playlist (fake failure)".into()));
                    } else {
                        lib.items.retain(|i| i.uri != uri);
                    }
                    ev.push(CatEvent::Library(lib.clone()));
                    ev
                }
                // a name with "fail" in it is refused
                CatCmd::EditPlaylist { uri, name, description } => {
                    let ok = !name.contains("fail") && lib.items.iter().any(|i| i.uri == uri && i.own);
                    let mut ev = vec![];
                    if ok {
                        lib.items.iter_mut().filter(|i| i.uri == uri).for_each(|i| i.name = name.clone());
                    } else {
                        ev.push(CatEvent::Error("Couldn't save the playlist's details (fake failure)".into()));
                    }
                    ev.push(CatEvent::Edited { uri, name, description, ok });
                    ev.push(CatEvent::Library(lib.clone()));
                    ev
                }
                // rows of others' playlists, or with "fail" in their uid, are refused
                CatCmd::RemoveFromPlaylist { playlist, uids } => {
                    let ok = lib.items.iter().any(|i| i.uri == playlist && i.own) && !uids.iter().any(|u| u.contains("fail"));
                    let mut ev = vec![];
                    if ok {
                        removed.entry(playlist.clone()).or_default().extend(uids.iter().cloned());
                    } else {
                        ev.push(CatEvent::Error("Couldn't remove it from the playlist (fake failure)".into()));
                    }
                    ev.push(CatEvent::Removed { playlist, uids, ok });
                    ev
                }
                // others' playlists, and anything that isn't a JPEG, are refused
                CatCmd::SetCover { playlist, jpeg } => {
                    let ok = lib.items.iter().any(|i| i.uri == playlist && i.own) && jpeg.as_ref().is_none_or(|j| j.starts_with(&[0xFF, 0xD8]));
                    let mut ev = vec![];
                    if !ok {
                        ev.push(CatEvent::Error("Couldn't change the cover (fake failure)".into()));
                    }
                    ev.push(CatEvent::CoverSet { playlist, ok });
                    ev.push(CatEvent::Library(lib.clone()));
                    ev
                }
                CatCmd::MoveInPlaylist { playlist, uids, before, after } => {
                    let ok = lib.items.iter().any(|i| i.uri == playlist && i.own) && !uids.iter().any(|u| u.contains("fail"));
                    let mut ev = vec![];
                    if ok {
                        moves.entry(playlist.clone()).or_default().push((uids.clone(), before, after));
                    } else {
                        ev.push(CatEvent::Error("Couldn't move it (fake failure)".into()));
                    }
                    ev.push(CatEvent::Moved { playlist, uids, ok });
                    ev
                }
                CatCmd::Tracks(uri) => {
                    let mut pages = fake_tracks(&uri);
                    // a show: its newest episodes only, as the real one sends
                    pages.retain(|e| !matches!(e, CatEvent::Tracks(p) if p.uri.starts_with("spotify:show:") && p.offset >= library::SHOW_BATCH));
                    // what was added comes last (the fake playlists fit one page)
                    if let (Some(extra), [CatEvent::Tracks(p)]) = (added.get(&uri), pages.as_mut_slice()) {
                        let n = p.tracks.len();
                        p.tracks.extend(extra.iter().enumerate().map(|(i, u)| singularity_proto::TrackRow { uri: u.clone(), name: format!("Added {}", u.rsplit(':').next().unwrap_or("")), playable: true, duration_ms: 180_000, uid: Some(format!("fakeuid{}", n + i)), ..Default::default() }));
                        p.total = p.tracks.len() as u32;
                    }
                    if let (Some(gone), [CatEvent::Tracks(p)]) = (removed.get(&uri), pages.as_mut_slice()) {
                        p.tracks.retain(|t| t.uid.as_ref().is_none_or(|u| !gone.contains(u)));
                        p.total = p.tracks.len() as u32;
                    }
                    if let (Some(ms), [CatEvent::Tracks(p)]) = (moves.get(&uri), pages.as_mut_slice()) {
                        for (uids, before, after) in ms {
                            crate::playlists::apply_move(&mut p.tracks, |t| t.uid.as_deref(), uids, before.as_deref(), after.as_deref());
                        }
                    }
                    pages
                }
                CatCmd::PlayFrom { context_uri, index, track_uri } => vec![CatEvent::ToEngine(Command::PlayFrom { context_uri, index, track_uri })],
                // no Connect here: the fake Engine edits its own queue
                CatCmd::Queue(e) => vec![CatEvent::ToEngine(match e {
                    QueueEdit::PlayNext(uri) => Command::PlayNext { uri },
                    QueueEdit::Remove { uri, uid } => Command::RemoveFromQueue { uri, uid },
                    QueueEdit::Clear => Command::ClearQueue,
                    QueueEdit::Move { uri, uid, to, into_queue } => Command::MoveInQueue { uri, uid, to, into_queue },
                })],
                CatCmd::Lyrics { uri, .. } => vec![CatEvent::Lyrics(fake_lyrics(&uri))],
                CatCmd::Artist(uri) => vec![CatEvent::Artist(fake_artist(&uri, &home))],
                CatCmd::MoreTracks { uri, offset } => fake_tracks(&uri).into_iter().filter(|e| matches!(e, CatEvent::Tracks(p) if p.offset >= offset)).collect(),
                CatCmd::FindShow(uri) => vec![CatEvent::EpisodeShow { episode: uri, show: singularity_proto::Link { name: "Fake Show".into(), uri: "spotify:show:fakeshow".into() } }],
                CatCmd::Search(q) => vec![CatEvent::Search(fake_search(&q, &home))],
                CatCmd::Devices => vec![CatEvent::Devices(fake_devices(None)), CatEvent::Jam(jam.clone())],
                CatCmd::StartJam => {
                    jam.get_or_insert_with(if personal { personal_jam } else { fake_jam });
                    vec![CatEvent::Jam(jam.clone())]
                }
                CatCmd::EndJam => match jam.take() {
                    Some(_) => vec![CatEvent::Jam(None)],
                    None => vec![CatEvent::Error("There's no Jam to end".into()), CatEvent::Jam(None)],
                },
                CatCmd::Transfer(id) => vec![CatEvent::Devices(fake_devices(Some(&id)))],
                // the fake has no Connect: the Engine plays the track and seeks
                CatCmd::Resume(r) => {
                    let mut v = vec![CatEvent::ToEngine(Command::Play { uri: Some(r.track_uri) }), CatEvent::ToEngine(Command::Seek { position_ms: r.position_ms })];
                    if r.paused {
                        v.push(CatEvent::ToEngine(Command::Pause));
                    }
                    v
                }
                CatCmd::Remote(_) | CatCmd::Engine(_) => vec![],
                CatCmd::Query { op, vars, reply } => {
                    let _ = reply.send(fake_query(&op, &vars, &home));
                    vec![]
                }
                // anything with "fail" in its URI fails, so the Client's rollback can be seen
                CatCmd::SetSaved { uri, saved: want } => {
                    let ok = !uri.contains("fail");
                    let mut shows = None;
                    if ok {
                        saved.insert(uri.clone(), want);
                        // a followed show joins the library (#60)
                        if uri.starts_with("spotify:show:") {
                            lib.items.retain(|i| i.uri != uri);
                            if want {
                                lib.items.insert(1, singularity_proto::LibraryItem { uri: uri.clone(), name: "Fake Show".into(), subtitle: "Podcast · Fake Publisher".into(), kind: "show".into(), image_url: None, depth: 0, own: false, authors: vec!["Fake Publisher".into()] });
                            }
                            shows = Some(CatEvent::Library(lib.clone()));
                        }
                    }
                    let saved = want;
                    let mut ev = vec![CatEvent::Saved(vec![singularity_proto::SavedFlag { saved: if ok { saved } else { !saved }, uri }])];
                    if !ok {
                        ev.insert(0, CatEvent::Error("Couldn't save it (fake failure)".into()));
                    }
                    ev.extend(shows);
                    ev
                }
                CatCmd::CheckSaved(uris) => vec![CatEvent::Saved(uris.into_iter().map(|uri| singularity_proto::SavedFlag { saved: saved.get(&uri).copied().unwrap_or_else(|| fake_saved(&uri)), uri }).collect())],
                CatCmd::Describe(uris) => vec![CatEvent::Described(
                    uris.into_iter()
                        .map(|u| fake_song(&u).unwrap_or_else(|| singularity_proto::Item {
                            name: format!("Described {}", u.rsplit(':').next().unwrap_or("")),
                            uri: u,
                            artists: vec!["Fake Artist".into()],
                            artist_uris: vec!["spotify:artist:fakeartist".into()],
                            album_uri: Some("spotify:album:fake1".into()),
                            duration_ms: 180_000,
                            ..Default::default()
                        }))
                        .collect(),
                )],
            };
            for ev in evs {
                let _ = ev_tx.send(ev).await;
            }
        }
    });
    (cmd_tx, ev_rx)
}

/// The fake Catalogue's Jam: on the fake Engine, with a made-up invite link and one guest.
fn fake_jam() -> singularity_proto::Jam {
    use singularity_proto::JamMember;
    singularity_proto::Jam {
        session_id: "fakejam".into(),
        owner: true,
        host: Some("Singularity".into()),
        host_is_engine: true,
        invite: Some("https://open.spotify.com/socialsession/FakeJamToken123".into()),
        queue_only: false,
        members: vec![
            JamMember { id: "m-fake".into(), name: "fake".into(), owner: true, listening: true },
            JamMember { id: "m-guest".into(), name: "Guest Friend".into(), owner: false, listening: true },
        ],
    }
}

/// The Engine, a phone and a speaker; `active` moves when transferred to.
fn fake_devices(active: Option<&str>) -> Vec<singularity_proto::Device> {
    let active = active.unwrap_or("engine");
    let names = if personal() { [personal::ENGINE, personal::PHONE, personal::SPEAKER] } else { ["Singularity", "Pixel", "Kitchen"] };
    [("engine", names[0], "speaker"), ("phone", names[1], "smartphone"), ("kitchen", names[2], "speaker")]
        .into_iter()
        .map(|(id, name, kind)| singularity_proto::Device { id: id.into(), name: name.into(), kind: kind.into(), active: id == active, volume: Some(60), is_engine: id == "engine" })
        .collect()
}

/// `SINGULARITY_FAKE_PERSONAL=1`: the fake account carries made-up personal data (D90), for demo mode's pty check
/// that none of it is drawn: a username and display name, a "Demo" folder beside private playlists, a personal
/// Shelf, devices named after their owner, a Jam with a named guest and a secret token.
pub fn personal() -> bool {
    std::env::var("SINGULARITY_FAKE_PERSONAL").is_ok_and(|v| v == "1")
}

/// The personal account's values (all made up), which demo mode must never draw.
pub mod personal {
    pub const USERNAME: &str = "zq-fakeuser";
    pub const DISPLAY_NAME: &str = "Zara Quillfeather";
    pub const FRIEND: &str = "Bodhi Friendly";
    pub const ENGINE: &str = "Singularity on zarabox";
    pub const PHONE: &str = "Zara's Pixel";
    pub const SPEAKER: &str = "Zara's Kitchen";
    pub const DIARY: &str = "Zara's Secret Diary";
    pub const PRIVATE_FOLDER: &str = "Private Stuff";
    pub const INVITE: &str = "https://open.spotify.com/socialsession/ZqSecretJamToken42";
}

/// Home with a personal Shelf first: "Made For <display name>", holding the private playlist.
fn personal_home(mut home: HomeFeed) -> HomeFeed {
    let diary = Card { uri: "spotify:playlist:fakediary".into(), name: personal::DIARY.into(), subtitle: format!("By {}", personal::DISPLAY_NAME), image_url: None, kind: "playlist".into() };
    home.shelves.insert(1, singularity_proto::Shelf { id: "spotify:section:fakemadefor".into(), title: format!("Made For {}", personal::DISPLAY_NAME), kind: "shelf".into(), cards: vec![diary] });
    home
}

/// The personal library: Liked Songs, a "Demo" folder (the own playlist to edit and a friend's), a private folder
/// with the diary, and the usual Home-made rows after them.
fn personal_library(home: &HomeFeed) -> Library {
    use singularity_proto::LibraryItem;
    let me = personal::DISPLAY_NAME;
    let playlist = |uri: &str, name: &str, depth: u8, own: bool, who: &str| LibraryItem { uri: uri.into(), name: name.into(), subtitle: format!("Playlist · {who}"), kind: "playlist".into(), image_url: None, depth, own, authors: vec![who.into()] };
    let folder = |uri: String, name: &str| LibraryItem { uri, name: name.into(), subtitle: String::new(), kind: "folder".into(), image_url: None, depth: 0, own: false, authors: vec![] };
    let mut items = fake_library(home).items;
    items.retain(|i| i.uri != "spotify:playlist:fakemine");
    let rest = items.split_off(1);
    items.push(folder(format!("spotify:user:{}:folder:demo", personal::USERNAME), "Demo"));
    items.push(playlist("spotify:playlist:fakemine", "Fake Mixtape", 1, true, me));
    items.push(playlist("spotify:playlist:fakefriend", "Road Trip", 1, false, personal::FRIEND));
    items.push(folder(format!("spotify:user:{}:folder:private", personal::USERNAME), personal::PRIVATE_FOLDER));
    items.push(playlist("spotify:playlist:fakediary", personal::DIARY, 1, true, me));
    items.extend(rest);
    Library { items }
}

/// The personal account's Jam: on its Engine, a named guest, a secret token.
fn personal_jam() -> singularity_proto::Jam {
    use singularity_proto::JamMember;
    singularity_proto::Jam {
        host: Some(personal::ENGINE.into()),
        invite: Some(personal::INVITE.into()),
        members: vec![
            JamMember { id: "m-fake".into(), name: personal::DISPLAY_NAME.into(), owner: true, listening: true },
            JamMember { id: "m-guest".into(), name: personal::FRIEND.into(), owner: false, listening: true },
        ],
        ..fake_jam()
    }
}

/// Invented results: the Home feed's Cards whose names contain the query, and matching fake tracks.
fn fake_search(q: &str, home: &HomeFeed) -> singularity_proto::SearchResults {
    let ql = q.to_lowercase();
    let all: Vec<Card> = home.shelves.iter().flat_map(|s| s.cards.clone()).filter(|c| c.name.to_lowercase().contains(&ql) || c.subtitle.to_lowercase().contains(&ql)).collect();
    let of = |k: &str| all.iter().filter(|c| c.kind == k).cloned().collect::<Vec<_>>();
    let tracks = (0..8)
        .map(|i| singularity_proto::TrackRow { uri: format!("spotify:track:s{i}"), name: format!("{q} song {}", i + 1), artists: vec!["Nova Drift".into()], album: "Search Results".into(), duration_ms: 190_000, playable: true, ..Default::default() })
        .collect();
    singularity_proto::SearchResults { error: None, query: q.into(), top: all.iter().take(3).cloned().collect(), tracks, artists: of("artist"), albums: of("album"), playlists: of("playlist"), podcasts: fake_podcasts(), episodes: fake_show("spotify:show:fakeshow").into_iter().find_map(|e| match e {
        CatEvent::Tracks(p) => Some(p.tracks.into_iter().take(4).map(|mut t| { t.artists = vec!["Fake Show".into()]; t.artist_uris = vec!["spotify:show:fakeshow".into()]; t }).collect()),
        _ => None,
    }).unwrap_or_default(), audiobooks: fake_audiobooks() }
}

/// A book (#62), in every search.
fn fake_audiobooks() -> Vec<Card> {
    vec![Card { uri: FAKE_BOOK.into(), name: "The Long Orbit".into(), subtitle: "Listen free until Oct 31 · Ada Quill".into(), image_url: None, kind: "show".into() }]
}

/// A show and one of its older episodes (#59), in every search.
fn fake_podcasts() -> Vec<Card> {
    vec![
        Card { uri: "spotify:show:fakeshow".into(), name: "Fake Show".into(), subtitle: "Fake Publisher".into(), image_url: None, kind: "show".into() },
        Card { uri: "spotify:episode:fakeep120".into(), name: "Episode 10: Orbits and Echoes".into(), subtitle: "Fake Show".into(), image_url: None, kind: "episode".into() },
    ]
}

/// An invented artist page: top tracks, and the Home feed's first Shelves as releases.
fn fake_artist(uri: &str, home: &HomeFeed) -> singularity_proto::ArtistPage {
    let mut shelves: Vec<Shelf> = home.shelves.iter().filter(|s| s.kind == "shelf").take(3).cloned().collect();
    for (s, t) in shelves.iter_mut().zip(["Popular releases", "Albums", "Fans also like"]) {
        s.title = t.into();
    }
    let top_tracks = (0..10)
        .map(|i| singularity_proto::TrackRow {
            uri: format!("spotify:track:top{i}"),
            name: format!("Hit {}", i + 1),
            artists: vec!["Nova Drift".into()],
            artist_uris: vec![uri.into()],
            album: "Greatest Orbits".into(),
            album_uri: Some("spotify:album:fake1".into()),
            uid: None,
            progress: None,
            added_at: None,
            duration_ms: 200_000 + i * 3_000,
            playable: true,
        })
        .collect();
    singularity_proto::ArtistPage { uri: uri.into(), name: "Nova Drift".into(), monthly_listeners: Some(1_234_567), image_url: None, top_tracks, shelves }
}

/// Invented synced lyrics, one line every 4 s, for UI work. Track 2 (`fake2`) sings for the Karaoke Widget instead:
/// see `karaoke_lyrics`.
fn fake_lyrics(uri: &str) -> singularity_proto::Lyrics {
    if uri == "spotify:track:fake2" {
        return karaoke_lyrics(uri);
    }
    let words = ["Light bends where the music falls", "Every line a slow orbit, and the long ones fold over onto the next row when the Pane is narrow", "We drift past the photon sphere", "", "Nothing escapes the chorus", "Hold on to the event horizon"];
    let lines = (0..48).map(|i| singularity_proto::LyricLine { start_ms: 2_000 + i * 4_000, text: words[i as usize % words.len()].into(), ..Default::default() }).collect();
    singularity_proto::Lyrics { uri: uri.into(), synced: true, lines, source: "fake".into(), ..Default::default() }
}

/// Invented lyrics for the Karaoke Widget, a 30 s verse sung six times: lines of uneven length and pace, some with
/// every word timed (as LRCLIB's word-synced ones come) and some only by the line (their words are estimated), an
/// instrumental gap of about 8 s marked by an empty line, and a long line that wraps.
fn karaoke_lyrics(uri: &str) -> singularity_proto::Lyrics {
    use singularity_proto::{LyricLine, LyricWord};
    // SINGULARITY_FAKE_LYRICS=scripts: a verse in scripts with separate marks (Thai, Hindi) and a long CJK line whose
    // words aren't spaced, every word timed (#146)
    if std::env::var("SINGULARITY_FAKE_LYRICS").is_ok_and(|v| v == "scripts") {
        let verse: [(u64, &[&str], u64); 4] = [
            (500, &["สวัสดี ", "ครับ ", "ทุก ", "คน"], 1_000),
            (5_000, &["नमस्ते ", "दुनिया ", "गाओ ", "साथ"], 1_000),
            (9_500, &["夜空", "星屑", "光る", "風鈴", "海辺", "雨音", "花火", "月影", "雪国", "桜色"], 700),
            (17_000, &[], 0),
        ];
        let lines = (0..6u64)
            .flat_map(|round| verse.iter().map(move |(at, words, step)| (round * 20_000 + at, words, step)))
            .map(|(start_ms, words, step)| LyricLine {
                start_ms,
                text: words.concat(),
                words: words.iter().enumerate().map(|(k, w)| LyricWord { start_ms: start_ms + k as u64 * step, end_ms: None, text: (*w).into() }).collect(),
            })
            .collect();
        return singularity_proto::Lyrics { uri: uri.into(), synced: true, lines, source: "fake".into(), ..Default::default() };
    }
    // (start in the verse, ms; text; word starts relative to the line, or none: line-timed only)
    let verse: [(u64, &str, &[u64]); 6] = [
        (3_000, "Turn the lights down low", &[0, 450, 800, 1_400, 2_100]),
        (6_500, "Sing it like the night is young", &[]),
        (10_500, "Every echo finds its way back home", &[0, 600, 1_100, 1_500, 1_900, 2_200, 2_700]),
        (14_500, "", &[]),
        (22_000, "Here comes the chorus, hold on to the note and let the whole room carry it", &[]),
        (29_000, "", &[]),
    ];
    let mut lines = vec![];
    for round in 0..6u64 {
        for (at, text, starts) in verse {
            let start_ms = round * 30_000 + at;
            let words = text
                .split(' ')
                .zip(starts.iter())
                .enumerate()
                .map(|(i, (w, off))| LyricWord { start_ms: start_ms + off, end_ms: None, text: if i + 1 < starts.len() { format!("{w} ") } else { w.to_string() } })
                .collect();
            lines.push(LyricLine { start_ms, text: text.into(), words });
        }
    }
    singularity_proto::Lyrics { uri: uri.into(), synced: true, lines, source: "fake".into(), ..Default::default() }
}

/// The fake library: Liked Songs plus the Home feed's shortcuts and first Shelf.
fn fake_library(home: &HomeFeed) -> Library {
    let mut items = vec![singularity_proto::LibraryItem {
        uri: library::LIKED.into(),
        name: "Liked Songs".into(),
        subtitle: "250 songs".into(),
        kind: "liked".into(),
        image_url: None,
        depth: 0,
        own: false,
        authors: vec![],
    }];
    for c in home.shelves.iter().take(2).flat_map(|s| &s.cards).filter(|c| c.uri != "spotify:playlist:liked") {
        items.push(singularity_proto::LibraryItem { uri: c.uri.clone(), name: c.name.clone(), subtitle: c.subtitle.clone(), kind: c.kind.clone(), image_url: c.image_url.clone(), depth: 0, own: false, authors: vec![] });
    }
    // one playlist of the fake user's own, to edit
    items.push(singularity_proto::LibraryItem { uri: "spotify:playlist:fakemine".into(), name: "Fake Mixtape".into(), subtitle: "Playlist · fake".into(), kind: "playlist".into(), image_url: None, depth: 0, own: true, authors: vec!["fake".into()] });
    Library { items }
}

/// Raw query answers in Spotify's shapes, built from the fake library and tracks (the backup reads these). The fake
/// account ("fake") owns every other playlist.
fn fake_query(op: &str, vars: &Value, home: &HomeFeed) -> anyhow::Result<Value> {
    let offset = vars["offset"].as_u64().unwrap_or(0) as u32;
    let names = |a: &[String]| a.iter().map(|n| json!({ "profile": { "name": n } })).collect::<Vec<_>>();
    match op {
        "libraryV3" => {
            let items: Vec<Value> = fake_library(home)
                .items
                .iter()
                .filter(|i| i.kind == "playlist" && vars["folderUri"].is_null())
                .enumerate()
                .map(|(n, i)| {
                    let owner = if n % 2 == 0 { "fake" } else { "someone" };
                    json!({ "item": { "data": { "__typename": "Playlist", "uri": i.uri, "name": i.name, "revisionId": "fake",
                        "ownerV2": { "data": { "name": owner, "username": owner } } } } })
                })
                .collect();
            Ok(json!({ "data": { "me": { "libraryV3": { "totalCount": items.len(), "items": items } } } }))
        }
        "fetchLibraryTracks" | "fetchPlaylist" => {
            let uri = vars["uri"].as_str().unwrap_or(library::LIKED);
            let page = fake_tracks(uri).into_iter().find_map(|e| match e {
                CatEvent::Tracks(p) if p.offset == offset => Some(p),
                _ => None,
            });
            let (total, rows) = page.map(|p| (p.total, p.tracks)).unwrap_or_default();
            let items: Vec<Value> = rows
                .iter()
                .map(|t| json!({ "itemV2": { "data": { "__typename": "Track", "uri": t.uri, "name": t.name, "artists": { "items": names(&t.artists) },
                    "albumOfTrack": { "name": t.album } } }, "addedAt": { "isoString": t.added_at } }))
                .collect();
            Ok(if op == "fetchPlaylist" {
                json!({ "data": { "playlistV2": { "content": { "totalCount": total, "items": items } } } })
            } else {
                // Liked Songs' shape: {track: {_uri, data}}
                let items: Vec<Value> = items.into_iter().map(|i| json!({ "track": { "_uri": i["itemV2"]["data"]["uri"], "data": i["itemV2"]["data"] }, "addedAt": i["addedAt"] })).collect();
                json!({ "data": { "me": { "library": { "tracks": { "totalCount": total, "items": items } } } } })
            })
        }
        _ => anyhow::bail!("the fake Catalogue doesn't answer {op}"),
    }
}

/// A made-up song for `spotify:track:song<n>` (a full queue for the contact sheet, #59): a title, an artist, an album,
/// a length and a cover that differ from song to song, as a real queue's do.
fn fake_song(uri: &str) -> Option<singularity_proto::Item> {
    let n: usize = uri.strip_prefix("spotify:track:song")?.parse().ok()?;
    let first = ["Glass", "Slow", "Neon", "Paper", "Quiet", "Silver", "Hollow", "Velvet", "Static", "Northern", "Golden", "Distant"];
    let second = ["Orbit", "Harbour", "Tide", "Signals", "Lanterns", "Weather", "Rooms", "Satellites", "Gardens", "Echoes", "Summer", "Engines", "Light"];
    let artists = ["Nova Drift", "Mira Vale", "Lumen Fields", "Glass Harbour", "Polar Static", "The Low Hours", "Ada Moreno & The Quiet Machines"];
    let title = format!("{} {}", first[n % first.len()], second[(n * 5) % second.len()]);
    let album = (n * 5) % 11;
    Some(singularity_proto::Item {
        uri: uri.into(),
        // now and then a long title, as real ones have
        name: if n % 7 == 3 { format!("{title} (Extended Night Version)") } else { title },
        artists: vec![artists[n % artists.len()].into()],
        artist_uris: vec![format!("spotify:artist:fake{}", n % artists.len())],
        album: Some(format!("{} {}", second[(album * 3) % second.len()], ["EP", "LP", "Sessions"][album % 3])),
        album_uri: Some(format!("spotify:album:fakesong{album}")),
        cover_url: Some(format!("https://i.scdn.co/image/fixture{:03}", album * 7 % 30)),
        duration_ms: 125_000 + (n as u64 * 91_331) % 230_000,
        ..Default::default()
    })
}

/// The fake library before any change: the fake tracks are all in Liked Songs; other things are saved when their URI
/// ends in 1.
fn fake_saved(uri: &str) -> bool {
    uri.strip_prefix("spotify:track:fake").is_some_and(|n| n.parse::<u32>().is_ok_and(|n| n < 250)) || uri.ends_with('1')
}

/// Invented tracks in pages of 100: 250 for Liked Songs, 40 elsewhere.
fn fake_tracks(uri: &str) -> Vec<CatEvent> {
    if uri == FAKE_BOOK {
        return fake_book();
    }
    if uri.starts_with("spotify:show:") {
        return fake_show(uri);
    }
    if uri.contains(":fakenew") {
        return vec![CatEvent::Tracks(TrackPage { uri: uri.into(), offset: 0, total: 0, tracks: vec![], header: None })];
    }
    let total = if uri == library::LIKED { 250 } else { 40 };
    let artists = ["Nova Drift", "Mira Vale", "Lumen Fields", "Glass Harbour", "Polar Static"];
    (0..total)
        .step_by(100)
        .map(|offset| {
            let tracks = (offset..(offset + 100).min(total))
                .map(|i| singularity_proto::TrackRow {
                    uri: format!("spotify:track:fake{i}"),
                    name: format!("Fake Track {:03}", i + 1),
                    artists: vec![artists[i as usize % artists.len()].into()],
                    album: format!("Album {}", (i * 7) % 13 + 1),
                    added_at: Some(format!("2025-{:02}-{:02}T12:00:00Z", 1 + (i % 12), 1 + (i * 3) % 28)),
                    duration_ms: 120_000 + (i as u64 * 7919) % 240_000,
                    playable: i % 17 != 16,
                    artist_uris: vec![format!("spotify:artist:fake{}", i as usize % artists.len())],
                    album_uri: Some(format!("spotify:album:fake{}", (i * 7) % 13 + 1)),
                    uid: uri.contains(":playlist:").then(|| format!("fakeuid{i}")),
                    progress: None,
                })
                .collect();
            let header = (offset == 0 && uri.contains(":album:")).then(|| singularity_proto::ListHeader {
                subtitle: "Album · 2024 · Fake Records".into(),
                artists: vec![singularity_proto::Link { name: "Nova Drift".into(), uri: "spotify:artist:fake0".into() }],
                cover_url: None,
                description: String::new(),
                book: false,
            });
            let header = header.or_else(|| (offset == 0 && uri == "spotify:playlist:fakemine").then(|| singularity_proto::ListHeader { subtitle: "Playlist · fake".into(), description: "Made up for the pty".into(), ..Default::default() }));
            CatEvent::Tracks(TrackPage { uri: uri.into(), offset, total, tracks, header })
        })
        .collect()
}

const FAKE_BOOK: &str = "spotify:show:fakebook";

/// A fake audiobook (#62): 24 chapters, the first finished and the second half-way.
fn fake_book() -> Vec<CatEvent> {
    let tracks = (0..24u32)
        .map(|i| singularity_proto::TrackRow {
            uri: format!("spotify:episode:fakech{i}"),
            name: if i == 0 { "Opening Credits".into() } else { format!("Chapter {i}: The Long Orbit") },
            duration_ms: if i == 0 { 23_000 } else { 1_200_000 + (i as u64 * 104_729) % 900_000 },
            playable: true,
            progress: Some(match i {
                0 => singularity_proto::Progress { position_ms: 0, done: true },
                1 => singularity_proto::Progress { position_ms: 600_000, done: false },
                _ => singularity_proto::Progress::default(),
            }),
            ..Default::default()
        })
        .collect();
    let header = singularity_proto::ListHeader {
        subtitle: "Audiobook · Ada Quill · Listen free until Oct 31 · narrated by Rowan Tide · 9 h 12 min".into(),
        description: "A ship, a slow orbit, and the long way home.".into(),
        book: true,
        ..Default::default()
    };
    vec![CatEvent::Tracks(TrackPage { uri: FAKE_BOOK.into(), offset: 0, total: 24, tracks, header: Some(header) })]
}

/// A fake show (#59): 130 episodes, newest first, the first three started, finished, and half-way; a page each
/// for the first `SHOW_BATCH` (so `MoreTracks` has something left to fetch).
fn fake_show(uri: &str) -> Vec<CatEvent> {
    let total = 130u32;
    let pages: Vec<(u32, u32)> = vec![(0, 50), (50, 100), (100, 130)];
    pages
        .into_iter()
        .map(|(from, to)| {
            let tracks = (from..to)
                .map(|i| singularity_proto::TrackRow {
                    uri: format!("spotify:episode:fakeep{i}"),
                    name: format!("Episode {}: Orbits and Echoes", total - i),
                    added_at: Some(format!("2026-{:02}-{:02}T06:00:00Z", 10 - (i / 28).min(9), 28 - i % 28)),
                    duration_ms: 1_800_000 + (i as u64 * 104_729) % 4_000_000,
                    playable: true,
                    progress: Some(match i {
                        0 => singularity_proto::Progress { position_ms: 600_000, done: false },
                        1 => singularity_proto::Progress { position_ms: 0, done: true },
                        _ => singularity_proto::Progress::default(),
                    }),
                    ..Default::default()
                })
                .collect();
            let header = (from == 0).then(|| singularity_proto::ListHeader { subtitle: "Podcast · Fake Publisher".into(), description: "Conversations about light and gravity.".into(), ..Default::default() });
            CatEvent::Tracks(TrackPage { uri: uri.into(), offset: from, total, tracks, header })
        })
        .collect()
}
