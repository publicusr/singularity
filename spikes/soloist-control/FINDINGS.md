# Spike 1 findings

Appended automatically by each run. Contains no secrets.

## Known before the first run

- Build `soloist 1.3.9.32 build 1792173722 (20261016)`, downloaded from the public CDN without login.
- CLI has no quality / crossfade / normalisation options (see `soloist-help.txt`, git-ignored).

## Run 2026-10-17 21:13

- Version: `soloist 1.3.9.32 build 1792173722 (20261016) (g440165f200) (linux/x86_64)`
- Build lifetime log: `2026-10-17 21:13:35.567: client expires in 88 days`
- Pairing from the phone worked; session stored
- PipeWire stream: `float32le 2ch 44100Hz · app="Spotify" · binary=null`
- Audio tap of only the Soloist stream works: peak level 0.03 over 10s

### Interpretation (2026-10-17)

- ✅ ADR-0001 holds: download → start → pair from phone → control over WebSocket all work. Push events
  arrive (no polling), including autoplay queue items with their source (`context` / `queue` / `autoplay`).
- ✅ Expiry is visible at startup (`client expires in 88 days`), so the Daemon can parse it and auto-update ahead of time.
- ✅ The audio tap isolates Soloist's stream (PipeWire calls it `app="Spotify"`). Caveat: the level is *post-volume*
  (peak 0.03 at volume 40), so the visualiser needs automatic gain (the prototype has it) or a pre-volume tap.
- ❓ Lossless is not provable from the stream format: PipeWire carries float32 regardless of source. Next: enable
  Lossless in the phone app (Settings → Media quality), rerun with `--verbose`, and use the `codec` command.
- Some queue items arrive without metadata (`?`). The Client will fill those from the Catalogue by URI.
