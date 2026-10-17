# Spec (draft v0, pre-grilling)

> **Status:** updated after grilling round 3. It was first written *before* the grilling session so there is something concrete to
> push against. Everything under *Implementation Decisions* is **PROPOSED** and tagged with the
> design-tree decision (`Dn`) that will confirm or overturn it. After grilling, this file is
> rewritten from the conversation (Matt Pocock's `to-spec` step) and the tags disappear.

## Problem Statement

I live in the terminal and want Spotify there too. The existing terminal clients fail me:

- the best-known one (spotify-tui) is unmaintained;
- the maintained one (spotify_player) is awkward, has no Home page, and **does not work on my
  machine**: every Web API call is rate-limited (HTTP 429) under Spotify's 2026 Development Mode
  quota;
- none of them offers lossless audio, and none feels like the real app: Home, Made For You,
  recommendations, settings;
- they expect me to memorise a wall of keybindings, and they are hard to use with a mouse;
- they look plain. I want the "hacker terminal" aesthetic (a live black hole in one corner of the screen)
  as a built-in, customisable part of the app, plus btop-grade theming.

## Solution

A terminal Spotify client that feels like the desktop app, plays lossless audio, and looks
spectacular:

- **Feature parity where Spotify permits it:** Home feed, Made For You, search, library,
  playlists (create/edit), artist/album pages, queue, devices, lyrics, settings.
- **Lossless playback** through Spotify's official headless engine (Soloist), so audio quality matches the official app.
- **Two equal ways to drive it:** full mouse support (click, scroll, drag the seek bar and pane
  borders, right-click menus) and keyboard. A **command palette** and **on-screen hints** mean you
  never need to memorise anything; power users can learn shortcuts gradually.
- **Built-in visuals and widgets** (black hole, spectrum, lyrics, stats, …) that react to the
  actual audio, can sit in any pane, and can be configured live.
- **btop-style theming:** theme files, a gallery of bundled themes, a live switcher with preview,
  and an optional theme that adapts to the album art.

## User Stories

### Getting started
1. As a new user, I want a guided first-run setup that explains each step (keys, pairing, audio), so that I'm not left reading a README to get sound out.
2. As a user, I want setup to tell me exactly which Spotify requirements apply (Premium, developer key), so that I'm not surprised later.
3. As a user, I want to stay logged in across restarts, so that launching the app is instant.
4. As a user, I want the playback engine to update itself before it expires, so that I never notice its 90-day limit.
5. As a user, I want a clear in-app health check (engine, network, audio device, account) when something's wrong, so that I can fix it without digging through logs.

### Home & discovery
6. As a listener, I want a Home page with the same shelves as the Spotify app (recently played, Made For You, Daily Mixes, Discover Weekly, Release Radar, jump back in), so that I can start music in one click.
7. As a listener, I want to see cover art on Home shelves, so that I recognise playlists at a glance.
8. As a listener, I want to browse genres and moods, so that I can find something new.
9. As a listener, I want "radio" from any track, artist, album or playlist, so that I can keep similar music going.
10. As a listener, I want autoplay when my queue runs out, so that the music doesn't stop.
11. As a listener, I want new releases from artists I follow surfaced, so that I don't miss drops.

### Search
12. As a user, I want instant search across tracks, artists, albums, playlists, podcasts and audiobooks, so that I find anything fast.
13. As a user, I want to filter search results by type, so that I can narrow down quickly.
14. As a user, I want my recent searches remembered, so that I can repeat them.

### Library
15. As a user, I want my Liked Songs, playlists, saved albums, followed artists, podcasts and audiobooks in a sidebar like the desktop app, so that navigation feels familiar.
16. As a user, I want to like/unlike the current track with one click or key, so that saving music is effortless.
17. As a user, I want to create, rename, delete, and reorder playlists, and add or remove tracks in them, so that I can curate without opening another app.
18. As a user, I want to sort and filter long playlists (title, artist, date added, duration), so that I can find tracks inside them.
19. As a user, I want playlist folders shown as folders, so that my library structure matches the official app.
20. As a user, I want to follow/unfollow artists and playlists, so that my library stays current.

### Entity pages
21. As a listener, I want artist pages (top tracks, discography, appears on, about), so that I can explore an artist.
22. As a listener, I want album pages with track list, release date and label, so that I can listen to whole records.
23. As a listener, I want to play a playlist starting from any track, so that I can jump in where I like.

### Playback & queue
24. As a listener, I want play/pause, next/previous, seek, shuffle, repeat and volume always visible and clickable, so that basic control needs no shortcuts.
25. As a listener, I want to drag the seek bar and volume with the mouse, so that it behaves like a GUI player.
26. As a listener, I want to see and manage the queue (add, play next, remove, clear), so that I control what comes up.
27. As an audiophile, I want lossless (FLAC, up to 24-bit/44.1 kHz) streaming, so that I hear full quality.
28. As an audiophile, I want to see the current stream's codec, bit depth and sample rate, so that I know I'm actually getting lossless.
29. As a listener, I want media keys, KDE's media widget and `playerctl` to control the app, so that it integrates with my desktop (MPRIS).
30. As a listener, I want desktop notifications on track change (optional), so that I know what's playing when the terminal is hidden.
31. As a listener, I want to transfer playback between this computer and my other Spotify devices, so that I can move music between rooms or devices.
32. As a listener, I want to control my phone or speaker's playback from the terminal, so that the app works as a remote too.

### Lyrics
33. As a listener, I want time-synced lyrics that highlight the current line, so that I can sing along.
34. As a listener, I want to click a lyric line to seek there, so that I can jump to a part of the song.

### Visuals & widgets
35. As a user, I want a built-in audio-reactive black hole visual, so that my terminal looks incredible while music plays.
36. As a user, I want to tune each visual (colours, speed, intensity, render style) live with a settings popover, so that I can make it mine without editing files.
37. As a user, I want to put any widget (visualiser, lyrics, album art, stats, queue) in any pane, so that I design my own layout.
38. As a user, I want layout presets (Classic, LARP quarter-screen, Cockpit, Terminal, Wall), so that I can switch setups in one action.
39. As a user, I want the black hole to take over the screen when I am idle and vanish the moment I touch a key, so that the app doubles as a screensaver without ever getting in my way.
40. As a user, I want album art rendered at full resolution in terminals that support it, and as good-looking block art elsewhere, so that it works in any terminal.
41. As a user, I want visuals to cap their CPU usage and pause when hidden, so that the app is light when I'm not looking.
42. As a user, I want private listening stats (heatmap, top artists this week), so that I get a live "Wrapped".

### Navigation & accessibility of UX
43. As a mouse user, I want to click everything that looks clickable (sidebar items, rows, buttons, tabs), so that I never need the keyboard.
44. As a mouse user, I want right-click context menus on tracks, albums, artists and playlists, so that actions are discoverable.
45. As a mouse user, I want scroll-wheel scrolling everywhere and drag-to-resize panes, so that it behaves like a desktop app.
46. As a keyboard user, I want a command palette that fuzzy-finds every action and every place, so that I don't have to memorise bindings.
47. As a keyboard user, I want a key-hint popup that shows the available next keys after a short pause, so that I learn shortcuts gradually.
48. As a keyboard user, I want a context-sensitive hint bar showing the 5–8 most useful keys for the focused pane, so that the common actions are always discoverable.
49. As a keyboard user, I want optional vim-style navigation, so that it matches my muscle memory if I have it.
50. As a user, I want back/forward navigation history (mouse back button, Alt+←/→), so that browsing feels like a browser.
51. As a user, I want to remap any key in a config file and see conflicts reported, so that customisation is safe.

### Theming
52. As a user, I want a gallery of bundled themes (Dracula, Catppuccin, Gruvbox, Nord, Tokyo Night, Spotify-green…), so that it looks good out of the box.
53. As a user, I want a live theme picker that previews as I move through it, so that I can choose by eye.
54. As a user, I want themes as simple text files I can copy and share (like btop), so that the community can make more.
55. As a user, I want an "album art" adaptive theme, so that the UI tints itself to what's playing.
56. As a user, I want a transparent-background option, so that my terminal's background or blur shows through.
57. As a user, I want themes to also drive visual colour ramps, so that widgets match the UI.

### Settings
58. As a user, I want an in-app Settings screen (audio quality, normalisation, crossfade, autoplay, explicit filter, output device, notifications, visuals, theme), so that I configure the app like the official one.
59. As a user, I want every setting also saved to a readable config file, so that I can version my dotfiles.
60. As a user, I want config changes on disk to hot-reload, so that I can tweak while it runs.

### Robustness
61. As a user, I want the app to keep working (cached data, playback) through brief network drops, so that it isn't fragile.
62. As a user, I want the UI to stay responsive while data loads (skeletons/spinners, no freezes), so that it feels fast.
63. As a user, I want the app to never hammer Spotify with polling, so that I don't hit the rate limits that broke spotify_player.
64. As a user, I want it to adapt down to very small terminal sizes (mini-player), so that it works in a tiny split.

## Design principles (from prototype feedback, Round 2)

- **OLED black.** Every theme's background is pure `#000000`; panels are separated by thin borders, not grey fills.
- **Calm motion.** Nothing erratic or strobing. Oscilloscopes are triggered (the waveform stands still) and
  phosphor-smoothed; spectra are smoothed across bands; backgrounds drift slowly; audio *nudges* visuals.
- **No blown-out highlights.** Colour ramps never top out in pure white (no white peaks on spectra).
- **Full resolution first.** The pixel tier is the product; the cell tier is a graceful fallback.
- **Visuals are theme-native.** A visual takes all of its colours from the active theme, and each slot offers a
  curated set of visuals that fit its shape and the design, so swapping never looks off-brand.
- **Function first, spectacle second.** No visual may cover or move content you are using. The black hole lives in its
  own pane (the *gravity well* in the Singularity design); its gravity bends only *structure* nearby (neighbouring
  border lines curve toward it), never content. Only when the app is idle does the hole break out of its pane and
  drift over the interface bending its text. Idle starts after 3 minutes (configurable), and can be switched off
  (Shift+I, command palette, `config.toml`). Any key or click wakes the UI instantly and is swallowed.
- **The black hole** is a port of ghostty-blackhole: thin Keplerian disk, blackbody colour, Doppler beaming, photon ring.
  Disk looks: Pixel ember (orange/red pixel art, the Event Horizon default), Inferno, Gargantua, M87*, Quasar, Blazar,
  Face-on ember. A disk look always fits its pane at any aspect ratio.
- **Recolouring is one switch.** A design's palette variants (`t`) recolour every element together: spectra,
  graphs, box borders and accents (e.g. Cockpit: btop, ice, amber, violet, mono).
- **Cheap to leave open.** Pixel visuals render on the GPU (ADR-0004); a full-resolution hole costs ~14% of one core.
- **Terminal-driven aesthetics are welcome** (file-tree library, lyrics as a live log, prompt-style player),
  but progress is drawn as a thin line, not `####`.

## Implementation Decisions (PROPOSED)

- **Two-backend architecture** (D2, D3, D22): a *Catalogue* side (Home, search, library, entity
  pages, playlist edits) and a *Player* side (audio engine and transport), each behind its own
  interface so implementations can be swapped as Spotify changes the rules.
  - Catalogue (settled, ADR-0002): Spotify's **private APIs** via a librespot-core session (the only
    source of Home / Made For You), with the official Web API as fallback. Metadata only, no crawling,
    local library backups (see `research/2026-10-private-api-risk.md`).
  - Player (settled, ADR-0001): **Spotify Soloist**, supervised by the app, auto-updated from Spotify's
    CDN before its 90-day expiry, and controlled over its local WebSocket. Push events replace polling. Commands Soloist lacks (start playlist at index, queue
    edits) go through Spotify Connect remote commands. librespot is an optional fallback engine
    without lossless.
