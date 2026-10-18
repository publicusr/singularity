# Roadmap (tentative)

Phases follow the v1 slice settled in D13 (phases 1–4 are v1; 5–6 are v1.x). Each phase ends with something
usable, not just infrastructure.

| Phase | Goal | Exit criterion |
|-------|------|----------------|
| 0. Plan ✅ | Grilling → final spec → tickets | Design-tree frontier empty, owner confirms (done 2026-10-17, except D17 which needs a spike) |
| 1. Spike ✅ | Prove the risky integrations | Throwaway programs: (a) start, pair and control Soloist over WebSocket, and find out which settings it exposes (D17); (b) fetch the Home feed from the private Catalogue; (c) tap the Soloist PipeWire node and print an FFT. Already proven in the prototype: GPU black hole via wgpu, kitty shared-memory transfer |
| 2. Walking skeleton 🔄 | Daemon + Client, layout, sidebar, now-playing bar, mouse + palette, one theme | Play a Liked Songs track with transport controls by mouse and keyboard; music survives closing the Client |
| 3. Core parity | Home, search, library, entity pages, queue, devices | Daily-drivable replacement for the desktop app |
| 4. Eye candy | Visual engine, black hole, spectrum, album art, lyrics, layout presets | LARP preset works and stays within its CPU budget |
| 5. Personalisation | Theme gallery + picker, album-adaptive theme, settings screen, keymap config | Everything configurable in-app and in files |
| 6. Polish | Stats widgets, notifications, MPRIS, idle mode, mini-player, health check, podcasts | Owner stops opening the official app |
| 7. macOS | Engine = hidden Spotify app over Connect, Core Audio tap, Keychain, LaunchAgent | Same daily-driver bar on the M2 Air |

Phase 1 comes before any UI work on purpose. If the Soloist or catalogue spike fails, the whole
architecture changes, and it is better to learn that in a day than after a month of UI.

**Progress (2026-10-18):** v1 tickets are GitHub issues under spec issue #1. Done: #2 Daemon↔Client, #3 Client shell,
#4 Soloist playback, #5 setup & health, #7 Catalogue session, #8 Home, #9 Library, #10 search, #11 album/artist pages,
#12 queue, #13 devices, #14 lyrics, #15 AudioTap, #16 black hole, #24 seek precision. In progress: #22 like/follow.
Next: #21 MPRIS, #6 auto-update, #20 config, #18 designs, #19 layouts, #17 idle, #23 local data. See `docs/HANDOFF.md`.
