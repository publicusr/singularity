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
