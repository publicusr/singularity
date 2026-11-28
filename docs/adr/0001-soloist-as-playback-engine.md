---
status: accepted (validated by spike 1, 2026-10-17: FLAC streamed after setting the device to Lossless)
---

# Spotify Soloist is the playback engine, not librespot

Audio playback goes through Spotify's official headless client, Soloist, which the app supervises
and controls over its local WebSocket API. librespot is used, if at all, only as an optional
fallback engine. Soloist is the only way to get lossless (24-bit/44.1 kHz) in a third-party client.
It pushes player events, so we never poll Spotify, and keeping audio inside an official client
lowers our ToS exposure. We accept its costs: the owner needs Premium, a personal API key, a
one-time pairing from another Spotify app, Linux only, and 90-day build expiry. The expiry is
handled by auto-updating from Spotify's public CDN.

## Considered Options

- **librespot embedded**: one process and cross-platform, but capped at 320 kbps Vorbis, with
  lossless declined upstream and periodic breakage.
- **Official Web API player endpoints**: control only, no audio, and rate-limited (the reason
  spotify_player fails on this machine).

## Validation (spike 1)

Pairing, control, push events and the audio tap all worked. Streaming quality is a **per-Connect-device setting**
(phone → device picker → Change quality settings). Once set to Lossless, Soloist streamed FLAC (≈1.7 Mbit/s average).
librespot also reportedly won't play on accounts created since Dec 2024, which is one more reason for this decision.

## Note (2026-11-25, D89)

"Linux only" is still true of Soloist itself, but no longer of Singularity: on macOS, Soloist runs in the Soloist VM
(D55), behind the same Soloist runner as the local Linux process (D89), and the macOS default Engine is librespot
(D54). The decision above stands.

## Note (2026-11-28)

"Linux only" no longer holds for Soloist itself: since 2026-10-23 Spotify also ships it for Apple Silicon macOS 13+
and x86_64 Windows (`docs/research/2026-11-reverify.md`). Also, only the key needs Premium: once Soloist runs, Free
accounts can listen (lossless and crossfade stay Premium). Singularity's Mac Engines are unchanged for v1.0
(librespot by default, Soloist in the VM, the app); whether the Mac build replaces the VM is for the Mac session
(#85). The decision above stands.
