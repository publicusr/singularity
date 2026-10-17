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

## Run 2026-10-17 21:26

- Version: `soloist 1.3.9.32 build 1792173722 (20261016) (g440165f200) (linux/x86_64)`
- Build lifetime log: `20:26:27.844 I [f:128                           ] client expires in 88 days[0m`
- Pairing from the phone worked; session stored
- Codec/quality log line: `20:27:12.783 I [f:134                           ] FileStreamer file average bitrate: 0[0m`

### Lossless check (2026-10-17, `--verbose`)

- ❌ First check streamed **Ogg Vorbis 160 kbps** (`codec: vorbis`, `bitrate: 160000`), decoded to 32-bit float PCM at 44.1 kHz.
- Cause (per Spotify support): quality is set **per Connect device**. The phone's own Media quality setting doesn't
  apply. Fix to verify: phone → device picker → tap "Singularity (spike)" → *Change quality settings* → Lossless. Music
  Assistant's docs (which also use Soloist) describe lossless as a ceiling that Soloist honours.
- Side note from the same docs: librespot usually won't play on accounts created since Dec 2024, and Soloist works
  with all Premium accounts. That is another point for ADR-0001.
- Implication for the real app: the Daemon can read the codec/bitrate from Soloist's verbose log (for the "LOSSLESS" badge
  and stream-stats panel) and should tell the user if the device isn't set to Lossless.
