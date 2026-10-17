# How risky is Spotify's private API for a one-person client? (researched 2026-10-17)

Question from the owner (Round 1, Q2): "research how risky this is for a one person project, in depth."

## TL;DR

| Risk | Likelihood | Impact | Main mitigation |
|------|-----------|--------|-----------------|
| Account **ban** for normal Premium listening via private APIs | **Low.** No confirmed case found in ~10 years of librespot/spotifyd/ncspot/spotatui | High: account terminated, no refund, **playlists gone** | Premium only, metadata-only usage, official-client-like traffic, **local library backups** |
| Forced logout / password reset | Medium (happened in waves in 2024) | Low: log in again | Token-based auth (what librespot does now) |
| **Breakage** (protocol change, login5 outage) | **High.** Happens several times a year | Medium: browsing breaks for days | Catalogue behind an interface, Web API fallback, playback is independent (Soloist) |
| Legal action against a personal, non-distributed tool | Negligible | n/a | Don't distribute downloaders, don't publish under the Spotify name |

**The real risk is breakage, not bans.** The architecture should treat the private API as something
that *will* break periodically and degrade gracefully when it does.

## Evidence

### Bans that were reported, and what was behind them
- **Zotify #314 (Nov 2025):** an account was banned. The user was running a **downloader** (Zotify rips audio) on a
  self-described *burner* account, and suspected the credential-generation tool. Downloading audio is the behaviour
  Spotify actually polices. ([issue](https://github.com/zotify-dev/zotify/issues/314))
- **Jetispot (archived Feb 2025):** suspected "shadow ban" of the *app's* client identity, and
  suspicious-login emails. The app was blocked; users' accounts were not banned.
  ([repo](https://github.com/BobbyESP/Jetispot))
- **Spotifast (2026)** says it knows of no confirmed bans from normal Premium listening through
  librespot. This is a vendor claim, but consistent with everything else found.
- The HN thread (2026) that claims "Spotify is killing librespot" is about **breakage and
  lossless**, not bans. ([thread](https://news.ycombinator.com/item?id=49517870))

### Lockouts (not bans)
- **librespot discussion #1311 (Aug 2024–Feb 2025):** waves of forced password resets and
  "log in on every device again". These were fixed for users by switching to token-based auth.
  ([discussion](https://github.com/librespot-org/librespot/discussions/1311))

### Breakage
- **librespot #1771 (2026-09-29):** `login5` returned 503 and Connect would not start. Still open,
  and possibly a Spotify outage. ([issue](https://github.com/librespot-org/librespot/issues/1771))
- Steady small breaking changes (#1737), and some accounts/tracks failing (#1649).
- Unverified claim from a third-party client (Outify) that old stored credentials are revoked from
  2026-09-01.

### Terms
- Spotify's Terms of Use allow termination if they *believe* a breach occurred, with no refund
  ([terms](https://www.spotify.com/us/legal/end-user-agreement/)). librespot's own README says
  using it is "probably forbidden". Using the private API **is** a ToS breach; the open question is
  only enforcement.

## Why our design is lower-risk than a typical librespot client

1. **Audio comes from Soloist**, Spotify's own official engine. We never fetch audio keys or audio
   files through a reverse-engineered client, which is the activity tied to ripping and the one
   most likely to be policed.
2. Private-API use is **metadata only** (Home, search, library, playlists, lyrics), much like
   what open.spotify.com does in a browser.
3. A single user with human-paced traffic doesn't show up the way bulk crawlers do.

## Rules the code must follow (proposed for ADR)

- **Never download, export, or cache audio** outside Soloist's own cache.
- **No crawling.** Request only what the user is looking at; prefetch at most one page ahead.
  Aggressive local caching with TTLs.
- Back off on 429/5xx with jitter and **never retry in a tight loop** (the spotify_player failure
  mode).
- Present a stable, plausible client identity and use the token-based auth flow.
- **Library backup**: periodically export playlists, liked songs, and follows to local JSON, so
  even the worst case (termination) doesn't lose the library's contents.
- **Graceful degradation**: if the catalogue backend is down, playback, queue, and now-playing still work through
  Soloist, and the UI says "Browse is unavailable, Spotify changed something" instead of crashing.
  The Web API fallback covers library/playlists/search where it can.

## Bonus finding: the 90-day Soloist refresh can be fully automatic

Soloist builds are served from a **public CDN URL with no login**
(`https://soloist-builds.spotifycdn.com/soloist_release_x86_64.tar.gz`, the same URL the AUR
package uses). It returns `ETag` and `Last-Modified` (the latest build was dated 2026-10-16, so builds
are frequent). The app can check this on startup, download to the user's own machine (which is not
redistribution), verify it runs (`soloist --version`), atomically swap the binary, and restart the
daemon between tracks. The user never sees the 90-day limit.
