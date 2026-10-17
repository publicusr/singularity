# Spike 2 findings

Appended automatically by each run. Structure only, no personal content.

## Run

- ✅ Device-code login + librespot session (fork 0.8.6): connected in 1.18462762s, account type `?`
- ✅ `home` query (hash `5fb7da7a…`) answered in 1.489695938s: 31 shelves
- Shelf shapes: HomeShortsSectionData×10, HomeGenericSectionData×8, HomeGenericSectionData×10, HomeGenericSectionData×10, HomeGenericSectionData×10, HomeGenericSectionData×10, HomeGenericSectionData×9, HomeGenericSectionData×8, HomeGenericSectionData×9, HomeGenericSectionData×7, HomeGenericSectionData×10, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1
- Item types seen: {"Album": 25, "Artist": 3, "Playlist": 58}

## Run 2026-10-17 (owner's account)

- ✅ Device-code login → approved at spotify.com/pair → librespot session (fork 0.8.6) connected in ~1.2 s.
- ✅ Pathfinder `home` (hash `5fb7da7a…`) answered in ~1.5 s with **31 shelves**: 1 untitled shortcuts grid
  (`HomeShortsSectionData`, 10 items), 10 regular shelves (`HomeGenericSectionData`, 7–10 items: Made For You, Recommended
  Stations, Recently played, New releases for you, Your top mixes, genre/"for fans of" shelves, Jump back in), then 20
  single-item `HomeFeedBaselineSectionData` entries (the endless feed tail).
- Item types: Album, Playlist, Artist. Daily Mixes, DJ, radios, "track IDs" playlists and the daylist all present.
- The country and account-type attributes were empty because they were read too early (fixed: wait for them).
- **Conclusion: ADR-0002 holds.** The real Home feed works without any developer app or Web API quota.

## Run

- ✅ Device-code login + librespot session (fork 0.8.6): connected in 580.953601ms, account type `premium`
- ✅ `home` query (hash `5fb7da7a…`) answered in 1.605798006s: 31 shelves
- Shelf shapes: HomeShortsSectionData×10, HomeGenericSectionData×8, HomeGenericSectionData×10, HomeGenericSectionData×10, HomeGenericSectionData×10, HomeGenericSectionData×10, HomeGenericSectionData×9, HomeGenericSectionData×8, HomeGenericSectionData×7, HomeGenericSectionData×10, HomeGenericSectionData×6, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1, HomeFeedBaselineSectionData×1
- Item types seen: {"Album": 27, "Artist": 3, "Playlist": 56}
