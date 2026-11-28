# Design tree

The map of every decision this project has to make, following Matt Pocock's
[grilling](https://github.com/mattpocock/skills/blob/main/skills/productivity/grilling/SKILL.md)
method:

- Each decision lists the decisions it **depends on**.
- The **frontier** is every open decision whose dependencies are all settled. A grilling
  **round** asks the whole frontier at once.
- **Facts** are looked up by the agent (see [`research/`](research/)). **Decisions** are the owner's.
  The agent recommends; it never decides.
- Hard-to-reverse, surprising decisions that were real trade-offs become ADRs in `docs/adr/`.
  Resolved vocabulary goes to `GLOSSARY.md`. Both files are created lazily, when the first entry exists.
- Nothing gets implemented until the owner confirms the frontier is empty and understanding is shared.

Status: `open` · `frontier` (askable now) · `settled` · `prototype` (needs something to react to,
so it cannot be settled by talking)

## Decisions

| ID | Decision | Depends on | Status | Outcome |
|----|----------|-----------|--------|---------|
| D1 | Audience: personal tool vs public open-source release | none | settled | Personal tool first; maybe open-source later. More features beat publishability. |
| D2 | Catalogue backend and ToS risk appetite (where Home, search and library data come from) | none | settled | Private API as primary catalogue source, official Web API as fallback. ADR-0002. Risk researched: `research/2026-10-private-api-risk.md`. |
| D3 | Playback engine (Soloist / librespot / both) | none | settled | Soloist (owner has Premium). The 90-day refresh must be painless → auto-update from Spotify's CDN. ADR-0001. |
| D4 | Language & TUI framework | none | settled | Rust + ratatui. |
| D5 | Rendering tiers (text-cell baseline vs kitty graphics) | none | settled | Pixel tier (kitty graphics: kitty, ghostty, wezterm) is the primary target; cell tier is the fallback. |
| D6 | Interaction model (how to be keyboard-first *and* mouse-friendly without 1400 binds) | none | settled | Desktop-app layout, fully clickable, context menus, command palette, which-key popups, hint bar. |
| D7 | Theming model | none | settled | btop-style TOML themes, bundled gallery, live picker, album-adaptive theme, transparency, themes drive visuals. |
| D8 | Which visuals/widgets to prototype first | none | settled | Five designs ship as built-in layout + theme pairs; **Singularity is the default**. Singularity = gravity-well pane + neighbouring borders bending toward it (structure only). Idle takeover: after 3 min (configurable) the visible hole breaks out of its pane and gobbles the UI; Shift+I / palette / config to switch it off. Pixel ember kept for now (not quite the remembered one). |
| D9 | Project name | none | settled | **Singularity**. |
| D10 | Platform scope (Linux-only?) | D1, D3 | settled | Linux first, **macOS second**, never Windows. macOS research: `research/2026-10-macos.md`. Engine choice on macOS → D25. |
| D11 | Soloist lifecycle: who starts it, setup wizard (API key, pairing), expiry handling | D3 | settled | The Daemon supervises Soloist: setup wizard (API key → keyring, one-time pairing), silent auto-update from the CDN between tracks. (Folded into D22.) |
| D12 | Remote-controlling other Connect devices (phone, desktop app, speakers) | D2, D3 | settled | Yes: control and transfer between all Connect devices. |
| D13 | v1 feature slice (first usable milestone) | D2, D3 | settled | v1 = stop opening the official app: Home, search, library, album/artist pages, queue, devices, playback, black hole + spectrum + lyrics, one layout + theme picker. Playlist editing, stats, Settings screen, more layouts and visuals in v1.x. |
| D14 | Podcasts & audiobooks in scope? | D2, D13 | settled | Podcasts in v1.x (show pages, episodes, resume position); audiobooks later. |
| D15 | Lyrics source (internal color-lyrics vs LRCLIB vs both) | D2 | settled | Spotify's synced lyrics first, LRCLIB as fallback. |
| D16 | Audio tap for visualisers (PipeWire capture of the player node) | D3, D10 | settled | No real alternative: PipeWire monitor of the Engine node (Linux), Core Audio process tap (macOS). Lives in the Daemon. |
| D17 | In-app Settings screen: which Spotify settings are controllable | D2, D3 | settled (for now) | Spike fact: Soloist's CLI exposes no quality, crossfade or normalisation options (only output device, initial volume, cache size). Remaining: can the private API set account playback settings? Spike 1/2. **Answer (#13, 2026-10-18): not from Singularity yet.** Soloist carries a Connect feature flag `supports_remote_audio_quality_control` and the HiFi/AudioQuality protos, but no command for setting it is visible, and probing it live would change the owner's device setting. Lossless stays a once-per-device step on the phone; the wizard, the health check and the stream Pane all say how. |
| D18 | Layout system (fixed presets vs free tiling vs presets + slot swapping) | D6 | settled | (b) in v1: built-in layouts, swappable slots, drag-to-resize, save as your own layout. The layout engine is a split tree from day one, so (c) free tiling becomes a v1.x *layout edit mode*. Research: `research/2026-10-free-tiling.md`. |
| D19 | Default keymap flavour (arrows/standard vs vim) | D6 | settled | Standard keys default (arrows, Enter, Space, Ctrl+K); vim preset as a one-line option; everything remappable. |
| D20 | Local data: listening log/stats, caches, privacy | D1, D8 | settled | Local listening log + nightly library backups, on by default. Privacy: local only, never uploaded, no telemetry, owner-only file permissions, export/delete commands. |
| D21 | Config format, locations, hot-reload | D4, D7 | settled | `~/.config/singularity/` (`config.toml`, `keys.toml`, `themes/`, `layouts/`), hot-reload. **Requirement:** the Settings screen and the files are one source of truth, edits preserve comments and formatting, and config is schema-versioned with automatic migrations so app updates never break it. |
| D22 | Architecture & crate/module boundaries | D2, D3, D4 | settled | Two processes: `singularityd` Daemon (user service) + `singularity` Client. ADR-0003. |
| D23 | Test seams | D22 | open | After D27. |
| D24 | Credential storage (KWallet/Secret Service vs files) | D2, D3, D10 | settled | OS password store (KWallet/Secret Service on Linux, Keychain on macOS); owner-only file fallback with a warning. |
| D25 | macOS Engine (Spotify.app over Connect vs librespot) | D10 | settled | The hidden official Spotify app as the macOS Engine (lossless, controlled over Connect); librespot fallback. |
| D26 | Where visuals are computed (CPU vs GPU compute) | D4, D5 | settled | GPU (wgpu compute) first, CPU fallback; frames reach kitty via shared memory. ADR-0004. Measured 626% → 14% CPU for a full-res hole. Low-power adapter (iGPU) by default, `gpu` setting to pick the dGPU (#20, owner 2026-10-18); 10 fps when unfocused. |
| D27 | Daemon↔Client protocol & local socket | D22 | settled | Local Unix socket, versioned length-prefixed messages; the audio spectrum streams over it at 60 Hz so several Clients share one tap. |
| D28 | Arrow keys in content Panes; Home scrolling | D19 | settled | Arrows + Enter navigate the focused Pane (Home Cards, later lists), like the desktop app; seek is Shift+←/→ (±5 s) and `,`/`.` (±1 s); ←/→ still seek in Panes with nothing to navigate. Home scrolls by whole rows and each Shelf sideways by whole Cards, so covers are never cut. |
| D29 | v1.x order | D13 | settled | (1) stats, (2) unbuilt v1 items + polish, (3) playlist editing, (4) Settings screen, (5) album-adaptive theme + more Visuals, (6) podcasts, (7) free tiling, (8) macOS. |
| D30 | Unbuilt v1 items | D13 | settled | Waveform seek bar and mini-player in batch 2; album-adaptive theme in batch 5 (on #33's tinting). |
| D31 | Stats | D20 | settled | A Stats page (live Wrapped: top artists/tracks/albums per week/month/year/all time, minutes, streak, play counts, listening-by-hour) + Widgets "today" and "top this week"; the heatmap stays. |
| D32 | Playlist editing scope | D2 | settled | Own playlists: create, rename/description, delete (asks twice), "Add to playlist…" picker with search, remove, drag to reorder. No folder management. Custom covers wanted "if not too hard": feasible (`research/2026-10-playlists-and-podcasts.md`). |
| D33 | Settings screen | D21 | settled | Built from config.toml (comments as descriptions), clickable, written through the comment-keeping editor; a read-only keymap tab; rebinding later. |
| D34 | Polish | D13 | settled | Mouse back/forward buttons (own input layer); ♥ on every track row (batched CheckSaved). |
| D35 | Podcasts scope | D14 | settled | Show pages + episodes, play/resume, played marks, saved shows in the sidebar, search. Audiobooks / video / downloads: feasibility looked up. |
| D36 | More Visuals | D8 | settled | Demos (`client/src/demos.rs`) judged in round 2: keep starfield, marquee, glow (#44 finishes them); drop up next (it cuts song names). |
| D37 | Free tiling | D18 | settled | Build it (batch 7), remove it if the owner doesn't like it. Parked before it was built (D59). |
| D38 | macOS | D10, D25 | settled | Last; started when the owner has the Mac ready. Pulled forward by D48. |
| D39 | Stats ranking | D31 | settled | By minutes (play counts alongside); calendar periods (week from Monday, month, year, all time). |
| D40 | Playlist covers | D32 | settled | In batch 3: a path or an image dropped on the terminal, previewed, cropped square, ≤ 256 KB JPEG, uploaded the web player's way. |
| D41 | Podcast extras | D35 | settled | Audiobooks in batch 6 if Soloist plays chapters (checked first, else dropped and reported); video podcasts as audio; downloads never. Demos as in-app Widgets (D36); spec for all of v1.x now, tickets per batch. |
| D42 | Album song into "Next in queue" | #32 | settled | Allowed one way (batch 2 polish): dropped among the queued songs it becomes a queued song there and leaves the album run; queued songs never go into the context. |
| D43 | What every Design keeps | D18 | settled | Owner rule (round 2): every built-in Design has Panes for Home, the library, the queue and lyrics; the rest is its own. |
| D44 | Where ◂ ▸ go | D6 | settled | All at the right end of a Pane's title; the main Pane's back/forward there too, as ← →. |
| D45 | Mouse back/forward buttons (#48) | D34 | settled | Dropped (owner): it needed a vendored crossterm or our own input parser; ← → in the main Pane's title do the job. |
| D46 | Waveform seek bar (#46) | D30 | settled | Removed (owner): the plain seek bar looks cleaner; the audio Widgets already show loudness. |
| D47 | Album-adaptive theme (batch 5) | D30 | settled | Dropped (owner, 2026-10-19): they like the theming as it is. More Visuals were already done in #44, so batch 5 is gone; next is batch 6 (podcasts and audiobooks). |
| D48 | When macOS is built | D38, D29 | settled | Now (owner, 2026-10-19: "I will be working on macos temporarily, the next step will be getting the TUI to run on macos"). Batch 8 goes before #62 (audiobooks) and batch 7 (free tiling); both stay planned. |
| D49 | Platform parity | D10, D38 | settled | Owner (2026-10-19): "both the existing linux version and a separate, same functionality macos version, nothing exclusive to either if possible". One codebase; OS-specific code only behind the platform seams (Engine, AudioTap, Secrets, OS media integration, autostart, process lookup). Every feature works on both or has a row in `docs/PLATFORMS.md` saying why not. |
| D50 | Paths on macOS | D49, D21 | settled (agent call, open to veto) | The same XDG-style paths as Linux (`~/.config/singularity`, `~/.local/share/…`, `~/.local/state/…`, `~/.cache/…`, the `XDG_*` variables respected), not `~/Library/…`: dotfile-friendly, one set of docs. Already what the code does. |
| D51 | How the macOS Engine works | D25, D49 | settled (agent call, open to veto) | *AppEngine*: the Daemon starts Spotify.app hidden and drives its Connect device through ConnectRemote (play, transport, queue); its playback and queue come from the Connect cluster. Platform-neutral code (only starting the app is per-OS), so the Engine choice is `[engine] kind` in config.toml, default Soloist on Linux, the app on macOS. Needs the Catalogue login; librespot stays the fallback (D25). |
| D52 | A light Engine (librespot in the Daemon) | D25, D51 | settled | Owner (2026-10-19): "600 mb and 5-25 cpu is kinda insane. Lets add the librespot lightweight player." `[engine] kind = "librespot"` on both OSes (#70): librespot's Connect receiver and player inside the Daemon, driven over Connect like the app, audio through cpal and straight into the AudioTap. 320 kbps at most. The app stays the macOS `auto` until D54 says otherwise. |
| D53 | Lossless on macOS without the Spotify app | D52, D3 | settled | Owner asked how feasible a lossless player like Soloist is on macOS. Facts: librespot can't (Spotify keeps lossless to its own players and asked librespot not to; going around that is DRM circumvention, out). The only other lossless player Spotify makes is Soloist, Linux-only (arm64 too). Option: run it in a tiny Linux VM (Apple's Virtualization framework), its PCM sent to the Daemon over vsock and played and tapped there. Feasible but a project: a VM image to build and download, pairing needs the phone to reach the VM (mDNS proxied from the Mac), the VM's memory (~150–250 MB estimated, unmeasured). Owner: "if its somehow less resources than spotify client thats hilarious and worth it, try it out": a time-boxed spike that measures it, built only if it clearly beats the app. **Spike done** (`research/2026-10-soloist-vm-spike.md`): runs; ~200–270 MB and ~1–3 % CPU estimated (a 192–256 MB VM), against the app's ~600 MB; playback and phone pairing unmeasured until the owner provides the Soloist key. |
| D54 | macOS default Engine | D52, D53 | settled | Owner (2026-10-19): librespot, "since widgets work out of the box, and it uses the least resources by far and its standalone. requiring the spotify app on top of it makes it just a second interface to an app already there working, dont love that for my player." `auto` = librespot on macOS (Soloist stays Linux's); the app remains `kind = "spotify-app"`, the lossless choice. |
| D55 | Soloist in a VM as a macOS Engine | D53 | settled | Owner (2026-10-20): yes, after the spike measured ~300 MB and ~5.6 % CPU playing FLAC. `[engine] kind = "soloist"` on macOS runs Soloist in a Linux VM the Daemon manages: lossless, standalone, the audio Widgets fed from its PCM. |
| D56 | macOS default with the VM available | D54, D55 | settled | Owner (2026-10-20): librespot stays the default (26 MB, no pairing); the Soloist VM is the setting to switch to for lossless. |
| D57 | Audiobooks on an Engine that can't play them | D41, D49, D52 | settled | Owner (2026-10-20): librespot can't (Spotify refuses it chapter keys; going around that is DRM circumvention). Audiobooks are built for every Engine: book pages, library rows and search work everywhere; on librespot, playing a chapter says "Audiobooks need Soloist or the Spotify app: switch the player in Settings". |
| D58 | Spotify's access line on books | D57 | settled | Owner (2026-10-20): book rows and the book page show Spotify's own `accessInfo.signifier` text ("Listen free until Oct 31", "Access with Audiobooks+"); a refusal to play still says so. |
| D59 | Free tiling (batch 7) | D37 | superseded by D66 | Parked (owner, 2026-10-20: "lets pause on the free tiling, maybe as an idea to come back to, current state is very satisfactory"). Not built; kept as an idea (`ideas/layout.md`, research in `research/2026-10-free-tiling.md`). Next is batch 9, the audit. |
| D60 | Visuals while nothing plays | D36, #71 | superseded by D62 | Owner (2026-10-20): "slow sounds good". While nothing plays and nothing was touched for 2 s, the Client draws at most 5 frames a second (`IDLE_FPS`); a key, the mouse or music brings 30 back at once. Idle Client in the harness: pixel tier 5.5 → 2.1 %, cell tier 2.8 → 1.2 %. |
| D61 | Soloist's cache size | #71 | settled | Owner (2026-10-21): "make the cap user customizable, I'd like to have some songs cached, we can let it go for 5 gb". `engine.cache_gb` (config schema 4, Settings page), default 5, 0 = no limit, passed as `--cache-size` (Soloist's minimum 100 MB). Linux only: the macOS VM keeps 200 MB (its disk is the small cloud image). Their cache had reached 2.8 GB in four days with no limit. |
| D62 | Smooth Visuals always | D60, #71 | settled | Owner (2026-10-21): "I don't like the 5fps out of focus animations, they look choppy af so lets gut them". D60's 5 fps while nothing plays is gone; `visuals.unfocused_fps` defaults to 30 (was 10; config schema 5 moves an untouched 10 to 30, a chosen number stays). The rain runs at 1.6× without music (it stepped visibly slower). Measured cost on the owner's machine, playing and focused: ~3 % Client + ~5 % kitty. |
| D63 | Playback changed elsewhere (a phone on Connect, a Jam guest) | D3, D12 | settled | Owner's report (2026-10-23, T0005): a friend's phone playing on Singularity made the previously playing song come back. **The Daemon follows a change it didn't make; it never puts its own remembered state back** (agent call, open to veto). Two defects made up the cause: the Soloist updater restarted Soloist *because of* the phone's track change and then re-played the state it remembered (and, when it came back with a track already playing, the remembered track won); and a `get_queue` answer, a snapshot from before the change, could put the old track back in what the Daemon reports. Built: the updater takes a quiet moment only for a change its own command caused (`SoloistState::change_is_ours`), the resume waits 1.5 s and is dropped if the player came back playing something, and a queue answer only fills the queue (`phone_test.py`, contract tests). |
| D64 | Jam: hosting or joining one on a Singularity device | D3, D12, D63 | settled | Question for the owner, see the round log ("T0005", 2026-10-24): Jam is Spotify's server-side shared session, hosted *by one device*; a Jam's commands arrive as ordinary Connect play/queue commands, which D63 now follows, but a guest who joins lands on the Jam's host, not on Singularity. **Research (T0033, 2026-10-25): `docs/research/2026-10-jam.md`.** The `social-connect` API is reachable from a librespot session (a librespot bot starts a Jam on its own device and gets the invite link); Premium is needed to host; the invite is `https://open.spotify.com/socialsession/{token}` (QR is just that link); the dealer pushes `social-connect/v2/session_update` (members, permissions, `host_active_device_id`); the cluster only carries `PlayerState.session_id`, not a Jam flag. Open question: whether our Catalogue session may name *another* device (Soloist's) as `local_device_id` — one live check decides whether start works on Soloist or only on the librespot Engine. ➡️ Recommendation (open): build "start/end + invite QR + N guests" in the Devices panel if that check passes; otherwise scope it to the librespot Engine and keep "start from the phone" for Soloist. Joining a Jam from Singularity is not worth it. **Settled (2026-10-30, D65):** the live check passed: the Catalogue session started a Jam hosted on Soloist's own device (research §6, Results). Build: start/end in the Devices panel, invite link + terminal QR, a live guest list; kick and permission toggles later. The Jam is read from social-connect, never from the cluster (its `session_id` is not the Jam's). |
| D65 | The public release | D1, D49, D64 | settled | Owner (2026-10-29, release round 1): **Linux is the supported platform, macOS ships as "beta"** (its open live checks become public issues). Only owner checks a user would hit block the release (T0005 is done: the 10-28 log shows the dropped resume); #71's soak and per-Engine numbers become follow-up issues. **History**: scrub a mirror clone and push it to a new public repo; the private repo and its issues stay as the archive (no force push). Before launch: Q4 (fake Catalogue out of `catalogue.rs`) and Q1 (per-frame allocations); the big splits after. The public tree drops `prototypes/`, `spikes/`, `.project.yml`, `docs/HANDOFF.md` (kept in history) and keeps `docs/`, `GLOSSARY.md`, `CLAUDE.md`, `.claude/skills`. README: not affiliated with Spotify, private APIs, account at own risk, no Spotify name or logo. CI on Linux and macOS: test, clippy, `cargo deny`. **Jam is in the release if it can work** (D64): start/end in the Devices panel, invite link + terminal QR, a live guest list (kick and permissions later: their endpoints are assumed); if Soloist's device can't host, librespot hosts and Soloist shows a Jam started from the phone; only if both fail does it ship without Jam. Licence **MIT** only. First tag **`v0.1.0`**; source install plus CI-built binaries for Linux x86_64 and macOS arm64 on a tag (AUR/Homebrew later). README screenshots from the pty harness on the fake Daemon, committed in `docs/media/` (2026-11-02: replaced by the owner's own screenshots, account and other people's names removed; the Jam picture stays from the harness). Fresh issues in the public repo for what is open; the private ones close with links. Public commits are credited to the owning GitHub account's no-reply address. The private repo is renamed `singularity-archive`, the scrubbed mirror becomes `publicusr/singularity`. **Order**: probe merge → Jam check → Jam → Q4/Q1 → tree, LICENSE, README, screenshots → CI → mirror scrub → owner looks it over while private → owner makes it public, tags `v0.1.0`. |
| D66 | Free tiling, for v0.1.0 | D37, D59, D65 | settled | Owner (2026-10-31, round 1, all recommendations accepted, one amendment): reopens D59 and ships in v0.1.0 (the release waits for it and the owner's retest). **Mouse, always on**: grab a Pane by its title bar and drag; no edit mode; ◂ ▸ and divider drags stay. **Live reflow**: while dragging, the layout shows the result of the hovered drop (the moved Pane follows the pointer as a ghost); it commits on release; Esc or dropping where it started cancels. **Drop zones**: a Pane's centre swaps; its four edges split it and put the moved Pane on that side; the screen's outer edges give a full-height column / full-width row; the moved Pane's old sibling takes its room. **Bulletproof**: no move, split, close or resize may produce a Pane below its minimum (or squash the well/hole below its shape); such a drop isn't offered; property tests run random edit sequences on many terminal sizes. **Close and split**: `×` in the title closes (the neighbour fills), right-click title / palette "Split right" / "Split down" adds a Pane and opens the Widget picker; no Widget twice; Home, library, queue and lyrics can move but not close (D43's guard). **Edits**: editing a built-in Design never overwrites it; the first edit opens the naming prompt with a suggested name ("Singularity 2"); Enter saves it as a Design of your own (with its Theme), Esc keeps one unnamed "<Design> (edited)" slot per Design (overwritten, never a pile). "Reset design" goes back. **Keyboard**: a move key on the focused Pane enters move mode: arrows swap with the neighbour that way, Shift+arrows move to that side as a split, `+`/`-` resize, `x` closes, Enter/Esc ends; the same zone highlight as the mouse. |
| D67 | Karaoke: a Widget and a Design | D8, D15, D43 | partly settled (2026-11-05) | Owner (2026-11-05): "a karaoke widget … the bouncing karaoke ball … maybe a karaoke separate design with the widget as the central piece". Built with these choices, each for the owner to confirm (round log, "Karaoke"): word times from LRCLIB's word-synced Lyricsfile and enhanced LRC when present, else estimated in the Client by syllables (≤ 550 ms a syllable); Spotify is still asked first and LRCLIB only when Spotify has nothing; the Karaoke design keeps D43's four (Home, library, queue, lyrics) beside the Karaoke Widget, plus glow; a new shared theme Neon is its own; the pixel-tier ball runs frames at up to 60 a second while it hops. **Owner's answers (2026-11-05):** (1) yes: while a Karaoke Pane is on screen, LRCLIB is also asked for word timing when Spotify's lyrics only time lines (`GetLyrics.words`, once per track; LRCLIB's word-synced lyrics replace Spotify's for that track); (2) yes: the Karaoke design keeps the whole-song lyrics Pane; (3) big rendered text for the lyrics line: the owner tests the new lyrics first; (4) Neon: the owner tests it first; (5) yes: no palette Action or key for karaoke. **Sync (2026-11-05, owner: "offbeat quite often")**: estimated words now follow the song's fitted pace and hold a line's last word (mean miss 450 → 300 ms on modelled singing); `lyrics.delay_ms` (default 0) shifts lyrics and karaoke for a lagging output; audio onsets looked at and not built (round log, "Karaoke sync"). |
| D68 | Home's shortcuts grid without empty slots | D13 | settled (2026-11-20) | Owner (2026-11-20, screenshot: "empty album slots on the home view"): the shortcuts count varies (7, 8, 10) and the grid has up to 4 columns, so its last line was ragged. The last line's tiles now share the whole width (1 left over spans the line, 2 split it); nothing is hidden (Spotify's own trick, trimming to whole lines, was declined). |
| D69 | A one-Card Shelf is a Spotlight | D13 | settled (2026-11-20), owner to see it live | Spotify sends "New release from <artist>" as a Shelf of one Card, which left two empty Card slots. Such a Shelf now draws the cover at Card size with the name, artists, type and "Enter opens" beside it (`RowKind::Spotlight`). Applies to every Shelves view (Home, artist pages, search). |
| D70 | Search is built into Home | D13 | settled (2026-11-20) | Owner: "move the search bar from the library tab onto the home tab, and make it clearer than just the word search". A filled one-line bar on top of Home reading "Search" (owner: just "Search", not a question), the Search key's label at its right; a click, Ctrl+L, or ↑ from Home's top row then Enter or typing (the typed letters become the query) opens Search. |
| D71 | The library is only the library | D13 | settled (2026-11-20) | Owner: "library should be completely separate from home and home should have the search built in". The library's Search row is gone (`sidebar::FIXED` = 1); Home stays its first row. Ctrl+L and the palette still open Search anywhere. |
| D72 | The Search page uses Home's bar | D70 | settled (2026-11-20) | The query line is the same bar (`search::bar`) in the same place, so opening Search brings the bar to life instead of jumping. |
| D73 | Linux on ARM (aarch64) | D10, D49 | settled (2026-11-20), built in code, untested on hardware | Owner: "add support for linux ARM based … ill run a session later". The Daemon downloads Soloist's arm64 build (`update::cdn_for`, published next to the x86_64 one; both checked live 2026-11-20); `auto` on a Linux CPU without a Soloist build is librespot; CI tests on `ubuntu-24.04-arm` (public repo only), a release adds `aarch64-unknown-linux-gnu` built on `ubuntu-22.04-arm`. A cross type-check from the x86 PC stops at C dependencies needing an aarch64 sysroot; the owner's ARM session does the first real build and run. |
| D74 | Search owns its query (architecture review candidate 1; issue #8) | D70, D72 | settled (2026-11-22), built on `deepen/search-query` | Owner accepted all seven: **Client only** (the Daemon's half of #8, searches queued behind Connect work, is candidate 4); a picked Recent search or Enter on typed words goes out **at once** (typing keeps the 300 ms pause); results **kept by query** (16, 15 min: the owner raised it from 5 on review; matched ignoring case and outer spaces, as the Daemon matches), shown at once while a fresh answer is asked for; the **recent searches and the kept page live inside Search** (no `App.recent`, no `search_kept`); an interface of **methods** named for what the user does (`type_text`, `erase`, `clear`, `pick`, `submit`, `receive`, `remember`, `due`); **tests at Search's interface** plus a pty scenario; glossary term **Recent search**. Calls made while building: Enter with nothing highlighted still steps into the results once the answer for these words is on screen (it only sends while no answer has come); the results stay on screen while typing on, as before, and clear only when the query empties. |
| D75 | Song lyrics own the lyrics (architecture review candidate 2) | D74 | settled (2026-11-22), built on `deepen/song-lyrics` | Five App fields (`lyrics`, `lyrics_scroll`, `lyrics_wrap`, `karaoke`, `lyrics_asked`) became one module, `songlyrics::SongLyrics`: what was asked for, what came back, the lyrics Widget's rows and view, the Karaoke Widget's timed lines and sizes. Interface: `receive` (every arrival, through `App::lyrics_event`), `ask`, `reconnected`, `lyrics_view`, `sung`, the wheel's two moves. Every arrival starts both views afresh (no fingerprint guessing); a reconnect re-asks only what wasn't answered. Owner accepted on review: Karaoke's held scroll survives an arrival; `Follow::release` removed. Fixes B11. Looks unchanged (pty, 7 sizes). |
| D76 | One Soloist session per run (architecture review candidate 9) | D63 | settled (2026-11-22), built on `deepen/soloist-session` | `run_once`'s 300-line `select!` became `soloist::session::Session`, a pure state machine: `step(Input, now) -> Vec<Action>` (inputs: WebSocket, log news, command, staged Build, tick; actions: send, event, save volume, store key, stop), `deadline()`, and a **Handover** to the next run (where to resume, the last stuck-stream restart). `run_once` = `start` + `pump`. Contract tests are whole runs on a made-up clock. Fixes three C4 bugs: a key refused after the WebSocket is up ends the run (Failed, not a 2–60 s restart loop); every `get_queue` in flight is answered as one; an underrun while paused or idle doesn't start a dry spell. Owner accepted on review: a failed `get_queue` send ends the run; exit code 10 on socket close reads as Expired; a locked data dir shows Failed but keeps the 2–60 s backoff. |
| D77 | The Engine assembly decides the Engine in one step (architecture review candidate 10) | D49, D51, D52, D54, D55 | settled (2026-11-22), built on `deepen/engine-assembly` | `assembly::assemble(config, args, env, host)` is pure and returns the Engine's Plan (Fake, Soloist, SoloistVm, SpotifyApp, Librespot), its Identity (one device name, so Connect and the Catalogue agree; fixes C5), the AudioTap's source, the Catalogue and the problems; `Assembly::build` makes them. Device-name defaults moved out of the Engine constructors. Config: a bad value defaults only its key (`config.toml: engine.kind = "x" isn't known; using auto`), a wrong-shaped section only that section; the format is unchanged. Owner accepted on review: an unknown `SINGULARITY_ENGINE` is reported; `keys.toml` parses the same way; problems show in the existing toast and Settings Config tab (no new Notice); a macOS Plan built on Linux panics (tests only). |
| D78 | A query port under the Catalogue (architecture review candidate 6) | D77 | settled (2026-11-22), built on `deepen/query-port` | Owner accepted all six: the **port is one method**, `Fetch::query(op, vars)` (backup's `Fetch` is now the same trait); Connect, the cover upload, Spotify's lyrics and LRCLIB stay outside it (Connect leaves in candidate 4). The **Pathfinder adapter** (`pathfinder.rs`) holds the hash lookup, the ask-again after a hang, the refresh on a retired hash and C3's fix (an operation a fresh crawl didn't find fails at once until the next crawl), tested over a stand-in raw call; a missing operation fails at once for 15 min (review: a crawl that lost a file mustn't hide one for the 3-day TTL). **`spawn_fake` stays** for the pty harness and protocol tests for now; retiring it is a follow-up. Tests at **two seams only**: the Catalogue's `CatCmd`/`CatEvent` interface over a fixtures adapter, and the adapter's `query` over a stand-in raw call. Logged out, nothing is looked up. Changed while building: the test Catalogue runs **logged out**, not logged in as accepted, since queries go through the port either way and a logged-in state needs a real session; nothing that needs one is tested there. The describe retry *after* `DESCRIBE_RETRY` isn't tested (the Catalogue keeps `std::time::Instant`, which a test clock can't move). No glossary term. |
| D79 | Connect out of the Catalogue's loop (architecture review candidate 4; #8, C5, C6) | D78, D64 | settled (2026-11-22), built on `deepen/connect` | Owner accepted all six: **Connect gets its own loop** (`catalogue/connect.rs`: the ConnectRemote, the cluster and dealer listener, devices, moving playback, the Jam, our buttons for another device, an Engine's own commands, queue edits, play-from, the resume after an update), beside browsing in the Catalogue's one supervised task; a dispatcher passes each command on as it arrives (`is_connect`), so the outside keeps one channel and a search still sees what came behind it. **Login stays with the Catalogue**: Connect asks through D78's session slot, drops its remote when the session ends or changes, and asks browsing to log in again (`CatCmd::Reconnect`). **Two questions**: the remote (`remote()`) and this computer's device on it (`ours()`); controlling another device, moving playback, the device list and ending a Jam don't look ours up (C6). **Events keep their path** to the hub; Connect Engines' own channel stays with candidate 5. On logout, or when the Catalogue stops, the hub lets go of the other device (C5). Tests at three seams: routing, a search answered while Connect is busy, logout. C6 has no test (it needs a Connect port). After review: a Connect command **waits for a login under way** (up to 30 s; the session slot is a `watch`), as it did when it queued behind the login, instead of failing; a session Spotify ended is logged in again and the command goes through; Ready is sent once the session is in the slot; the hub also lets go of the other device when the Catalogue is Unavailable; `is_connect` names every command. Known: a Catalogue panic now loses the commands already passed on (they used to wait in the channel). |
| D80 | Now playing, one module in the hub (architecture review candidate 5) | D79, D51 | settled (2026-11-22), built on `deepen/now-playing` | Owner accepted all eight: **`NowPlaying`** (`playing.rs`; `nowplaying.rs` is macOS's media integration) holds what Clients see, the Engine's state, the other device's, the track names known and those being asked for; plain code behind **one lock** in the hub, never held across an await, so an update can't be lost between reading and writing it back (it was: names arriving cloned the playback, awaited, and wrote it back over a newer state). Inputs: the Engine's state, the other device's (or gone), names found, names failed, resync; each returns the playback to send (sent under the lock, so Clients hear states in order) and the tracks to name. It also **routes** a play/pause button (the other device while one plays; a toggle on a buffering one pauses) and gives the **lyrics hint**, which now needs a name *and* an artist, else the Catalogue looks the track up (LRCLIB was searched with no artist). Going back to this computer shows the Engine's rows **named** with what came meanwhile, not bare until its next update. The Connect Engines' updates keep their path through the Catalogue; the one lock makes the two senders safe, so no new channel (closes D79's note). Tests at `NowPlaying`'s interface (seven, two red first: names kept on coming back, a hint without an artist) and the hub's logout test. The lost update has no test: it's a race the shape removes. After review: **every input returns the same answer** (playback to send, tracks to name), so a track found bare on coming back or when names arrive is asked about instead of marked and forgotten (test: `a_bare_track_found_on_coming_back_is_asked_for`, red first); without a Catalogue a button goes to the Engine unchanged, as before; the logout test checks the bar shows the Engine's own state. Glossary: Now playing. |
| D81 | The open Track list keeps rows by identity (architecture review candidate 3) | D74 | settled (2026-11-22), built on `deepen/open-list` | Owner accepted all eight: **`OpenList`** (`crates/client/src/openlist.rs`) owns the Track lists received so far, the open view (sort, filter, selection, scroll) and the views remembered per list; callers ask it what shows in what order, which row is selected and where a row is, instead of rebuilding that from `lists` + `view` + `view_order()` in seven places. A row is known by its **`RowId`**: the playlist row's uid, else its track URI. A list changes only through `OpenList::edit`, a guard that, when the edit ends, puts the selection back **on the row it was on, on the same line of the screen** (a page arriving into a sorted list moved it to another song); if that row is gone, the row that took its place. A row's **menu records the row's identity** when it opens and finds it again when an action is chosen; a row gone meanwhile does nothing and the bar says "That song isn't in the list any more" (it used to act on whatever then showed at that position: "Remove from this playlist" removed another song). **A filter** keeps the selected row, on its line, while it still shows, then the row that took its place (changing the sort still goes back to the top, as before). **Cards** have the same problem and are a ticket of their own (#23). Tests at `OpenList`'s interface (four) and one through the real menu, `a_row_menu_acts_on_its_row_even_if_a_page_arrives_while_it_is_open` (red first: it removed alpha, not bravo), and the filter test (red on the old go-to-the-top rule). After review: a drag remembers the pressed row by identity and finds it at the drop (a row taken out meanwhile cancels it with the same message); a double-click must land on the same row, not just the same position; moving a row (drag, Alt+↑↓) keeps the scroll as it was; the dead `view_order` and the hand-rolled row lookups went. Known: a click lands on the row drawn last frame, so one between a change and the next frame (a few ms) picks the row now there. No glossary term (Track list already is one). |
| D82 | Daemon events enter the Client through one door (architecture review candidate 7) | D74 | settled (2026-11-22), built on `deepen/one-door` | Owner accepted all six. **The pump** (`conn.rs`, `pump`): a frame was read inside the `select!` (a length, then the body: several awaits), so a command arriving between them dropped the half-read frame and the stream fell out of step ("frame too large", garbage, a reconnect; likeliest while Track list pages arrive). Frames are now read on a task of their own into a bounded channel (a slow App makes the reader wait, as before; nothing dropped); both branches of the `select!` only take from a channel. It takes any reader and writer. Built first, its own commit. **`App::receive`** (`receive.rs`): the ~30 event arms moved out of `main.rs`, unchanged, into the one way `main` and tests deliver a Daemon event or a change in the connection. A pending login no longer starts the browser from there: it leaves the page in `App::open_url` and the main loop opens it (like the clipboard), so tests go through `receive` safely. Starting and replacing the Daemon stay in `conn.rs`. The 46 test lines that faked a real Daemon event by setting a field (Engine and Catalogue status, Home, Library, devices, Jam, health) now go through `receive` (`App::hear` in tests). Tests: the pump over `tokio::io::duplex`, `a_command_sent_while_a_frame_is_half_in_does_not_cut_the_frame` (red first: the frame was lost and the pump ended), and three at `receive` (link, the pairing page opened once, the Engine that was ready). Not covered: the real socket and starting the Daemon (the existing connection tests). |
| D83 | One Popup instead of 14 fields: what's drawn on top takes the keys (architecture review candidate 8) | D74, D82 | settled (2026-11-22), built on `deepen/popup` | Owner accepted all five. **One value**: `App::popup: Option<Popup>`, each variant holding its data (palette, picker, naming, Widget list, playlist details, add to playlist, cover form, devices with their highlighted row, health check and help with their scroll, context menu); opening one replaces the one open. The setup wizard, the login code and Move mode stay derived from state. **One precedence**: `App::layers` lists what's on top, the top first (the popup you opened, the wizard, the login code, Move mode); keys, the pointer, the wheel, paste and drawing (`ui::layer_rect`/`draw_layer`, the bottom drawn first) all follow it, so a popup opened over the wizard is drawn over it and takes the keys and the click (the palette took the keys while drawn under the wizard; devices, health, a menu and the help were drawn under it and lost the keys to it). Follows from the one order, not separately asked: Login now ranks above Move mode (Esc hides the code, a click doesn't end Move mode); with the code showing, Esc hides it before a list filter, search or page sees it; paste no longer reaches a list filter behind a popup; a Pane drag's key guard comes after the popups. Unchanged: the wizard still takes every key, so a popup gets over it by a click or an event, not Ctrl+K. Tests through the App (keys, clicks, a `TestBackend`): every popup over the wizard is drawn on top (its box as without the wizard), Esc closes it and not the wizard, a click outside closes the ones a click closes, a click on its first row chooses it (red first: the palette drawn under the wizard; the click red with the wizard ranked first); and a menu then the palette leaves only the palette (red first). Page has its own ticket. Looks unchanged (pty `popup-wheel`, `popup`). No glossary term. |
| D85 | A Card's menu keeps its Card by identity (#23; the Cards half of architecture review candidate 3) | D81 | settled (2026-11-22), built on `fix/card-menu-identity` | Ticket #23's answers: a Card's menu remembered its Card as a position (`CardRef`: Shelf index, Card index), so a Home feed arriving while it was open (a Shelf more, a Card less) made Play, Go to, Add to playlist or Copy link act on whatever Card then sat there. A menu now records a **`CardId`** (`crates/client/src/home.rs`): the Card's URI and its Shelf (title and kind), since one URI can be on two Shelves; when an action is chosen it is found on its own Shelf, else the same URI on another one. A Card gone meanwhile does nothing and the bar says "That isn't here any more". **The selection** (Home's and an artist page's, `HomeView::refeed`) stays on its Card while that is still on its Shelf, else on the Card that took its place (as D81's rows); each Shelf keeps its sideways scroll, which was by Shelf index too. Search results already start a fresh view. `CardRef` stays for one frame (the layout and the hit map). Tests: `a_card_menu_acts_on_its_card_even_if_home_is_refreshed_while_it_is_open` through the real menu (red first: it copied the new Shelf's link), and `a_new_feed_keeps_the_selection_on_its_card_and_each_shelf_its_scroll` at `HomeView` (red without `refeed`, and with the selection following its URI to another Shelf). Known: as in D81, a click lands on the Card drawn last frame, so one between a new feed and the next frame (a few ms) picks the Card now there; a Shelf renamed by a refresh loses its scroll and its selection falls back to the Card in its place. No glossary term (Card and Shelf already are). |
| D84 | Moving Panes, smooth and bulletproof (#21) | D66 | built on `smooth/pane-moves`, decisions open | Not grilled before building: the three decisions the ticket names go back to the owner (see the PR). **Pointer paths** (`arrange/paths.rs`): whole gestures replayed with a test clock, a mouse (500 Hz, still at rest) and a trackpad (90 Hz, a trembling finger, a slip as it lifts), frames when the Client's pacer has them due (`pace::Pacer`, motion rate 60 or 144) with pointer moves combined as the main loop does (`pace::Moves`, now shared with `main.rs`) or not; each shape on every Design: a fling, a creep, jitter on a band's and a Pane's border, an overshoot, a pause then on, out of the window, let go over the top bar and the player, a resize mid-drag, a right click mid-drag, every divider swung, flung and released unheard, and 300 random human-like paths. Checked on every path: the layout and the preview fit, frame by frame, **what's dropped is what was previewed**, no preview gone again within 100 ms, nothing held after the button is up. **Found and fixed** (each red first): a Pane let go before its zone was shown landed where it was let go, never shown (now: the preview is the drop, rechecked against the window's size; nothing shown, it goes back; this reverses the old "a quick flick lands where it was let go", owner to confirm); a release the Client never heard (another window, the focus gone) left the Pane held for good, swallowing every click, and a divider held (now a bare move or a fresh press ends it: a Pane goes back, a divider is let go where it is); a right click mid-drag dropped the Pane on the spot (now ignored until the left button comes up); after a resize mid-drag the preview kept a zone of the old window, and a release dropped there (now the zone under the pointer is found again in the new window, shown at once if a preview was up); a divider flung past its limit at the well stopped short (now at the furthest cell that fits). **Not found**: flicker (the dwell and the sticky borders keep every preview up ≥ 100 ms). After review: an unheard release is also caught by the event that only wakes the idle UI (no test). Not modelled: pixel pointer reports and the wheel. The dwell rule (120 ms, 40 ms creeping) and how the preview animates are unchanged. pty `tiling-paths`. No glossary term. |
| D86 | Touchpad scrolling; the wheel scrolls the view (#32) | D73 | settled (2026-11-23), not built | Found on the first ARM run: the touchpad in Ghostty scrolls jerkily and picking a row is hard (`WheelBursts` drops repeats within 8 ms; the wheel moves the highlight). Owner: "lets try it": the wheel and touchpad scroll the **view**, hover highlights, a click picks; keys move the highlight as before. After D87. |
| D87 | Moving Panes: rethink before smoothing (#21) | D66 | settled (2026-11-23, five grilling rounds; ADR 0005) | Owner: the rearranging UX "sucks rn" and gets priority. The next session (on the PC) grills how moving Panes should work before deciding what happens to #31. Order: D87, then D86 (#32), the Soloist check on ARM whenever the Mac is on the charger. **Grilling, round 1 (2026-11-23):** owner on the Mac: dropping a Pane where you pictured it "feels almost impossible", the app "keeps reorganizing itself and changing shape", swapping two is hard; mouse preferred, the keyboard way "a bit more solid in its limitations". Accepted: a **Tray** where Widgets wait while you rearrange (a Pane can be empty meanwhile); `×` sends a Widget to the Tray, clearing the Tray removes what's in it; a **small chip** (the Widget's title) follows the pointer instead of the full-size floating Pane (adjust after the owner's test). **Round 2:** accepted: a title drag only swaps contents (with another Pane, an empty Pane or the Tray); the floor plan changes only by dragging a line, splitting or closing (owner: reshaping on hover is iOS's "apps teleport around" problem); an **Arrange mode** (Tray, empty Panes, split/close buttons, Done; a title drag enters it), reversing D66's "no edit mode"; **New design…** = Arrange mode from one empty Pane with the required Widgets in the Tray, + Widget from a list, Done names it. Owner's condition: every Widget must scale well to any Pane size on many displays (lyrics is poor today). **Round 3:** accepted: **Home is the only Required Widget** (library and queue open as palette popups when they have no Pane; lyrics optional; built-ins keep D43's four); scaling is fixed from a **contact sheet** the owner judges (each Widget at a fixed set of Pane sizes, both tiers; lyrics and Karaoke first), its own ticket, before or beside Arrange mode; **every drop is allowed**: a Widget below its smallest size says so in one clean line (no over-explaining), Panes keep a minimum when a line is dragged; keyboard Move mode folds into Arrange mode; the drop target highlights at once, no dwell (smoothness to iterate); the Tray is kept per Design until cleared and **never shows outside Arrange mode**; **#31 merges as the baseline**. New from the owner: a quick way to swap a Pane's Widget for any other (typed filter, scrollable list, on-screen Widgets left out, maybe duplicates) instead of today's split-then-type-a-name. **Round 4:** accepted: keyboard in Arrange mode is arrows (move between Panes), Enter (pick up, aim with arrows, Enter swaps), `t` (to the Tray), `s`/`S` (split right/down), `x` (close), `+`/`-` (move the focused Pane's line), Esc (Done), with a one-line key legend at the bottom; no "carry" with Shift+arrows (it shuffles the Widgets passed, the teleporting again); the **Widget picker** is a palette-style list (type to filter, wheel and arrows scroll), Widgets not on screen first, on-screen ones dimmed below a line (picking one swaps the two Panes' Widgets), the replaced Widget just leaves the screen, Home's Pane has none; **no duplicate Widgets** for now (owner may want them later for Visuals such as oscilloscope or starfield; cost: Widgets are known by name in the floor plan); the too-small look is one dim centred line `needs 30 × 10` (just the title when even that doesn't fit); the Air's sizes are emulated in the pty (about 180×54 full screen, 90×50 half, Retina cells), confirmed with `stty size` once on macOS. **Build order:** merge #31 → contact sheet (lyrics, Karaoke first) → Widget picker → Arrange mode (one module replacing the zones, dwell, live reflow and motion) → New design… → keyboard; each a ticket, the owner tests after the picker and after Arrange mode. After D87: #27, then `/improve-codebase-architecture` round 2 on `ui.rs`/`app.rs`. **Round 5:** accepted: **right-click** is the main way in: a Pane's title, or its body where the body has no menu of its own (song rows, Cards and the queue keep theirs), opens the Pane's menu: Replace widget… first, then Split, Close, Arrange; clicking the Widget's name and a key are shortcuts. Split makes a new empty Pane on the chosen side and opens the Widget picker on it at once (Esc leaves it empty and enters Arrange mode). **Split in all four directions** (Q21): the menu has one row `Split  ←  →  ↑  ↓`; in Arrange mode `s` then an arrow (replacing `s`/`S`), shown in the legend; the palette has Split left/right/up/down. ADR 0005 records the change from D66. **Spec #34, tickets #35–#43** (identity → contact sheet, picker → Home only, split → swap drag → Arrange mode + Tray → New design, keyboard); #31 merged as the baseline (`81e45c3`). Owner: one app test after all of D87's tickets, not between them. **Q33 (owner, 2026-11-24, in #41):** the name prompt never interrupts while editing: an edit to a built-in Design is kept at once as "<Design> (edited)" (the built-in never changes; Reset design goes back); in Arrange mode the prompt comes only on Done and only if something changed; quick edits outside Arrange mode (Replace widget, a menu split) are kept silently with the bar note "Kept as <Design> (edited)"; Save design as… names one any time. It replaces D66's prompt on the first edit and #39's wait for the picker, and supersedes spec #34's story 34 ("naming prompt on the first edit"). Built in #41 (Arrange mode and the Tray). **Owner, 2026-11-24 (while building):** Q24 keyboard Move of rows (Alt+↑↓ + Move in a row's menu) is its own ticket #50, after D87; Q25–Q27 the library/queue popups behave like their boxes, with the hint line, palette words and bindable `library`/`queue` commands (no default key), and a box too small is no box; Q28 the well's ratio is 0.65; Q29 split + pick is one undo step; Q30 "Split" = split along the longer side (lit arrow), hints kept, no "new empty"; Q31 saved Designs are untouched; Q32 the scaling tickets #53–#59 come before the final test; Q34 lyrics grow up to 1.5×; Q35 the JetBrains Mono fix for big text stays for now; Q36 medium Home unchanged; Q37 no column-jump key; #54 Karaoke narrow-tall layout A on both tiers; Q38 the library's even column sharing and dropping "Playlist ·" probably yes, shown in the final test; **Q39 changed #41**: the player bar stays visible and usable while arranging, the Tray sits under everything and the Panes shrink evenly for it (#69); Q40 `×` outside Arrange mode sends to the hidden Tray; Q42 #58's centring extras kept, confirmed on sight. **Owner, 2026-11-24 (later):** Q43 the big Queue shows covers (C: a Now playing cover, two-row songs with a small cover each; #59), Q67 only from a Queue Pane of 90×32 (the PC's everyday 152×30 Queue keeps one row a song; owner may still prefer the covers there after the app test), Q68 the album under Now playing; Q44/Q45 Now playing narrow-tall and wide-short both A (title in the pixel font), Q46/Q54 tiny = cover + song name + artist; Q47 a title drag that starts Arrange mode shrinks the Panes once for the Tray (judge by feel); Q48 in Arrange mode the arrows, `s` and `+`/`-` belong to arranging ("arrange mode is for arranging"); Q49 the legend follows what you're doing; Q50 full keyboard support: ↓ onto the Tray, ←→ along it, Enter picks up, "+ Widget" its last stop; Q51 `move_pane` stays a name for `arrange`; Q52 Move mode's `u`/`U` gone; Q53 the wheel over the player bar works while arranging; Q55–Q58 legend wording, `s` then a non-arrow cancels, a click puts a keyboard pick-up back, `t` may send Home to the Tray; Q59–Q66 New design: the name prompt starts empty, Esc on it goes back to building, nothing saved before Done (supersedes spec story 73 for unnamed builds), "+ Widget" in all of Arrange mode listing only Widgets not on screen or in the Tray, no default key, New design… from Arrange mode starts over, keep the picker hint. **Owner's app test (2026-11-24):** passed overall; fixes before d87 reaches main (Q69): queue highlight follows its song (not its slot), the drag chip steady, a Widget carried into the Tray drawn in the Tray, `=` grows like `+`, the vertical `needs` line, tiny Home search results visible, the tall Now playing cover centred, a New design can be thrown away (Ctrl+D on its prompt, "Discard new design" in the palette while building, Q78; the prompt says "Esc keep building"); Q70 `m` (the `arrange` key) leaves Arrange mode like Esc; Q71 `w` opens "+ Widget" anywhere in Arrange mode; Q77 `+`/`=`/`-` move a Pane's side borders, `[`/`]` its top and bottom; Q79 the short `16×4` line at small sizes is fine; Q72/Q75 Delete design replaces Reset design (#80); Q73 `main` protected against force pushes and deletion; Q74 look tweaks with pictures first (#76); Q76 a fresh Daemon shows the last Home and library from disk and loads faster (#81). |
| D88 | The main Pane's page as one value (#27; architecture review candidate 8, step 2) | D74, D81, D83 | settled (2026-11-25), built on `page-one-value` | Owner accepted all four (Q84–Q87). **One value**: `App::page: Page`, each variant holding its view (Artist its `ArtistView`, Stats its `StatsView`, Settings its `SettingsView`; List names its list), replacing `open`'s "which list is open", `artist`, Search's open flag, `stats_view` and `settings_view`; keys, drawing, the wheel and history match on it exhaustively, so a page can't leave a stale view behind. **Q84 no visible change**: history keeps page names, not views, as now (back to a list restores its sort, filter and scroll; to Search its words and results; to Home where you were; Stats its period; an artist page and Settings start fresh); restoring an artist page's or Settings' place on back is an idea of its own. **Q85** `OpenList` keeps the lists and each list's remembered view (D81); the page only says which list shows. **Q86** Search stays beside the page with its query, recent searches and kept results (D74); `Page::Search` only says it shows. **Q87** an episode being looked up (`pending_episode`) is not a page: going to another page before the show is found cancels it (it used to pull you to the show anyway); going to the page you're already on doesn't. **Q88** (found while building, owner: yes) keys reach only the page shown: a key Stats or Settings doesn't use no longer falls through to Home's hidden Shelves (F10 there opened a menu for a Card on Home); with Home's search bar highlighted, a letter typed on Stats or Settings now runs its own key (q quits) instead of opening Search. **Q89** (owner: yes) a list you've left keeps today's behaviour when it changes while you're away (its highlight stays on the same row number); keeping the highlight on its song is #90. **Changed by #90** (`v1`): a list you've left now keeps its highlight on its song (or the row that took its place) when it changes while you're away, as the list shown does; a list dropped and fetched again in full still keeps the row number. |
| D89 | Architecture round 2: platform seams before the Mac, then the Client's hot spots (candidates 11–20) | D49, D50, D76, D83, D87, D88 | settled (2026-11-25), not built | Report: https://claude.ai/artifact/WUzRVYbMHDSh3G3CCCzygv (private). Owner: "take care of all of it"; accepted Q90–Q97. **Q90 two waves**, each on its own integration branch: `r2-platform` (11 Soloist runner, 12 OS media integration, 13 Host, 14 App folders, the PLATFORMS.md/glossary/ADR wording refresh, small fixes) reaches `main` **before the macOS session (#85)**; `r2-client` (15 Shown Design, 16 Arrange mode as its own module, 17 Screen plan, 18 Widget catalogue, 19 Gesture, 20 tiling folded into the Arrangement) after, with the owner's app test. **Q91 no visible change** except fixes found on the way: a seek-bar, queue or list-row drag whose release was missed lets go (the D84 stuck-drag fix covered only a held Widget and a divider); the stream Widget's memory figure uses the real page size (ARM kernels with 16 K/64 K pages read 4–16× too low); the Linux health check no longer requires `tar`, and requires `pgrep`/`pkill` only when the Spotify app is the Engine. **Q92** one switch, `SINGULARITY_NOWPLAYING=off`, on both OSes; `SINGULARITY_DBUS` still accepted. **Q93** `HOME` unset: one rule, stop with a clear message (today three behaviours, one writing into a temp folder); the socket folder keeps its own rule. **Q94** the Client asks the Daemon whether Soloist runs in a VM (an additive protocol field) instead of guessing from its own OS. **Q95** the VM runner's contract is tested on Linux against a fake VM, as the local Soloist is against `fake_soloist`; the real VM still runs only on the Mac. **Q96** glossary: Soloist runner, Host (widened), App folders, Shown Design, Screen plan, Widget catalogue, Gesture; OS media integration and Arrange mode become modules of their own. **Q97** 20 is the last, optional ticket of wave 2 (dropped if 16 already covers it). **Q98** on Linux, `SINGULARITY_NOWPLAYING=on` turns MPRIS on, even for a fake Engine. **Q99** on macOS, `SINGULARITY_DBUS` can only turn Now Playing off (`=off`), never on (an address counts only on Linux); with both names set, off wins. **Q100** with the Spotify app as the Engine, Linux keeps its own MPRIS entry beside the app's (as today). **Q101** the "moves with the music" list becomes a Widget catalogue fact, as groundwork for more music-reactive Widgets. **Q102** the health check no longer asks for `pgrep`/`pkill` on any Engine (nothing runs them since #96), replacing Q91's rule. **Q103** #104 also fixes a held divider that a popup/Ctrl+K, Ctrl+Z or a Design change interrupts (the line moved but wasn't recorded, or a bogus undo step was added), added to Q91's fixes. **Q104** the HOME-unset stop message and the export's "no Downloads folder" message are accepted as worded in PR #114; without `HOME` the app runs if all four `XDG_*_HOME` are set. **Q105** the Pane menu stops offering "Close pane" on the only Pane on screen (like the ×), and keeps offering it on narrow Panes where the × has no room. **Q106** the v0.1.0 release page warns that its macOS download doesn't start (no `b2c8d15`); a new release follows after the Mac session (#85) has run the fix. **Q107** a seek-bar drag whose release was missed seeks to where the bar was let go. **Q108** Ctrl+Z while a divider is held takes back that drag (the line returns; Ctrl+Y brings it back). **Q109** a Design switched while a divider is held keeps the move as the old Design's edit (its "(edited)" copy or its file); the new Design comes up untouched. A missed queue or list-row release lets go with nothing moved (follows D84's held Pane going back). |
| D90 | v1.0 release: what goes in, how it reaches `main`, and demo mode for videos | D89 | settled (2026-11-27), not built | **Q111** the wave 2 owner test passed (A1–A4, B1–B2, C1–C3, D1–D3, E1–E4; E5 skipped): "feels very good". Wanted next: Delete design (#80), faster search (#126), a faster first load (#81). **Q112** the v1 work reaches `main` as one PR (#127) whose release notes go biggest change first, in an honest tone. **Q113** v1.0 is tagged only after the macOS session (#85). **Q115** PR #127 (`v1` → `main`) stays a draft; #85's fixes land on `v1`; the merge and the tag happen together. **Q114** the project gets a YouTube channel (its plan is kept outside the repo). **Q116** no second Spotify account: demo mode works on the owner's own account. **Q117** a demo switch is built. **Q118** videos use YouTube-safe music that is also on Spotify. **Q119** the demo-mode design is accepted as a whole, all 12 points as recommended: (1) a launch switch only, `--demo` / `SINGULARITY_DEMO=1`, with a DEMO chip in the top bar and no toggle mid-session; (2) Home drops the shortcuts grid, every personalised Shelf and own-playlist Cards outside the Demo set, keeping the editorial Shelves; (3) the library's playlists are the contents of a Spotify folder called "Demo", albums, artists, podcasts and audiobooks come from a `demo.toml` allow-list; (4) Liked Songs is hidden unless `demo.toml` says `liked = true`; (5) other people's names become "a friend" / "Guest 1", the owner's own name "you"; (6) the Added column stays blank; (7) a playlist created during the demo stays visible for that session; (8) a separate demo Listening log (optionally seeded with made-up plays) feeds the stats, and plays during a demo still count in the real log too; (9) neutral device names ("Singularity", "Phone", "Speaker 2"), the audio output's name hidden; (10) the Jam shows a fake QR and no invite link; (11) the login doesn't open the browser by itself; (12) desktop track notifications stay as they are, turning them off goes on a recording checklist. Demo mode is one Client's property: a filter where Daemon events enter (`App::receive`) plus a redactor on every shown text; recent searches get their own file. **Q120** #80, #81 and #126 go into v1 before the tag. **Q121** the demo switch ships in v1 (#128). 2026-11-27, second round: **Q126** from the 11-27 bug-hunt re-check, C7 (Soloist downloads time out as a whole), B12 (the `spotify-app` Engine's zombie), B5 (a short nightly backup), B9 (the Daemon left outside systemd/launchd after an update) and the Karaoke fix (combining marks, CJK scrolling in tiny Panes) go into v1; C1 and the tiny ones come after v1. **Q127** search's typing pause goes from 300 to 200 ms. **Q128** "songs first" is built only if the log shows Spotify regularly taking more than 500 ms. **Q129** asked, not answered yet (the reordering is in v1 already). **Q130**/**Q131** open, reworked: the owner wants Delete built into the Pick a design box, with a way to undo it (follow-up **Q151** pending); Q131 depends on it. **Q132** not answered yet. **Q133** offline, the last Home and library stay on screen. **Q134** they don't expire. **Q135** Backspace deletes, q means no, the picker comes back, and the hint, texts and palette words as recommended. **Q136** in Search with a song highlighted: ↑ on the first song stays put, Home/End go to the first/last song, PgUp/PgDn page through the songs (in short Panes too), ← → do nothing. **Q137–Q142** #76's picks: today narrow B, listening narrow A, Now playing narrow A, wide-short A, big A, queue covers A. **Q143** the owner dislikes the beige/accent square behind the Home icon (option C): follow-up **Q152** pending. **Q144** the wheel and touchpad only scroll the view. **Q145** the hover highlight stays for now, decided for good on the Mac. **Q146** popups scroll their view too, in a ticket after v1. **Q147** the owner records the PC's wheel. **Q148** hover doesn't focus a Pane, and Cards don't light up. **Q149** demo extras: Add to demo in the Card menu, search without episode progress, the you/your rule for Shelf titles. **Q150** the owner runs demo mode once on the real account to check Home's section ids (moot since Q155). **Q155–Q157** (2026-11-28, built in #159) replace point (2) of Q119: demo Home is built from the Demo set alone (its playlists, then the albums and artists their songs are on, then Liked Songs if on), and the player stays empty until a song of the Demo set, or one played or queued from that Client, plays (only those count in the demo Listening log); whether `demo.toml`'s old `[home]` rules and Q149's you/your Shelf rule go is with the owner. |

```mermaid
flowchart LR
  D1[D1 Audience] --> D10[D10 Platforms]
  D3[D3 Playback] --> D10
  D3 --> D11[D11 Soloist lifecycle]
  D2[D2 Catalogue] --> D12[D12 Connect remote]
  D3 --> D12
  D2 --> D13[D13 v1 slice]
  D3 --> D13
  D13 --> D14[D14 Podcasts]
  D2 --> D15[D15 Lyrics]
  D10 --> D16[D16 Audio tap]
  D2 --> D17[D17 Settings]
  D3 --> D17
  D6[D6 Interaction] --> D18[D18 Layout]
  D6 --> D19[D19 Keymap]
  D1 --> D20[D20 Local data]
  D8[D8 Visuals] --> D20
  D4[D4 Language] --> D21[D21 Config]
  D7[D7 Theming] --> D21
  D4 --> D22[D22 Architecture]
  D2 --> D22
  D3 --> D22
  D22 --> D23[D23 Test seams]
  D10 --> D24[D24 Credentials]
```

## Known impossibilities (facts, not decisions)

- **Offline downloads**: DRM-protected and not exposed by any engine. Out of scope.
- **Lossless via librespot**: not supported and not planned upstream. Only Soloist streams lossless.
- **Home / Made For You / Browse via the official Web API**: no such endpoints exist.

## Round log

- **Round 1** (2026-10-17): D1–D9 asked. All settled except D8, which became a prototype (6 directions). Owner
  asked for in-depth private-API risk research (done) and a painless Soloist refresh (found: public CDN, auto-update possible).
- **Round 2** (2026-10-17): D8 reaction, D10, D11+D22 (process model), D12, D13, D15, D19, D20, D21 asked.
  Everything settled. D10 expanded to macOS (researched). D8 produced detailed feedback → prototype iteration 2.
  Facts found: the "popular terminal black hole" is s0xDk/ghostty-blackhole (MIT, ~1.7k★), now ported. The CPU
  cost of pixel visuals is high (~4 cores for a full-quality hero hole), which raised D26.
- **Round 3** (2026-10-17): D8 iteration-2 reaction, D14, D24, D25, D26 asked, plus the GitHub repo.
  All settled. D8 iteration 3 built (see the D8 row). The GPU path was prototyped and measured (ADR-0004). Private repo
  `publicusr/singularity` created. Black hole search: couldn't identify an orange/red pixel-art terminal
  black hole with confidence, so the owner may supply a link; a pixel-art look was built meanwhile.
- **Round 4** (2026-10-17): D8 iteration-3 reaction, D18, D27, shipping set, Phase 1 go-ahead asked. All settled.
  Owner asked for: more polished bending around the well (done: border lines re-drawn as braille, moving only
  sideways, with a moat around the well); the idle takeover using *the* hole from the well (done); a 3–5 min idle timer
  (3 min) with an easy off switch (Shift+I, palette, config); an assessment of free tiling (moderate effort, see research).
  Phase 1 started: spike 1 (Soloist control + audio tap) built, waiting on the owner's API key and pairing.
- **Phase 1** (2026-10-17): all spikes ✅. Soloist: pair, control, events, expiry, audio tap, and **lossless FLAC** once the
  device's quality is set to Lossless (it's per Connect device). Private API: device-code login + Pathfinder `home`
  gave the owner's real Home feed (31 shelves, ~1.5 s). Open: D17 (can Singularity set the device quality itself, via
  connect-state, instead of the phone?) and the automatic refresh of Pathfinder query hashes.
- **Phase 2** (2026-10-17): spec written (`docs/SPEC.md`), 22 tickets published (#2–#23), plus #24 from owner feedback.
  Built and closed #2, #3, #4, #7. Owner test: "everything feels good", but seek dragging must be precise to 1 s (#24).
  Handoff: `docs/HANDOFF.md`.
- **#24 seek precision** (2026-10-17): pixel-precise pointer via SGR-Pixels (DECSET 1016) on kitty/Ghostty/WezTerm (not
  in tmux; `SINGULARITY_PIXEL_MOUSE=0|1` overrides); elsewhere the press jumps to the cell and the drag then moves 1 s per
  cell (Shift/Ctrl/Alt: coarse absolute). Target time in a tooltip, one seek on release. Wheel ±1 s (Shift ±5 s),
  Shift+←/→ ±1 s; nudges coalesce (180 ms) into one command. Volume: same drag, wheel 1 %.
- **#8 Home** (2026-10-17): D28 asked and settled (both recommendations accepted). Home renders shortcuts grid, Shelves of
  square cover Cards and the feed tail as a grid ("More for you"); covers fetched lazily from the image CDN into
  `~/.cache/singularity/covers` (200 MB, pruned by age). Pixel tier: kitty images transmitted once, then only placed
  (z=-1); any cover under a popup steps down to half-blocks for that frame. Short Panes get smaller square covers. Click
  plays, right-click menu (Play / Go to / Copy link via OSC 52; Add to queue for single tracks). `--fake-catalogue <json>`
  serves a recorded feed for UI work.
- **#9 Library** (2026-10-17): owner allowed live read-only probes and quiet test plays. Facts in
  `research/2026-10-library-and-connect.md`: Soloist can't start a context at an offset; a Connect `play` with
  `skip_to.track_index` from a hidden controller (ConnectRemote) can, for albums, playlists and Liked Songs. Built:
  sidebar (Home, Liked Songs, playlists with folders indented, albums, artists), track lists that stream in page by page,
  header sort (asc → desc → list order), `/` filter, double-click/Enter plays from that track; the Engine plays the track
  alone if Connect fails. Esc inside a list goes back to Home.
- **#15 AudioTap** (2026-10-17): the Daemon taps Soloist's own PipeWire stream (`application.name="Spotify"` without a
  process binary, so the official app is never picked up) with `parec --monitor-stream`, only while a Client listens and
  only while playing. 2048-pt FFT at 30 Hz, 64 log bands, bass-flux onsets, automatic gain against the track's own
  loudness envelope (re-seeded on volume changes). Frames go only to subscribed Clients and stop after silence. Spectrum,
  loudness history and VU Widgets fill the `fft` and `loudness` Panes. Cost: analysis 0.04 % + parec < 0.1 % of a core.
  `--fake-engine` feeds the tap a synthetic groove.
- **#16 Black hole** (2026-10-17): Visual engine in the Client (`client/src/visual/`): the WGSL tracer on wgpu with
  persistent buffers, the CPU tracer of the same model (rayon, 420 px cap), equal within tolerance (test). Pixel tier:
  the hole is an animated kitty image (re-sent each frame, shm on kitty, z=-1 under text); cell tier: half-blocks. The
  gravity well's frame sags with the bass; neighbouring borders bend toward it (structure only, tested). `v`, a click
  or the palette cycles the Disk looks (Inferno first, as in the prototype; Pixel ember at 1/5 resolution, dithered).
  Unfocused, the Client drops to 10 fps (no terminal reports "hidden"). Budget met: 16.5 % / 6.5 % of a core.
- **Owner feedback** (2026-10-18): the black hole flickered (fixed: double-buffered kitty frames, 30 fps pace,
  redraws capped at 60/s) and clicking an album played it (fixed: Cards open their track list; ▶ Play there). The Client
  now replaces an outdated Daemon by itself after a rebuild.
- **#5 Setup & health** (2026-10-18): secrets in the keyring via `secret-tool` (Soloist key and, migrated on the next
  connect, the Spotify login), owner-only file fallback with a notice; `SINGULARITY_NO_KEYRING=1` for tests. Soloist's
  output is piped through the Daemon: stream format (codec, bitrate, average, rate), build lifetime, rejected key and
  a locked data dir are parsed from it, and the log rotates instead of being truncated. First-run wizard (requirements
  up front, masked key entry with paste, download, pairing, login, audio check), Health check in the palette, stream
  stats Pane with a LOSSLESS badge and the exact fix when it isn't.
- **#12 Queue** (2026-10-18): Soloist only appends, so play next / remove / clear go through ConnectRemote as a Connect
  `set_queue` (the player's `next_tracks` from the cluster, edited, sent back with its `queue_revision`; only
  `provider=queue` entries can be removed). Verified live on the owner's (paused) Soloist with one test track, added and
  removed. Queue entries Soloist sends bare get names from `getTrack` (cached). Queue Widget: now playing, Next in queue,
  Next from <context>, Autoplay (marked); right-click or Delete edits; track rows got a context menu (play from here,
  play next, add to queue, copy link).
- **#14 Lyrics** (2026-10-18): Spotify's `color-lyrics` (via the Catalogue session; Musixmatch, line-synced) with
  LRCLIB as fallback (artist, title, album, duration only), cached per track including "none". Widget: current line
  lit and held ~40 % down, `[mm:ss]` stamps like a log (the Pane is `tail -f lyrics.log`), click a line to seek, unsynced
  lyrics scroll, calm empty state, credit line. Lyrics are only asked for while a lyrics Pane is on screen.
- **#11 Album & artist pages** (2026-10-18): artist pages from `queryArtistOverview` (▶ Play, monthly listeners,
  Popular with play-from-here in the artist's context, Shelves: popular releases, albums, singles and EPs, appears on,
  fans also like, drawn by Home's Card renderer); album pages are track lists with a header (artists as links, year,
  label). History: back/forward stacks of pages (Alt+←/→, Esc/Backspace back), each list keeps its sort and scroll.
  Every artist and album name is a link (track rows, the player bar, list headers, popular tracks). Mouse back/forward
  buttons aren't possible yet: crossterm drops SGR buttons 8/9 (would need our own input parser).
- **#10 Search** (2026-10-18): `searchDesktop` (variables verified live), cached 5 min; superseded searches are skipped in
  the Daemon and the Client debounces (≥ 2 characters, 200 ms quiet since Q127, 300 before), so typing never storms. Search page (sidebar row,
  palette, Ctrl+L): chips All · Songs · Artists · Albums · Playlists · Podcasts (click or Tab), songs as a playable list
  (double-click, right-click menu, artist links), other types as Card Shelves (or a grid under their chip), track Cards
  play. Recent searches: remembered when a result is used, kept owner-only in `~/.local/state/singularity/`.
- **#13 Devices** (2026-10-18): ConnectRemote starts with the Catalogue session and follows the dealer's cluster
  pushes: a live device list, and while another device plays, its playback is what Clients see and the transport goes
  to it over Connect (pause/resume/skip/seek/volume/shuffle/repeat). Connect's `transfer` endpoint answers 500 to this
  session, so moving playback is a `play` on the target with the same context, track (`skip_to`), position (`seek_to`)
  and `initially_paused`; verified live both ways (owner's desktop app ↔ Singularity, paused). Fixed on the way: after
  any Connect failure the remote was dropped, and the dealer can't start twice on one session, so Connect features died
  until the next login. Device picker: `d`, the palette, or the player bar's ⧉ label. D17 answered (above).
- **#22 Like/follow** (2026-10-18): Daemon: `SetSaved {uri, saved}` → `addToLibrary`/`removeFromLibrary`
  (`libraryItemUris`), `CheckSaved {uris}` → `areEntitiesInLibrary` (`lookup[].data.saved`, verified live read-only),
  `Event::Saved` carries the truth (the old value on failure, for rollback). Client (`client/src/saved.rs`): optimistic
  toggles settled by `Event::Saved`; answers to older toggles are ignored while a newer one is out. ♥/♡ after the title
  in the player bar (click, key `l`, palette), "Save to / Remove from Liked Songs" (and Your Library / Follow wording
  by kind) in every track and Card menu, ♡ Save / ♥ Saved on album and playlist headers, + Follow / ✓ Following on
  artist pages. Removing a saved playlist takes a second click (it may be your own). Liked Songs is patched in place
  (a like lands on top, an unlike leaves, a rollback restores the row where it was; counts follow) and seeds what's
  liked. The playing track and every opened page re-ask, so changes from other devices show.
- **#21 MPRIS + notifications** (2026-10-18): `daemon/src/mpris.rs` (zbus 5.19; 6.0 was two days old) owns
  `org.mpris.MediaPlayer2.singularity` and follows `Hub::playback`, so it also shows another device's playback; its
  methods go through the same transport routing as Clients (`lib.rs: transport`). Player: play/pause/stop (= pause),
  next/previous, Seek (past the end = next) and SetPosition (ignored for a stale track id), OpenUri (`spotify:` URIs and
  open.spotify.com links), Volume, Shuffle, LoopStatus; Metadata with track id, length, title, artists, album,
  `mpris:artUrl` (the cover URL) and `xesam:url`; Seeked on jumps (> 1.5 s off the extrapolated position).
  Notifications (`org.freedesktop.Notifications`, each replacing the last, transient, cover as a cached file) are
  opt-in with `SINGULARITY_NOTIFY=1` until config files (#20). `SINGULARITY_DBUS=<address>|off` picks the bus; a fake
  Engine stays off the session bus unless asked. Soloist has no D-Bus connection, so there's no second player. Verified
  with real `playerctl` on a private bus (pty scenario `mpris`).
- **#6 Soloist auto-update** (2026-10-18): `daemon/src/update.rs`. The CDN (`soloist-builds.spotifycdn.com`, sends
  ETag and Last-Modified, verified 2026-10-18) is checked at start, then daily, every 6 h in a build's last week, and
  after failures at 15 min doubling to 6 h; the old build always stays. A new build is unpacked into `soloist.new/`,
  must answer `--version`, and is installed by renaming directories (previous kept as `soloist.old/`), which is safe
  under a running Soloist. The SoloistEngine restarts into it only at a quiet moment: paused or idle at once, else at
  the next track change (the new track then starts over); playback is picked up through ConnectRemote (`resume`, the
  generalised transfer: context, track, position, paused), else by the Engine alone. Installs from before tracking are
  adopted when unpacked after the CDN's Last-Modified (no pointless re-download). The expiry left the stream Pane; the
  health check shows version, build date and days left ("updates itself"), and warns only under 3 days. Verified in a
  pty with a stand-in Soloist (`tools/pty/fake_soloist.py`) and a fake CDN: v2 staged while v1 played, taken over at
  the next track, playback continued.
- **#20 Config files** (2026-10-18): new crate `singularity-config` (toml_edit 0.25), used by Daemon and Client.
  `~/.config/singularity/config.toml` and `keys.toml` are written with comments on first run; `schema = 1` with a
  migration chain that edits the document in place (`rename` keeps a key's spot and comments; a v1→v2 test proves it
  byte for byte); app edits go through `set` (comments and layout stay); saves are noticed by mtime+size once a second
  and apply live in both processes; bad values fall back to defaults and are reported (toast / Daemon log), never fatal.
  Settings so far: `notifications.track_change` (replaces `SINGULARITY_NOTIFY`; palette toggle writes the file),
  `visuals.gpu` (low-power default, high-performance switches adapter on the next frame), `visuals.unfocused_fps` (10).
  Keys: actions have ids (`like`, `play_pause`, …); `preset = "standard" | "vim"` (vim: h/j/k/l, g/G, Ctrl+D/U move;
  like moves to `f`); a user binding replaces that action's keys and takes them from others; a key bound twice is
  reported and the first (by name) keeps it; unknown actions/keys are reported. Movement aliases are off while typing.
  Help screen, hint bar and palette show the keymap in use. The pty harness now gives every run scratch config/state.
- **#18 Designs and themes** (2026-10-18): `design.rs` (five Designs: Layout preset, Themes, Disk look, player-bar
  prompt, Pane titles), `theme.rs` (built-in Themes incl. Cockpit's 5 and Terminal's 3 variants, theme files with
  `base` inheritance, per-Pane border colours, numbered titles, scanlines), `looks.rs` (switching, `t`, the pickers
  with live preview, Esc puts back, Enter keeps in `[ui]` of config.toml; `themes/` re-read when it changes, also while
  the picker is open; an `_example.toml` is written, files starting with `_` are skipped), `widgets.rs` (Now playing,
  levels, listening placeholder for #23, rain, scope rebuilt from the bands since the Daemon sends spectra, `top` from
  /proc). Every Design keeps a `home` Pane (browsing needs one), so Cockpit and Terminal differ slightly from the
  prototype. `ui.transparent` hands the theme's black to the terminal. Spectrum frames are requested whenever an audio
  Widget is on screen (re-evaluated on design switches). Layout gained `Size::Last` (fixed right/bottom column).
- **#19 Slots and saved layouts** (2026-10-18): `layouts.rs`. Slots swap within curated groups (audio: spectrum,
  loudness, levels, scope, rain; side: lyrics, queue, Now playing, stream, top, listening) by `v` on the focused Pane
  or the ◂ ▸ in its title, skipping Widgets already on screen; Home, the library and the black hole keep their Panes
  (`v` on the well still cycles Disk looks). Dragging and window resizes clamp by each subtree's own minimum size (a
  test drags every divider of every Design to extremes at several sizes: no overlap, no Pane under its minimum; found
  and fixed nested splits shrinking to one row). "Save layout as…" writes `layouts/<name>.toml` (the split tree,
  ratios written exactly, one split per line as TOML 1.1 inline tables); saved layouts follow the built-in Designs in
  the picker, the one in use is `ui.layout`, and it comes back identically after a restart (unit test and pty).
- **#17 Idle takeover ✅** (2026-10-18): `visual/idle.rs` (break-out from the hole's Pane or the top-right
  corner, Lissajous drift, body dimmed, cells lensed with the prototype's `lens_cells`, the hole as kitty image 2 under
  the text or half-blocks), `BlackHole::free_frame` (free centre, no stars; `Frame.center` in CPU and GPU), App idle
  clock with the waking input swallowed (`wake()` in key/pointer/paste), Shift+I writes `idle.enabled`, palette preview,
  `[idle] enabled/minutes` in config.toml, the well empty meanwhile. App tests: the waking key/click isn't acted on (the
  first `n` sends no `Next`, the second does), Shift+I persists across a restart, the well reads "the well is empty".
  pty `idle-pixels`/`idle-cells`: the free hole goes up as z=-1 frames (~19/s, under the lensed text), is deleted on
  wake, the well's own image comes back, and the waking Space doesn't pause.
- **Owner answers** (2026-10-18): keep the 10 fps slowdown when the terminal is unfocused; keep the black hole on the
  low-power adapter (iGPU) and add a `gpu = "low-power" | "high-performance"` setting in #20.
- **Round (#23 local data)** (2026-10-18), owner said yes to all five: a Play is ≥30 s (or half a shorter track),
  logged when the track changes or stops, Connect devices included and named; nightly backup = Liked Songs + owned
  playlists in full, followed ones by name + link, ~1 request/s; keep 14 nightlies + the first of each month for a
  year; export = one JSON file (log + latest backup) to Downloads, delete asks twice; Cockpit's Pane = 26-week heatmap
  of minutes per day + "N h this year · top: Artist · streak N days".
- **#23 Listening log and library backups ✅** (2026-10-18): `daemon/src/store.rs` (LocalStore: SQLite via rusqlite
  0.40 bundled, `plays` with a local `day` column, `Recorder` (wall-clock playing time, pauses don't count),
  `summary` (local days via `localtime_r`, no time-zone crate), backups `backups/library-YYYY-MM-DD.json` with
  retention, export, delete + VACUUM; files 0600, `backups/` 0700), `backup.rs` (one `CatCmd::Query` at a time, 1 s
  apart, so browsing isn't held up; owned = `ownerV2.data.username` equals the session's; a playlist whose
  `revisionId` is unchanged is copied from last night's backup, not fetched; checked at start and hourly, due after
  24 h). Protocol (additive): `GetListening`, `ExportLocalData`, `DeleteLocalData`, `Event::Listening`, broadcast
  after each logged play and after a delete. `SINGULARITY_DATA=<dir>|off`; a fake-Engine Daemon keeps no log unless
  asked. Client: Cockpit's listening Pane (week columns, Monday on top; short Panes stack two days per cell in
  half-blocks), palette Export / Delete (a second run within 10 s confirms). The fake Catalogue answers raw queries in
  Spotify's shapes. pty `listening`: seeded scratch log, a 31 s play shows up live, backup taken in 10 s (paced),
  export 0600 with the backup inside, delete empties the Pane and removes the backups.
- **Owner test round 1** (2026-10-18), feedback in 9 points; my findings, and the questions put to the owner (pending):
  - Found first: the owner's running Daemon was the 00:08 build (started 00:12, before `c026772` "Client replaces an
    outdated Daemon"), so it ignored the new Clients' `Shutdown`. Everything tested ran new Client screens on an old
    Daemon (no like/follow, MPRIS, config following, listening log). Owner to restart it once; fix so a Daemon that
    ignores `Shutdown` is stopped anyway (peer pid via `SO_PEERCRED`, SIGTERM).
  - 1 + 6 wheel in track lists / library "works 50/50": over a row's artist or album name the hit is `Hit::Link`, which
    the wheel handler ignores. Wanted: the highlight follows the wheel. Small.
  - 2 Home drops a Shelf row unless it fits whole, and scrolls a row at a time. Fix: line-based scrolling, rows clipped
    at both edges (kitty placements can crop the source; half-blocks clip by row). Medium.
  - 3 a clickable back (◂) / forward (▸) in the main Pane's title while there's history. Small.
  - 4 loudness "maxed at 10 %": by design (D16 automatic gain, the tap is post-volume). Proposal: meters show the real
    level (follow the volume), spectrum/Visuals keep auto gain. Small.
  - 5 lyrics: Spotify's, then LRCLIB, already there (#14); misses are logged at debug only. Re-test after the Daemon
    restart; say why when there are none. Idea: Genius as an opt-in last fallback (no lyrics API: search via its API or
    site, scrape the page; unsynced text only; against Genius' terms, brittle). Medium.
  - 7 tint covers and the black hole with the Theme: feasible (gradient-map luminance onto the Theme's ramp, once per
    cover and Theme, both tiers; the hole: a post-pass in the CPU tracer and a uniform in the WGSL). Medium.
  - 8 drag to reorder the queue: feasible over Connect `set_queue` (already used for remove / play next); likely only
    the songs you queued yourself, to be verified quietly. Medium.
  - 9 fullscreen: reproduced at 256×68 (pty `PTY_COLS`/`PTY_ROWS`): the well gets very wide, the track list stretches,
    the queue is mostly empty. The hole renders round here; the owner sees it stretched (cell size? screenshot asked).
    Proposal: the well keeps a ~16:10 shape on wide terminals and the middle gets the width; track-list columns capped.
  - Also seen at 256×68: a kitty escape printed as text (`Ga=…`) in a track row once; harness or real, to check.
- **Owner answers, round 1** (2026-10-18): yes to 1–4 and 6–9 (queue drag limited to own songs "for now, may change
  after testing"). On 5: after the Daemon restart lyrics work and sync "works very well"; new bug: at half-monitor
  width long lines are cut off (wrap them). Genius parked: it has no timings, so it could only scroll at the song's
  pace; asked whether to keep it. Owner's fullscreen screenshot: the hole is round, the Pane is very wide (fixed by
  #31); the right column and the bottom hint row looked cut off (checked in #31). Owner screenshots stay out of git
  (`docs/screenshots/` ignored: personal data). Tickets: #25 stale Daemon, #26 wheel/highlight, #27 Home lines,
  #28 back/forward, #29 meters, #30 lyrics wrap, #31 wide terminals, #32 queue drag, #33 Theme tint.
- **#25 Stale Daemon ✅** (2026-10-18): the Client reads the Daemon's pid from the socket (`SO_PEERCRED`) before asking
  it to make way; if it still answers 5 s later and `/proc/<pid>/exe` is a `singularityd` (also "(deleted)" after a
  rebuild), SIGTERM, SIGKILL after 2 s, then it starts its own. Daemon test knob `SINGULARITY_IGNORE_SHUTDOWN`.
  Unit test (a renamed `sleep` is stopped, a plain one isn't); pty `stale_test.py`: replaced 5.2 s after connecting.
- **#26 Wheel moves the highlight ✅** (2026-10-18): the wheel goes by the Pane under the pointer (`pane_at`), not the
  hit under it, so a row's artist/album links no longer swallow it. Over a track list or the library it moves the
  highlight one row per notch (Shift: a page), the list follows, and the Pane takes focus so the highlight shows.
  Popups keep the wheel to themselves. Test wheels over an artist link and the library; pty `wheel`.
- **#27 Home by lines ✅** (2026-10-18): `HomeView.offset` (lines) replaces the first-row index; rows are drawn with an
  `i32` y and clipped to the Pane (text lines, Card hits, covers). A part-visible cover is the same upload placed with
  a source crop (kitty `x,y,w,h`; `Kitty::place_cropped`), or its visible half-block rows (`blit_halfblocks_from`).
  2 lines per wheel notch; keyboard moves keep the selected row whole in view. Search and artist pages share it.
  The pty harness had two bugs behind the stray `Ga=` text and a misplaced hole: a read ending inside `\x1b_G`
  went to pyte as text, and an image's position came from a regex over a possibly split cursor move; it now keeps
  the partial escape and places images at pyte's own cursor, as kitty does. Its compositor honours crops. pty
  `home-scroll`.
- **#28 Back / forward arrows ✅** (2026-10-18): the main Pane's title starts with ◂ ▸ whenever there's history
  (`Hit::History(±1)` → `go_back` / `go_forward`); the one with nowhere to go is dimmed. Title built from spans
  (number, arrows, name). Test clicks ◂ then ▸; pty `history`.
- **#29 Meters follow the volume ✅** (2026-10-18): `SpectrumFrame` gained `out_level`, `out_vu`, `out_bands`
  (bass / mid / treble): the real output level, no automatic gain, on a fixed 60 dB scale (bands against a
  full-scale sine's −12 dB). The loudness Pane (VU + history) and the levels widget read them; the spectrum, the
  black hole, rain and the setup's "audio heard" check keep the gained values. Additive (`serde(default)`). The fake
  Engine's synthetic audio is now post-volume (square law) and resets the gain on volume changes, like the real
  capture. Test: 20 dB quieter = a third of the scale lower on every output meter, the gained level unchanged.
  pty `meters` (Cockpit at full volume and at 10 %).
- **#30 Lyrics wrap; say why there are none ✅** (2026-10-18): long lines wrap by words (`lyrics::wrap`, display
  widths via `unicode-width`, a word wider than the Pane is broken); continuation rows are indented 2, the timestamp
  only on the first row; a synced line's rows are all click targets. The current line is held ~40 % down, moved up
  just enough that its last row stays in view (its first row wins in a Pane shorter than the line). Unsynced lyrics
  scroll by rows and stop at the end (the scroll is clamped, so the wheel turns back at once). The credit shortens to
  fit ("LRCLIB · unsynced"). `Lyrics.missing` (additive) says why there are none: "no lyrics on Spotify or LRCLIB",
  "an instrumental, says LRCLIB", or which source couldn't be asked; the Daemon logs every lookup at info
  (`lyrics for <uri>: Spotify Nothing, LRCLIB Found`), tells Spotify's 404 (`ErrorKind::NotFound`) from a failure, and
  remembers only definite answers, so a failed lookup is retried at the next ask. Tests: wrap widths 3–40 and wide
  characters; every line of a 30-line song at Pane widths 20–60: the current line's rows are contiguous, in view and
  join to the whole line; unsynced scroll stops at the end. pty `lyrics` at 140×40 and 96×34 (the fake lyrics gained
  a long line).
- **#31 Wide terminals ✅** (2026-10-18): `Node::compute_shaped(area, cell_aspect)` (used to draw; `compute` stays
  for "is this Slot on screen" checks): a gravity well more than 10 % wider than 16:10 in pixels (the terminal's real
  cell size, else 1:2) gives the extra width to its neighbour across the nearest side-by-side split, so in
  Singularity the middle column grows (256×68: well 95 → 83 columns, ~1.56:1; Home shows 7 Cards a row instead of 5).
  Dragging that divider still works; past the cap it stops. Track-list columns stop growing (title 60, artist and
  album 40), so Added and Time stay next to the names. The owner's "cut off" right column and hint row: the
  screenshot's terminal grid runs past the captured area (the status text and the Panes' right borders end exactly at
  the image's edge, the hint row is just below it); the pty at 256×68 draws both whole, so not ours. The `Ga=` text
  was the harness (fixed in #27). Tests: shapes at 256×68 / 400×60 with cell aspects 2.0 and 2.2, the middle takes
  exactly what the well gives, no change at 120×40 or without a well; column caps at widths 60–400. pty `library` and
  `home-pixels` at 256×68 and 140×40.
- **#32 Queue drag (built; live check pending)** (2026-10-18): `Command::MoveInQueue { uri, uid, to }` (`to` = place
  among the songs you queued; additive), `QueueEdit::Move` over Connect `set_queue` (the song is taken out and put
  before the `to`-th remaining queued song, else after the last; context and autoplay untouched; a context song
  can't move). Client: pressing a queued row arms a drag (`QueueDrag`), the pointer's row is the place (clamped to the
  queued songs, never into the context), the list shows the new order while dragging with the song highlighted, and
  the drop reorders at once and sends the move. A failed edit sends a toast and `CatEvent::Resync`: the hub re-sends
  its last real playback state, which undoes the early order. The fake Engine moves too. Tests: Connect edit (front,
  past the end, context refused), protocol round trip through the fake Engine, App drag (preview, optimistic order,
  command, clamp, context row inert). pty `queue`: three songs queued, the last dragged to the top, the Daemon's
  order comes back as dropped. Not yet: the quiet live check on the owner's account; their Daemon (03:53, #29) predates
  the command, and the next Client start replaces it.
- **#33 Theme tint ✅** (2026-10-18): `theme::Tint`, a gradient map from brightness (Rec. 709, lifted ^0.8) onto
  black → the Theme's ramp, so dark stays black and the top is the ramp's (never white). Themes have `tint` (on for
  Cockpit's five and Terminal's three; theme files can set `tint = true/false`); `ui.tint_art` (default true) turns it
  off everywhere. Covers: the tinted copy is its own cache entry (kitty upload key and the cell tier's memo are the
  URL plus the tint's colours), made once per cover and tint; the kitty LRU (300) retires the old ones. The black
  hole: `Frame.tint` (4 stops) in the shared `finish` (CPU) and the WGSL's tint mode, which reuses the palette
  uniforms, so the uniform layout is unchanged; pixel-art looks quantise onto the tint's stops. `BlackHole::set_tint`
  drops the cached frame, so a Theme switch shows at once. Config schema 2: a migration adds `tint_art = true` with
  its comment at the end of `[ui]` (an untouched old file becomes today's default byte for byte); `keys.toml` keeps
  its own schema (1). Cost: GPU (the default) no measurable difference (Inferno 16.4 → 14.7 %, Pixel ember 6.6 → 6.7 %
  of a core at 1280×720, noise); CPU fallback at 420×236: Inferno within noise, M87* ≈ +0.3 ms (≈ 7 %) per frame
  after replacing a per-pixel `powf` with a table. Tests: tint maths, built-ins that tint, theme-file override,
  migration, tinted memo, cell-tier cover colour follows a Theme switch and `tint_art = false`, a tinted hole is
  green-dominant, GPU vs CPU tinted within tolerance (mean |Δ| ≤ 0.05). pty `home-pixels` in Singularity with
  `terminal-green`, tint on and off.
- **Owner answers, after round 1** (2026-10-18): queue drag works while playing. Genius **dropped** (no timings,
  fragile, against its terms; the Pane already says why there are no lyrics). v1.x planning: go.
- **#32 follow-up: drag while paused; "Next from" songs** (2026-10-18): the paused snap-back was Connect, not the
  drag. The Daemon log showed every move failing with "that track isn't in the queue any more": Soloist hadn't
  played since the Daemon started, so it wasn't the active Connect device and Spotify's copy of the queue wasn't its
  queue. Not patched over by rebuilding the list from Soloist's own (it reports only 20 upcoming of e.g. 80, so a
  `set_queue` from it could cut the context short). Instead every Connect queue edit now checks that Singularity is
  the active device first, and says so if not ("press play once"); play next then falls back to Soloist's
  add-to-queue as before (before, it could be sent against another device's list). Quiet live test (probe
  `--swap-context`, while the owner played): swapping two upcoming context songs via `set_queue` is accepted, both
  stay `provider = context` in their new places, and swapping back restored the queue exactly. So "Next from" songs
  are draggable now, each song within its own section (queued among queued, context among context); autoplay stays
  Spotify's. `MoveInQueue.to` = place within the song's own section. Open: the paused case is unconfirmed (asked the
  owner what "not playing" was).
- **v1.x planning, round 1** (2026-10-18): owner said yes to the order (D29), the unbuilt v1 items (D30), stats (D31),
  the Settings screen (D33), polish (D34), podcasts (D35, with the extras' feasibility to look up), demos for Visuals
  (D36: "build out the demos"), free tiling as a trial (D37), macOS last (D38). Playlist editing as proposed, plus
  custom covers "if it's not too hard" (D32). Found while preparing: waveform seek bar (story 40), mini-player (story
  67) and the album-adaptive theme (D7) were never built or ticketed. Facts for round 2 in
  `research/2026-10-playlists-and-podcasts.md`: every playlist edit and cover upload has a private route in the web
  player; audiobooks launched in Poland 2026-10-15 (12 h/month with Premium).
- **v1.x planning, round 2** (2026-10-18): yes to all six: covers in batch 3 (D40), audiobooks behind a Soloist
  check (D41), video podcasts as audio, ranking by minutes with calendar periods (D39), in-app demo Widgets, spec now
  and tickets per batch. Spec: `SPEC.md` "v1.x" (stories 68–86). #32 closed: the owner confirms drags work while
  paused; the earlier failures were a just-started Daemon (Soloist not yet the active Connect device), which now says
  "press play once".
- **v1.x confirmed** (2026-10-18): D42 yes; the owner confirms the plan ("start building"): batch 1 (#35 → #36 → #37),
  then the Visual demos, then batch 2.
- **#35 Stats in the Daemon ✅** (2026-10-18): `Command::GetStats { period }` → `Event::Stats` (`Stats`, `StatsRow`,
  `StatsPeriod`; additive). `Store::stats`: calendar periods in local time (`period_start`: week from Monday, month,
  year, all time = from the first logged day), minutes and plays, the current streak and the longest run in the
  period, minutes per local hour (`local_hour`), top 50 artists (every artist on a track gets its minutes, via
  `json_each`), tracks (with all-time play counts) and albums (grouped by album and first artist), ranked by minutes,
  then plays, then name. The log is schema 2 (`migrate`: `album_uri`, `artist_uris`, index on `track_uri`; the old
  code re-set `user_version = 1` on every open, so versioning starts working now); Soloist's and Connect's items
  already carry the links, older rows keep NULL and carry a `track_uri` to find them through. Freshness: rather than
  the Daemon remembering who asked, a Client asks again after `Event::Listening` (sent after every logged Play).
  Tests: rankings, duet credit, week/month/year/all boundaries, streaks, empty log; a schema-1 file migrates keeping
  its row; protocol round trip; `tests/stats_tz.rs` (own binary, `TZ=Europe/Warsaw`): plays on either side of the
  2026-10-25 change land in the right week and hour.
- **#36 Stats page ✅** (2026-10-18): `client/src/stats.rs`, `Page::Stats` (in history; Ctrl+K → Stats, or a click on
  the listening Widget). Period tabs (long labels, short, or short without padding, whichever fits; ←/→ always walk
  them and never seek), the date range when there's room, a summary (minutes · plays · streak, longest in the
  period), a 24-hour clock in eighth-blocks on the Theme's ramp (left out below 16 rows, so the summary and lists fit),
  and the top 10 artists / tracks / albums: side by side from 108 columns, stacked below. Rows show minutes, a bar
  and plays as room allows ("N× (M ever)" for tracks outside All time); text lines wrap (`lyrics::wrap`) rather than
  being cut. A click on an artist or album opens it, a track plays on Enter or a double-click, right-click gives the
  track menu, the wheel moves the highlight. The Client asks again after `Event::Listening`. Old plays' links:
  answering `GetStats` makes the Daemon describe up to 20 unlinked tracks (`Store::unlinked_tracks`, cached
  `getTrack`, one at a time) and `backfill` their plays, then broadcast `Listening`, so the page refreshes with
  links; a row still without one says "Finding its link". `Theme::ramp_at` replaces two private copies. Tests: every
  period at 80×24 / 140×40 / 256×68 with nothing over the Pane's border and the summary in view, artist opens and Back
  returns (asking again), Enter plays, tab click, refresh, unlinked row, empty log wraps. pty `stats` at the three
  sizes: a schema-1 seeded log migrates, gets its links from the fake Catalogue, and the top artist opens.
- **#37 Stats Widgets ✅** (2026-10-18): two side-group Widgets in `stats.rs`. **today**: minutes today ("under a
  minute today" once something played), plays today (`ListeningSummary.plays_today`, additive), the streak, and a
  7-day sparkline with weekday initials when there's room; a click opens Stats. **top this week**: the week's top 5
  artists or tracks (`App::top_week_tracks`; the title "top artists · week ⇄" is the switch), rows as on the
  Stats page (artist opens, track plays on a second click, right-click menu). Data: `want_widget_data()` (replacing the
  layout-change calls to `want_spectrum`) asks for the week's stats when the Widget is on screen, and
  `refresh_stats` asks again after every `Event::Listening`. `row()` subtracts saturating (tiny Panes). Tests: both
  swap into every side Slot of every Design and render; Pane minimums 12×3 up to 60×12 draw nothing outside; the
  switch, a track's second click, an artist opening; the week asked for on swap and on refresh. pty `stats-widgets`:
  both swapped in with the Panes' ▸, a 31 s play is logged and both update.
- **Visual demos built** (2026-10-18, D36): `client/src/demos.rs`, in the Slot cycles (`v` / ◂ ▸), titled "… · demo".
  **starfield** (audio group): 3-D stars toward the viewer, speed eased toward 0.15 + energy + a beat kick (no
  strobing), streaks back to the last position when a star moves more than a cell, round in pixels (cell aspect).
  **up next** (side): the queue as a band, each song as long as it lasts over the next ≤ 30 min (queued / context /
  autoplay coloured apart, neighbours shaded apart), names under, a "now · +5m · +10m" axis. **marquee** (side): the
  title in a 3×5 pixel font in half-blocks (accents fall back to plain letters), centred or scrolling at 8 columns/s,
  the artist under it. **glow** (side): the cover in the middle, its average colour (saturated ×1.6, tinted when the
  Theme tints) bleeding out to black, breathing with the bass. Starfield and glow ask for spectrum frames. Also: fake
  Engine tracks have fixture covers now (pty screenshots show real pictures); a Daemon without a log no longer toasts
  on questions (only on export/delete). Tests: every demo stays inside its area from 12×3 to 160×30; the font's
  glyphs; pty `demos` (pixel tier) swaps each in while a song plays.
- **Owner report: stuck Slots** (2026-10-18): cycling a narrow Pane onto a long title ("listening · 26 weeks",
  "starfield · demo") hid its ◂ ▸ (they showed only when the Pane was 12 wider than the title), so the Slot could
  only leave with `v`. Now every swappable Pane keeps its ◂ ▸ and a long title is shortened with "…"; "top this week"
  is titled "top artists · week ⇄". Test walks every swappable Pane of every Design through its whole cycle at 80×24
  and 140×40; pty `arrows` does the owner's walk (bottom-left Pane, spectrum Pane) by clicking ▸. Owner likes rain
  and the starfield demo. Asked: where ◂ ▸ belong (the main Pane's back/forward sit left, Widget swaps right).
- **Owner test round 2** (2026-10-18): steps 1–9 work (wheel, Home by lines, back/forward, meters, lyrics, fullscreen,
  queue drag, Theme tint, Stats). Asked for: the artist page doesn't scroll with the wheel (#38); rain "should be way
  chiller" (#39; later "slower now, nice": calmer and eased, not jumping per beat); the well still too wide on
  fullscreen (#40: 4:3); Ctrl+K turns covers and the hole into blocks (#41: they were above cell backgrounds, so any
  image a popup overlapped stepped down; the owner runs kitty 0.47.1, which can place images under backgrounds); every
  Design keeps Home, library, queue, lyrics (D43, #42); arrows standardised (D44, #43); demos: starfield, marquee, glow
  kept, up next dropped, and `top` read as a stats Widget (#44). Swap arrows no longer vanish (owner confirms).
- **#38–#41** (2026-10-18): **#38** over an artist or search page the wheel goes by the Pane, not the hit: over a
  song row (or its name links) it moves that list's highlight, anywhere else it scrolls the Shelves by lines (Shift:
  sideways). **#39** rain: base fall halved (3–9 rows/s), the music eases a boost up to about ×2 and back over a second
  or two (a beat adds +0.3 to the target, no jumps). **#40** the well's cap is 4:3 (5 % slack); at 256×68 it is ~70
  columns (95 before #31). Tests now find Panes with `compute_shaped`, as the screen does. **#41** in kitty
  (`term::below_backgrounds`: `KITTY_WINDOW_ID` or `TERM=xterm-kitty`; `SINGULARITY_BELOW_BG=1|0`), images go under cell
  backgrounds (z = -1073741825) and the cells under them get the default background (`App::under_image`), so popups
  hide what they overlap and nothing steps down to half-blocks; the idle takeover's hole stays at z=-1
  (`Kitty::keep_over_backgrounds`). Other terminals keep the old step-down. The pty compositor layers as kitty does
  (default background, under-background images, cell backgrounds, z=-1 images, text); pty `popup`.
- **#42 Every Design keeps Home, library, queue, lyrics ✅** (2026-10-18, D43): Classic+ = library | Home over the
  queue | Now playing over lyrics (right column `Last(40)`); Cockpit's bottom row = Home | queue over lyrics | library;
  Terminal's right column = osc / queue / fft (`top` is one ◂ ▸ away in the queue's Slot). Singularity and Event
  Horizon already had all four. Test `every_design_has_home_the_library_the_queue_and_lyrics`; pty `designs`.
- **#43 Arrows standardised ✅** (2026-10-18, D44): the main Pane's back/forward moved from before its title to the
  right end, where every Pane keeps its arrows, drawn as ← → (dim when there's nowhere to go); the Disk look keeps
  "◂ Inferno ▸" at the right. Test pins the arrows in the title's right half; pty `history` clicks ← and →.
- **#44 Ambient Widgets ✅** (2026-10-18): the kept demos are regular Widgets in `client/src/ambient.rs` (was
  `demos.rs`), titled plainly; up next is gone. starfield: stars sit a little apart on the Theme's ramp and far ones
  twinkle; ~600 stars cost well under a millisecond a frame (debug-build check < 8 ms). marquee: a new title starts
  from the beginning, beats lift its colour and fade over half a second. glow: its colour eases into the next cover's
  over ~0.4 s instead of jumping. Terminal's `top` is titled "top · system". pty `ambient` (pixel tier, kitty layers).
- **Owner report: music pausing and unpausing** (2026-10-18): not Singularity. Soloist's log: buffer underruns every
  15–20 s, its download sampled 224 → 122 → 70 → 59 → 50 → 44 kB/s (lossless needs ~175–205). The machine sent
  ~1.5 MB/s: Stremio's streaming server (`node …/stremio/server.js`, a background service, 1 GB sent) saturating the
  uplink. Owner told to quit it (not killed by me). Owner said yes to surfacing this in-app: #45.
- **#41 follow-up: covers through Ctrl+K** (2026-10-18): the owner saw Home's covers through the palette. kitty
  decides "default background" by colour: the popups' pure black equals its own default, so images under backgrounds
  showed through them. A pass at the end of each frame (`opaque_overlays`, kitty layer only) turns black/default
  backgrounds inside every popup into #010101. The pty compositor now judges by colour too (explicit #000000 counts as
  default), so the bleed would have shown in screenshots. Test asserts the palette's background is neither default
  nor the Theme's black.
- **Owner report: rain and fft "lag for a second"** (2026-10-18): not the Client. Soloist's log at 17:08:48–54: the
  next track was prepared with a thin buffer (still cautious after the slow-connection episode, "downgrade bandwidth
  factor 2"), so the song change had ~1 s of silence; the audio Widgets froze/eased and caught up. GPU readback
  measured 4–5 ms at worst over 540 frames (no spikes). If it happens mid-song without the music stopping, the next
  step is a non-blocking GPU readback (the UI waits on `PollType::wait_indefinitely` today). #45 will make such stalls
  visible in the stream Pane.
- **#45 Say when the connection is too slow ✅** (2026-10-18): `LogParser` reads `FileStreamer bandwidth sample: N kBps`
  and `audioThreadMain: underrun` (the recovery line, `updateInternalDriverToPlaying: underrun`, isn't a stall); the
  SoloistEngine keeps a `Delivery` (latest speed + when, stalls of the last 2 min) and sends `Event::Delivery`
  (additive; also in the connect snapshot). Client: the stream Pane adds "download 44 kB/s, need 205" (red when below
  what the stream needs: its measured average, else nominal bitrate, ÷ 8000) and "stalls N in the last 60s"; a toast
  when ≥ 2 stalls in a minute on a too-slow (or unmeasured) connection, at most once per 10 min: "Connection too slow
  for lossless right now: 44 kB/s, about 205 needed; is something else using it?". Tests: the real lines from the
  owner's episode; App test (one stall: quiet; repeated: one toast; Pane rows; healthy: no warning). Fake Engine knob
  `SINGULARITY_FAKE_STALLS=1`; pty `stalls`.
- **#50 Album song into "Next in queue" ✅** (2026-10-18, D42): the drag model is now (from, to, into_queue) over a
  `preview` of the queue (the moved song shown in place, under the right heading); `target` maps the pointer through
  the queue's real lines, headings included: a context song onto a queued row takes that place in the queue, onto the
  "Next from" heading goes last of the queued, onto a context row stays in the context; queued songs stay queued.
  `MoveInQueue.into_queue` (additive) → `QueueEdit::Move { into_queue }`: the context entry is removed and a queued
  one (`{uri, uid:"", is_queued, provider:"queue"}`) inserted at that place. Fake Engine: lifts too, and a PlayFrom now
  adds the list's next four songs as context entries. Tests: preview and sections, the Connect edit, the App drag
  (onto a queued row, onto the heading, queued never into the context). pty `queue-lift`. Live check: probe
  `--lift-context` (snapshot → lift → read back → restore) skipped, the owner's queue had no queued song (it then
  would play next); Spotify's context run also holds `spotify:delimiter` markers, which the probe skips.
- **#49 ♥ on every track row ✅** (2026-10-18): `saved::heart` draws ♥ (liked, accent) / ♡ (not, dim) / blank (not yet
  known) and returns a `Hit::Save` (the existing optimistic toggle with rollback). Track lists get a 2-cell column at
  the end of the row (after the time, so the sortable headers keep their columns); search songs and artist popular
  tracks get one too; Liked Songs' rows are known liked. `App::want_saved(rows from the first in view, visible)`: once a
  row on screen hasn't been asked about, the next 50 go in one `CheckSaved` (the Daemon already queries 50 a time), so
  scrolling sends about one batch a screenful (the first try sent one per new row: 55 batches over 70 rows, caught by
  the test). Test: a 500-row playlist, one batch of 50 on open, none within the window, ≤ 2 past it; an answer shows
  ♥/♡; a click sends SetSaved and shows at once. pty `like`.
- **#46 Waveform seek bar ✅** (2026-10-18): `client/src/waveform.rs`. A track is 200 slots; each spectrum frame while
  playing records the loudest gained level there (heard silence counts as heard). The seek bar draws heard slots in
  eighth blocks on the track's own scale (quietest heard ▁ … loudest █: the automatic gain keeps most music near the
  top), the played part in the accent, the rest dim; unheard stretches stay a line (━ played, ─ not). A track heard to
  95 % is kept as a 200-byte file under `~/.cache/singularity/waves/` (FNV hash of the URI; at most 2000, oldest go)
  and shows whole from the start next time. Seeking unchanged. Tests: filling, max per slot, keep/load round trip,
  partly heard not kept, pruning; the bar shows loud/quiet stretches and a plain line where unheard. pty `waveform`.
- **#47 Mini-player ✅** (2026-10-18): `client/src/mini.rs`; below 50 columns or 14 rows the layout gives way to a
  player (was "terminal too small" under 20×8). One row: |◀ ❚❚ ▶|, title — artist, time; two rows: title + ♥ over
  controls, progress (seekable) and time; more: a centred block (title + ♥, artist, progress with both times,
  controls). Keys work as usual; toasts show on the bottom row. Test at 20×1, 40×2, 60×5, 49×13, 120×13 (title, nothing
  spilling, ▶ sends TogglePlay, a seek bar from two rows) and back to the full layout at 80×24. pty `mini`
  (`PTY_COLS`/`PTY_ROWS`).
- **#48 mouse back/forward: blocked on a decision** (2026-10-18): crossterm 0.29's `parse_cb` turns SGR buttons 8/9
  (codes 128/129) into a parse error and drops the event, and it reads the tty itself, so nothing can see them
  without (a) a vendored crossterm with a 3-line patch (`[patch.crates-io]`, ~12k lines in the repo, re-applied on
  upgrades) or (b) our own full input parser. Put to the owner with (a) recommended.
- **Owner test round 3** (2026-10-18): #45 stream Pane works; #50 lifting an album song into the queue works on the
  real account (the pending live check, done by the owner); ♥ on rows works, checked against the phone; the
  mini-player works. #48 dropped (D45). Open: the owner couldn't find the waveform (#46): it lives in the player bar's
  seek bar and only shows where a song has been heard while Singularity plays it; check next session whether it reads
  as a waveform on real music (a 1-row eighth-block bar may look like a thick line) and whether to make it more visible
  (taller, or show it in a Widget), with the owner.
- **#46 follow-up: the waveform was a flat bar** (2026-10-18, fifth session): the owner's four cached waveforms were
  all full blocks: each slot kept the *loudest* auto-gained level, and the gain lifts some frame to the top in every
  ~1 s stretch. Slots now average `out_level` (no gain, fixed 60 dB scale), a cell averages its slots, and the bar
  scales from the 5th percentile to the loudest. Cache moved to `waves2/` (the old files hold the saturated data).
  Test: steady swings with a quieter bridge and a silent tail. Visibility (taller bar, Widget) put to the owner.
- **D46: waveform removed** (2026-10-18, fifth session): shown the fix, the owner said they don't like a waveform on
  the player bar ("it looks cleaner and better normal, the widgets can serve that purpose"). `waveform.rs`, its cache,
  its test and pty scenario are gone; the seek bar is the plain ━─ line again. SPEC story 72 struck.
- **#51 Create and delete your own playlists ✅** (2026-10-18): `LibraryItem.own` (the library's `ownerV2` username =
  the session's); `CreatePlaylist { name }` → `Event::PlaylistCreated` + a fresh Library; `DeletePlaylist { uri }` →
  a fresh Library (with it back if Spotify refused). Daemon `playlists.rs`: JSON bodies built purely, sent once with
  reqwest to `spclient.wg.spotify.com/playlist/v2` (create = `POST /playlist` with a bare `{ops}`: an `info` field
  there is a 400; then rootlist ADD addFirst; delete = rootlist REM itemsAsKey). Client `playlists.rs`: the sidebar
  gets a right-click/Menu-key menu (Play, New playlist…, Delete playlist… on your own, Copy link); palette "New
  playlist…" and "Delete this playlist"; the naming popup now names layouts or playlists (`Naming`, `NameFor`); a
  delete asks again within 10 s, leaves the sidebar at once and closes its page; the playlist you created opens. An
  empty list says "no songs yet" (was "loading…" forever). Tests: body shapes, `own`, the protocol round trip (fake
  Catalogue keeps created/deleted playlists, refuses others'), App create/open and delete-twice. pty `playlists`.
  Live check (`examples/playlist_probe.rs --create-delete`): created, listed and marked ours, deleted, gone. The
  first two tries were 400s with the `info` field; nothing was created by them.
- **#52 Rename and re-describe ✅** (2026-10-18): "Edit details…" on your own playlists (sidebar right-click, Ctrl+K
  "edit this playlist"): Name and a wrapped Description (300 chars; pasted line breaks become spaces), Tab between
  them, Enter saves, Ctrl+U clears a field. The new name shows at once in the sidebar, page and Pane titles and
  back/forward; the description at once in the list header (now shown after the subtitle: "Playlist · owner — …").
  A refusal puts the old ones back with a toast (`Event::PlaylistEdited { ok }`). The form waits for the playlist's
  first page before it can save, so a description not yet fetched can't be cleared by mistake. An emptied
  description is sent as `noValue: [LIST_DESCRIPTION]`. Live check (`playlist_probe --details`): accepted in ~0.3 s,
  read back, emptied, scratch deleted. Spotify stores descriptions HTML-escaped (`&lt;b&gt;`, `&#x2F;`): the
  decoder now handles those and decodes `&amp;` last.
- **#53 Add to playlist… ✅** (2026-10-18): in every track menu (list rows, queue, search, an artist's tracks, stats,
  track Cards) and Ctrl+K "add to playlist" for the playing song. The picker lists your own playlists from the
  library the Client already has (scope: own playlists only), filtered as you type, so opening it costs no request;
  "+ New playlist “filter”" on top makes one with the song in it (without a filter, the naming popup first); a
  playlist made this way doesn't open. Before adding, the Daemon asks `editablePlaylists` (by name, one small page)
  whether it has the song (`curates`); if so the Client says so and the same pick within 10 s adds it anyway. An
  open view of the playlist fetches its new end; the Daemon drops its cached pages after an add or a details edit
  (#52 left a stale description in that cache for up to its TTL). Live check (`playlist_probe --add`):
  `curates` false → added in ~0.14 s (`data.addItemsToPlaylist`, no `errors`) → read back with a row `uid` →
  `curates` true; scratch deleted. Mutations go through `pathfinder_query`, which retries only on a 429 (nothing
  written).
- **#54 Remove tracks ✅** (2026-10-18): "Remove from this playlist" in a row's menu and the Delete key, only on
  your own playlists (Liked Songs keeps ♥). `TrackRow.uid` (from `fetchPlaylist`'s `uid`) names the row, so one
  copy of a song that's in twice goes. Gone at once, put back in place with a toast if Spotify refuses
  (`Event::RemovedFromPlaylist { ok }`); another Client's removal leaves this one's open list too. Live check
  (`playlist_probe --remove`): the same song added twice → two rows, distinct uids → one removed in ~0.16 s
  (`data.removeItemsFromPlaylist`) → the other copy stayed; scratch deleted. Also: the MPRIS integration test's seek
  check was flaky (Seek is relative and it assumed the pause came within 1 s of play); it now measures from the
  paused position.
- **#55 Drag to reorder ✅** (2026-10-18): in your own playlist, press a row and drag: once the pointer reaches
  another row the list previews the new order (the row keeps its old number until it lands); the drop sends one
  `moveItemsInPlaylist`. Alt+↑↓ moves the selected row one place (the queue never had a keyboard move; this is the
  first). The move is anchored to the row it lands before (`BEFORE_UID`), or after the one it follows at the end
  (`AFTER_UID`): never "bottom", so an unloaded tail can't misplace it. Several quick moves of a row settle on the
  last answer; a refusal puts it back where it started. A sorted or filtered view says "Clear the sort and the
  filter…" instead; others' playlists don't arm a drag at all (a shaky click in Liked Songs stays quiet). Live check
  (`playlist_probe --move`): three rows → after the last, then before the first, both read back as expected
  (~0.16 s, `data.moveItemsInPlaylist`); scratch deleted.
- **Owner test of #52–#55** (2026-10-18): "looks good", but the wheel did nothing in the Add to playlist picker
  (only clicks): the wheel code didn't count it as a popup, so it scrolled the list behind. Fixed: while the picker is
  open the wheel moves its highlight wherever the pointer is (Shift: ten); the details form and naming popup swallow
  it. Unit test + pty `playlist-add` wheel step.
- **#56 Custom covers ✅** (2026-10-18): "Change cover…" and "Remove cover" on your own playlists (sidebar menu;
  Ctrl+K "change cover"). Type a path or drop a file on the terminal (it arrives as a paste: quotes, `file://` with
  %-escapes, `\ ` and `~/` are undone) and it previews at once: cropped square from the centre, at most 640 px, JPEG
  quality stepped 90→40 (then 480/320 px) until ≤ 256 KB (`client/src/cover.rs`, the Client does the image work and
  sends the JPEG as base64; the Daemon decodes it with a small decoder of its own, no new dependency). The preview is
  half-blocks inside the popup (pixel images are kept out of overlays). Enter uploads; the cover shows at once
  wherever covers are drawn and goes back if Spotify refuses. Daemon: upload (`image-upload…/v4/playlist`, only
  authorization + `Content-Type: image/jpeg`, as the web player's `withoutGlobalHeaders`) → `register-image` →
  changes `{values:{picture}}`; removal `noValue:[LIST_PICTURE]`. Live check (`playlist_probe --cover`): token in
  ~2.5 s, a 28-char picture id, the new cover URL at once on both the page and the library (`image-cdn-fa`), removal
  back to none; scratch deleted. Batch 3 done.
- **Library miniatures** (owner, 2026-10-19: "I can make the cover, but I can't see it inside the tui"): every sidebar
  row with a cover (playlists, albums, artists, Liked Songs has none) shows it two cells wide (about square) where the
  glyph was; the glyph stays until the cover has arrived, and for folders. Pixel tier in kitty, half-blocks elsewhere
  (a 2×2 swatch). Names moved one cell right so rows line up. Covers are fetched only for rows in view.
- **Queue wheel** (owner, 2026-10-19: smooth scrolling "does not work in queue"): the queue was the one list left
  where the wheel jumped the view three lines and the next frame pulled it back to the highlight. Now it moves the
  highlight one song per notch (Shift: a page) anywhere over the queue Pane, focuses it, and the list follows, as in
  track lists and the library (#26). Unit test + pty `queue-wheel`.
- **Batch 4 tickets** (2026-10-19): #57 Settings page from config.toml, #58 a read-only keys tab (story 80, D33).
- **#57 Settings page ✅** (2026-10-19): Ctrl+K "settings" opens it in the main Pane (back/forward as other pages;
  no key of its own and no sidebar row yet: open to veto). Built from the file: `singularity_config::entries` reads
  each setting of `SETTINGS` (key + `Kind`: Toggle, Choice, Named, Number with range/step) from `config.toml`, with
  the comment block right above it as its description (the default file's when the user's has none; a setting left
  out of the file shows at its default). Toggles flip on Enter/Space/click; lists (design, theme, layout, gpu) step
  with ←/→ or ◂ ▸, wrapping, theme/layout offering "the design's own" (empty) plus built-in, `themes/` and
  `layouts/`; numbers step (fps 1–30, idle minutes 0.5–120 by 0.5) or are typed after Enter (out of range: a toast
  says the range). Each change is one `singularity_config::set` and applies at once (`config_written` keeps the
  watcher from applying it twice); an edit in the file shows within a second. Problems in the file are listed at
  the top of the page. Descriptions wrap to two lines under the value; the list never starts with a description's
  tail (a first try looped between two such lines and froze the pty run; `scrolled` is now a pure function, tested
  over every selection and height). Tests: config `entries` + App edits/apply/file-follow; pty `settings`.
- **#58 Keys tab ✅** (2026-10-19): tabs "config.toml" · "keys.toml" on the Settings page (a click; or ↑ from the
  first row onto the tab row, then ←/→; on the keys tab ←/→ switch at once, it has nothing else to change). Every
  action of `Action::ALL` with its keys as they are now (preset + `keys.toml`; "—" when unbound), its label, and its
  name in `keys.toml` at the right when there's room; the keys column fits the longest keys (≤ a third of the page).
  The preset and the file's path are on the tab row, `keys.toml`'s problems above the list; both follow the file
  live. Read-only (D33). The help screen now points to Ctrl+K settings. App test; pty `settings` (keys tab, a
  binding and a mistake added to keys.toml). Batch 4 done.
- **Batch 5 dropped (D47)** (owner, 2026-10-19): asked what the album theme was, then: "I like the themeing as is
  currently". Story 81 struck; story 82 was done in #44. Next: batch 6, podcasts and audiobooks.
- **Batch 6 tickets** (2026-10-19): #59 show pages + episodes, #60 saved shows + Follow, #61 episodes in search, #62
  audiobooks (if Soloist plays chapters). Played marks read-only (the web player can't set them either). Soloist's
  first real update ran today (1.3.9.35).
- **#59 Show pages ✅** (2026-10-19): a show opens as a list in the main Pane (search/Home cards, links): header
  "Podcast · publisher — description", columns # · Title · Progress · Released · Time (no ♥). Progress from
  `playedState`: "✓ played", "N min left", blank when new (read-only: the web player can't set marks either).
  Durations over an hour read h:mm:ss everywhere. The Daemon sends a show's newest 100 episodes (`SHOW_BATCH`, pages
  of 50 from `queryPodcastEpisodes`, header from `queryShowMetadataV2`); `Command::MoreTracks { uri, offset }`
  fetches 100 more when the list is within 20 rows of its end (a long show has thousands: no eager crawl). An
  episode card sends `FindShow` (`getEpisodeOrChapter`) → `Event::EpisodeShow`; its show opens with that episode
  highlighted, loading older pages until it's there. Episodes in the queue get names via `getEpisodeOrChapter`
  (the show as artist and album); a Soloist episode without a creator shows its show. Episodes play in the show's
  context like tracks. Not yet run live: playing an episode (would interrupt the owner's player and write a resume
  point), so whether Soloist resumes at the saved point is the owner's test. Tests: parsers (anonymised shapes of
  2026-10-19), App show page/paging/episode card; pty `show`.
- **#60 Saved shows ✅** (2026-10-19): `libraryV3`'s `Podcast` items become sidebar rows (kind `show`, "Podcast ·
  publisher", cover miniature; ◈ until it arrives) and open the show page. The show page's chip reads ♡ Follow /
  ♥ Following (the same optimistic `SetSaved` as albums; the Daemon refreshes the library after it). Live check
  (2026-10-19): `addToLibrary` of a public show → it appears under the filter id "Podcasts & Shows" (named
  "Podcasts"; the filter is looked up by id) as `PodcastResponseWrapper` → `Podcast {name, publisher.name,
  coverArt.sources, uri}` → `removeFromLibrary` → gone (the library's filters back to three). Four ~2 s follows in
  all, each reverted. Tests: parser, App (chip), pty `show`.
- **#61 Episodes in search ✅** (2026-10-19): `searchDesktop` already answers with `episodes` (Episode with
  `playedState`, `releaseDate`, `podcastV2`), so no `searchEpisodes` request is needed. `SearchResults.episodes` are
  rows with their show as the (linked) artist. Under the Podcasts chip they list above the show cards: name, show,
  progress, minutes; double-click/Enter plays; the row menu (and an episode card's) has "Go to show", which opens
  the show at that episode. Tests: parser, App; pty `show` (chip step).
- **Batch 8 pulled forward (D48–D51)** (owner, 2026-10-19, on the M2 Air for a while): "the next step will be getting
  the TUI to run on macos", then "goal is to have both the existing linux version and a separate, same functionality
  macos version, nothing exclusive to either if possible". First look on macOS 15.7 (Spotify.app 1.2.94 installed and
  logged in): the workspace built after one fix (`prctl(PR_SET_PDEATHSIG)` is Linux-only), the Client ran in a pty
  against a fake Daemon (Home, library, play, GPU visuals on Metal: "Apple M2"), and 2 of 254 tests failed (`/proc`
  in the stale-Daemon check; a Settings test whose temp path is too long). Parity table: `docs/PLATFORMS.md`. Tickets
  under spec issue #34 (story 86), in build order: #63 build and tests green, #64 Keychain, #65 AppEngine, #66 setup
  and health per platform, #67 AudioTap on Core Audio, #68 notifications and Now Playing, #69 LaunchAgent and install.
- **#63 macOS builds and tests green ✅** (2026-10-19): `prctl(PR_SET_PDEATHSIG)` is Linux-only (`cfg`); the stale-Daemon
  check reads the process name with `proc_name` on macOS (`proc_pidpath` fails once a rebuild has deleted the binary,
  which is exactly when it's needed; tokio's `peer_cred` already has the pid there). The #25 test's stand-in is now the
  test binary itself, not a copy of `/bin/sleep` (macOS SIGKILLs copies of its own signed binaries, even re-signed).
  A Settings test keeps its files in `/tmp` (macOS's temp path pushed the tab line's text off). The pty harness runs on
  macOS: fonts found per OS, sockets in `/tmp/sgl-<uid>/` without `$XDG_RUNTIME_DIR`, `SO_PEERCRED` → `LOCAL_PEERPID`
  in `stale_test.py`. Five scenarios had drifted on both platforms and are fixed: `home` clicked a shelf at the Pane's
  bottom edge, `like` picked a menu item by position (the menu grew in #53), `layouts` looked for "loudness" (cut to
  "loudne…" since #43), `search`/`config` failed on the Client having quit already, and `mini` had gone with the
  waveform scenario (D46). On macOS every scenario but `mpris` (D-Bus) passes, plus `stale_test` and `update_test`.
  `tools/check-linux.sh` type-checks the Linux build from a Mac (zig as the C toolchain): clean.
- **#64 Keychain ✅** (2026-10-19): `secrets.rs` keeps one interface with a store per OS: `secret-tool` on Linux
  (unchanged), `/usr/bin/security` on macOS. The macOS store goes through Apple's tool on purpose: items it creates
  trust that signed binary, so a rebuilt (ad-hoc signed) `singularityd` reads them without a Keychain prompt, which a
  Keychain crate linked into `singularityd` would not. Values reach it over stdin (`security -i`, hex via `-X`), never on
  a command line; `-U` replaces in place. The protocol keeps `"keyring"` for "in the OS store"; the Client and the
  Daemon's messages say "Keychain" on macOS. Tests: the owner-only fallback (own test binary, env set), and an opt-in
  live round trip (`--test secrets_live -- --ignored`: one `account=probe` item written, replaced, removed; run on the
  owner's Mac, gone afterwards). pty: the health view says "owner-only file (no Keychain found)" with
  `SINGULARITY_NO_KEYRING=1`.
- **#65 AppEngine 🔄** (2026-10-19, built; live check pending): `daemon/src/app_engine.rs`. The Engine's Connect
  identity (`engine::Identity`: name, `via_connect`, version) replaces the hard-wired `soloist::DEVICE_NAME` in the
  Catalogue. The AppEngine finds and starts the app hidden (`open -gj -a Spotify`; Linux: `spotify --minimized`) and
  forwards every command as `CatCmd::Engine`; the Catalogue sends them to the app's device (TogglePlay from the
  cluster's state; resuming while the app isn't active moves the account's last playback onto it; `add_to_queue`
  endpoint). Every cluster update also becomes Engine events (`app_engine_events`): Ready (active or not) / Failed
  ("isn't on Spotify Connect") by the device's presence, and while it's active its playback with `next_tracks` as the
  queue (≤ 50, bare, named by the Catalogue; delimiters dropped) and `playback_quality` as the stream (hifi → FLAC
  16-bit, hifi24 → 24-bit, else Vorbis at the level's bitrate); idle when nothing plays anywhere. The Hub handles
  them with the same code as Soloist's (`on_engine_event`), and now passes on Engine status and stream only when
  they change. Another device's playback still has no queue (edits always go to this computer's device). The tap is
  off for the app until #67. `config.toml` schema 3 adds `[engine] kind = "auto" | "soloist" | "spotify-app"`
  (auto: Soloist on Linux, the app on macOS; read at Daemon start, on the Settings page) and `device`. Tests:
  migration 2 → 3, a Daemon with the AppEngine and a stand-in Catalogue (commands arrive as `CatCmd::Engine`, a
  cluster's playback/queue/quality reach the Client), status from the cluster (present, active, idle, missing).
- **#66 Setup and health per Engine ✅** (2026-10-19): `Welcome` carries the Daemon's `EngineKind` (soloist,
  spotify-app, fake; defaulted, so no protocol bump: older Daemons ran Soloist). The wizard's steps are the Engine's:
  Soloist as before (key, download, pairing, login, audio); the Spotify app: found (else why: not installed, or no
  Catalogue to drive it with), log in (first, as the app is driven over Connect), found on Connect (or the Catalogue's
  "isn't on Spotify Connect"), audio check (playing counts until #67 taps the app). Lossless hints name the app's own
  setting. The health check names the app ("Spotify app 1.2.94 · "<name>" on Connect") and drops the Soloist key row.
  The macOS audio-output row moved to #67 (it needs Core Audio). Every pty script pins `SINGULARITY_ENGINE`, and
  `SINGULARITY_START_APP=0` keeps the AppEngine away from the real app, so a Daemon a test Client starts by itself never
  drives the owner's Spotify app. Tests: App (both flows, health rows), pty `app_wizard_test.py` (three Daemons),
  `stale_test`/`update_test`/scenarios rerun.
- **#65 live check, round 1** (owner, 2026-10-19): playing worked; after moving to another wifi the wizard said the
  Mac "isn't on Spotify Connect", Enter did nothing, and restarting the Spotify app fixed it. The Daemon's log: the
  dealer was offline ~30 min and reconnected, but the hidden controller was never registered under the new connection
  id, so cluster pushes stopped; and the app's own Connect link stayed stale until it restarted. Fixes: the
  Catalogue keeps the dealer's connection-id stream and registers again on every new id (`connect::reconnected`); a
  cached device id the latest cluster contradicts is looked up again; an AppEngine command waits up to 20 s for the
  app's device, woken by each pushed cluster (no polling); a command while the app isn't running starts it hidden
  first; `Command::RestartEngine` ("Restart the player" in Ctrl+K; Enter on the wizard's Connect step) quits and
  restarts the app (SIGTERM, then `open -gj`) or restarts Soloist (parity); once the Engine was ready this run, losing
  it shows in the status bar and health check instead of reopening the wizard. Owner's question: is the app the only
  way on macOS (it's ~600 MB and 5–25 % CPU while playing, against Singularity's ~46 MB)? Facts: the only lossless
  way, yes (Soloist is Linux-only); librespot in the Daemon would be light but 320 kbps at most. Put to the owner:
  librespot as an `[engine] kind` choice on both OSes, the app staying the macOS default (D52, open).
- **D52 answered, #70 librespot Engine built** (owner, 2026-10-19): "Lets add the librespot lightweight player." Also:
  "How feasible would it be to build a lossless capable player like soloist for macos?" (D53), "the widgets reacting to
  music live are not working on macos … I need full feature parity … make sure you know EXACTLY where we stand" (the
  macOS audio tap is #67; `docs/PLATFORMS.md` was re-audited against the code, feature by feature), and "add towards the
  very end of the spec a codebase analysis meant for finding bugs, vulnerabilities etc, making sure the app uses as
  little resources as it needs" (batch 9, story 87, #71, SPEC "Batch 9: codebase audit"). Built:
  `daemon/src/librespot.rs`: Spirc + Player (spotatui's librespot fork 0.8.6, `librespot-connect`/`-playback`), a
  separate session with the Catalogue's saved login and a stable device id (FNV of the name), "Singularity on
  <computer>" on Connect, commands through `CatCmd::Engine` like the app (one Connect path for both), its status/
  playback from the cluster; a `Tee` sink sends the samples to cpal and, while a Client listens, to the tap
  (`tap::Feed`: bounded, `try_send`, never blocks playback); restarts after a dropped session with the saved state;
  remembers the volume; starts as soon as the Catalogue logs in. `Identity.player` names the Engine in messages.
  The tap's PipeWire matcher now takes the Engine (`StreamOf`: Soloist, the desktop app, the Daemon), so the app as
  Engine is tapped on Linux too. The health view's audio output on macOS is cpal's default device. Client: the wizard's
  librespot steps (login → player on Connect → audio check, sound must really come through), health rows, "320 kbps:
  librespot's best" instead of a lossless warning. Found by the parity audit and fixed: the login page never opened on
  macOS (`xdg-open`), the `top` Widget was empty on macOS (`/proc`; now libproc, and it lists the Spotify app's
  helpers). Live, read-only: a scratch Daemon reached Connect with the owner's login in ~4 s and idled at ~26 MB.
  `tools/check-linux.sh` answers alsa-sys's pkg-config with a stand-in. Linux builds now need `alsa-lib`.
- **D54: librespot is the macOS default; D53 spike** (owner, 2026-10-19): "make on macos the librespot the default, since
  widgets work out of the box, and it uses the least resources by far and its standalone", and about Soloist in a VM:
  "if its somehow less resources than spotify client thats hilarious and worth it, try it out". Spike
  (`research/2026-10-soloist-vm-spike.md`): vfkit + Debian 13 arm64 + the CDN's arm64 Soloist (1.3.9.36) runs; the
  owner's VPN drops the Virtualization framework's NAT, so user-mode networking (gvproxy) is required; the VM costs
  the RAM it's given (192 MB VM → 199 MB footprint with Soloist up, 72 MB free inside); idle ~0 % CPU; PCM out of the
  VM to the Mac ~0.5 % CPU; Soloist's pairing endpoint reachable from the Mac through a forward. Unmeasured without
  the owner's key: lossless playback cost and pairing from the phone (a Bonjour proxy record).
- **#68 Now Playing and notifications on macOS 🔶** (2026-10-19, built; the owner's media-key check pending): with
  librespot the default (D54) there's no Spotify app to publish Now Playing, so the Daemon does
  (`daemon/src/nowplaying.rs`, objc2-media-player): the Now Playing entry follows `Hub::playback` (title, artists,
  album, duration, position + rate, playback state), and play/pause/toggle/stop/next/previous and Control Centre's
  position drag become transport commands (`crate::transport`, as MPRIS's do). Apple delivers those on the main
  thread's run loop, so on macOS `singularityd`'s main thread runs a CFRunLoop (kept alive by a daily timer) and
  tokio runs on another thread, which ends the process with the Daemon's exit code. Not for the Spotify app as Engine
  (its own entry would fight ours for the keys) or a fake Engine (`SINGULARITY_NOWPLAYING=on|off` overrides). Track
  notifications: which changes are news is now shared with Linux (`nowplaying::TrackNews`, tested); macOS posts them
  with `osascript` (title and body as arguments, never script text), without the cover. Smoke: start, a Play,
  "already running", SIGTERM all exit cleanly; `stale_test` (replace an old Daemon) passes.
- **#69 LaunchAgent and install ✅** (2026-10-19): `packaging/launchd/singularityd.plist` (label
  `io.github.publicusr.singularityd`; `KeepAlive.SuccessfulExit = false` like systemd's `Restart=on-failure`, so a
  crash or exit 75 restarts it; `ProcessType = Interactive`, never throttled while it plays; `TMPDIR` set to the
  per-user temp dir, where the Client looks for the socket, as launchd doesn't always set it) and
  `packaging/launchd/install.sh` (fills in the paths, `launchctl bootstrap gui/<uid>`, `--uninstall`). Tested on the
  Mac under a scratch label with a fake-Engine Daemon: running at load, a `Shutdown` (exit 75) restarted by launchd,
  removed cleanly. `singularityd` itself now builds its own runtime (`daemon()`), so exit codes come from one place.
  The "Incompatible" message names the OS's restart (`RESTART`). README: install steps for both OSes. Installing it
  for real is the owner's choice (one command).
- **Owner bug (2026-10-20): the audio Widgets kept moving while paused ✅**. The tap only sends its last, all-zero
  frame after it has *heard* silence; a paused Engine sends nothing at all (librespot stops writing, Soloist's stream
  goes quiet), so every Client kept the last loud frame and the oscilloscope kept animating it (the rain stayed
  boosted). Fixed at both ends: the tap feeds a frame period of silence whenever no samples arrive, so the bars fall
  as they do between tracks and the clearing frame goes out, then it waits without cost (`Analyzer::silence`,
  `is_quiet`); and a Client drops a frame older than 0.5 s (`Meters::expire`). Tests on both; pty: the Terminal design's
  osc flattens and stops within 3 s of pausing. The rain keeps falling at its calm base speed (ambient), unboosted.
- **#72 Soloist in the Soloist VM 🔶** (2026-10-20, built; live-checked, the owner's own switch pending): `[engine] kind =
  "soloist"` on macOS runs the same SoloistEngine with a VM host (`soloist_vm.rs`, `vm.rs`, `pcm.rs`). `vm.rs`:
  vfkit 0.6.4 and gvproxy 0.8.9 downloaded once and pinned by SHA-256; Debian 13's latest arm64 cloud image checked
  against Debian's SHA512SUMS and copied sparse; a cloud-init seed (`hdiutil`) makes a `singularity` user (SSH key made
  for the VM, loopback only), PulseAudio with a null sink, `sgl-pcm` (its monitor as raw PCM on TCP 5005) and
  `sgl-soloist` (the guest fetches, checks with `--version` and swaps Soloist's arm64 build itself, ETag-based, so the
  quiet-moment update flow works unchanged: `keep_current_in_vm`); Soloist's audio cache capped at 200 MB (its default is unlimited; Linux's: #71); first boot at 512 MB then 256; package caches
  dropped and `fstrim` so the disk file holds ~1.3 GB. `soloist_vm.rs`: Soloist is an SSH session (key and name on its
  stdin, a watcher on fd 3 stops Soloist when it closes: a script's background jobs get /dev/null as stdin, which
  first killed it at once); WebSocket and PCM through gvproxy forwards; pairing announced (Bonjour proxy record via
  `dns-sd`, relay in Apple's python3) only while it waits, stopped once logged in. `pcm.rs`: the PCM to cpal with a
  jitter buffer (0.3 s, catch up past 2 s) and to the tap's Feed. Pairing without the phone was tried and doesn't work
  (Soloist wants an authorization code for its own client id). Device name: "Singularity on <computer>" (apart from
  Linux's "Singularity"). Live: start → waiting for pairing in 5.6 s; the owner paired, FLAC 1.4 Mbit/s on the
  speakers; ~300 MB, ~10 % CPU (VM 6.4 % mostly virtualisation overhead with 2 vCPUs, gvproxy 1.5 % moving the PCM,
  Daemon 1.7 %); later: PCM over virtio-vsock, 1 vCPU.
- **#72 round 2: idle cost, then the owner's volume report** (2026-10-20). Measured idle (nothing playing) at ~10 %
  CPU, as much as playing: the null sink's monitor streamed silence nonstop (parec → socat → gvproxy → the Daemon →
  cpal). Now Soloist plays into a pipe sink on its own clock (`module-pipe-sink`, `use_system_clock_for_timing`) that
  sleeps when nothing plays, `sgl-pcm` sends it over virtio-vsock (the Daemon listens on a Unix socket vfkit bridges;
  no gvproxy), the Daemon pauses its cpal stream after 3 s without data, and the VM runs on 1 vCPU (2 only for the
  first setup). Setup version 2 re-applied on the owner's VM in seconds, pairing kept. Idle: **~1 %** (VM 0.95 %,
  Daemon 0.02 %, gvproxy ~0). Playing: smooth, no clipped starts (owner). Owner: "adjusting volume makes it jump back
  to whatever it was pre-adjustment, at least with mouse": a Connect device reports its new volume ~0.5 s late and
  any playback update meanwhile carried the old one. The Client now holds a volume it just set until the player
  confirms it (or 2.5 s pass, so a refused change still shows), as the seek bar does with the position. And the VM's
  Soloist defaults to "Singularity on <computer> (lossless)": devices are found by name, and this Mac's librespot
  ("Singularity on <computer>") can linger in the device list after a switch (during the test both were online, so
  the owner's real Client may have been steering the wrong one).
- **Soloist remembers its volume** (2026-10-20): it starts at its own 40 % unless told; the SoloistEngine now saves the
  volume on every `volume_changed` (`soloist-state/volume`) and starts Soloist with `--initial-volume` (Linux and the
  VM alike). `SINGULARITY_SOLOIST=local` keeps a stand-in Soloist local on macOS for the pty harness (`soloist` there
  now means the VM). librespot measured while playing: ~1 % CPU, 28 MB.
- **#67 AudioTap on the Spotify app (macOS) 🔶** (2026-10-20, built; live check with the app pending): `process_tap.rs`:
  a `CATapDescription` (stereo mixdown of the app's audio process objects, private, unmuted) → a process tap → a
  private aggregate device holding only the tap → a C IO callback on Core Audio's thread that only resamples
  (48 → 44.1 kHz, linear, `tap::Resampler`, tested to keep a 1 kHz tone in its band) and hands the samples to a
  `Feed`, so the rest is librespot's path. `manage` keeps the tap while a Client wants audio and the app runs
  (all "Spotify"/"Spotify Helper" processes: which one plays isn't fixed), follows an app restart, and if the app plays
  but only silence arrives for 6 s, says once where the permission is. Probe (`examples/tap_probe`): mechanically
  right (48 kHz stereo, real time), but all zeros from a shell: the "record audio from other apps" permission goes to
  the app Singularity runs in, terminals don't ask for it, so it has to be granted by hand (System Settings → Privacy &
  Security → Screen & System Audio Recording → System Audio Recording Only → the terminal). The wizard's audio check
  now needs real sound for the app on both OSes.
- **librespot: a watchdog for a dead session** (2026-10-20): at 01:13 the owner's Daemon logged "Connection to server
  closed" while idle and never came back: librespot's Connect loop only checks the session between events. The
  LibrespotEngine now looks every 5 s and restarts the receiver where it was.

- **#62 audiobooks, live check** (2026-10-20, macOS): a book is a `spotify:show:` (`__typename: Audiobook`), its
  chapters `spotify:episode:` URIs. librespot can't play them: a silent probe (`examples/chapter_probe`) gets a
  chapter's files but the audio key is refused (an ordinary episode's isn't). Soloist (the VM, 1.3.9.39) plays them:
  "Opening Credits" of a book played ~4 s, and Spotify recorded `playedState IN_PROGRESS` at 907 ms. Owner round: yes
  to all three (the VM check; D57 audiobooks everywhere, a message on librespot; D58 Spotify's access line shown).
  The check was meant to be at volume 0 but was audible at 40 %: Soloist's updater restarted it into a new build just
  as the check began, the volume asked for reached the new Soloist before it had logged in and was lost, and a volume
  of exactly 0 was never saved (it looked like no change from the state's default). Fixed: the SoloistEngine saves
  the volume a Client asks for (a restarted Soloist starts at it), saves every `volume_changed`, and sets the saved
  volume again once Soloist logs in. The pty `update_test.py` now mutes before the update and checks the new build
  starts muted.
- **#62 audiobooks, built** (2026-10-20): a show whose `queryPodcastEpisodes` answer says `Audiobook` is remembered as
  a book and paged with `queryBookChapters` (chapters are `Chapter` rows with the episodes' `playedState`, so progress
  and "min left" come for free); `queryShowMetadataV2` gives the header "Audiobook · authors · access line · narrated
  by … · 8 h 25 min" (`ListHeader.book`), the description without its "Author(s): … Narrator(s): …" preamble.
  `getEpisodeOrChapter` names a chapter in the queue with its book. Search asks `includeAudiobooks` and the Client has
  an Audiobooks chip; a saved book in `libraryV3` (`Audiobook`) is a library row. Cards and library rows lead with
  the access line ("Listen free until Oct 31 · J.K. Rowling"): Cards are ~8–12 columns, and whether it plays is the
  useful part (D58). On librespot (D57) the Client refuses to play a book's chapters and says
  `AUDIOBOOKS_NEED`; a chapter reaching librespot another way (the phone, a queue) fails to load, and the
  LibrespotEngine checks the episode's metadata (`is_audiobook_chapter`) to say the same instead of "not available".
  Book page labels: "24 chapters", ♡ Save (not Follow), no Released column. Fake Catalogue: "The Long Orbit"
  (`spotify:show:fakebook`), pty scenario `book`. Not yet seen live: a saved book in the library (the owner has none),
  resuming a half-heard chapter.
- **Batch 7 parked** (2026-10-20): owner: "lets pause on the free tiling, maybe as an idea to come back to, current
  state is very satisfactory." D59: free tiling becomes an idea (`ideas/layout.md`); story 85 stays in the spec, marked
  parked. Next and last: batch 9, the codebase audit (#71).
- **Audit round 1** (2026-10-20, #71): first pass on macOS (report `research/2026-10-audit.md`): private daemon.log,
  socket directory and peer uid checks, a config crash on inline tables, bounded Catalogue caches, cargo audit/deny
  triaged. The owner's idle Client used 9–10 % CPU animating the Visuals with nothing playing: D60 (yes): 5 fps while
  idle (`App::frame_rate`).
- **Audit round 2: back on Linux** (2026-10-20, #71): tests (278) and all 43 pty scenarios green on Linux; clippy and
  cargo audit as on the Mac plus two new TLS advisories (patched). The owner asked whether the audio Widgets still
  move while paused: their Linux build predated `12a9e47`, but the osc still held its full height ~3 s after a pause
  (scaled by the falling bands' own sum); it now goes flat at once. Found while reviewing the `unsafe` blocks: kitty
  shared-memory segments shared one name, so most uploads fell back to zlib and frames could be unlinked unread.
  `tools/soak.sh`: the Linux soak sampler.
- **Audit round 3: Linux tools** (2026-10-21, #71): cargo deny, Miri (proto, config), ASan/LSan (workspace), TSan
  (Daemon), `cargo fuzz` (four targets in `fuzz/`, ~22 M runs), udeps: nothing found but a test's deliberate leak and
  Mesa's own. Soloist's build download reviewed (fine; polish: no size cap). Found: Soloist's cache at 2.8 GB on the
  owner's machine (`--cache-size` unused): a cap goes to the owner. Logged-in test Daemons (librespot, app) wait on the
  owner (a keyring copy was refused by the tool permission check).
- **D61** (2026-10-21): Soloist's cache is capped by `engine.cache_gb`, default 5 GB (owner). The owner postponed the
  logged-in Engine sessions (they'll switch `[engine] kind` themselves later) and said yes to `perf`/`heaptrack`.
- **Frame profile and owner report** (2026-10-21): heaptrack halved the Client's allocations (lyrics wrapped once per
  song and width, tint keys in one buffer). The owner saw the osc lag the music: its persistence was per frame (0.3 s
  of trail at 10 fps); now per second. PipeWire latencies measured while playing: Soloist follows the headset's
  21 ms cycle, no buffering ahead. **D62** (yes ×2): no 5 fps idle rate, unfocused default 30; rain faster without
  music.
- **Ghostty wheel** (2026-10-23, owner report): on Ghostty (1.3.1, Nobara) one wheel notch moved every list three
  rows. Ghostty sends `mouse-scroll-multiplier` reports per notch (`discrete:3` by default) in one go. Terminal input
  is now read on its own thread, timestamped as it arrives; wheel reports in one direction within 8 ms of the first
  are one step (`term::WheelBursts`). Separate notches, one report each (kitty), come tens of ms apart and all count.
  pty scenario `wheel-burst`.
- **Audit round 4: review areas** (2026-10-23, #71): Time, polling loops (measured on the owner's idle Daemon and
  Client) and the Engine state machines. Fixed: Soloist's restart wait grew for the Daemon's whole life and asked-for
  restarts (Restart the player, a new key) waited it out too; the AppEngine ran `pgrep` per command. Open: the VM's
  guest clock may lag after the Mac sleeps (finding 13, a macOS check).

- **T0005: a phone's play must not be undone** (2026-10-24, owner report 2026-10-23; D63, D64). Owner: "queueing a song
  and skipping to it, or playing a song from the phone, made the previously playing song play again. Only play next
  (or maybe queue) got around it." Reproduced with the real Daemon and a stand-in Soloist that can act as a phone
  (`tools/pty/phone_test.py`): a plain remote change gets no command from the Daemon at all, so nothing "restores"
  there. The cause is the updater's restart: on a remote track change with a build staged, `quiet_moment` said yes,
  Soloist was restarted, and the state remembered at that instant was put back over Connect — starting the new track
  again from 0, and, when the restarted player came back already playing a track (Spotify handing it the phone's
  play), replacing that track with the remembered one. The `before` run shows exactly the owner's symptom:
  `Fake Track 002` (remembered) instead of the phone's `Fake Track 003`. Fixed: `quiet_moment` takes a track change
  only when `SoloistState::change_is_ours` says our own command (a Play's track or a Next/Previous/TogglePlay, 30 s
  window) caused it, so a phone's skip is followed, not interrupted; after a restart the resume waits `RESUME_GRACE`
  (1.5 s) and is dropped the moment the player reports a track of its own; and the answer to the Daemon's own
  `get_queue` (a snapshot from before the change) now only updates the queue, never the track/status/position it may
  still show. Tests: three contract tests (ours vs not, quiet moments, queue answer) and pty `phone_test.py`.
  Jam (D64): asked the owner; what a Jam needs is a *host device* (started from a phone), and guests' plays and
  queue additions arrive as normal Connect commands, which the Daemon now follows. Facts: `PlayerState` carries
  `session_id`/`session_command_id` (the shared-session plumbing) and `PlayOrigin.feature_identifier`; whether either
  names a Jam is unverified (no Jam to try). Live proof needs the owner's phone and account.
- **T0008: audit polish #74, #77, #80** (2026-10-24, Linux; branch on T0005). #74: Soloist's build archive streams to
  disk and is counted while it arrives, under a 256 MB cap (a Content-Length over it fails before writing; any failure
  removes `soloist.new/`, the build stays). #77: `pgrep`, `pkill`, `tar`, `secret-tool`, `pactl`, `parec` are resolved
  from fixed system locations (`programs.rs`; Homebrew's dirs last on macOS), never `PATH`; a missing one is named in
  the error and in a new **Tools** health row. #80: clippy clean on Linux (style fixes, type aliases, three large-enum
  allows and `heart`'s eight arguments each with a reason); macOS clippy still to run.

- **T0033: Jam research** (2026-10-25; D64, docs only). `docs/research/2026-10-jam.md`: the `social-connect` requests
  (`current`, `current_or_new`, `info/{token}`, join, leave, delete, kick), the session/permission fields, the dealer
  topic `social-connect/v2/session_update` and its reasons, the `transfer` shape and the link/QR forms, each marked
  verified or assumed. Premium hosts; any listed Connect device can (spoticord does it from librespot); the cluster's
  `PlayerState.session_id` is the Jam id on the host, and `PlayOrigin.feature_identifier = "jam"` marks a guest's Jam
  play, but there is no Jam flag in the cluster. The one live check that scopes the feature: may the Catalogue's
  session start a Jam with *Soloist's* device id as `local_device_id`? If not, Jam start belongs to the librespot
  Engine and Soloist keeps "start from the phone". No product code changed.

- **Release round 1** (2026-10-29, D65; D64). Status read: T0034 (`jam_probe`) was built on its branch but not merged;
  tests green on `main`; the installed binaries predate T0027–T0029. Owner: 1 (b) Linux supported, macOS beta; 2 yes,
  and the release should have Jam if it can work; 3 yes (T0005 done: the 2026-10-28 log shows the resume dropped
  after an updater restart); 4 (b) a scrubbed mirror goes to a new public repo; 5 asked what MIT/Apache-2.0 dual
  means (open); 6–9 yes.
- **Release rounds 2–3** (2026-10-29, D65 settled; D64). All yes: MIT only; the owner is present for the Jam probe's
  read/start/end on Soloist's device (guest steps wait for a second phone); Jam's release scope and the librespot /
  phone-started fallback; Jam blocks the release only while one form can work ("otherwise we need to lock it in");
  `v0.1.0`; prebuilt binaries by CI; screenshots from the harness; fresh public issues; the launch order with
  publishing last; no-reply commit identity; the private repo becomes `singularity-archive`. Frontier empty; the owner
  confirmed the plan.
- **Jam live check** (2026-10-30, D64 settled; owner present, Singularity playing on Soloist). `jam_probe start
  --device "Singularity"`: **hosted on Singularity** (`host_active_device_id` = Soloist's device), playback kept going;
  `end` answered 2xx and the music kept playing. Cluster `player_state.session_id` ≠ the Jam id (not a re-encoding
  either), so a Jam is read from social-connect only. Guest steps (join, permissions, dealer capture) wait for a
  second phone. Next: build Jam (D65 step 3).
- **Jam built** (2026-10-30, D64, D65 step 3). Protocol: `Command::StartJam`/`EndJam`, `Event::Jam { jam }` (additive,
  no version bump), kept in the Hub's snapshot. Daemon: `jam.rs` (loose-JSON parse, snake or camel case; start
  `current_or_new` on the Engine's device id; end by DELETE v2, then v3; a dormant record is no Jam); the Catalogue's
  dealer task listens on `social-connect/v2/session_update` and reads `current` again on each; the Devices list asks
  too. The fake Catalogue runs a fake Jam with one guest. Client: the device picker's last row (`j`, Enter, a click)
  starts one; the row then shows host and guests, the whole invite link (the picker widens for it), a black-on-white
  half-block QR when there's room, and the guests; ending asks again within 10 s (as deleting does); someone else's
  Jam can't be ended here. The player bar shows `◎ Jam · <guests>`. Ctrl+K "Jam" opens the picker on that row. The
  vim preset's `j` stays ↓ (translated first), and the hint drops "j: Jam" then. Tests: `jam::tests` (parse, dormant,
  camel case, token fallback, paths), protocol `a_jam_starts_reaches_late_clients_and_ends`, Client picker test and a
  test that the half blocks are exactly the QR's modules; pty `jam`. Not tried yet: a real guest joining (a second
  phone) and a phone scanning the QR off a real terminal.
- **Release steps 4–6, step 7 tried** (2026-10-30, D65). Q4: the fake Catalogue behind a `fake` feature; Q1: cached
  list order and fewer per-frame copies; LICENSE (MIT), README for the release, `docs/media/` screenshots; CI (Linux +
  macOS tests/clippy, MSRV 1.89, `cargo deny`) and a tag-driven draft release with binaries. The history scrub ran on
  a scratch clone only (one no-reply identity, 0 personal hits); a check of `spak_`/`sk-`/IP strings and the owner's
  go-ahead for the repo rename and push are next (`docs/HANDOFF.md`, "Release plan").
- **Free tiling, round 1** (2026-10-31, D66). The owner reopened D59 for v0.1.0 and accepted all nine recommendations
  (drag by title, live reflow, swap/split/screen-edge drop zones, the minimum-size rule, close + split with the core
  four guarded, edits fork a Design, keyboard move mode, the well keeps its shape, ships in v0.1.0). Amendment: the
  user names an edited Design (prompt on the first edit) so the picker isn't a wall of "(edited)" copies.
- **Karaoke** (2026-11-05, D67). Feasibility, re-checked on the web: Spotify's `color-lyrics` documents only
  `LINE_SYNCED` and `UNSYNCED` (every public client and wrapper shows `syllables: []`; no `SYLLABLE_SYNCED` seen), so
  Spotify gives no word times. LRCLIB now serves a **Lyricsfile** (YAML, `lines[].words[]` with `start_ms`/`end_ms`)
  on every record and flags word timing with `hasWordSync` (published by LRCGET 2.0's word editor; still rare);
  `syncedLyrics` never has word stamps, but enhanced LRC (`<mm:ss.xx>` before each word) is parsed anyway. Protocol:
  `LyricLine.words` (additive, skipped when empty). The Daemon reads the Lyricsfile with yaml-rust2's event parser
  (no aliases, depth 12, 512 kB). Timing is the Client's (`karaoke/timing.rs`): exact words cleaned (inside the line,
  never backwards, each ending by the next), else estimated (vowel groups, a silent final e, CJK a syllable per
  character; 88 % of the room, ≤ 550 ms a syllable, an 80 ms lead); empty lines are marks that end the line before.
  Staging: the next line comes on 1 s before it is sung (after the one before is sung out); a gap of 6 s or more is
  an instrumental (notes, the seconds to go as a figure, the next line), and its line is counted in by dots for 4 s.
  The ball rests on a word and hops (≤ 650 ms, a parabola) to land as the next one starts. The Widget scales: one or
  two rows (the line scrolled to the word), plain text with a 1–2 row lane, the marquee's pixel font at two sizes;
  on the pixel tier the ball is a small RGBA image (Visual id 4) at pixel precision. The Karaoke design: library over
  queue | the Karaoke Widget over Home, the whole song and glow; themes Neon (new, shared), Sunset, Lagoon. pty
  `karaoke` (pixel tier) and `karaoke-cells`, track 2 of the fake Catalogue. Decisions for the owner: see D67.
- **Karaoke sync** (2026-11-05, D67; owner: "the dot jumping and text filling up with color is offbeat quite often").
  Three causes looked at apart. *Line timing*: the Client's position is Soloist's `position_ms` at its `timestamp_ms`
  (epoch ms, same machine) run on at `speed`; nothing offsets the lyrics; the plain lyrics Widget and the karaoke use
  the same position and the same line stamps, so a line-level error would show in both. The PC's PipeWire graph runs
  Soloist's native stream at a quantum of 1024 at 48 kHz (~21 ms) to a USB output; Soloist keeps a ~1 s "sink buffer"
  of its own (its log), and whether its position counts that couldn't be checked without the owner's live session (it
  presumably does: else the plain lyrics would run a second early, which the owner hasn't seen). No constant offset
  to correct by default.
  *Rendering*: the ball lands on a word as it starts and the fill sweeps from its start to the next word's (tests),
  at 30 frames a second (60 mid-hop on the pixel tier): ≤ 33 ms, not it. *Estimation*: the cause. 88 % of a line's
  room shared out by syllables (≤ 550 ms each) spread the words over the held last note and the rest after it, so
  words came later and later through a line. Measured on modelled singing (`timing::tests::singing`: invented words,
  4 styles × 40 songs: ballad, pop, uptempo, rap; syllables on the beat grid taking 1–4 steps, a held last syllable,
  comma breaths, rests, instrumentals, stamps ~60 ms early ± 90 ms, the syllable counter wrong one word in ten): mean
  miss of a word start 450 ms, median 214, worst tenth over 1.04 s; ballads 741 ms mean. **Built**: the song's pace is
  fitted over all its lines (room ≈ syllables × pace + the hold and rest after them; least squares twice, without the
  far-off lines and the instrumentals), and each line sings at that pace (scaled to its own room, at most 15 % slower),
  pauses a syllable at a comma, and holds its last word into its tail (the fill sweeps over the held note). After: mean
  298 ms, median 191, worst tenth 694 ms (ballad 741 → 495, pop 357 → 311, uptempo 403 → 274, rap 406 → 219). The rest
  is what a text can't know: which syllables a singer stretches (with the true first and last word starts given, an
  even spread still misses by 133 ms on average). **Not built: the audio**. The Client's frames are 64 smoothed log
  bands at 30 a second from the whole mix plus a kick-only beat flag; telling a voice's syllable onsets from drums and
  hats needs source separation (an ML model: too heavy for a live terminal Client, and the GPU is shared). The weaker
  form, snapping estimated starts to the beat grid, was measured with a perfectly known tempo and phase: mean 373 →
  365 ms but within 200 ms 50 → 45 %: a guess off by more than half a step lands on the wrong beat as often as the
  right one (and it could only work live). **A delay setting**: `lyrics.delay_ms` (Settings, −2000 to 2000 in steps of
  25, default 0; config schema 7) holds the lyrics and karaoke back (or forward) against the music; a click on a line
  seeks to where the music is when it shows. Decisions for the owner: the delay as a setting with 0 as its default
  (rather than a guess at Soloist's latency); the "word timing estimated" credit stays as it was.
- **Widgets at any Pane size** (2026-11-05, D66 follow-up; owner: "all widgets should be better at scaling into any
  possible size for the window, particularly lyrics so they can be followed more easily"). Audited every Widget with
  the new pty `sizes` scenario (each alone in a Pane of 10×2 up to 256×64) and a property sweep
  (`every_widget_stays_in_its_pane_at_every_size`: every Widget at widths 1–300 and heights 1–100 through
  `ui::widget`, the one place a Pane's Widget is drawn now). The sweep found real bugs: the queue panicked in a
  one-row Pane (the minimum Pane is 12×3), Now playing drew its title on the Pane's frame in short Panes, the library
  and levels drew past a very narrow Pane. **Lyrics**: the layout comes from the size alone (`lyrics::fit`): the
  current line starts a third of the way down and is always whole in view, glides there between lines (`ease`, ~0.1 s),
  sits on a faint accent band in a big Pane; the lines after it fade with distance, the sung ones quietest
  (`tone`); ~~a tall Pane puts a blank row between lines; a wide one centres a column as wide as the song's longest
  line (at most 72)~~ (both reverted the same day, see below); the credit row only from 8 rows;
  the empty states wrap and centre. Unsynced lyrics follow the song's progress (the reading place a third down) until
  the wheel moves them; that holds for the song. Layout pieces are plain functions (`wrap`, `wrap_indented`, `fit`,
  `first_row`, `ease`, `tone`) for the Karaoke Widget to share. Big rendered text for the current line was looked at
  and not built (see the open question below). **Now playing**: words only when too small for a cover; cover at the
  left in a wide Pane (and the spectrum across the rest when it is also short); stacked when tall, the title in the
  pixel font in a big Pane. **levels**: bars thicken with height (up to 6 rows) and centre; a wide, short Pane puts
  L/R beside bass/mid/high. **marquee**: the pixel font scales (up to 8×) and puts words on up to two lines (four
  in a narrow Pane) before it scrolls; `ambient::pixel_fit`/`pixel_line` are shared with Now playing. **glow**: the
  song in words when too small; the label beside the cover in a wide, short Pane, on two lines when narrow.
  **stream** (compact): facts flow into columns in a wide, short Pane, each value under its name in a narrow one.
  **today**: the figure and chart in a narrow, tall Pane; the week's columns beside the lines in a wide, short one.
  **top this week**: a narrow, tall Pane lists as many as fit. **top**: the CPU summary and graph from 14 columns.
  **listening**: its totals take a row each in a narrow Pane. **library**: a big Pane (32×24+) gives each entry two
  rows (name over subtitle) and a bigger cover. **queue**: its empty-queue hint wraps. **Home**: a wide Pane whose
  longest Shelf has fewer Cards than fit makes the Cards bigger (up to 1.5×) so the Shelves reach across. Fine as they
  were: fft, loudness, scope, rain, starfield, the mini-player. Open (for the owner): big text for the current lyric
  line (needs a bundled font and a rasterizer for the pixel tier, or kitty's text-sizing protocol, which Ghostty
  doesn't have); the queue keeps one row per song at every size (two-row entries would need its drag maths changed).
  **Reverted (2026-11-05, owner's live test on the Terminal design):** "lyrics splitting into two lines … it was good
  before; there's dead space on the left and right". In a tall Pane (18+ body rows) the blank row between lines read as
  every line broken in two (the #30 look the owner had fixed), and a Pane with 60+ columns of text room put the text
  in a centred column only as wide as the song's longest line, leaving bands of empty Pane either side. Both are gone:
  `lyrics::fit` is flush left again (`[mm:ss] text` from the Pane's edge, a cell of margin, wrapped only at the Pane's
  width, continuations indented 2, as in #30) with one row after another. Kept from the scaling round: the current
  line a third down and always whole, the glide, the accent band, fading by distance, the credit only from 8 rows, the
  wrapped empty states, unsynced lyrics following the song. Test `lyric_lines_follow_each_other_from_the_left_edge_at_every_size`
  (72×27, the owner's Pane, up to 256×64); pty `sizes` takes `PTY_SIZES=72x27` and the new `design-sizes` scenario
  shoots every built-in Design at 80×24 … 264×68.
  **Size audit (2026-11-05, after the revert):** every Widget at the seven `sizes` plus 30×5 and 72×27, every Design at
  five terminal sizes, against the pre-scaling build. Fixed: synced lyrics no longer show the tail of a line that
  starts above the view alone on the top row (it read as a line of its own; older than the scaling round); a Pane
  too narrow for its title showed the title's last letter between the ◂ ▸ and the corner ("lis ◂ ▸ g┐"): the title
  now ends before the arrows; the stream Pane's "play something to see the stream" wraps instead of being cut off;
  today's "last 7 days" line in a narrow Pane says "7 days 1h53" instead of cutting the total to its first digit.
- **Big text** (2026-11-05, D66/D67 follow-up; owner: "lyrics … in the big window … just occupying the top left corner,
  like 20% maybe of the screen it should be" and "karaoke also reverted to shitty non-scaling lyrics … now I skipped a
  song and it's fine"). Cause of the second: the Karaoke Widget's 3×5 pixel font gave up on a line too long for it (two
  rows at most) and fell back to terminal text. Checked: Ghostty 1.3 parses kitty's text-sizing protocol (OSC 66) but
  doesn't draw it, so the sizes have to be pixels. **Built**: `bigtext.rs` rasterizes with `fontdue` from a bundled
  subset of Noto Sans Regular and Bold (Latin, Greek, Cyrillic, punctuation; OFL 1.1, `crates/client/assets/fonts`,
  plus ♩♪♫ from Noto Sans Math): an image per row, RGBA, keyed by its content (`gfx::Kitty::place_rgba`: always over
  cell backgrounds and under text, so the accent band shows through; its own budget of 48 off-screen images), glyphs
  cached per size. The release binary grows by 337 kB. **Lyrics**: a Pane of 96×24 or more inside its frame, on the
  pixel tier, with no popup over it, draws the same view (timestamps, current line in bold accent on its band a third
  down, the next bright, the rest fading, the glide, wrapped rows indented) as big text: sized so the song's longer
  lines (nine in ten, but at most 1.5× the middle line) span the Pane and at least 8 lines fit its height; each row
  `pitch` cells tall; the glide moves by cells. Below 1.6× a cell's height it stays the cells. Changed beyond scale:
  timestamps are 0.55× the lines' size; a row cut off at the bottom shows only while the view glides. **Karaoke**: on
  the pixel tier the line on stage is big text at one size per song (its longer lines span the Pane, 1.1× the middle
  one at most; a line too long for three rows there shrinks for itself), the fill splits each row into its sung image
  and its waiting image at a column, the ball lands on each word's middle as the font measures it (radius 0.16 em);
  the lines before and after it are big text at 0.42× when that is 2 rows or more. The pixel font stays for the cell
  tier and for songs the font can't draw (CJK); the instrumental screen and unsynced songs are unchanged. Cost
  (release, 240×65): a steady lyrics frame 266 µs against 309 µs in cells (the images are only placed again), a line
  change about 2.3 ms (its rows rendered and sent once); a karaoke frame mid-line 354 µs against 277 µs in cells.
  Tests: `big_text_suits_every_big_pane` (every size to 300×100), `a_big_pane_on_the_pixel_tier_fills_with_big_text`
  (placements, nothing re-sent next frame, a popup gives the cells back and removes the images),
  `big_text_draws_at_every_size_and_moment_inside_its_pane`, karaoke `big_text_words_sit_where_the_font_puts_them`,
  `a_big_pane_sings_in_big_text_split_where_the_fill_is`, `every_size_and_moment_draws_inside_the_pane_on_the_pixel_tier`;
  pty `sizes` takes `PTY_PIXEL=1`, `PTY_SEEKS` (several moments) and `PTY_TRACK=2`. Unsolved for the owner: the
  `ttf-parser` under fontdue is flagged unmaintained (RUSTSEC-2026-0192; ignored in `deny.toml`: it reads only the
  bundled fonts).
- **Big text in the terminal's font; the listening Pane's gap** (2026-11-08; owner: big lyrics had "a cooked ass basic
  font … it looked so much better before the scaling changes", and asked "can we somehow make the plain terminal text
  scale? the font I have worked really well for the smaller lyrics widgets"). Big text now draws with the terminal's
  own font (`termfont.rs`): `SINGULARITY_BIG_FONT`, else Ghostty's first `font-family` / kitty's `font_family` (kitty
  first when it is the terminal), else `fc-match monospace`, else JetBrains Mono (Ghostty's built-in), Menlo, DejaVu
  Sans Mono, Noto Sans Mono, Liberation Mono; the faces come from the system font folders (`fontdb`), the static file
  before a variable one, bold the family's bold (≥ 600) or its regular. The bundled Noto Sans stays for characters the
  terminal's font lacks and when none is found. Line metrics come from the font (`bigtext::ascent()`/`line()`). The
  scan runs on a thread at startup (pixel tier only). Lyrics and Karaoke both use it. Tests keep the bundled font so
  they don't depend on the PC. **Listening**: the big Pane pinned its numbers to the bottom, leaving the rows between
  the grid (or the breakdown) and them empty ("a ton of dead space"); the parts now stack a row apart and the block is
  centred in the Pane's height (`the_big_listening_pane_keeps_its_numbers_close_under_the_grid`, ten sizes).
- **Lyrics at big sizes: the same font a little bigger, centred** (2026-11-24, D87; #53, the owner's contact sheet
  marks: big text "jumps from the normal size to a far bigger font"; "in the context of the app this font and style are
  out of place"; the full screen "centre it and use a slightly larger font, but not like the second option"). A big
  lyrics Pane (96×24 and more inside its frame) now draws the cells' own view (timestamps as big as the lines, the
  current line bold on its band) `lyrics::step` times bigger: 1 at 96×24, growing with the Pane's width or height
  (whichever is less) to 1.5×, never more than a 96th a column, so no size jumps to another font; from 1.1× on the
  pixel tier as big text, below that the cells. Rows are `step` cells tall, so a row that starts inside a cell is drawn
  that far down its image (the view still glides by whole cells, so each row keeps its image), and the band is painted
  in the current line's images. Both tiers centre the block of timestamps and the song's widest row, eased in over the
  first 20 columns and 8 rows past 96×24 (`lyrics::centring`). Also found: in Ghostty with no `font-family` set, big
  text took fontconfig's monospace (Noto Sans Mono here) while Ghostty draws JetBrains Mono; it now takes Ghostty's
  built-in JetBrains Mono (installed, or a Nerd Font build) first. The pty draws big text in its own cell font
  (`SINGULARITY_BIG_FONT`). Tests: `lyrics_grow_a_step_with_the_pane_and_never_jump`,
  `a_big_pane_on_the_pixel_tier_draws_the_cells_text_a_step_larger_and_centred`,
  `lyric_lines_follow_each_other_from_the_left_edge_and_a_big_pane_centres_them`.
- **Home without empty slots; search on Home; Linux on ARM** (2026-11-20, D68–D73; owner's screenshot "empty album
  slots on the home view"). Asked the owner's running Daemon read-only for its Home feed: 10 shortcuts (7 in the
  screenshot) in a 3-column grid, and "New release from …" as a Shelf of one Card. Built: the short last line stretches
  (D68), one-Card Shelves are Spotlights (D69), Home's search bar and Search's matching bar (D70, D72), no Search row in
  the library (D71), Soloist's arm64 build and the CI/release jobs (D73). Tests: `no_empty_slots_…`,
  `up_from_the_top_row_reaches_the_search_bar_…`, `home_has_search_built_in_and_the_library_does_not`,
  `each_cpu_gets_its_own_soloist_build`; the Home fixture gained a one-Card Shelf. pty `home-search`. Nothing committed:
  the repo is public and commits wait for the owner's OK.
- **Bug hunt, first fixes** (2026-11-20; a read-only hunt by another model, findings re-checked here before fixing).
  Fixed: (A1) drawing below the buffer panicked in ratatui 0.30 (a context menu opened low, then the terminal shrank;
  also the Jam picker's guest rows on a short screen): `ui::text` leaves out what's outside the buffer, the menu is
  kept on screen (`text_outside_the_buffer_is_left_out_not_a_panic`); (A3) a Karaoke word's `clamp` could panic on an
  absurd LRCLIB stamp; (B1) a playlist with a podcast episode, a local file or an unavailable song stopped after its
  first page (the Daemon dropped the item, the page came up short and the Client took the next page for a gap): every
  item keeps its row now (an episode playable, the rest greyed), so pages stay dense and a row's index is its place in
  the playlist. Not changed: a seek back of more than 5 s starting a new play is deliberate (store.rs says so); the
  rest of the findings wait for the owner.
- **Bug hunt, second batch** (2026-11-20; owner: "yeah, lets go for it", one at a time, each re-checked first). Replay
  rule (owner, B8: "20s maybe? or 30? your call"): a jump back counts as a new play only when it lands within the first
  20 s of the track; a seek back mid-track is the same play (`RESTART_NEAR_START_MS`). (A2) the Catalogue sent its
  follow-ups (the sidebar after a playlist change, Home/Library/Devices after a login) back into its own channel of 16
  and locked up for good once it was full, taking every Client session and the Engine task with it: they go on its
  local queue now (`a_full_queue_of_playlist_edits_does_not_lock_the_catalogue`, which hung on the old code). (B2) a
  session dropped any playback state older than the newest it had forwarded, for its whole life: when a phone stopped,
  the computer's own state (its anchor older than the phone's arrival stamps) never reached the Clients; only what was
  queued while the snapshot was read is checked now (`when_another_device_stops_the_client_goes_back_to_this_one`,
  failed on the old code). (B4) Soloist's `track_changed` kept the old track's position until the next sync, and a
  pause snapped back to the last anchor. (B3) a new librespot Spirc consumes the saved state even when it can't
  connect, so a slow network after sleep lost the queue: it's handed over only once Spotify's access-point resolver
  answers (up to ~2 min), and "Restart the player" waits at most 5 s for the old task (B13). (B10) a cover that failed
  is asked for again after 30 s, and a cached file that doesn't decode is fetched again. (B6) the Client fetches a list
  or artist page again when it's opened more than 10 min after it arrived (the Daemon's own cache time), and keeps the
  newest 64. Left: B5, B7, B9, B11, B12 and the C/D findings.
- **First public CI with macOS and ARM** (2026-11-20): Linux on ARM passed (D73). macOS showed the Daemon couldn't make
  its socket on a Mac at all since A10/S5 (2026-10-25): macOS's temp directory is behind root's `/var` link and every
  link was refused; root's own links are accepted now (only root can make one; anyone else's is still refused).
  cargo deny: rusqlite 0.41 (libsqlite3-sys 0.39, SQLite 3.54) for RUSTSEC-2026-0459; chrono (unmaintained, only under
  librespot's oauth2) ignored with its reason.

- **Architecture review, candidate 1: Search owns its query** (2026-11-22, D74; issue #8). The review (mattpocock-skills
  `improve-codebase-architecture`, two read-only agents) listed ten deepening candidates; the owner picked this one first.
  Cause of #8, confirmed: Esc cleared the query but not the record of what was sent, so picking that same Recent search sent
  nothing and the page said "searching…" forever (also after Backspace to empty, and after leaving and coming back).
  `search::Search` now owns the query, what was sent, the recent searches and results kept by query; App holds one for
  its whole life (`is_open`, `searching()`/`searching_mut()`). Tests: `picking_the_recent_search_just_cleared_runs_it_again`
  (App keys; failed on the old code) and ten at Search's interface (`a_cleared_search_picked_again_goes_out_at_once`, kept
  results shown at once and expiring, Enter, retry, older answers kept). pty `search-recent`. Not closed: #8's slowness is
  the Daemon's (candidate 4).

- **Architecture review, candidate 2: Song lyrics** (2026-11-22, D75). Built overnight by an agent, reviewed (two-axis)
  and fixed. Each draw used to guess from a fingerprint (song, line count, length) whether the lyrics had changed, so
  B11 happened: a Lyrics Pane asked without word timing, a Karaoke Pane asked with it, the word-synced answer had the
  same fingerprint and Karaoke sang the estimate all song long. Also fixed: a request lost while disconnected was never
  sent again. Tests that set `app.lyrics` now go through the event; unsynced scroll tests go through the wheel. pty
  lyrics and Karaoke shots at every size, both tiers, match main up to timing noise.

- **Architecture review, candidate 9: Soloist session** (2026-11-22, D76). Built overnight by an agent, reviewed and
  fixed. Login, the volume re-sent on login, a pending resume, `track_changed`, `change_is_ours`, `get_queue` and the
  restart at a quiet moment now interact inside `Session`, whose time windows take the time passed in; scenario tests in
  `tests/soloist_contract.rs` (an update staged during play restarted at our skip and resumed after login; a skip with
  `position_sync` and the late queue answer; a dry stream). C4's three bugs each got a red test first. Not tested: a
  session stop through Failed to the 1 h wait (real I/O). macOS paths are checked by CI only.

- **Architecture review, candidate 10: Engine assembly** (2026-11-22, D77). Built overnight by an agent, reviewed and
  fixed. `main.rs` had ~80 lines of interleaved Options choosing the Engine kind, Identity, device name, tap and
  Catalogue, and threw the config's problems away; one typo in `[ui]` silently switched the Engine back to auto. Both
  OSes' choices are unit-tested on either; `parse` finds the bad key by its span and parses again without it. Known
  gap: if the Daemon reads a different config dir than the Client, its problems aren't shown. macOS paths are checked
  by CI only.

- **Architecture review, candidate 6: a query port under the Catalogue** (2026-11-22, D78). Every test used to swap the
  whole Catalogue for `spawn_fake`, so its scheduling, caching, retries and parsing never ran in one. The Catalogue's
  ~25 Pathfinder calls already went through `State::query`; that function's rules moved into `pathfinder::Pathfinder`
  and the Catalogue now takes any `Fetch` (`spawn_with`). The registry got a stand-in crawl (`with_crawl`). Tests:
  four at the adapter (a hang is asked once more; a retired hash refreshes and is asked once with the new one; C3,
  red first: three calls for a vanished operation made three crawls, now one; logged out, nothing is looked up) and
  five at the Catalogue over recorded fixtures (the library walked into its folders, a search kept and asked once
  ignoring case and spaces, a failed search answered with its error and asked again, an undescribable track failing
  alone and not asked again, a track list kept; after review: a long list sent page by page, and the newest search
  first with what waited behind it in order). Groundwork for candidate 4, which moves Connect out under these tests.

- **Architecture review, candidate 4: Connect out of the Catalogue's loop** (2026-11-22, D79; #8, C5, C6). Searches
  waited behind Connect work in the Catalogue's one command loop: an Engine's command waits up to 20 s for its device
  to show up, the resume after an update retries for 6.5 s. Connect's twelve methods moved to `catalogue/connect.rs`
  as `Connect`, run beside browsing with `join!` in the task `supervise` already restarts (a panic in either is caught
  as before). Tests: `a_search_is_answered_while_connect_is_busy` (red on D78's code: no answer within 1 s, behind a
  resume), `connect_work_goes_to_connect_and_browsing_stays`, and at the hub `logging_out_lets_go_of_the_other_device`
  (C5, red first). Closes #8.
- **Architecture review, candidate 5: Now playing** (2026-11-22, D80). Five hub fields and eight functions taking their
  own locks became `NowPlaying` behind one lock: the Engine's state, another device's, the names the Catalogue found,
  logout and resync in; the playback to show, the tracks to name and where a button goes out. Fixes: a lost update when
  names arrived, bare rows on coming back to this computer, a lyrics search with no artist. Tests:
  `names_that_came_while_another_device_played_are_kept_when_playback_comes_back` and
  `a_lyrics_hint_needs_a_name_and_an_artist` (both red on the old rules moved behind the new interface), five more at
  that interface, and `logging_out_lets_go_of_the_other_device` at the hub.
- **Architecture review, candidate 3: the open Track list, rows by identity** (2026-11-22, D81). A row's menu and the
  selection were positions in the sorted, filtered view, so a page arriving (or another Client's removal) made them
  point at another song. `OpenList` owns the lists, the open view and the remembered views; rows are known by uid,
  else URI; every list edit settles the selection back on its row. Tests: four at `OpenList`, and
  `a_row_menu_acts_on_its_row_even_if_a_page_arrives_while_it_is_open` (red first). Cards: #23.
- **Architecture review, candidate 7: one door for Daemon events** (2026-11-22, D82). The Client's connection read a
  frame inside a `select!`, so a command sent mid-frame cut it and the stream fell out of step; frames are now read
  on their own task. The event handling moved from `main.rs` (no test reached it) into `App::receive`, which tests
  use too; the browser for a login opens from the main loop. Tests: the pump over an in-memory pipe (red first), three
  at `receive`, and 46 test lines that faked events now go through it.
- **Architecture review, candidate 8: one Popup** (2026-11-22, D83). Fourteen popup fields on `App`, with which one is
  on top written three times in three orders (keys, pointer, drawing), became one `Popup` value and one order,
  `App::layers`. Tests: `a_popup_opened_over_the_wizard_is_on_top_for_the_eye_the_mouse_and_the_keys` and
  `opening_a_popup_replaces_the_one_open` (both red first).
- **A Card's menu by identity** (2026-11-22, D85; #23). The Cards half of candidate 3: a Card's menu and Home's
  selection were positions in the Shelves, so a Home feed arriving while a menu was open made it act on another Card.
  The menu records the Card's URI and Shelf and finds it again; a Card gone says so. The selection and each Shelf's
  scroll follow their Shelf across a new feed. Tests: one through the real menu (red first), one at `HomeView`.
- **#21 Moving Panes, smooth and bulletproof** (2026-11-22, D84). Replayed real pointer paths first (a test clock,
  mouse and trackpad, moves combined as the main loop does) on every Design; found four kinds of jank, and review a fifth: a drop that
  landed somewhere the preview never showed (a fling, an overshoot let go at once, a release over the top bar, the
  player or after a resize), a release the Client never heard leaving a Pane or a divider held for good, a right click
  dropping the held Pane, a divider flung past the well's limit stopping short; after review, a preview kept from the old
  window after a resize. All fixed, each red first; no flicker found. `arrange/paths.rs` holds the replayer, the hands and the properties checked on every path; pty
  `tiling-paths`. Open for the owner: the drop is now the preview (the old rule landed a quick flick where it was let
  go), and whether the dwell or the preview's motion between zones should change.
- **First run on ARM hardware** (2026-11-23, D73, D86, D87). The M2 Air on Asahi built natively and passed
  `tools/check.sh`; the Visuals ran on Asahi's Vulkan driver; librespot played (a muted capture of its stream carried
  signal) and fed the Widgets; MPRIS paused it. Soloist waits for the key. The owner's touchpad found jerky scrolling
  (#32: the wheel will scroll the view) and the Pane-moving UX gets a rethink first (#21).
- **Pane moving rethought** (2026-11-23, D87; #21, #31). Five grilling rounds from the owner's Mac test ("dropping one
  in a specific location … feels almost impossible, the entire app keeps reorganizing itself"). Root cause in D66: one
  title drag both moved a Widget and reshaped the layout (edge and screen-edge zones, live reflow), so the app changed
  shape under the pointer and a plain swap was the smallest target. Outcome (D87 row, ADR 0005, glossary: Tray, Arrange
  mode, Required Widget, Widget picker, Drop zone, Free tiling, Pane): a drag swaps contents on a fixed floor plan; the
  Tray holds Widgets while you rearrange; Home is the only Required Widget; every drop is allowed (a Widget too small
  says `needs 30 × 10`); right-click opens a Pane's menu with Replace widget… first; Split opens the Widget picker on a
  new empty Pane; New design… builds a Design from one empty Pane. A size contact sheet the owner judges comes first,
  because a hand-built floor plan makes odd Pane sizes common. #31 merges as the baseline (its replayer and stuck-drag
  fixes stay; its zones and dwell go with Arrange mode). Next: `/to-spec`, `/to-tickets`.
- **Architecture review, candidate 8 step 2: one Page** (2026-11-25, D88; #27). Grilled in one round (Q84–Q87):
  the main Pane's page becomes one value holding its view; behaviour unchanged except that leaving a page cancels an
  episode still being looked up.
- **Architecture review round 2** (2026-11-25, D89). Two read-only explorers: the platform seams (74 OS cfg sites in 16
  files; the Soloist VM threaded through the session pump with 20; process lookup in 5 places; D50's paths copied 12
  times) and the Client's hot spots since 11-22 (Arrange mode ~100 methods on App; input reading last frame's hit map;
  a Widget's facts in 8 modules; five separate drag states). Ten candidates (11–20), all accepted, in two waves.
  2026-11-25: owner answered Q98–Q101 during the build (Q101: the 'moves with the music' list becomes a Widget
  catalogue fact, as groundwork for more music-reactive Widgets).
  2026-11-25: owner answered Q102–Q105 during the build.
  2026-11-26: owner answered Q106–Q109 (release note; how a cut-short drag ends).
- **v1.0 release and demo mode** (2026-11-27, D90). The owner's wave 2 app test passed (Q111), and Q112–Q121 settled how v1
  reaches `main` (one draft PR, #127, merged and tagged after the macOS session #85), what else goes in first (#80, #81,
  #126) and a demo mode for recording videos on the owner's own account. A read-only privacy audit of the Client and
  Daemon found 59 places that can show personal data in 12 groups (account names, device names, the Jam link and QR,
  paths, the library, Home, context names, stats, recent searches, the login browser, toasts); its design (one filter
  where Daemon events enter the Client, a redactor, the Demo set) was accepted whole (Q119). Build ticket: #128.
  2026-11-27: owner answered Q126–Q128 and Q133–Q150 (bug-hunt fixes for v1, search pause, offline Home, Search keys,
  #76 picks, the wheel, demo extras); Q129, Q132 open, Q130/Q131 reworked (Delete in Pick a design with undo, Q151),
  Q143 follow-up Q152. Tickets: #142–#146 (v1), #147, #148 (after v1).
  2026-11-28: owner answered Q129 yes, Q132 yes (Del on Linux, delete on macOS), Q151 Undo only for deleting one's own
  Designs (deleted at once, `Deleted "X" · Undo` with `u`/click until the picker closes; Ctrl+K path with Ctrl+Z;
  built-ins unchanged), Q152 not B: a larger version of the original Home icon with no square, Q153 yes (full-screen
  Now playing re-laid, block letters at 60×8; #154 merged), Q154 yes (one Settings line for an incomplete backup),
  Q155 yes (demo hides Home Shelves from the account), Q156 yes (demo shows an empty player until a Demo-set song plays),
  Q157 yes (demo Home: Demo playlists, then albums and artists from their songs, then liked if on). Q147: a notch is
  exactly three reports at once on the PC's mouse (fits D86).
  2026-11-28, later: owner answered Q158 yes (the incomplete-backup line as built: red, on both tabs, "tried again
  tomorrow"), Q159 yes (the picker's line reads `Deleted "X" · u Undo`, the key before the action like the other
  hints), Q160 yes (on a Mac the delete hint reads `⌫ delete`; the PC keeps `Del delete`), Q161 yes ("(edited)" copies
  keep the yes/no; the one-line list keeps the small ⌂; ♡ stays small), Q162 yes (`demo.toml`'s `[home]` rules are
  dropped, superseding that part of Q149), Q163 yes (a song this Client plays or queues shows in demo mode; the owner
  picks showcase songs that way), Q164 yes (the KDE media widget and notifications are left to the recording
  checklist), Q165 yes (songs by the artists and albums `demo.toml` lists show even when queued elsewhere; autoplay
  stays hidden), Q166 yes (big text uses the terminal's own Ghostty font, found through its includes, #160).
  2026-11-28, day: owner answered Q167–Q175 yes (v1 review, research re-verify, search diagnosis; Q172 native Soloist
  on the Mac in #85; Q173 Karaoke fixed in #165), Q176 yes (Delete listening data also deletes the fast-start copy and
  the demo log; the recording checklist names the media widget / Now Playing), Q177 yes (unknown arguments and bad
  `SINGULARITY_DEMO` values are errors; a short `--help`), Q178 yes (`--seed-demo` after v1.0, before the first
  recording), Q179 yes (one tidy-up pass), Q181 yes (without real word timing Karaoke lights whole lines), Q182 yes (the
  fill stops at a word's end, the ball waits), Q183 yes (a line-only LRCLIB match leads to one search for a word-timed
  version, only with Karaoke on screen), Q184 yes (start-up login retried with a growing wait, stopping after a while),
  Q185 yes, Q186 yes (as built in #165). Q187: Polish rap still wrong on some lines; songs unknown, checked in #166.
  Tickets: #166, #167, #168, #170 (v1), #169 (after v1.0).
  2026-11-28, evening (PC): owner answered Q198 yes (during start-up tries a Client connect or a play tries at once,
  at most every 5 s), Q199 yes (a Spotify refusal ends every retry, not only start-up's), Q200 yes (a keyring not ready
  at start isn't a logout when logged in last time), Q201 yes (`-h` = `--help`), Q202 yes (a play pressed during a
  login waits for it and plays), Q203 yes (the Delete hint names everything it removes): #175. Karaoke (#166, PR #173):
  Q204 yes with care ("some rap songs really will go that fast"): a line lights whole only when its word times are
  faster than anyone raps (under 40 ms a syllable); Q205 yes (the ball rests left of a whole line), Q206 yes ("no word
  timing"), Q207 yes (search only after a line-only match), Q208 yes (edge punctuation lights with its letter), Q209
  yes (the fill stops early only in quick songs, under 200 ms a syllable). Owner search test: the wheel doesn't scroll
  results, Esc doesn't keep a recent search, older queries hold up the newest: #176.