- **Language/framework** (D4): Rust + ratatui + crossterm + tokio. Rust is already installed, and
  the librespot ecosystem and image/graphics crates are Rust.
- **State architecture** (D22): unidirectional data flow. Backend events and user input become
  actions, a single store reduces them, and the UI renders from state. Network work is async, so the UI
  never blocks.
- **Rendering tiers** (D5, settled): modern terminals (kitty, ghostty, wezterm) are the primary target
  and get the pixel tier (kitty graphics) for album art and visuals. Everything also renders in the cell
  tier (half-blocks / braille, truecolor) as a fallback.
- **Audio-reactive visuals** (D16, settled): the Daemon captures the Engine's audio (PipeWire monitor on Linux,
  Core Audio process tap on macOS), runs FFT and onset detection, and publishes a spectrum/energy feed to Clients.
- **Visual rendering** (D26, settled, ADR-0004): pixel-tier visuals are wgpu compute shaders rendered off-screen,
  read back, and sent to kitty via shared memory. CPU fallback.
- **Process model** (D22, settled, ADR-0003): `singularityd` Daemon (user service) + `singularity` Client(s).
- **Platforms** (D10/D25, settled): Linux first (Soloist), macOS second (hidden official Spotify app as Engine).
- **Secrets** (D24, settled): OS password store; owner-only file fallback with a warning.
- **Layout** (D18, settled): a split-tree layout engine. v1 ships built-in layouts with swappable slots,
  drag-to-resize and save-as; v1.x adds a free-tiling edit mode. All five designs ship; Singularity is the default.
