# Singularity

A terminal Spotify client, built for the owner first, with desktop-app parity, lossless playback
through Spotify Soloist, and built-in audio-reactive visuals.

## Language

### Backends

**Engine**:
The thing that actually decodes and plays audio: Spotify Soloist on Linux (SoloistEngine), the hidden Spotify.app on
macOS (AppEngine, D51), or librespot inside the Daemon on either (LibrespotEngine, D52). Whichever runs is "this
computer" in the device list; `[engine] kind` picks it.
_Avoid_: player backend, daemon, streamer

**AppEngine**:
The Engine that is the official Spotify desktop app, started hidden and driven over Connect; what it plays is read
from the Connect cluster. The lossless choice on macOS.
_Avoid_: DesktopAppEngine (its old name), Spotify engine

**LibrespotEngine**:
The Engine that is librespot (a Connect receiver and player) running inside the Daemon, on Linux and macOS alike: the
light choice and the macOS default (D54), at most 320 kbps, never lossless. Driven over Connect like the AppEngine; its samples feed the AudioTap
directly (the tap's *Feed*).
_Avoid_: built-in player (fine in UI text, not as a term), spotifyd

**Soloist VM**:
Soloist (Linux-only) run on macOS inside a small Linux VM the Daemon manages (`kind = "soloist"` there, D55): the
SoloistEngine with a VM host. Its audio comes to the Daemon over vsock and plays from there.
_Avoid_: Linux VM engine, "the VM Engine" (the Engine is still Soloist)

**Process tap**:
The macOS AudioTap source for the Spotify app as Engine (#67): Core Audio's per-process capture, which needs the
"System Audio Recording Only" permission for the terminal Singularity runs in.
_Avoid_: screen recording, loopback

**Feed**:
The AudioTap's in-process input: an Engine that plays from the Daemon (librespot, the Soloist VM) or the process tap
hands it samples, only while a Client shows audio Widgets.

**Platform seam**:
One of the few places where Linux and macOS have their own code (Engine, AudioTap, Secrets, OS media integration,
autostart, process lookup). Everything else is shared (D49); `docs/PLATFORMS.md` lists them.
_Avoid_: port, backend

**Build**:
One Soloist release from Spotify's CDN. Builds expire after about 90 days; the Daemon stages a newer one and restarts
the Engine into it at a quiet moment (paused, idle, or a track change), so the user never sees the expiry.
_Avoid_: version (that's what `--version` prints), release, binary

**Catalogue**:
Everything browsable about Spotify: Home, search, library, entity pages, playlists, lyrics. Comes
from a catalogue source that is separate from the Engine.
_Avoid_: metadata API, backend, Spotify API

**Catalogue source**:
One implementation the Catalogue can come from: *private* (Spotify's internal APIs) or *official*
(the public Web API, fallback only).
_Avoid_: provider, adapter

**Daemon**:
`singularityd`, the background user service that owns playback, the Catalogue session, the audio tap
and local data. Music keeps playing without any Client.
_Avoid_: server, backend, service

**Client**:
`singularity`, a terminal window that connects to the Daemon and draws the interface. Several can run at once.
_Avoid_: frontend, TUI (as a noun for the process)

**ConnectRemote**:
Spotify Connect commands sent through the Catalogue session, for what the Engine can't do itself (start a context at
track N, later device transfer and quality). Registers as a hidden device that never shows in device pickers.
_Avoid_: remote API, Connect client

### Interface

**Keymap**:
Which keys do which actions: a preset ("standard", or "vim" which adds h/j/k/l movement) plus the user's own
bindings from `keys.toml`.
_Avoid_: shortcuts file, bindings (for the whole map)

**Shelf**:
One titled horizontal row on the Home page ("Made For You", "Recently played").
_Avoid_: section, row, carousel

**Card**:
One item on a Shelf (album, playlist, artist, show): its cover, name and subtitle. Clicking it opens it (a track Card
plays); ▶ Play is on the page it opens.
_Avoid_: tile, item, thumbnail

**Track list**:
The tracks of Liked Songs, a playlist or an album, shown in the main Pane. Sorting and filtering change only what
you see; playing a row always starts the list at that track.
_Avoid_: playlist view, table

**Saved**:
Whether something is in the user's library: a liked track (in Liked Songs), a saved album or playlist, a followed
artist. One toggle covers all four ("like", "save", "follow" are its wordings per kind); it shows at once and settles on
Spotify's answer, rolling back if Spotify refuses.
_Avoid_: favourite, bookmarked, starred

**Play**:
One entry in the listening log: a track heard for at least 30 s (or half of it, if shorter), logged when the track
changes or playback stops, with where it played (this computer or the Connect device Singularity followed).
_Avoid_: scrobble, stream, listen

**Listening log**:
The private, local record of Plays (SQLite in the LocalStore). Never uploaded; exportable and deletable from the palette.
_Avoid_: history (Spotify's own "recently played" is something else), stats

**Library backup**:
The nightly JSON copy of Liked Songs and every playlist the account owns (followed playlists by name and link only),
kept in the LocalStore: the last 14 days plus the first of each month for a year.
_Avoid_: export (that's the one-file copy the owner asks for), snapshot

**LocalStore**:
The Daemon's private data under `$XDG_DATA_HOME/singularity/`: the listening log and the library backups. Owner-only
files, no network.
_Avoid_: database, cache (caches can be thrown away; this can't)

**Stats page**:
A page in the main Pane that sums up the listening log for a calendar period (this week, month, year, all time): top
artists, tracks and albums by minutes, minutes and plays, streaks, a 24-hour clock. A live "Wrapped".
_Avoid_: dashboard, analytics, Wrapped (Spotify's yearly thing)

**Show page**:
A podcast show's episodes as a list in the main Pane, newest first, with each episode's progress as Spotify keeps it
(played, minutes left, new). Long shows load their older episodes as you scroll.
_Avoid_: podcast page, channel

**Book page**:
An audiobook's page: a Show page whose rows are chapters (Spotify keeps a book as a show and its chapters as
episodes), with "Audiobook · authors · Spotify's access line · narrated by … · length" above them. The access line is
Spotify's own wording ("Listen free until Oct 31", "Access with Audiobooks+"). librespot can't play chapters (D57).
_Avoid_: audiobook show, book show

**Settings page**:
A page in the main Pane built from `config.toml`: every setting with its value and the comment above it in the file
as its description. A change is written into the file (its comments kept) and applies at once; an edit in the file
shows on the page. The file is the one source of truth; the page is a view of it.
_Avoid_: preferences, options, settings screen (the spec's old word)

**Mini-player**:
What Singularity becomes in a terminal too small for any layout (under 50×14): title, controls and progress only.
_Avoid_: compact mode, tiny mode


**Widget**:
Any self-contained thing that can occupy a Pane: a visual, lyrics, album art, queue, stats.
_Avoid_: module, applet, panel

**Visual**:
A Widget whose purpose is decoration that reacts to the audio (black hole, spectrum, starfield).
_Avoid_: visualizer, effect, animation

**Slot**:
A Pane whose Widget the user can swap, limited to the Visuals the current design allows for it.
_Avoid_: hero, container

**Gravity well**:
The Pane that holds the black hole in the Singularity design. Its frame sags toward the hole; content elsewhere never moves.
_Avoid_: hole pane, black hole widget

**Idle mode**:
After a stretch without input (3 min by default), the black hole on screen breaks out of its Pane and drifts over the
whole interface, bending its text. Any input ends it and is swallowed. Can be switched off.
_Avoid_: screensaver mode, drifting hole, floating mode

**Palette variant**:
One of a design's alternative colour sets; switching it recolours every element of that design together.
_Avoid_: colour mode, sub-theme

**Disk look**:
A named preset for the black hole's accretion disk: temperature, tilt, thickness, streaks (Pixel ember, Inferno,
Gargantua, M87*, Quasar, Blazar, Face-on ember).
_Avoid_: black hole theme, skin

**Pane**:
A rectangular region of the screen that holds exactly one Widget or view.
_Avoid_: window, panel, split

**Layout preset**:
A named arrangement of Panes and the Widgets in them; each Design has one.
_Avoid_: profile, workspace, scene

**Design**:
A Layout preset shipped together with the Themes made for it: Singularity (default), Event Horizon, Classic+, Cockpit,
Terminal. Picking a Design sets the layout, its own Theme and its Disk look; its Palette variants are a `t` away.
_Avoid_: skin, mode, look (that's the Disk look)

**Design direction**:
One complete visual identity under evaluation (layout, palette, and Visual choices together).
It is a prototyping term; the shipped app has Designs.
_Avoid_: mockup, skin

**Theme**:
A set of named colours that styles the whole UI and the colour ramps of Visuals: built in (each Design's Palette
variants) or a TOML file in `themes/`, which may start from a built-in one.
_Avoid_: colorscheme, skin, palette

**Render tier**:
How a Widget draws: *cell* (text characters, works anywhere) or *pixel* (kitty graphics protocol
in modern terminals). Pixel is the primary target; cell is the fallback.
_Avoid_: backend, mode
