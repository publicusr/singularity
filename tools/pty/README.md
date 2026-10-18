# pty harness

Drives the real `singularity` binary in a pseudo-terminal and screenshots it (pyte + PIL), with its own `singularityd`
on a short test socket (`$XDG_RUNTIME_DIR/sgl-<pid>.sock`) and scratch config/state/data dirs under the output folder. It kills only what it started.

```sh
cd tools/pty
python3 -m venv venv && venv/bin/pip install pyte pillow websockets
cargo build                                    # debug binaries are what it runs
venv/bin/python seed_covers.py ../../crates/client/tests/fixtures/home.json xdgcache/singularity/covers
venv/bin/python shot.py <scenario> out-<scenario>   # writes PNGs + text dumps (PTY_COLS/PTY_ROWS: size, default 140×40)
```

Scenarios: `cells`, `pixel` (seek), `home-cells`, `home-pixels`, `library`, `queue` (#32: also drags a queued song to the top), `lyrics` (#30: try `PTY_COLS=96 PTY_ROWS=34` for wrapped lines), `artist`, `search`,
`like` (#22: ♥, `l`, unlike from a menu, Follow), `mpris` (#21: real `playerctl` on a private D-Bus), `config` (#20: live keys.toml/config.toml edits), `designs` (#18: every design via the picker, Cockpit's palettes), `layouts` (#19: swap, drag, save, restart), `idle-pixels`/`idle-cells` (#17: the takeover via the palette, a key wakes it), `meters` (#29: Cockpit at full volume and at 10 %), `history` (#28: ◂ ▸ in the main Pane), `home-scroll` (#27: Home by lines, cropped covers), `wheel` (#26: the wheel over a list's links and the library), `mini` (#47: run with a small `PTY_COLS`/`PTY_ROWS`), `playlists` (#51: sidebar menus on your own and someone else's playlist, New playlist… via Ctrl+K, delete asked twice), `playlist-details` (#52: Edit details… from the menu, the new name and description everywhere, a refused edit put back), `playlist-add` (#53: Add to playlist… from a row's menu, filtered picker, a duplicate asked first, a new playlist made with the song), `playlist-remove` (#54: Remove from this playlist from the menu and with Delete; not offered on others'), `queue-lift` (#50: play from a list, queue a song, drag a "Next from" song up onto it), `stalls` (#45: a starving fake stream: one toast, the stream Pane's speed and stalls), `popup` (#41: the palette over pixel covers in kitty's under-background layer), `arrows` (every Widget of two Panes, clicking ▸: the arrows never vanish), `ambient` (#44: starfield, marquee, glow swapped in while a song plays, pixel tier), `stats-widgets` (#37: "today" and "top this week" swapped in, a live play updates both), `stats` (#36: a seeded schema-1 log, tabs, links filled in, the top artist opens; try 80×24 and 256×68), `listening` (#23: seeded scratch log in Cockpit, a live play, backup, export, delete), `audio`, `well-cells`, `well-pixels`, `well-hover` (counts black-hole frames while the mouse moves). The pty reports
10×20 px cells, so pixel-mouse and pixel-tier paths run; kitty APCs (static and animated) are decoded and composited
under the text, as kitty does with z=-1, at pyte's cursor when the APC arrives, cropped by `x,y,w,h`.

- `wizard_test.py out-wiz`: first run with no key and no keyring (scratch XDG dirs, a stand-in `soloist` script in
  `fresh/data/singularity/soloist/soloist`: create it first, see the script). Never touches your real Soloist.
- `update_test.py out-update` (#6): a real Daemon runs an expiring stand-in Soloist (`fake_soloist.py`: WebSocket
  protocol, `--version`, the lifetime line) from scratch dirs; a fake CDN then offers a newer build, which must be
  staged while a track plays and taken over at the next track. No key, keyring, audio tap or D-Bus of the owner's.
- `stale_test.py out-stale` (#25): a Daemon that ignores `Shutdown` (`SINGULARITY_IGNORE_SHUTDOWN=1`, playing a build
  from before the replace logic) meets a newer Client, which stops it and starts its own (scratch dirs, no keyring,
  tap, D-Bus or LocalStore, a dead CDN).
- `ws.py '<json>' [secs]`: talk to Soloist's WebSocket directly (`WS=addr:port` overrides the port file). Only for
  deliberate, quiet live tests on the owner's player; restore volume afterwards.

Careful: a Client autostarts a *real* Daemon if none answers on its socket; always start the fake one first (the
harness does). Never start a second real Daemon against the owner's Soloist state.
