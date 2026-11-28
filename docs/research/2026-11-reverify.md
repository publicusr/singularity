# Re-verifying the Spotify research before v1.0 (researched 2026-11-28)

Why: CLAUDE.md asks for facts in `docs/research/` older than ~3 months to be re-checked before building on them. v1.0
goes public with a YouTube channel (D90), so the October notes are checked here against primary sources: Spotify's
developer docs, blog, terms and support pages, Spotify's `soloist` repo, librespot's GitHub, and LRCLIB's own API
docs. Nothing was sent to Spotify's private APIs; the only live requests were anonymous `HEAD`s to Soloist's public
CDN and one LRCLIB lookup of a well-known song.

## TL;DR

| Area | What we said (Oct 2026) | Now (2026-11-28) | Action |
|---|---|---|---|
| **Soloist platforms** | Linux only (x86_64, aarch64, armv7) | **Also Apple Silicon macOS 13+ and x86_64 Windows**, since 2026-10-23 | README, PLATFORMS.md, ADR-0001 and the macOS VM design are out of date: owner decision |
| Soloist and Free accounts | Premium needed (implied for everyone) | The **key** needs Premium; once running, **Free accounts can connect** too. Lossless and crossfade stay Premium | README's "needs Premium" is still right for Singularity as a whole (librespot and remote Jam need it) |
| Soloist key | per user, CLI flag only | unchanged (`--api-key`, no env var or file documented) | none |
| Soloist builds | 90-day expiry, exit 10, no redistribution | unchanged; CDN builds dated 2026-11-27 | none |
| Soloist WebSocket API | player only, no offsets, no queue edits | unchanged | none |
| Web API | Feb 2026 limits, Jul 2026 shared quota | unchanged since July; **new: refresh tokens of registered apps expire after 6 months** (from 2026-07-20) | doesn't touch Singularity (no Web API calls, no registered app) |
| Terms | private API use is against the ToS | Terms of Use updated **2026-09-04**; the prohibitions sit in the **User Guidelines** (reverse engineering, circumventing technology, automated means) | README stays true; cite the User Guidelines |
| librespot | breakage, no lossless | latest release still **v0.8.0 (2025-11-10)**; **audio-key refusals** for some accounts still open (#1649), a back-off fix is an open PR (#1844); login5 503 of 2026-09-29 was a Spotify outage | macOS default (librespot) may fail on some newer accounts |
| Jam | Premium hosts; Free joins in person | unchanged | none |
| LRCLIB | free, open, no key | still free and keyless, but **stricter limits for unidentified clients** and **word timing now lives in a `lyricsfile` YAML field** | our word-sync fallback may miss words: owner decision |
| Brand rules | no Spotify name/logo | confirmed: no "Spotify" in the name, "for Spotify" allowed, no look-alike logo, no implied endorsement | rules for the YouTube channel below |

## 1. Files in `docs/research/` and what relies on them

| File | Date | Spotify facts others rely on | Re-checked here |
|---|---|---|---|
| `2026-10-platform-constraints.md` | 2026-10-17 | Web API limits; librespot no lossless; Soloist facts (platforms, key, pairing, 90 days, WebSocket); private APIs; LRCLIB | yes, §2–§6 |
| `2026-10-private-api-risk.md` | 2026-10-17 | ToS position (README "Not affiliated"); ban vs breakage risk; Soloist CDN auto-update | yes, §3, §4 |
| `2026-10-library-and-connect.md` | 2026-10-17/18 | Pathfinder query names; Soloist ignores `play` offsets; Connect `skip_to`, `set_queue`, transfer | WebSocket half yes (§2); private-API half can't be checked from public sources (§7) |
| `2026-10-playlists-and-podcasts.md` | 2026-10-18–20 | playlist edits, covers, podcasts, audiobooks over private API; librespot can't play chapters | librespot half yes (§4); private half §7 |
| `2026-10-jam.md` | 2026-10-25 | who can host/join; `social-connect` endpoints | support facts yes (§5); endpoints §7 |
| `2026-10-macos.md` | 2026-10-17–20 | "Soloist is Linux-only"; lossless on desktop app; librespot 320 kbps | yes, §2 (**changed**) |
| `2026-10-soloist-vm-spike.md` | 2026-10-19/20 | Soloist arm64 Linux build; pairing only from a phone | yes, §2 (the VM's reason has changed) |
| `2026-10-free-tiling.md` | 2026-10-17 | none (ratatui crates) | not needed |
| `2026-10-history-scrub.md` | 2026-10 | none (git history plan) | not needed |
| `2026-10-audit*.md` (4 files) | 2026-10 | code audits, no platform facts | not needed |

What the code relies on, for orientation: the Catalogue logs in with librespot's reusable credentials and the desktop
client id (`crates/daemon/src/catalogue.rs`), uses Pathfinder/spclient only (no `api.spotify.com/v1` calls anywhere in
`crates/`), downloads Soloist from `soloist-builds.spotifycdn.com` (`crates/daemon/src/update.rs`, builds named in
`crates/config/src/host.rs`), and asks LRCLIB's `/api/get` once per track (`crates/daemon/src/lyrics.rs`).

## 2. Soloist

**Changed: it runs natively on macOS and Windows.** Spotify's repo README (commit "README: add macOS and Windows
(#19)", 2026-10-23) now says: "a lightweight, command-line Spotify Connect receiver for Linux, macOS, and Windows",
requirements "An Apple Silicon Mac running macOS 13 or newer" and "An x86\_64 Windows PC"
([spotify/soloist](https://github.com/spotify/soloist)). The launch post carries an "Update (October 2026)": "Spotify
Soloist is now available on desktop (macOS and Windows)"
([blog](https://developer.spotify.com/blog/2026-08-13-introducing-spotify-soloist)). The docs' overview says "It is
available for Linux, Apple Silicon Mac, and x86\_64 Windows devices", with audio through Core Audio on macOS
([overview](https://developer.spotify.com/documentation/soloist/concepts/overview)). The Getting Started page names the
builds: `soloist_release_osx_arm64.tar.gz` and `soloist_release_windows_x86_64.zip`
([getting started](https://developer.spotify.com/documentation/soloist/tutorials/getting-started)). Checked live
2026-11-28 (anonymous `HEAD`): `osx_arm64`, `x86_64`, `arm64`, `arm32` and the Windows zip all answer 200, last
modified 2026-11-27. Intel Macs: no build.

Consequences (facts, not decisions):

- README "macOS quirks" ("Soloist … is Linux-only, so on a Mac the default is librespot"), `docs/PLATFORMS.md`'s
  Soloist row ("Spotify ships it for Linux only"), and ADR-0001's "Linux only … is still true of Soloist itself" are
  no longer true.
- The Soloist VM (D53/D55) was built only because Soloist had no macOS build. A native build would play lossless on
  a Mac without the VM's ~300 MB and without the pairing relay. Its audio goes to the default Core Audio output, not
  a PipeWire node, so on macOS the audio Widgets would need the process tap (which hands over silence without the
  "System Audio Recording Only" permission, `2026-10-macos.md`), unlike the VM's PCM stream.
- `host::soloist_build` maps only `x86_64` and `aarch64`. The Linux armv7 build is named `arm32` on the CDN (it was
  called armv7 in our notes).

**Changed: Free accounts can listen.** "A Premium account is needed to set up Spotify Soloist, but once it's running
both Free and Premium users can connect. Premium unlocks lossless audio and mixing" (repo README). Jam, Connect
control, normalisation, Smart Shuffle and Autoplay are listed for all users (blog).

**Unchanged:**

- Key: "Each Spotify Soloist user must generate their own key"; "The account that generates the key must have
  Spotify Premium"; passed with `--api-key` at every startup, kept "in a private local environment file or secret
  manager"; no other way to pass it is documented
  ([authentication](https://developer.spotify.com/documentation/soloist/concepts/authentication)). So the README's
  `ps` warning stands.
- Login only by selecting the device from a Spotify app on the same network; `--pair` re-pairs (same page).
- "Spotify Soloist builds expire 90 days after their build date", exit code `10`; "Do not redistribute Spotify Soloist
  archives or binaries directly" (getting started). Downloading on the user's own machine, as the Daemon does, is
  still what the docs describe.
- WebSocket API: same commands and events as in our notes, `play` takes only a `uri`, no offset, no queue editing,
  `add_to_queue` tracks only; endpoint in `ws.addr`/`ws.port`
  ([WebSocket API](https://developer.spotify.com/documentation/soloist/reference/websocket-api)). So the Connect
  workarounds in `2026-10-library-and-connect.md` are still needed.
- Lossless up to 24-bit/44.1 kHz, Premium (repo README; [support](https://support.spotify.com/us/article/lossless-audio-quality/)).

## 3. Web API

No change since July. The February migration guide still lists: owner must have Premium, 5 users and (since July) 25
client IDs per developer, batch and browse endpoints removed, search `limit` max 10, library moved to `/me/library`;
`external_ids` restored in March
([migration guide](https://developer.spotify.com/documentation/web-api/tutorials/february-2026-migration-guide),
[July 2026 changelog](https://developer.spotify.com/documentation/web-api/references/changes/july-2026),
[quota post](https://developer.spotify.com/blog/2026-07-23-web-api-quota-updates)). The developer blog's newest posts
are Soloist (2026-08-13), quota (2026-07-23), refresh-token expiration (2026-06-18), access and security (2026-02-06)
([community page](https://developer.spotify.com/community)).

**New since our notes: refresh-token expiry.** Refresh tokens "issued to apps registered on Spotify for Developers"
(Authorization Code and PKCE flows) last 6 months from the original authorisation, enforced for existing apps from
2026-07-20; afterwards the token endpoint answers `400 invalid_grant`
([post](https://developer.spotify.com/blog/2026-06-18-refresh-token-expiration)). Singularity isn't a registered app
and keeps librespot's reusable credentials, not a Web API refresh token, so this doesn't apply today; the post doesn't
say whether Spotify's own clients will follow.

SPEC still names an *OfficialCatalogue* (Web API fallback); nothing in `crates/` calls the Web API, and with the
limits above it couldn't serve a public user base anyway.

## 4. Terms, risk and librespot

**Terms.** The Terms of Use were last updated **2026-09-04**. Section 3: "not to use the Spotify Service, the Content,
or any part thereof in any manner not expressly permitted by the Agreements"; Section 6: Spotify may "suspend your
access to the Spotify Service at any time if we believe you have breached any of these Terms", with no refund
([terms](https://www.spotify.com/us/legal/end-user-agreement/)). The explicit prohibitions are in the User
Guidelines: "reverse-engineering, decompiling, disassembling, modifying, or creating derivative works",
"circumventing any technology used by Spotify", "using any automated means (including bots, scrapers, and spiders)"
([guidelines](https://www.spotify.com/us/legal/user-guidelines/)). The Developer Terms (v10, 2025-05-15) forbid
"reverse-engineering, or extracting source code from the Spotify Platform" and using Spotify Marks in a service's name
([developer terms](https://developer.spotify.com/terms)). **The README's statement ("against Spotify's Terms of Use",
"could also restrict or close an account") is still accurate.** Citing the User Guidelines next to it would be more
precise.

**Bans.** No new reports of accounts closed for listening through librespot-based clients were found in
librespot's or spotatui's trackers since our October note. The risk picture of `2026-10-private-api-risk.md` stands:
breakage is the real risk.

**librespot.** Latest release **v0.8.0, 2025-11-10**; the repo is active (last push 2026-11-07)
([releases](https://github.com/librespot-org/librespot/releases)). Relevant open items:

- **Audio-key refusals** (`AesKeyError`) for some accounts, mostly newer ones, open since 2025-11
  ([#1649](https://github.com/librespot-org/librespot/issues/1649)). A PR describes them as "an undisclosed rate limit
  on the key endpoint" triggered by bursts such as skipping quickly, and backs off 3/5/10 s
  ([#1844](https://github.com/librespot-org/librespot/pull/1844), open); another fails fast on a refused key
  ([#1869](https://github.com/librespot-org/librespot/pull/1869), open). This affects the macOS default Engine.
- **login5 503** of 2026-09-29 ([#1771](https://github.com/librespot-org/librespot/issues/1771)): commenters linked a
  Spotify status post and reported recovery the same day; an outage, not a policy change. Still open.
- Current Spotify apps ask ZeroConf version 2.12.0 and send an empty blob to librespot's 2.9.0
  ([#1847](https://github.com/librespot-org/librespot/pull/1847), open). Our librespot Engine logs in with the
  Catalogue's credentials, not ZeroConf, so this matters only if pairing from a phone is ever used for it.
- librespot stays Premium-only ([#1855](https://github.com/librespot-org/librespot/pull/1855) keeps it so).
- Lossless: still closed as not planned ([#1580](https://github.com/librespot-org/librespot/issues/1580)).
- Jam is documented in an open PR, not merged ([#1804](https://github.com/librespot-org/librespot/pull/1804)).
- Our fork crate `spotatui-librespot-*` is pinned at `=0.8.6`; 0.8.8 was published 2026-11-25 (crates.io). spotatui
  (v0.49.0, 2026-11-25) still runs on the private API without a client id.

## 5. Connect and Jam

Spotify Support (no date on the page): "Premium is needed to start and host a Jam"; Free users can join in person
only; invites by link, QR, Bluetooth within ~1 m, or same Wi-Fi; when the host leaves the Jam ends for everyone; Jam
works on smart speakers and TVs ([support](https://support.spotify.com/us/article/jam/)). Same as
`2026-10-jam.md`. Lossless on third-party Connect devices is set from the phone: device icon → the device → "Change
quality settings" → Lossless ([support](https://support.spotify.com/us/article/lossless-audio-quality/)), as the README
says.

## 6. Lyrics: LRCLIB

LRCLIB's API docs ([docs.md](https://lrclib.net/docs.md)) say it's still free, keyless and open, with two changes:

- **Rate limits by identification**: ~45 requests/min per IP for generic clients, ~90 for a `User-Agent` naming the
  app, more for registered apps (`Lrclib-App-Id`, optional). Clients "**must** honor `Retry-After`" on 429, with
  jitter, or risk a temporary ban. Singularity sends a named `User-Agent` and asks once per track with no retry loop
  (`lyrics.rs`), well within this; it treats a 429 as "couldn't be asked" rather than reading `Retry-After`.
- **Word timing moved**: each record now has `hasWordSync` and a `lyricsfile` (YAML) whose `lines[].words` hold word
  times. Checked on one record (2026-11-28): `hasWordSync: true`, but `syncedLyrics` has line stamps only, no
  `<mm:ss.xx>` word stamps. Our Karaoke fallback (D67) reads word times from enhanced LRC in `syncedLyrics`
  (`lyrics::parse_enhanced`), so for records like this it gets lines only.
- The source repo is MIT and active ([tranxuanthang/lrclib](https://github.com/tranxuanthang/lrclib)). LRCLIB's docs
  don't speak to the lyrics' own copyright; the texts belong to their rights holders.

Spotify's own lyrics (spclient `color-lyrics`) are private API (§7).

## 7. What public sources can't confirm

Pathfinder operation names and hashes, `color-lyrics`, connect-state commands, playlist and cover routes,
`social-connect` (Jam) endpoints: these are private and undocumented, so no primary source exists. They were verified
live in October; the evidence that they still work is the daily use of the app and spotatui's active development on
the same routes. They should get the usual quiet live check (the probe example) before the release, not a public
source check.

## 8. Brand rules (for the app and a YouTube channel)

From Spotify's Design & Branding Guidelines ([design](https://developer.spotify.com/documentation/design)) and the
Developer Terms:

- "The app name should not include 'Spotify' or be similar to 'Spotify' in sound or spelling." "For Spotify" as a
  description is acceptable, as long as nothing implies endorsement.
- The app's logo must not include or look like Spotify's logo, Spotify Green, the circle or the waves.
- Don't imply Spotify endorses the app; don't pair the Spotify brand with another brand. Developer Terms §IX: no
  suggestion of "a partnership with, sponsorship by, or endorsement by Spotify" without written approval; Spotify
  Marks may not be part of a service's name, and no trademark or domain name with "Spotify" in it.
- The guidelines are written for Web API/SDK apps: they ask apps that show Spotify metadata to attribute it with the
  Spotify logo and to show artwork uncropped, unblurred and unanimated. Singularity isn't a registered app and the
  README chose "no Spotify name or logo" (D65); the cover glow and animated visuals over artwork wouldn't meet those
  display rules if they applied.
- The guidelines say nothing about videos or marketing. For a channel: a name and thumbnails without "Spotify" in the
  title of the channel, no Spotify logo or green, and a line saying the project is independent are what the rules
  above point to. Saying in a video that it plays Spotify ("a terminal client for Spotify") is descriptive use, which
  "for Spotify" covers.
