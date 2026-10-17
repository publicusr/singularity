# Visuals & widgets: idea bank

Brainstorm, not commitments. Each idea gets promoted into the spec only after a grilling round
or a prototype. Visual taste is **ungrillable**, so the plan is to prototype the top picks and react
to them rather than argue about them in prose.

## Status after three prototype rounds (2026-10-17)

- **Kept (owner liked):** black hole (ghostty-blackhole port and pixel ember), cava-style spectrum (smoothed, no white
  peaks), btop-style loudness history, triggered oscilloscope (calm), VU/level meters, listening heatmap, matrix rain,
  warp starfield, cover-art cards, synced lyrics (karaoke + terminal log), stream-stats panel, waveform seek bar.
- **Rejected:** synthwave/outrun sun (too loud, takes too much space), plasma and aurora backgrounds (too erratic, "not a
  TUI"), stereo Lissajous scope (too erratic), the Zen layout, a black hole drifting over content during normal use.
- **New ideas from the rounds:** gravity-well pane, idle-mode screensaver hole, per-design palette variants.

Legend: 🎧 = reacts to the live audio signal · 🎨 = follows the active theme / album palette ·
🖼 = benefits from the kitty graphics protocol (all must still have a text-cell fallback)

## Hero visuals (the "LARP quarter-screen" tier)

1. **Black Hole**: ray-traced Schwarzschild lensing with a spinning accretion disk and
   photon ring. Bass drives disk brightness and spin, transients make the disk flare, track
   changes make it "swallow" the old cover art. Tunables: disk tilt, spin, colour ramp, star
   density, lensing strength, render mode (braille / half-block / ASCII ramp / kitty pixels). 🎧🎨🖼
2. **Wormhole / tunnel**: flying through a ring tunnel whose speed follows tempo. 🎧🎨
3. **Warp starfield**: stars stretch into streaks on drops (energy onset detection). 🎧🎨
4. **Plasma / metaballs**: slow organic blobs with colour from the album art palette. 🎧🎨🖼
5. **Doom fire**: the classic fire effect, intensity tied to loudness. 🎧🎨
6. **Matrix rain**: glyph rain in theme colours, density follows RMS level, with song title
   characters mixed into the rain. 🎧🎨
7. **Game of Life seeded by audio**: beats spawn gliders and the spectrum seeds rows. 🎧🎨
8. **Rotating wireframe**: a 3-D wireframe torus or icosahedron that deforms with the spectrum. 🎧🎨

## Audio-analytic widgets

9. **Spectrum analyser**: cava-style bars (mirrored, braille or block, with gravity and smoothing),
   log-frequency axis. 🎧🎨
10. **Oscilloscope / Lissajous (stereo XY)**: phosphor-trail look. 🎧🎨
11. **VU / peak meters**: analogue needle (ASCII) or LED ladder, L/R. 🎧🎨
12. **Waveform scrubber**: the seek bar *is* the waveform of the current track, built up
    progressively from captured audio. Click anywhere to seek. 🎧🎨

## Music-context widgets

13. **Album art**: pixel-perfect with the kitty protocol, truecolor half-block fallback. Optional
    ambient glow that bleeds the cover's dominant colours into the frame. 🎨🖼
14. **Album-palette theming**: extract a palette from the cover and temporarily re-tint the theme
    accents (a btop-style "auto" theme). 🎨
15. **Synced lyrics**: karaoke-style highlighting with smooth scrolling. Click a line to seek to it.
    Fullscreen lyrics mode. 🎨
16. **Spinning vinyl / cassette**: ASCII turntable whose tonearm position is track progress. 🎨
17. **Big "now playing" marquee**: figlet-style giant title/artist for wall-display use. 🎨
18. **Up-next river**: horizontal timeline of the queue, with autoplay vs user-queued items
    distinguished and crossfade overlap drawn. 🎨

## Personal-stats widgets (local scrobble log, private, never uploaded)

19. **Listening heatmap**: GitHub-contribution-style calendar of minutes listened. 🎨
20. **Live "Wrapped"**: top artists/tracks for this week/month/year, streaks, "you've played this
    47 times". 🎨
21. **Session ticker**: time listened today, tracks played, genre mix sparkline. 🎨

## Nerd panel (btop energy)

22. **Stream stats**: codec (FLAC 24/44.1 vs Vorbis 320), buffer health, normalisation gain,
    output device, latency, cache size. A "LOSSLESS" badge. 🎨
23. **Connect devices**: every device on the account, live, with click-to-transfer. 🎨

## Modes & layout ideas

- **Layout presets**: `Classic` (Spotify-like), `LARP` (hero visual in one quadrant), `Focus`
  (mini player plus one visual), `Zen` (fullscreen visual after N minutes idle, like a screensaver),
  `Wall` (lyrics plus big now-playing for a second monitor).
- **Widget slots**: any pane can hold any widget. Swap with a right-click menu or palette command;
  resize by dragging borders with the mouse.
- **Mini-player mode**: degrades gracefully down to a 1-line status for a tiny tmux/kitty split.
- **Visual presets are shareable files**, like themes.
