# Library queries and play-at-offset (verified live, 2026-10-17)

Found while building #9. Re-verify after ~3 months (CLAUDE.md).

## Pathfinder library queries

Variables come from the web player's main bundle (`web-player.<hash>.js`), where each persisted query is called.
Shapes were read with `cargo run -p singularityd --example probe -- <op> '<vars>'` (it prints structure only).

| Operation | Variables | Answer path | Notes |
|-----------|-----------|-------------|-------|
| `libraryV3` | `filters, order, textFilter, features[LIKED_SONGS, YOUR_EPISODES_V2, PRERELEASES, PRERELEASES_V2, EVENTS], limit (50), offset, flatten:false, expandedFolders, folderUri, includeFoldersWhenFlattening` | `data.me.libraryV3.items[].item.data` | `__typename` Playlist / Album / Artist / PseudoPlaylist (`spotify:collection` = Liked Songs, with `count`); folders are fetched with `folderUri` |
| `fetchLibraryTracks` | `offset, limit` | `data.me.library.tracks` | items `{addedAt, track{_uri, data}}`; `data.duration.totalMilliseconds` |
| `fetchPlaylist` | `uri, offset, limit, enableWatchFeedEntrypoint, includeEpisodeContentRatingsV2` | `data.playlistV2.content` | items `{addedAt, uid, itemV2{data}}`; `data.trackDuration.totalMilliseconds`; episodes mixed in |
| `getAlbum` | `uri, locale, offset, limit` | `data.albumUnion.tracksV2` | items `{uid, track{uri,…}}`; `queryAlbumTracks` has the same hash |

Timing: ~0.9–2.2 s per request. Lists page at 50 (library) and 100 (tracks), sequentially.

## Starting a context at track N

- **Soloist's WebSocket `play` ignores offsets.** Tried `skip_to.track_index`, `skip_to.track_uri`, `offset`, `index`,
  `track_uri`, `options.skip_to`: all start the context from the top. `play <track> + context_uri` plays the track
  alone (the context becomes the track).
- **Spotify Connect works.** With the librespot session: start the dealer, take `Spotify-Connection-Id` from
  `hm://pusher/v1/connections/`, register a device with `capabilities.hidden = true` (`PUT connect-state/v1/devices/<id>`,
  whose answer is the cluster with every device), then `send_player_command(<soloist id>, {endpoint: "play", context:{uri,
  url:"context://<uri>"}, options:{skip_to:{track_index}}})`. Verified for an album (index 4, 7, 11), and for Liked
  Songs as `spotify:user:<username>:collection`. The hidden controller doesn't show in the device list.
- Soloist's Connect device id is the UUID it logs as `device_id=` (also in the cluster, by its device name).

## Side note

Soloist is started with `--api-key <key>` on its command line, so the key shows in `ps` for any local user. Worth
fixing in #5 (keyring) if Soloist accepts the key another way.

## Editing the queue (verified live, 2026-10-18)

Soloist's `add_to_queue` appends. Everything else is a Connect `set_queue` command: take `player_state.next_tracks`
(and `prev_tracks`, `queue_revision`) from the cluster (the answer to the hidden controller's `PUT connect-state`),
edit the list, send it back: `{endpoint: "set_queue", next_tracks, prev_tracks, queue_revision}`. User-queued entries
have `provider: "queue"`; a new entry `{uri, uid: "", metadata: {is_queued: "true"}, provider: "queue"}` inserted at the
front plays next (Spotify assigns its uid). `getTrack {uri}` → `data.trackUnion` fills names for bare queue entries.

## Devices and transfer (verified live, 2026-10-18)

- Cluster pushes: dealer URI `hm://connect-state/v1/cluster`, protobuf `ClusterUpdate` (`Message::from_raw`); our
  hidden controller (`is_observable`) receives them.
- `POST /connect-state/v1/connect/transfer/from/<playing>/to/<target>` answered **500** from this session, with or without
  JSON headers and the connection id. Moving playback with `play` on the target (`context`, `options.skip_to.track_uri`,
  `seek_to`, `initially_paused`) works, and the previous device goes inactive by itself.
- Remote transport: `pause`, `resume`, `skip_next`, `skip_prev`, `seek_to {value}`, `set_shuffling_context {value}`,
  `set_options {repeating_track, repeating_context}`, volume via `connect/volume` (0–65535). A `pause` on an already
  paused player answers 403.
- Quality (D17): Soloist has `supports_remote_audio_quality_control` as a Connect feature flag; no setter found.
