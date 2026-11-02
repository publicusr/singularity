# pty harness

Drives the real `singularity` binary in a pseudo-terminal and screenshots it (pyte + PIL), with its own `singularityd`
on a short test socket (`$XDG_RUNTIME_DIR/sgl-<pid>.sock`; on macOS, which has no runtime dir, `/tmp/sgl-<uid>/`) and scratch config/state/data dirs under the output folder. It kills only what it started.

```sh
cd tools/pty
python3 -m venv venv && venv/bin/pip install pyte pillow websockets
cargo build && cargo build -p singularityd --features fake   # debug binaries, the Daemon with the fake Catalogue
venv/bin/python seed_covers.py ../../crates/client/tests/fixtures/home.json xdgcache/singularity/covers
venv/bin/python shot.py <scenario> out-<scenario>   # writes PNGs + text dumps (PTY_COLS/PTY_ROWS: size, default 140×40)
```

Scenarios: `cells`, `pixel` (seek), `home-cells`, `home-pixels`, `library`, `queue` (#32: also drags a queued song to the top), `lyrics` (#30: try `PTY_COLS=96 PTY_ROWS=34` for wrapped lines), `artist`, `search`,
`like` (#22: ♥, `l`, unlike from a menu, Follow), `mpris` (#21: real `playerctl` on a private D-Bus), `config` (#20: live keys.toml/config.toml edits), `designs` (#18: every design via the picker, Cockpit's palettes), `readme` (the README's pictures in `docs/media/`: each design playing, no popup, then the Jam picker; copy the chosen PNGs over), `layouts` (#19: swap, drag, save, restart), `tiling` (D66: drag a title to a swap, an edge and a screen-edge zone with a shot mid-drag, the naming prompt, Esc, the ×, a split from the title menu, Move mode, Reset design; pixel tier, 160×48, start from an empty output folder), `tiling-fast` (fast drags at `motion_fps = 240`: a sweep across Panes, a pause, a drop on a small Pane that swaps; prints the achieved frame intervals from `SINGULARITY_FRAME_LOG`; use `PTY_BIN=target/release`), `tiling-lift` (the owner's
2026-11-01 round: a near-neighbour drop that is no drop, the lifted Pane floating over a dashed slot while the others
ease into place, shot at several moments of each animation, the screen edges, the landing, undo from the naming
prompt and redo, Move mode's Shift+arrow refusals in the bar, `u`, Ctrl+Z mid-animation; 160×48, empty folder), `idle-pixels`/`idle-cells` (#17: the takeover via the palette, a key wakes it), `meters` (#29: Cockpit at full volume and at 10 %), `history` (#28: ◂ ▸ in the main Pane), `jam` (D64: the picker's Jam row, `j` starts one, the invite link, QR and guest, the bar's chip, `j` twice ends it), `home-scroll` (#27: Home by lines, cropped covers), `wheel` (#26: the wheel over a list's links and the library), `wheel-burst` (Ghostty: three reports per notch move one row), `popup-wheel` (wheel notches inside, on the edge of and outside Devices and Settings, the split Widget list ("add a pane": right-click a title → Split right) and the palette move the popup and leave the Panes behind alone), `mini` (#47: run with a small `PTY_COLS`/`PTY_ROWS`), `playlists` (#51: sidebar menus on your own and someone else's playlist, New playlist… via Ctrl+K, delete asked twice), `playlist-details` (#52: Edit details… from the menu, the new name and description everywhere, a refused edit put back), `playlist-add` (#53: Add to playlist… from a row's menu, filtered picker, a duplicate asked first, a new playlist made with the song), `playlist-remove` (#54: Remove from this playlist from the menu and with Delete; not offered on others'), `playlist-move` (#55: drag a row with its preview, Alt+↓, a sorted view says why not), `playlist-cover` (#56: library miniatures, a dropped picture previewed square, uploaded, then Remove cover), `book` (#62: the Audiobooks chip, a book's page with chapters, progress and Spotify's access line, a chapter plays), `show` (#59–#61: episodes under the Podcasts chip, a show from search, Follow puts it in the library, progress marks, older episodes at the end, an episode card opens its show there), `settings` (#57: a click flips a setting, → the next design, a typed number, an edit in the file shows; comments kept; #58: the keys tab, a binding and a mistake added to keys.toml), `queue-wheel` (the wheel over the queue moves its highlight a song at a time; nine songs queued so it scrolls), `queue-lift` (#50: play from a list, queue a song, drag a "Next from" song up onto it), `stalls` (#45: a starving fake stream: one toast, the stream Pane's speed and stalls), `popup` (#41: the palette over pixel covers in kitty's under-background layer), `arrows` (every Widget of two Panes, clicking ▸: the arrows never vanish), `ambient` (#44: starfield, marquee, glow swapped in while a song plays, pixel tier), `visuals-new` (vinyl, waterfall, aurora and ripples swapped into the lyrics Pane while a song plays, pixel tier; `visuals-new-cells` the half-block fallback), `widgets-large` (stream, top, listening, today and top this week in big Panes of a saved layout on a healthy fake stream, `SINGULARITY_FAKE_STALLS=fine`), `stats-widgets` (#37: "today" and "top this week" swapped in, a live play updates both), `stats` (#36: a seeded schema-1 log, tabs, links filled in, the top artist opens; try 80×24 and 256×68), `listening` (#23: seeded scratch log in Cockpit, a live play, backup, export, delete), `PTY_BIN` runs other binaries (a release build with symbols), `PTY_WRAP` starts the Client under a tool (`heaptrack -o out/heap`). `audio` (playing, then paused: the meters, osc and fft must hold still; also on Terminal), `well-cells`, `well-pixels`, `well-hover` (counts black-hole frames while the mouse moves). The pty reports
10×20 px cells, so pixel-mouse and pixel-tier paths run; kitty APCs (static and animated) are decoded and composited
under the text, as kitty does with z=-1, at pyte's cursor when the APC arrives, cropped by `x,y,w,h`.

- `wizard_test.py out-wiz`: first run with no key and no keyring (scratch XDG dirs, a stand-in `soloist` script in
  `fresh/data/singularity/soloist/soloist`: create it first, see the script). Never touches your real Soloist.
- `update_test.py out-update` (#6): a real Daemon runs an expiring stand-in Soloist (`fake_soloist.py`: WebSocket
  protocol, `--version`, the lifetime line) from scratch dirs; a fake CDN then offers a newer build, which must be
  staged while a track plays and taken over at the next track. No key, keyring, audio tap or D-Bus of the owner's.
- `phone_test.py out-phone` (T0005, D63): the same stand-in and fake CDN, with the stand-in acting as a phone on
  Connect: it plays a song and queues one and skips to it by itself (a file in its data dir). With a build staged,
  the Daemon must not restart Soloist over the phone's change and must send no play/pause/seek; and when a restart
  into a new build happens (our own Next), a build that comes back already playing a track must keep it, not the
  track the Daemon remembered.
- `app_wizard_test.py out-appw [--engine librespot]` (#66, #70): the wizard and health check with the Spotify app (or
  librespot) as Engine, three Daemons in turn
  (no Catalogue; a real, logged-out one in scratch dirs; the fake one). Never starts the real app (`SINGULARITY_START_APP=0`).
- Every script pins `SINGULARITY_ENGINE` (so a Daemon a Client starts by itself never drives the real Spotify app on a Mac).
- `stale_test.py out-stale` (#25): a Daemon that ignores `Shutdown` (`SINGULARITY_IGNORE_SHUTDOWN=1`, playing a build
  from before the replace logic) meets a newer Client, which stops it and starts its own (scratch dirs, no keyring,
  tap, D-Bus or LocalStore, a dead CDN).
- `ws.py '<json>' [secs]`: talk to Soloist's WebSocket directly (`WS=addr:port` overrides the port file). Only for
  deliberate, quiet live tests on the owner's player; restore volume afterwards.

On macOS (#63) the same scenarios run, except `mpris` (needs `dbus-daemon` and `playerctl`). Fonts: DejaVu Sans Mono
Nerd Font and Noto Sans Symbols 2 on Linux; on macOS a Nerd Font Mono in `~/Library/Fonts` (DejaVu or Hack), else
Menlo, with Apple Braille for the braille. Terminal variables (`TERM_PROGRAM`, `KITTY_WINDOW_ID`, `GHOSTTY_*`) are
removed from the Client's environment, so a run behaves the same whichever terminal starts it.

Careful: a Client autostarts a *real* Daemon if none answers on its socket; always start the fake one first (the
harness does). Never start a second real Daemon against the owner's Soloist state.
