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

## Re-verified 2026-10-18 (web-player.0079e2e5, desktop bundle + 158 chunks)

Live (2026-10-18): a description written as `probe & <b>words</b>` reads back from `fetchPlaylist` as
`probe & &lt;b&gt;words&lt;&#x2F;b&gt;` (escaped, never markup); `noValue: [LIST_DESCRIPTION]` clears it.
`addToPlaylist` answers `{data:{addItemsToPlaylist:…}}`; `editablePlaylists` with `textFilter` = the name and
`uris` = the songs gives each matching playlist's `curates` (true once it has them). `fetchPlaylist` rows carry `uid`,
distinct per copy of a song; `removeFromPlaylist {playlistUri, uids}` answers `{data:{removeItemsFromPlaylist:…}}`
and takes only those rows. `moveItemsInPlaylist {playlistUri, uids, newPosition:{moveType:BEFORE_UID|AFTER_UID,
fromUid}}` answers `{data:{moveItemsInPlaylist:…}}`; both anchors verified live. Covers: the upload takes the bare
JPEG with just `authorization` and `Content-Type: image/jpeg` (→ `{uploadToken}`, ~2.5 s); `register-image` →
`{picture}` (a 28-char base64 id) that goes straight into `values.picture`; the new `image-cdn-fa.spotifycdn.com`
URL shows at once in `fetchPlaylist` and `libraryV3`; `noValue:[LIST_PICTURE]` takes it away.

All playlist routes above are still there. Details read from the bundle:

- `addToPlaylist`, `removeFromPlaylist`, `moveItemsInPlaylist` are mutations sharing one persisted hash. Variables:
  add `{playlistUri, playlistItemUris, newPosition, interactionId:null}`; move `{playlistUri, uids, newPosition}`;
  remove `{playlistUri, uids}`. `newPosition = {moveType, fromUid}`, `moveType` one of `TOP_OF_PLAYLIST`,
  `BOTTOM_OF_PLAYLIST`, `BEFORE_UID`, `AFTER_UID` (`fromUid` null for top/bottom).
- `editablePlaylists` (query): `{offset, limit, textFilter, folderUri, uris:[the items to add], features}` →
  `{items:[{item, pinned, curates}], totalCount}`.
- Create: `POST /playlist` (spclient), body a *delta* (not list changes):
  `{ops:[{kind:UPDATE_LIST_ATTRIBUTES, updateListAttributes:{newAttributes:{values:{name, formatAttributes:[],
  pictureSize:[]}, noValue:[]}}}], info:{source:{client:WEBPLAYER}}}` → `{uri}`; then the rootlist `add`.
- Rootlist changes `POST /user/{username}/rootlist/changes` with list changes `{deltas:[delta], wantResultingRevisions:
  false, wantSyncResult:false, nonces:[]}`: add `ADD {addFirst | addBeforeItem…, items:[{uri, attributes:{timestamp}}]}`;
  delete `REM {itemsAsKey:true, items:[{uri}]}` (no base revision needed for either).
- Details: `POST /playlist/{id}/changes`, `UPDATE_LIST_ATTRIBUTES {newAttributes:{values:{name?, description?,
  picture?}, noValue:[LIST_DESCRIPTION=2 | LIST_PICTURE=3]}}`. Op kinds: ADD 2, REM 3, MOV 4, UPDATE_LIST_ATTRIBUTES 6.
- Cover: `POST https://image-upload.spotify.com/v4/playlist` (no global headers, `Content-Type: image/jpeg`) →
  `{uploadToken}`; `POST /playlist/{id}/register-image {uploadToken}` → `{picture}` (bytes); then details above.
- Our librespot fork (`spotatui-librespot-core` 0.8.6) already has `post_playlist_changes` and
  `post_rootlist_changes` (protobuf `ListChanges`, sent once, no retry).
