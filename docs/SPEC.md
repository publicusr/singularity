# Singularity v1: spec

> Synthesised from the grilling rounds (1–4) and the Phase 1 spikes, 2026-10-17 (Matt Pocock's `to-spec` step).
> Vocabulary follows [`GLOSSARY.md`](../GLOSSARY.md); hard decisions are ADRs in [`adr/`](adr/); decision history is in
> [`DESIGN-TREE.md`](DESIGN-TREE.md). Tickets: GitHub issues labelled `ready-for-agent`, linked from the spec issue.

## Problem Statement

I live in the terminal and want Spotify there too, but every terminal client fails me:

- spotify-tui is unmaintained;
- spotify_player is awkward, has no Home page, and **doesn't work on my machine**: since Spotify's 2026 Development
  Mode quota, every Web API call it makes comes back 429;
- none of them plays lossless audio, none feels like the real app (Home, Made For You, radio, settings), and they expect
  me to memorise a wall of keybindings while being clumsy with a mouse;
- they look plain. I want the "hacker terminal" aesthetic (a live black hole in a corner of the screen) as a built-in,
  customisable part of the app, with btop-grade theming.

## Solution

**Singularity**: a terminal Spotify client for Linux (then macOS) that feels like the desktop app, plays lossless
audio, and looks spectacular.

- **The real Spotify experience:** Home feed with all its Shelves, Made For You, radio, search, library, album and artist
  pages, queue, devices, lyrics. It comes from Spotify's private APIs (the only source of Home), with no developer app and no quota.
- **Official lossless playback:** audio plays through Spotify Soloist, Spotify's own headless engine (FLAC, verified),
  which Singularity downloads, pairs, supervises and silently keeps updated.
- **Music never depends on the window:** a background Daemon owns playback; terminal Clients come and go.
- **Mouse and keyboard as equals:** everything is clickable, scrollable and draggable. A command palette and on-screen
  hints mean nothing has to be memorised.
- **Built-in, audio-reactive Visuals that never get in the way:** a GPU-rendered black hole in its own gravity-well
  Pane, spectrum, loudness history and lyrics. When the app is idle, the hole breaks out and swallows the UI until you
  touch a key.
- **btop-style theming:** five designs (Singularity, Event Horizon, Classic+, Cockpit, Terminal), all OLED black,
  palette variants, theme files you can share.

## User Stories

### Getting started
1. As a new user, I want a guided first-run setup (Soloist key → keyring, Spotify login, pairing from my phone, audio check), so that I get sound without reading a README.
2. As a user, I want setup to say up front what's required (Premium, a Soloist key, a phone on the same network once), so that I'm never surprised.
3. As a user, I want to stay logged in and paired across restarts, so that launching is instant.
4. As a user, I want the playback engine to update itself before its 90-day expiry, between tracks, so that I never notice the limit.
5. As a user, I want an in-app health check (Engine, Catalogue session, network, audio device, stream quality), so that I can fix problems without logs.
6. As a user, I want Singularity to tell me when my device isn't set to Lossless and how to fix it, so that I actually get the quality I pay for.

### Home & discovery
7. As a listener, I want a Home page with the same Shelves as the Spotify app (shortcuts, Made For You, Recently played, Your top mixes, New releases, Jump back in, Recommended stations, "For fans of…"), so that I can start music in one click.
8. As a listener, I want cover art on Shelf cards at full resolution, so that I recognise things at a glance.
9. As a listener, I want to keep scrolling into the endless feed below the main Shelves, so that I can discover more.
10. As a listener, I want radio from any track, artist, album or playlist, so that similar music keeps going.
11. As a listener, I want autoplay when my queue runs out, with autoplay items marked as such, so that the music never stops unexpectedly.

### Search
12. As a user, I want instant search across tracks, artists, albums, playlists and podcasts, so that I find anything fast.
13. As a user, I want to filter results by type, so that I can narrow down quickly.
14. As a user, I want my recent searches remembered, so that I can repeat them.

### Library
15. As a user, I want Liked Songs, my playlists (with folders), saved albums and followed artists in a sidebar, so that navigation feels familiar.
16. As a user, I want to like/unlike the current track in one click or key, so that saving music is effortless.
17. As a user, I want to sort and filter long track lists (title, artist, album, date added, duration), so that I find tracks inside them.
18. As a user, I want to follow/unfollow artists and playlists, so that my library stays current.
19. As a user, I want to create, rename, delete and reorder playlists and their tracks (v1.x), so that I curate without another app.

### Entity pages
20. As a listener, I want artist pages (popular tracks, discography, appears on, related), so that I can explore an artist.
21. As a listener, I want album pages (tracks, release date, label, total length), so that I can listen to whole records.
22. As a listener, I want to start a playlist or album from any track in it, so that I can jump in where I like.

### Playback & queue
23. As a listener, I want play/pause, next/previous, seek, shuffle, repeat and volume always visible and clickable, so that basic control needs no shortcuts.
24. As a listener, I want to drag the seek bar and volume, so that it behaves like a GUI player.
25. As a listener, I want to see and manage the queue (add, play next, remove, clear), so that I control what comes up.
26. As an audiophile, I want lossless FLAC streaming, so that I hear full quality.
27. As an audiophile, I want to see the live codec and bitrate (a LOSSLESS badge and a stream-stats panel), so that I know what I'm getting.
28. As a listener, I want media keys, KDE's media widget and `playerctl` to control Singularity (MPRIS), so that it integrates with my desktop.
29. As a listener, I want optional desktop notifications on track change, so that I know what's playing when the terminal is hidden.
30. As a listener, I want music to keep playing when I close every terminal, so that the window is not the player.

### Devices
31. As a listener, I want to see all my Spotify Connect devices and transfer playback between them, so that music moves between rooms.
32. As a listener, I want to control my phone's or speaker's playback from Singularity, so that it doubles as a remote.

### Lyrics
33. As a listener, I want time-synced lyrics that highlight the current line, so that I can follow along.
34. As a listener, I want to click a lyric line to seek there, so that I can jump to a part of the song.
35. As a listener, I want lyrics to fall back to LRCLIB when Spotify has none, so that more songs have lyrics.

### Visuals
36. As a user, I want the black hole in its own gravity-well Pane, its gravity bending the borders around it but never my content, so that it looks incredible without costing usability.
37. As a user, I want to change the hole's Disk look (Pixel ember, Inferno, Gargantua, M87*, Quasar, Blazar, Face-on ember) by clicking it or pressing `v`, so that I can make it mine.
38. As a user, I want, after 3 idle minutes, the hole to break out of its Pane and swallow the UI, ending the moment I touch anything, so that the app doubles as a screensaver.
39. As a user, I want to switch the idle takeover off or change its delay (Shift+I, palette, config), so that it never annoys me.
40. As a user, I want a cava-style spectrum, loudness history, VU meters and a waveform seek bar that react to what's playing, so that the UI feels alive.
41. As a user, I want Visuals to be calm (no strobing, smoothed motion) and theme-coloured, so that they're easy on the eyes and on-brand.
42. As a user, I want Visuals to cost little CPU (GPU-rendered) and to pause when the window is hidden, so that leaving Singularity open all day is free.
43. As a user, I want everything to work in a terminal without the kitty graphics protocol too, just less sharply, so that I'm never locked out.

### Navigation & accessibility
44. As a mouse user, I want everything that looks clickable to be clickable (sidebar, rows, buttons, tabs, cards), so that I never need the keyboard.
45. As a mouse user, I want right-click context menus on tracks, albums, artists and playlists, so that actions are discoverable.
46. As a mouse user, I want scrolling everywhere and drag-to-resize Panes, so that it behaves like a desktop app.
47. As a keyboard user, I want a command palette (Ctrl+K or `:`) that fuzzy-finds every action and place, so that I don't memorise bindings.
48. As a keyboard user, I want a hint bar with the few keys that matter in the focused Pane, plus a which-key popup, so that I learn shortcuts gradually.
49. As a keyboard user, I want standard keys by default and a one-line vim preset, so that it matches my muscle memory either way.
50. As a user, I want back/forward history (Alt+←/→, mouse back button), so that browsing feels like a browser.
51. As a user, I want to remap any key in `keys.toml` with conflicts reported, so that customisation is safe.

### Designs, themes & layouts
52. As a user, I want five built-in designs (Singularity by default, Event Horizon, Classic+, Cockpit, Terminal), each a Layout preset plus a Theme, so that I can switch identity in one action.
53. As a user, I want palette variants within a design (e.g. Cockpit: btop, ice, amber, violet, mono) that recolour everything together, so that recolouring is one switch.
54. As a user, I want a live theme picker and themes as simple TOML files I can share, so that the community can make more.
55. As a user, I want to swap the Widget in a Slot, drag Pane borders, and save the result as my own layout, so that I can design my screen.
56. As a user, I want a free-tiling layout edit mode (v1.x), so that I can split and rearrange Panes like a window manager.
57. As a user, I want a transparent-background option, so that my terminal's background shows through.

### Settings, config & data
58. As a user, I want settings in readable files (`~/.config/singularity/`) that hot-reload, so that I can version my dotfiles.
59. As a user, I want the Settings screen (v1.x) and the files to always agree, so that there's one source of truth.
60. As a user, I want app updates to migrate my config automatically, keeping my comments and formatting, so that updates never break my setup.
61. As a user, I want a private local listening log (never uploaded, owner-only permissions, exportable, deletable), so that stats widgets can show my "live Wrapped".
62. As a user, I want nightly local backups of my playlists and liked songs, so that my library survives anything.
63. As a user, I want my Soloist key and Spotify tokens in the system keyring, so that no secret sits in a plain file.

### Robustness
64. As a user, I want the UI to stay responsive while data loads (skeletons, never freezes), so that it feels fast.
65. As a user, I want playback to keep working when the Catalogue breaks (Spotify changed something), with a clear message instead of a crash, so that private-API breakage never stops the music.
66. As a user, I want Singularity never to hammer Spotify (caching, backoff, no polling), so that I don't hit the limits that broke spotify_player.
67. As a user, I want it to degrade to a mini-player in a tiny terminal split, so that it works anywhere.

## Implementation Decisions

### Shape
- **Two processes** (ADR-0003): `singularityd`, the Daemon (systemd user unit; LaunchAgent on macOS), and `singularity`,
  the Client (any number). Rust, tokio, ratatui (D4).
- **Daemon↔Client protocol** (D27): a local Unix socket in the runtime dir, length-prefixed versioned messages. Clients send
  *commands* and *queries*. The Daemon pushes *events*: state snapshots and diffs, and spectrum frames at up to 60 Hz. A version
  handshake makes an updated Daemon restart without breaking running Clients; Clients reconnect automatically.

### Deep modules (each one interface, swappable implementation)
- **Engine** (ADR-0001): play a context (optionally at an offset), pause/resume, seek, volume, shuffle, repeat, add to
  queue; emits playback, queue, device and position events (position is an anchor plus speed, interpolated by the Client).
  - *SoloistEngine* (Linux): downloads Soloist from Spotify's CDN, starts it with the keyring-held key, connects to its
    local WebSocket, parses the build lifetime and auto-updates between tracks, reads codec/bitrate from its verbose log.
  - *AppEngine* (macOS, D25, D51): the hidden official Spotify app, controlled over Connect through ConnectRemote; its
    playback, queue and stream quality come from the Connect cluster. `[engine] kind` picks the Engine (default per OS).
  - *Soloist VM* (macOS, D55, #72): the SoloistEngine with a VM host: Soloist in a Linux VM (vfkit, gvproxy, Debian
    arm64) the Daemon sets up and runs; its audio over vsock to the Daemon, which plays and taps it. `kind = "soloist"`.
  - *LibrespotEngine* (both OSes, D52; the macOS default, D54/D56): librespot's Connect receiver and player inside the Daemon, driven over Connect
    like the app; audio to the system output through cpal, and to the AudioTap in-process. The light choice: at most
    320 kbps, never lossless.
  - Commands Soloist lacks (play-at-offset, queue removal, device quality) go through *ConnectRemote*.
- **Catalogue** (ADR-0002): Home, search, library, entity pages, lyrics, library writes.
  - *PrivateCatalogue*: device-code login with Spotify's desktop client id, a librespot session (spotatui's maintained
    fork), Pathfinder persisted queries and spclient endpoints.
  - *QueryRegistry*: discovers persisted-query hashes from the web player's public JS bundles and refreshes
    automatically on HTTP 412, so rotating hashes never need a release.
  - *OfficialCatalogue*: Web API fallback for library/search when the private source is broken.
  - Rules: metadata only, no crawling, cache with TTLs, back off on 429/5xx, never retry in a loop.
