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

**Engine assembly**:
The Daemon's choice, at start, of which Engine runs, the device name it shows on Connect, where the AudioTap's audio
comes from and the Catalogue, made in one step from the config, the command line and the computer. A bad config value
resets only that key, and says so.
_Avoid_: engine setup, bootstrap

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

**Now playing**:
What is playing and on which device (this computer's Engine, or another device Singularity follows over Connect),
the same for every Client. The Now playing Widget shows it.
_Avoid_: current track, player state

**ConnectRemote**:
Spotify Connect commands sent through the Catalogue session, for what the Engine can't do itself (start a context at
track N, later device transfer and quality). Registers as a hidden device that never shows in device pickers.
_Avoid_: remote API, Connect client

**Remote change**:
Playback moved by something other than this Daemon: a phone (or another Spotify app) on Connect playing, skipping or
queueing on this computer's device, a Jam guest's play, Spotify itself. The Daemon follows it and never puts its own
remembered state back (D63).
_Avoid_: external event, override

**Quiet moment**:
When the SoloistEngine may restart Soloist into a staged build: at once while paused or idle, and otherwise only
between tracks when the Daemon's own command (Play, Next, Previous, TogglePlay) caused the change — a remote change is
not an invitation to restart (D63).
_Avoid_: safe restart, idle window

**Handover**:
What one Soloist run leaves for the next when the Daemon restarts it (into a new Build, or for a stuck stream): where
playback was, so it carries on there, and when the last stuck-stream restart was.
_Avoid_: restart state, carry-over

