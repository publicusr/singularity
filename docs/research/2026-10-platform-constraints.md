# Platform constraints (researched 2026-10-17)

What Spotify actually allows a third-party terminal client to do as of October 2026.
These are **facts**, not decisions. Decisions that depend on them live in
[`../DESIGN-TREE.md`](../DESIGN-TREE.md). Re-verify anything older than ~3 months before relying on it,
because this landscape has changed every few months since late 2024.

## 1. The official Web API is nearly useless for a full client

- **Nov 2024**: recommendations, related artists, featured playlists, category playlists,
  audio features/analysis were removed for new apps.
  ([Spotify blog](https://developer.spotify.com/blog/2024-11-27-changes-to-the-web-api))
- **May 2025**: extended quota is only for registered organisations with ≥250k MAU.
- **Feb/Mar 2026**: Development Mode requires the app owner to have **Premium**, at most
  **5 allow-listed users** per app, browse endpoints and artist top tracks removed, search capped
  at **10 results**, library endpoints merged into generic URI-based ones. Endpoint removal for
  *existing* apps was postponed indefinitely on 2026-03-09.
  ([migration guide](https://developer.spotify.com/documentation/web-api/tutorials/february-2026-migration-guide),
  [TechCrunch](https://techcrunch.com/2026/02/06/spotify-changes-developer-mode-api-to-require-premium-accounts-limits-test-users/),
  [rspotify #550](https://github.com/ramsayleung/rspotify/issues/550))
- **Jul 2026**: up to 25 client IDs per developer, but they all share **one quota**. Exceeding it
  returns `429 QUOTA_EXCEEDED`.
- There is **no official endpoint for the Home feed**, and there never was.

**Consequence:** a client built on the Web API cannot have a Home page, Made For You, or browse,
and it gets rate-limited if it polls.

## 2. Why spotify_player broke on this machine

The local logs (`~/.cache/spotify-player/*.log`, 2026-10-13/14) show every Web API call
(`/me/player`, `/me/playlists`, `/me/tracks`, …) returning **429 Too Many Requests** with
`retry_after_secs=29` until the retries run out. Audio (librespot via ALSA/rodio) started fine.
The config polls with `app_refresh_duration_in_ms = 32`.

**Lesson for us:** never poll the Web API. Get player state from a source that pushes events.

## 3. librespot (the engine behind spotifyd, spotify_player, ncspot)

- **No lossless.** Issue #1580 is closed as *not planned*. Forum reports say Spotify asked the
  maintainers not to pursue it.
  ([#1580](https://github.com/librespot-org/librespot/issues/1580),
  [discussion #1685](https://github.com/librespot-org/librespot/discussions/1685))
- Max quality is Ogg Vorbis 320 kbps.
- Reports of a steady trickle of breaking changes, and of some accounts or tracks failing
  ([#1649](https://github.com/librespot-org/librespot/issues/1649),
  [#1737](https://github.com/librespot-org/librespot/issues/1737), [HN thread](https://news.ycombinator.com/item?id=49517870)).
  Other users dispute that it is broken generally.

## 4. Spotify Soloist: official, headless, lossless (released 2026-08-13)

([blog](https://developer.spotify.com/blog/2026-08-13-introducing-spotify-soloist),
[docs](https://developer.spotify.com/documentation/soloist),
[repo](https://github.com/spotify/soloist))

- A headless **Spotify Connect receiver** for Linux (x86_64, aarch64, armv7). It uses the
  **same playback engine as the desktop app**.
- **Lossless up to 24-bit/44.1 kHz** (Premium), crossfade/automix, loudness normalisation,
  Smart Shuffle, autoplay, Jam, podcasts, audiobooks.
- Audio goes out through **PipeWire** (PulseAudio fallback). `--pipewire-device` routes it to a
  specific node, which lets a visualiser tap the stream.
- **Local WebSocket API** (`--ws 127.0.0.1:0`, with the endpoint written to `ws.addr`/`ws.port` in its data dir):
  - Events: `auth_state`, `playback_state`, `track_changed`, `playback_changed`,
    `volume_changed`, `device_changed`, `context_changed`, `options_changed`, `queue_changed`,
    `position_sync` (a position anchor you interpolate locally, so no polling is needed).
  - Commands: `play [uri]`, `pause`, `skip_next/prev`, `seek`, `set_volume`, `set_shuffle`,
    `set_repeat_context/track`, `add_to_queue` (tracks only), `activate/deactivate`,
    `get_state`, `get_queue [limit]`.
  - Entities come with name, cover art URLs, album, artists, duration, and explicit flag.
  - **Gaps:** no search, no library, no home page, no "start playlist at track N", no queue
    reorder or remove, no likes. It is a player, not a catalogue.
- **Friction:**
  - Every user needs their own **Soloist API key** from the developer dashboard (Premium).
    It must not be embedded or shared.
  - **First login is done by selecting the device from another Spotify app** (for example a phone) on the
    same LAN. After that the session is stored and restored.
  - **Builds expire after 90 days** (exit code 10). The binary **may not be redistributed**, so
    we can only link to the download page and detect expiry.
  - No config file; it is configured by CLI flags.
- An AUR package (`spotify-soloist-bin`) and a Home Assistant add-on already wrap it.

## 5. Spotify's internal (private) APIs

These are what the official apps use: `spclient` (extended metadata, playlist v2, collection v2,
radio, connect-state, color-lyrics, audio-attributes) and `pathfinder` (GraphQL: search, home,
browse). Reached with a librespot session token.

- **spotatui v0.44** dropped the Web API and client ID entirely and moved to these. Library,
  playlists, search, artist pages, recommendations (radio), devices, player, lyrics and a Home
  page all work. Related artists come back empty.
  ([PR #752](https://github.com/LargeModGames/spotatui/pull/752),
  [PR #763](https://github.com/LargeModGames/spotatui/pull/763))
- **This is the only way to get Home, Made For You mixes, Discover Weekly, and browse.**
- **Costs:** against Spotify's terms of use, undocumented, can break without notice, and there is a
  theoretical account-suspension risk (no reports of suspensions found).

## 6. Open/neutral data sources

- **LRCLIB**: free, open, time-synced lyrics database. A ToS-clean fallback for lyrics.
- **MPRIS** (D-Bus): standard Linux media control. KDE's media widget and keyboard media keys work
  through it.

## 7. Local environment (this machine)

kitty (`xterm-kitty`, truecolor, kitty graphics protocol available), KDE Plasma on Wayland,
PipeWire, Rust 1.98.1 + cargo, cava and btop installed, Fira Code font.
