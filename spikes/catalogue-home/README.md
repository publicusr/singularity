# Spike 2: private-API Home feed

Answers: *can Singularity log in to Spotify's private API and fetch your real Home feed (Made For You,
mixes, recently played) without a developer app or Web API quota?*

```sh
cd ~/src/singularity/spikes/catalogue-home
cargo run --release
```

1. It opens a Spotify page in your browser and prints a short code. Approve it (you log in on
   spotify.com; this program never sees your password).
2. It connects and prints your Home shelves with the first items of each.
3. Later runs reuse the cached login in `state/` (git-ignored; delete it to log out).

`FINDINGS.md` records only structure (shelf counts, item types), never your playlist names. The raw
answer is saved to `state/home.json` for building the real parser.

Technique: the login uses Spotify's desktop client id with the OAuth device flow, and the query is the
web player's persisted Pathfinder `home` query, run over a session from spotatui's maintained librespot
fork (`spotatui-librespot-core` 0.8.6, MIT). Query hashes rotate; the real app must refresh them
automatically from the web player bundle (that refresh is the next thing to design).
