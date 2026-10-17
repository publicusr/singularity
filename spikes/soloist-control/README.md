# Spike 1: Soloist control + audio tap

Throwaway program that answers: *can Singularity run Spotify's official lossless engine, pair it,
drive it, and tap its audio for the visuals?* Results land in `FINDINGS.md`.

## Walkthrough (about 10 minutes, once)

**1. Get your Soloist API key** (needs your Premium account)

Open <https://developer.spotify.com/dashboard/soloist>, log in, and generate a key. Treat it like
a password: never paste it into chat, issues, or commits.

**2. Put the key in your keyring** (KWallet holds it; nothing lands in a file or shell history)

```sh
secret-tool store --label="Singularity: Soloist API key" service singularity account soloist
```

It prompts for the value; paste the key and press Enter. (Alternative: `SOLOIST_API_KEY=… cargo run --release`.)

**3. Run the spike**

```sh
cd ~/src/singularity/spikes/soloist-control
cargo run --release
```

It downloads Soloist from Spotify's CDN into `bin/` if it's missing (git-ignored, never redistributed),
starts it as a Spotify Connect device called **"Singularity (spike)"**, and connects to its local
WebSocket. Its session and cache live in `state/` (git-ignored); delete that folder to reset pairing.

**4. Pair from your phone** (first run only)

Phone on the same Wi-Fi → Spotify → play anything → tap the device icon → pick
**Singularity (spike)**. The spike prints `logged in`. Music now plays on the PC through PipeWire.
(Your firewall zone already allows mDNS and high ports, so discovery should just work.)

**5. Try it**, typing commands into the spike:

```
state                        what's playing
pause / resume / next / prev
vol 40
play spotify:playlist:37i9dQZF1DXcBWIGoYBM5M     (any playlist/album/track URI: Share → Copy link, then use the ID)
queue                        upcoming tracks
tap 10                       10 s live spectrum of *only* Soloist's audio, plus its sample format
quit
```

**6. Tell me what happened.** Paste the new part of `FINDINGS.md` (it contains no secrets) or just
describe it. Things worth noticing: did pairing work, does `tap` show a 24-bit stream (`s24le`/`s32le`
suggests lossless), and did anything error out?

## Already known (from the downloaded build)

- Version `1.3.9.32`, built 2026-10-16. The CLI help is saved to `soloist-help.txt`.
- The CLI has **no** options for audio quality, crossfade or normalisation: only output device,
  initial volume and cache size. Those settings must come from your Spotify account (set in the phone
  app) or the private API. That is the open question for D17.