- **ConnectRemote** (D12): device list, transfer, remote control of any device, per-device quality (D17, to verify).
- **AudioTap** (D16): captures only the Engine's stream (PipeWire on Linux; a Core Audio process tap on the Spotify app
  on macOS; librespot's and the Soloist VM's samples in-process), runs FFT,
  onset detection and **automatic gain** (the tap is post-volume), and publishes spectrum frames.
- **LocalStore** (D20): SQLite listening log and nightly JSON library backups under the XDG data dir, owner-only permissions,
  export/delete commands, no telemetry.
- **Config** (D21): TOML files with a schema version and automatic migrations, edited with comment/format preservation,
  hot-reloaded; the Settings screen writes the same files.
- **Secrets** (D24): system keyring (Secret Service / Keychain), with an owner-only file fallback that warns.

### Client
- **Store**: a single unidirectional state store (Daemon events + input → actions → state), the UI renders from it.
- **Layout engine** (D18): a split tree. Built-in designs are preset trees with swappable Slots and draggable dividers;
  user layouts are saved trees; a free-tiling edit mode builds on the same tree in v1.x.
- **Input**: hit-testing for every interactive element (click, drag, scroll, right-click), a command palette, which-key
  popups, a hint bar, a standard keymap with a vim preset, and back/forward history.