- **Daemon↔Client protocol** (D27, settled): local Unix socket, versioned length-prefixed messages, 60 Hz spectrum stream.
- **Interaction model** (D6): sidebar + main view + now-playing bar like the desktop app.
  Everything is clickable, with context menus, a command palette (`Ctrl+K` / `:`), which-key popups
  and a context hint bar.
- **Theming** (D7, D21): TOML theme files in the config dir, bundled theme gallery, live picker,
  album-adaptive theme, transparent background.
- **Desktop integration**: MPRIS server (Linux), optional notifications.
- **Podcasts** (D14, settled): v1.x; audiobooks later.
- **Local data** (D20): a local listening log (SQLite) for stats widgets, plus metadata and image caches.

## Testing Decisions (PROPOSED, D23)

- Test external behaviour at the highest seams: (1) the store (actions in, state out),
  (2) each backend interface against recorded fixtures (a fake Soloist WebSocket server, recorded
  catalogue responses), (3) UI snapshot tests rendering into ratatui's test backend at several sizes
  and themes.
- Visual widgets are tested for determinism (same seed and audio frame give the same frame) and
  for their frame-time budget, not for pixels.

## Out of Scope

- Offline downloads (DRM).
- Lossless through librespot (not possible).
- Windows, permanently. macOS follows Linux (D10, D25).
- Video podcasts and Canvas video loops (revisit later; ASCII Canvas could be a fun widget).

## Further Notes

- Facts behind these choices: [`research/2026-10-platform-constraints.md`](research/2026-10-platform-constraints.md).
- Decision map and dependencies: [`DESIGN-TREE.md`](DESIGN-TREE.md).
- Widget brainstorm: [`ideas/visuals-and-widgets.md`](ideas/visuals-and-widgets.md).
