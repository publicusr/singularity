# Playlist editing, covers, podcasts and audiobooks (researched 2026-10-18)

For the v1.x batches (playlist editing, podcasts). Read from the web player's public bundles
(`web-player.<hash>.js`, the operations list the QueryRegistry caches) and public reporting. Nothing here was sent to
Spotify yet: each path gets a quiet live check when its ticket is built. Re-verify after ~3 months (CLAUDE.md).

## Playlists

| What | How (web player) |
|------|------------------|
| Add tracks | Pathfinder `addToPlaylist` |
| Remove tracks | Pathfinder `removeFromPlaylist` |
| Reorder tracks | Pathfinder `moveItemsInPlaylist` |
| "Add to playlist…" picker | Pathfinder `editablePlaylists` (and `playlistPermissions`) |
| Create | spclient `POST /playlist`, then `POST /user/{username}/rootlist/changes` to put it in the library |
| Delete (Spotify's "delete" = remove from the library) | `POST /user/{username}/rootlist/changes` |
| Rename, description | `POST /playlist/{id}/changes` with `updateListAttributes` (`ListChanges` as JSON) |
| Cover image | 1. `POST https://image-upload.spotify.com/v4/playlist`, body the JPEG, `Content-Type: image/jpeg` → `{uploadToken}`; 2. `POST /playlist/{id}/register-image {uploadToken}` → `{picture}`; 3. `/playlist/{id}/changes` with `updateListAttributes` setting the picture |

The public Web API also documents cover upload (Base64 JPEG, at most 256 KB, scope `ugc-image-upload`), but the Web
API is the path the 2026 quota broke (spec, Problem Statement), so the private route above is the one to use; keep
images under 256 KB anyway (re-encode, square crop).

## Podcasts and audiobooks

- Pathfinder: `queryShowMetadataV2`, `queryPodcastEpisodes`, `getEpisodeOrChapter`, `showItemsPlayedState` (played
  marks), `decorateContextEpisodesOrChapters`, `searchPodcasts`, `searchEpisodes`, `searchFullEpisodes`.
- Audiobooks: `getAudiobooksMetadata`, `queryBookChapters`, `searchAudiobooks`, `similarAudiobooks`.
- Audiobooks launched in Poland on 2026-10-15 with 12 h a month included in Premium (spidersweb.pl, TheNextWeb;
  15 h in the US/UK markets). Whether Soloist plays `spotify:chapter:` URIs is unverified.
- Video podcasts: their audio is the episode; video in a terminal is out (spec: Canvas/video out of scope).
- Downloads: impossible (DRM; DESIGN-TREE "Known impossibilities").