- **Theme**: tokens (bg, panel, fg, dim, accent, accent2, border, selection, ramp, per-box borders) per design and palette
  variant; every Widget and Visual takes colours only from tokens. All backgrounds are pure black.
- **Render tiers** (D5): *pixel* via the kitty graphics protocol (kitty, Ghostty, WezTerm) is the primary target. Frames go
  through shared memory on kitty (`t=s`, swept if never consumed), otherwise direct. The *cell* tier (half-blocks, braille,
  truecolor) is the fallback. Synchronized output (mode 2026) every frame.
- **Visual engine** (ADR-0004): wgpu compute shaders off-screen with a CPU fallback. The black hole is the ghostty-blackhole
  model (credited, MIT); the Pixel ember look renders at reduced resolution with ordered dithering. Visuals render at most
  30 fps, pause when hidden, and cap resolution.
- **Gravity well & idle**: the hole lives in its own Pane; neighbouring border lines are re-drawn as braille displaced
  sideways toward it, with a moat around the well. Idle mode (default 3 min, switchable off) moves *that* hole out of its
  Pane, lensing the whole body; any input wakes the UI and is swallowed.
- Code from the prototype (`prototypes/design-directions`, in the git history) seeds the Visual engine, theme tokens, cell/pixel tiers and
  the hit-testing approach, rewritten to production standards rather than copied wholesale.

