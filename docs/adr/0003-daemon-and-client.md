# Two processes: a Daemon that owns playback, and Clients that draw it

Singularity is split into `singularityd`, the Daemon, and `singularity`, the terminal Client. The
Daemon is a user service (systemd user unit on Linux, LaunchAgent on macOS). It supervises the Engine,
holds the Catalogue session and caches, serves MPRIS, keeps the listening log and library backups, runs
the audio tap, and auto-updates Soloist. Clients connect over a local socket. We chose this so music
keeps playing when the terminal closes, the Client starts instantly, and several Clients (for example a
visuals-only one on a second monitor) can share one session. The cost is a local protocol and versioning
between the two, and more moving parts than a single binary.

## Consequences

- The Daemon↔Client protocol must be versioned. An app update can restart the Daemon underneath a
  running Client, and the Client must reconnect gracefully.
- Heavy visual rendering lives in the Client (it's per-window). Audio analysis lives in the Daemon
  (one tap, many subscribers).
