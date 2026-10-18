# pty harness

Drives the real `singularity` binary in a pseudo-terminal and screenshots it (pyte + PIL), with its own `singularityd`
on a short test socket (`$XDG_RUNTIME_DIR/sgl-<pid>.sock`). It kills only what it started.

```sh
cd tools/pty
python3 -m venv venv && venv/bin/pip install pyte pillow websockets
cargo build                                    # debug binaries are what it runs
venv/bin/python seed_covers.py ../../crates/client/tests/fixtures/home.json xdgcache/singularity/covers
venv/bin/python shot.py <scenario> out-<scenario>   # writes PNGs + text dumps
```

Scenarios: `cells`, `pixel` (seek), `home-cells`, `home-pixels`, `library`, `queue`, `lyrics`, `artist`, `search`,
`like` (#22: ♥, `l`, unlike from a menu, Follow), `mpris` (#21: real `playerctl` on a private D-Bus), `audio`, `well-cells`, `well-pixels`, `well-hover` (counts black-hole frames while the mouse moves). The pty reports
10×20 px cells, so pixel-mouse and pixel-tier paths run; kitty APCs (static and animated) are decoded and composited
under the text, as kitty does with z=-1.

- `wizard_test.py out-wiz`: first run with no key and no keyring (scratch XDG dirs, a stand-in `soloist` script in
  `fresh/data/singularity/soloist/soloist`: create it first, see the script). Never touches your real Soloist.
- `ws.py '<json>' [secs]`: talk to Soloist's WebSocket directly (`WS=addr:port` overrides the port file). Only for
  deliberate, quiet live tests on the owner's player; restore volume afterwards.

Careful: a Client autostarts a *real* Daemon if none answers on its socket; always start the fake one first (the
harness does). Never start a second real Daemon against the owner's Soloist state.