### Platforms
- Linux first (KDE/Wayland, kitty/Ghostty, PipeWire). macOS second (desktop app or librespot as Engine, Core Audio tap,
  Keychain, LaunchAgent). Never Windows.
- Parity (D49): the same features on both; OS-specific code only behind the platform seams (Engine, AudioTap, Secrets,
  OS media integration, autostart, process lookup). `docs/PLATFORMS.md` tracks every seam and every unavoidable gap.
  Same paths on both (D50).

## Testing Decisions

- **Good tests check external behaviour**, not internals: messages in, messages out; events in, state and screen out.
- **Seam 1, the Daemon protocol (primary):** tests start a Daemon on a temporary socket with a *FakeEngine* and a
  *FakeCatalogue*, drive it purely through the protocol, and assert on the events it emits. This covers playback flow, Catalogue
  degradation, auto-update scheduling and protocol versioning.
- **Seam 2, Client store and screen:** feed recorded Daemon events into the Client store and render into ratatui's test
  backend at several sizes and designs; assert on state and buffer snapshots, as the prototype's `--snapshot` mode does.
- **Adapter contract tests** against recorded fixtures from the spikes: the Soloist WebSocket transcript, Pathfinder `home` answers
  (structure only, no personal data), web player bundles for QueryRegistry, PipeWire stream listings.
- **Live tests** (real Soloist, real Spotify account) exist but are opt-in (`#[ignore]`), like spotatui's.
- **Visuals:** deterministic for a given time and audio frame; GPU and CPU outputs compared within tolerance; frame-time and CPU
  budgets checked in a benchmark rather than unit tests.

## Out of Scope

