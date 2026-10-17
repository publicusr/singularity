# Singularity

A terminal Spotify client, built for the owner first, with desktop-app parity, lossless playback
through Spotify Soloist, and built-in audio-reactive visuals.

## Language

### Backends

**Engine**:
The process that actually decodes and plays audio. Currently Spotify Soloist.
_Avoid_: player backend, daemon, streamer

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

### Interface

**Shelf**:
One titled horizontal row on the Home page ("Made For You", "Recently played").
_Avoid_: section, row, carousel

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
After a stretch without input, the black hole drifts over the whole interface and bends its text (a screensaver).
Any input ends it and is swallowed.
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
A named arrangement of Panes and the Widgets in them ("Classic", "LARP", "Cockpit").
_Avoid_: profile, workspace, scene

**Design direction**:
One complete visual identity under evaluation (layout, palette, and Visual choices together).
It is a prototyping term; the shipped app has Themes and Layout presets instead.
_Avoid_: mockup, skin

**Theme**:
A file of named colours that styles the whole UI and the colour ramps of Visuals.
_Avoid_: colorscheme, skin, palette

**Render tier**:
How a Widget draws: *cell* (text characters, works anywhere) or *pixel* (kitty graphics protocol
in modern terminals). Pixel is the primary target; cell is the fallback.
_Avoid_: backend, mode