**Jam**:
Spotify's shared listening session ("social session" internally): one device hosts, guests add to and skip in the
host's queue. Server-side, through `social-connect`; a host needs Premium, and any listed Connect player can host.
Singularity starts one hosted on its own device (the device picker's Jam row), shows the invite link, its QR and the
guests, and ends it; the Jam is read from `social-connect`, never from the Connect cluster. Joining someone else's
moves a guest onto the host's device, so Singularity doesn't join (D64; `docs/research/2026-10-jam.md`).
_Avoid_: party mode, group session

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

**Spotlight**:
A Shelf that holds a single Card ("New release from …"), drawn as that Card's cover with its name, artists and type
beside it, so no empty Card slots trail it (D69).
_Avoid_: hero, banner, featured card

**Search bar**:
The filled line on top of Home that reads "Search" (D70). Clicking it, Ctrl+L, or ↑ from Home's top row then Enter or
typing opens Search, whose query sits in the same bar in the same place. The library has no Search row (D71).
_Avoid_: search box, search field, omnibox

**Recent search**:
A query run before, kept on this computer only and listed newest first under the Search bar while it's empty.
Picking one runs it again at once (D74).
_Avoid_: history, search history

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

**Smallest size**:
The smallest Pane (columns × rows, frame included) a Widget is still usable in, picked from the size contact sheet; the black hole's is its shape rule (4:3 over four rows). In a smaller Pane the Widget gives way to one dim, centred line, `needs 30 × 10`; a Pane too small even for that shows only its frame, the Widget's name in its title.
_Avoid_: minimum size (that's the Pane's own, which the floor plan keeps), min dimensions

**Mini-player**:
What Singularity becomes in a terminal too small for any layout (under 50×14): title, controls and progress only.
_Avoid_: compact mode, tiny mode


**Widget**:
Any self-contained thing that can occupy a Pane: a visual, lyrics, album art, queue, stats.
_Avoid_: module, applet, panel

**Karaoke Widget**:
A Widget that shows the line being sung big, fills its words with the accent as they are sung and bounces a ball from
word to word, landing on each as it starts; dots count a line in after an instrumental. It scales from one row to
*big text* on the pixel tier (the marquee's pixel font on the cell tier).
_Avoid_: sing-along view, karaoke mode

**Big text**:
Text drawn as an image on the pixel tier in the terminal's own font (found on the system; Noto Sans, bundled with the
Client, for characters it lacks or when none is found), so a big lyrics or Karaoke Pane is filled with text sized to
the Pane instead of terminal-sized text in its corner. One size per song and Pane; a song with characters no font has, a
popup over the Pane, or the cell tier keeps terminal text.
_Avoid_: pixel font (the marquee's 3×5 blocks), big font

**Word timing**:
When each word of a lyrics line is sung. *Exact* when the lyrics come with it (LRCLIB's word-synced Lyricsfile,
enhanced LRC), else *estimated*: the song's syllables at one *pace* fitted over all its lines, each line's words
from its start, its last word held; the Karaoke Widget says which.
_Avoid_: syllable sync (Spotify's unused `SYLLABLE_SYNCED`), word sync

**Lyrics delay**:
`lyrics.delay_ms` in the config: how much later the lyrics and Karaoke Widgets follow the music than the player's
position (for an output that lags, such as Bluetooth headphones, or lyrics stamped early throughout); clicking a line
seeks to where the music is when it shows.
_Avoid_: lyrics offset (LRC's `[offset:]` runs the other way), latency

**Lyricsfile**:
LRCLIB's lyrics format (YAML): lines with start and end times and, when word-synced, their words timed too.
_Avoid_: elrc

**Song lyrics**:
The playing song's lyrics as the Client holds them: what was asked for (with or without word timing), what came back,
and what the lyrics and Karaoke Widgets show of them. A new answer starts both afresh.
_Avoid_: lyrics state, lyrics cache

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
A rectangular region of the screen that holds one Widget or view, or nothing while you rearrange (an empty Pane).
_Avoid_: window, panel, split

**Tray**:
Where Widgets wait while you rearrange: a strip at the screen's edge, shown only in Arrange mode. A Widget dropped
on it (or closed with `×`) leaves its Pane empty; dragged from it onto a Pane, it takes that Pane (swapping whatever
was there into the Tray). Each Design keeps its Tray until you clear it; clearing removes what's in it (D87).
_Avoid_: shelf (that's Home's row of Cards), limbo, dock, clipboard

**Layout preset**:
A named arrangement of Panes and the Widgets in them; each Design has one.
_Avoid_: profile, workspace, scene

**Design**:
A Layout preset shipped together with the Themes made for it: Singularity (default), Event Horizon, Classic+, Cockpit,
Terminal, Karaoke. Picking a Design sets the layout, its own Theme and its Disk look; its Palette variants are a `t` away.
A layout you saved ("Save design as…") is a Design of your own: it remembers the Theme it was saved with.
_Avoid_: skin, mode, look (that's the Disk look)

**Free tiling**:
Rearranging a Design by hand, in two separate kinds of edit: the *contents* (drag a Pane's title onto another Pane to
swap their Widgets, or onto the Tray) and the *floor plan* (drag the line between two Panes, split a Pane, close one).
A drag never changes the floor plan; nothing else moves while you drag (D87, replacing D66's live reflow).
_Avoid_: edit mode, window manager

**Required Widget**:
A Widget every Design must give a Pane: only Home, the main Pane where every page opens. The library and the queue
may go; without a Pane they open as popups from the palette (D87). Built-in Designs still have all four of D43.
_Avoid_: core Widget, core four

**Arrange mode**:
The state in which Free tiling happens: the Tray shows, empty Panes say so, each Pane gets split and close buttons, and
Done (once every required Widget has a Pane) or Esc ends it. Dragging a title enters it. "New design…" starts it from
one empty Pane filling the screen, with the required Widgets waiting in the Tray (D87).
_Avoid_: edit mode, layout mode, builder

**Widget picker**:
The list that chooses a Pane's Widget: Replace widget… (from the Pane's menu on a right-click, a click on the Widget's
name or a key) or a split's new empty Pane. Type to filter, scroll like any list; Widgets not on screen first, those on
screen dimmed below a line (picking one swaps the two Panes' Widgets). Each Widget appears once on screen (D87).
_Avoid_: widget list, chooser

**Drop zone**:
Where a dragged Widget would land: another Pane (it swaps with what's there, or fills it if empty) or the Tray (D87;
D66's edge and screen-edge zones are gone).
_Avoid_: target, dock

**Move mode**:
The keyboard way to tile; being folded into Arrange mode's keys (D87, #43).
_Avoid_: edit mode, layout mode

**Design direction**:
One complete visual identity under evaluation (layout, palette, and Visual choices together).
It is a prototyping term; the shipped app has Designs.
_Avoid_: mockup, skin

**Theme**:
A set of named colours that styles the whole UI and the colour ramps of Visuals: built in (each Design's Palette
variants, and the shared ones every Design can wear: Ember, Sunset, Honey, Paper…) or a TOML file in `themes/`, which may start from a built-in one.
_Avoid_: colorscheme, skin, palette

**Render tier**:
How a Widget draws: *cell* (text characters, works anywhere) or *pixel* (kitty graphics protocol
in modern terminals). Pixel is the primary target; cell is the fallback.
_Avoid_: backend, mode
