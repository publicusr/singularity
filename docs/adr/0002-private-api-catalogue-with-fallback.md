---
status: accepted (validated by spike 2, 2026-10-17: real Home feed, 31 shelves in ~1.5 s)
---

# The catalogue comes from Spotify's private APIs, with the Web API as fallback

Home, Made For You, search, library, entity pages, playlist edits, and lyrics come from Spotify's
internal APIs (spclient / pathfinder) reached with a librespot-core session. The official Web API
can't provide Home or recommendations at all, and under 2026 Development Mode quotas it
rate-limits even a single user. This breaches Spotify's ToS. The owner accepted that for a
personal tool after reviewing the risk (`docs/research/2026-10-private-api-risk.md`): bans are
low-likelihood, but breakage is expected. The catalogue therefore sits behind one interface with an
official Web API implementation as fallback. Usage is metadata only, never crawls, backs off on errors,
and the library is backed up locally.

## Consequences

- Browsing can break without notice. Playback must keep working (Soloist is independent), and the
  UI must degrade with a clear message instead of failing.
- Publishing the project later needs a second look, because shipping a ToS-breaching client
  publicly carries more risk than using one privately.