- Windows, permanently.
- Offline downloads (DRM) and lossless through librespot (impossible).
- Public distribution and anything that would redistribute Soloist (it's downloaded on the user's machine only).
- v1.x, not v1: playlist editing, stats widgets, the Settings screen, more Visuals, the free-tiling edit mode, podcasts and
  audiobooks, the macOS Engine.
- Video podcasts and Canvas loops (maybe later as a Visual).

## Further Notes

- Spike results (in the git history): `spikes/soloist-control/FINDINGS.md` (lossless FLAC confirmed once the Connect device quality is Lossless)
  and `spikes/catalogue-home/FINDINGS.md` (real Home feed, 31 Shelves in ~1.5 s).
- Research: `research/2026-10-platform-constraints.md`, `-private-api-risk.md`, `-macos.md`, `-free-tiling.md`.
- Open: D17 (can Singularity set the device quality itself?) is answered inside the ConnectRemote ticket.

---

# Singularity v1.x: spec

> From the v1.x grilling rounds (2026-10-18, DESIGN-TREE D29–D41). Facts: `research/2026-10-playlists-and-podcasts.md`.
> Tickets are written per batch, just before it's built (Spotify's private details go stale), under spec issue #34. Order (D29): stats → unbuilt v1 items + polish → playlist editing → Settings screen → album-adaptive theme +
> Visuals → podcasts and audiobooks → free tiling (parked, D59) → macOS (pulled forward to after #61: D48) → the
> codebase audit (batch 9, last of all).

## User stories

### Stats (batch 1, D31, D39)
68. As a listener, I want a Stats page (a live "Wrapped"): top artists, tracks and albums for this week, month, year and all time, ranked by minutes with play counts alongside, so that I see what I really listen to.
69. As a listener, I want minutes listened, my streak, "played N times" and a listening-by-hour clock on that page, so that my habits are visible.
70. As a listener, I want every row on the Stats page clickable (play the track, open the artist or album), so that stats lead back to music.
71. As a user, I want small "today" (minutes, tracks, streak) and "top this week" Widgets for the side Slots, so that stats can sit next to the music.

### Unbuilt v1 items and polish (batch 2, D30, D34)
72. ~~As a listener, I want the seek bar to be the track's waveform, built up from the captured audio (spec story 40).~~ Dropped (D46): the seek bar stays a plain line; the audio Widgets show loudness.
73. As a user, I want Singularity to shrink to a mini-player in a tiny split instead of "terminal too small" (story 67).
74. As a mouse user, I want my mouse's back/forward buttons to go back and forward (story 50).
75. As a user, I want a ♥ on every track row that shows and toggles whether it's liked.

### Playlist editing (batch 3, D32, D40)
76. As a user, I want to create, rename, re-describe and delete (asked twice) my own playlists.
77. As a user, I want "Add to playlist…" on any track, with a searchable picker of my playlists.
78. As a user, I want to remove tracks from my playlists and drag them into a new order.
79. As a user, I want to give a playlist my own cover: a file path or an image dropped onto the terminal, previewed, cropped square and shrunk to Spotify's 256 KB JPEG limit before upload.

### Settings (batch 4, D33)
80. As a user, I want a Settings page built from `config.toml` (its comments as descriptions, clickable values, written back keeping the file's comments), and a read-only keymap tab.

### Looks (batch 5, D30, D36)
81. ~~As a user, I want an album-adaptive Theme: its colours come from the playing cover (on #33's tinting).~~ Dropped (D47): the owner likes the theming as it is.
82. As a user, I want to try demo Visuals in my own layout (warp starfield, up-next river, big now-playing marquee, album-art glow) and keep the ones I like.

### Podcasts and audiobooks (batch 6, D35, D41)
83. As a listener, I want show pages with episodes, play and resume, played marks, saved shows in the sidebar, and podcasts in search; video podcasts play as audio.
84. As a listener, I want audiobooks the same way (book pages, chapters, resume, library, search), if Soloist plays chapters (checked first).

### Layout (batch 7, D37) and macOS (batch 8, D38)
85. As a user, I want to tile my Panes freely: drag a Pane by its title to swap it, split another with it or give it a
    screen edge (the layout reflows live; Esc cancels), close and split Panes (`×`, right-click a title, Ctrl+K), and do
    the same from the keyboard (Move mode, `m`). No drop may leave a Pane under its minimum; Home, the library, the
    queue and lyrics can move but never close. **Built (D66, 2026-10-31):** reopened from the parked state (D59). An edit
    never overwrites a built-in Design: the first one asks for a name (Enter saves a Design of your own, Esc keeps one
    unnamed "<Design> (edited)"); a Design of your own saves itself as you edit; "Reset design" goes back to the
    built-in one (a Design of your own stays in the picker). **Polished (owner round, 2026-11-01):** a held Pane is
    lifted and floats live under the pointer (its own Widget, at most ~45% of the screen each way, with a shadow)
    while the others ease into place around a dashed "drop here" slot; on release it lands there (Esc: everything
    goes back). A drop that would only change sizes is no drop (it looked like one Pane "just extended 15%"); the
    screen's edges are the outermost columns and past the top or bottom of the Panes. Shift+arrows in Move mode
    always move the Pane that way or say in the bar why not. Layout undo / redo: Ctrl+Z, Ctrl+Y (or Ctrl+Shift+Z),
    `u` / `U` in Move mode, the bar's "undo" after an edit, and Ctrl+K → "Undo layout" / "Redo layout"; 50 steps per
    session, the Design files follow, and undoing the first edit of a built-in Design returns to it (its unnamed
    "(edited)" file goes again; a Design you named stays in the picker). **Smoother (owner round, 2026-11-01):**
    while anything moves the Client draws at `visuals.motion_fps` (30–240, default 120; frames on a steady grid,
    input handled at once); Panes move on springs, so a new target mid-flight bends the path instead of jumping; a
    zone becomes the preview only after the pointer rests on it (~120 ms, sooner when it creeps), so a fast sweep
    leaves the layout be, and a release drops where the pointer is; a split is offered only where both halves stay
    comfortable (≥ 24 columns or ≥ 8 rows each): small Panes are swap targets, and ⇧arrows say so in the bar.
86. As a Mac user, I want the same daily driver on macOS (D25), when the owner has the Mac ready. Started 2026-10-19 (D48),
    tickets #63–#70, parity rule D49; the light librespot Engine on both OSes (D52, #70).

### Codebase audit (batch 9, the last; owner 2026-10-19)
87. As the owner, I want the whole codebase analysed for bugs, vulnerabilities and wasted resources once the features are
    in, so that Singularity is correct, safe, and uses as little CPU, memory, disk, network and battery as it needs (#71).

## Implementation notes

- **Stats**: aggregates in the Daemon's LocalStore (SQLite over `plays`), sent on request; the log gains album and
  artist URIs (schema `user_version` 2) so rows link; older plays link through their track. Calendar periods in local
  time, weeks from Monday; ranked by minutes.
- **Playlist editing**: Pathfinder `addToPlaylist` / `removeFromPlaylist` / `moveItemsInPlaylist` /
  `editablePlaylists`; spclient `POST /playlist`, `/playlist/{id}/changes`, `/user/{username}/rootlist/changes`;
  covers through `image-upload.spotify.com/v4/playlist` + `register-image`. Optimistic, rolled back on refusal (as
  Saved). Each path gets one quiet live check on the owner's account when built.
- **Visual demos** are ordinary Widgets marked "demo" in their Slot group's ◂ ▸ cycle; liked ones get tickets,
  the rest are deleted.
- Out for good: downloads (DRM), video in the terminal.

## Batch 9: codebase audit (#71)

The last batch, after every feature batch: no new features, a hard look at what exists. Run on both OSes (D49), on
release builds, with every Engine the OS has. Output: `docs/research/<date>-audit.md` (what was checked, how, what was
found), a ranked GitHub issue per real finding (crash / data loss / security / resources / polish), and the crash and
security fixes made in the batch itself. Numbers that matter become benchmarks in the repo, so they can't silently
regress.

### Bugs
- **Long-running tasks**: every `unwrap`/`expect`/index outside tests in the Daemon (a panic in a task silently stops
  that feature; a panic on the main task stops the music); lock poisoning (`std::sync::Mutex` in async code); tasks that
  end without anyone noticing (Engine, Catalogue, tap, MPRIS, backups) and whether the Hub reports or restarts them.
- **State machines**: Engine status transitions per Engine (Soloist, the app, librespot), reconnects (network changes,
  sleep/wake, the Daemon replaced mid-command), the Connect cluster vs local state racing, queue edits on stale uids,
  the wizard's steps for every status combination (property tests over `setup::step`).
- **Protocol**: frames at and past `MAX_FRAME`, truncated frames, unknown fields and variants both ways, a Client that
  stops reading (the Daemon must drop it, not buffer forever), many Clients, protocol version skew.
- **Data**: SQLite migrations from every past `user_version`, the listening log across DST changes and midnight, backup
  rotation, config migrations from every schema with comments kept, concurrent writes from the Settings page and an editor.
- **Time**: position interpolation (seek precision, pause/resume, speed 0), clock jumps, `now_ms` before the epoch.

### Vulnerabilities
- **Secrets**: never on a command line, in a log or in a crash message (Soloist key, Spotify credentials, tokens);
  Keychain / Secret Service usage; the owner-only fallback file's mode and directory; what `daemon.log` contains (it is
  0644 today and names the account) and its permissions.
- **Local attack surface**: the socket directory's mode and ownership on both OSes (macOS uses `$TMPDIR`), whether the
  Daemon checks the peer's uid, what an unprivileged local process could make it do (commands, file paths in
  `SetPlaylistCover`, `config.toml` paths).
- **Subprocesses**: every `Command::new` (`security`, `secret-tool`, `pactl`, `parec`, `pgrep`, `pkill`, `open`,
  `xdg-open`, `scutil`, `defaults`, Soloist): arguments never built from untrusted strings, absolute paths where it
  matters, `pkill` patterns that can only match what we mean.
- **Network input**: parsers of Spotify's private API answers, LRCLIB, Soloist's WebSocket and log, the web player
  bundles (QueryRegistry) treated as hostile: no panics, bounded sizes, no unbounded recursion.
- **Downloads**: Soloist builds from the CDN (integrity, where they're unpacked, permissions); cover images (size and
  decode limits).
- **`unsafe`** (15 blocks today: shared memory for kitty, libproc, signals, `proc_name`): each one justified, minimal,
  and under Miri or a sanitizer where it can run.
- **Dependencies**: `cargo audit` (advisories), `cargo deny` (licenses, duplicates, sources), unused crates; pinned
  `=0.8.6` librespot crates and how they're kept up to date.
- **Terminal output**: escape sequences in track, playlist or artist names can't reach the terminal unfiltered.

### Resources
- **Budgets per process, idle and playing**, measured on both OSes for each Engine (Daemon, Client, Soloist/the app):
  CPU %, RSS, wakeups per second, threads, open fds. Today's reference: the Daemon with librespot idles at ~26 MB, the
  Spotify app uses ~600 MB and 5–25 % CPU while playing.
- **Polling**: every timer and sleep loop (31 today), e.g. the Daemon re-reads `config.toml` every second, librespot
  waiting for a login looks every 10 s; each replaced by an event where the OS has one (file watching, notifications),
  or justified.
- **Idle cost**: nothing runs while nothing plays and no Client is attached: the tap, Visuals, spectrum frames and
  the Client's redraws stop; an open-but-silent audio stream (cpal) is measured and closed if it costs.
- **Memory growth**: caches with no bound (the Catalogue's tracks, described, lyrics, artists and searches maps; covers in
  the Client; images in the terminal) get limits or expiry; a 24 h soak per OS shows a flat RSS.
- **Hot paths**: allocations per frame in the tap, the renderer and the Visuals; frames encoded or sent when nobody
  shows them; JSON sizes on the socket (the full playback state on every change).
- **Disk and network**: what's written and how often (logs bounded and rotated, SQLite, backups, the query registry),
  requests per hour to Spotify while idle and while browsing, nothing fetched twice that could be cached.
- **Battery** (the M2 Air): energy impact in Activity Monitor with the Client open and hidden, idle and playing.
- **Binary size and build**: release profile (LTO, codegen units, strip), feature flags pulling in what isn't used.

### Method
- Read every module against the list above; tools for the rest: `clippy` (pedantic, triaged), `cargo audit`,
  `cargo deny`, `cargo udeps`, Miri, `cargo fuzz` on `read_msg` / config / private-API parsers, sanitizers on Linux,
  `perf`/`heaptrack` on Linux, Instruments (Time Profiler, Allocations, Energy) on macOS.
- Each finding gets a failing test first where one can be written; the soak runs a real session (play, browse, idle,
  network change, sleep/wake) with a script measuring every 5 minutes.
