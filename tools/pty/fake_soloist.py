"""A stand-in Soloist for the pty harness (`update_test.py`, `phone_test.py`): enough of the WebSocket protocol for
the SoloistEngine (auth, playback state, play/pause/next/seek/volume, `--initial-volume`), plus `--version` and the
"client expires in N days" log line.

`update_test.py` copies this into builds with VERSION and DAYS filled in and a shebang to the harness's venv. Every
start, stop and command is appended to $FAKE_SOLOIST_EVENTS, so the test can tell when a build ran and what the Daemon
told it.

Two things stand in for a phone (or any other Spotify app on Connect), for `phone_test.py`:

- a file `remote-play-N` in the data dir: the phone plays track N on this device (the events the real Soloist emits
  for that, and the command log stays empty: the Daemon didn't do it); `remote-queue-N`: the phone queues track N and
  skips to it.
- `FAKE_SOLOIST_STATE_AT_LOGIN=N`: the build comes back already playing track N, as when Spotify hands a restarted
  device the track another app started.
"""
import asyncio, json, os, sys, time

VERSION = "@VERSION@"
DAYS = "@DAYS@"
EVENTS = os.environ.get("FAKE_SOLOIST_EVENTS", "/dev/null")


def event(what):
    with open(EVENTS, "a") as f:
        f.write(f"{time.time():.3f} {VERSION} pid={os.getpid()} {what}\n")


def track(n):
    uri = f"spotify:track:fake{n}"
    return {"uri": uri, "decorations": {
        "identity": {"name": f"Fake Track {n + 1:03}"},
        "creators": [{"entity": {"uri": "spotify:artist:fake0", "decorations": {"identity": {"name": "Nova Drift"}}}}],
        "parent": {"entity": {"uri": "spotify:album:fake1", "decorations": {"identity": {"name": "Album 1"}}}},
        "playback": {"duration_ms": 200000}}}


class Player:
    def __init__(self, data="/dev/null"):
        self.n, self.status, self.pos, self.ts, self.volume, self.loaded = 0, "idle", 0, 0, 40, False
        self.data = data
        # FAKE_SOLOIST_STATE_AT_LOGIN: what another app (a phone) put there before this build came back
        if os.environ.get("FAKE_SOLOIST_STATE_AT_LOGIN"):
            self.n, self.status, self.loaded = int(os.environ["FAKE_SOLOIST_STATE_AT_LOGIN"]), "playing", True

    def position(self):
        now = int(time.time() * 1000)
        p = self.pos + (now - self.ts if self.status == "playing" else 0)
        return {"position_ms": p, "timestamp_ms": now, "speed": 1.0 if self.status == "playing" else 0.0}

    def anchor(self, pos=None):
        cur = self.position()["position_ms"]
        self.pos, self.ts = (cur if pos is None else pos), int(time.time() * 1000)

    def state(self):
        return {"type": "playback_state", "status": self.status, "item": track(self.n) if self.loaded else {},
                "context": {"uri": "spotify:collection"}, "position": self.position(), "volume": self.volume,
                "options": {"shuffle": False, "repeat": "off"}}

    def queue(self, n):
        return {"type": "queue_changed", "previous": [], "upcoming": [
            {"uid": f"phone{n}", "source": "queue", "item": track(n)}]}


async def main():
    import websockets
    data = sys.argv[sys.argv.index("--data-dir") + 1]
    os.makedirs(data, exist_ok=True)
    with open(f"{data}/soloist.pid", "w") as f:
        f.write(str(os.getpid()))
    with open(f"{data}/soloist.args", "w") as f:  # what the Daemon passed (the key left out)
        a = sys.argv[1:]
        f.write(" ".join("<key>" if i and a[i - 1] == "--api-key" else x for i, x in enumerate(a)))
    p = Player(data)
    if "--initial-volume" in sys.argv:
        p.volume = int(sys.argv[sys.argv.index("--initial-volume") + 1])
    sessions = set()

    async def phone():
        """Another app on Connect, driving this device: a file in the data dir says what it did."""
        while True:
            await asyncio.sleep(0.05)
            try:
                acts = sorted(a for a in os.listdir(p.data) if a.startswith("remote-"))
            except OSError:
                continue
            for a in acts:
                try:
                    os.remove(os.path.join(p.data, a))
                except OSError:
                    continue
                kind, _, arg = a.partition("-")[2].partition("-")
                if not arg.isdigit():
                    continue
                n = int(arg)
                event(f"phone {kind} {n}")
                if kind == "queue":
                    for ws in list(sessions):
                        await ws.send(json.dumps(p.queue(n)))
                # both end with the phone's track playing (a queue skip included)
                p.n, p.loaded, p.status = n, True, "playing"
                p.anchor(0)
                for ws in list(sessions):
                    await ws.send(json.dumps({"type": "playback_changed", "status": "playing"}))
                    await ws.send(json.dumps({"type": "track_changed", "item": track(n)}))
                    await ws.send(json.dumps(p.state()))

    async def session(ws):
        sessions.add(ws)
        try:
            await ws.send(json.dumps({"type": "auth_state", "logged_in": True, "is_active": True}))
            await ws.send(json.dumps(p.state()))
            async for raw in ws:
                c = json.loads(raw)
                cmd = c.get("command")
                event(f"cmd {raw}")
                if cmd == "play":
                    uri = c.get("uri")
                    if uri:
                        p.n = int(uri.rsplit("fake", 1)[1]) if uri.startswith("spotify:track:fake") else 0
                        p.loaded = True
                        p.anchor(0)
                    else:
                        p.anchor()
                    p.status = "playing"
                    await ws.send(json.dumps(p.state()))
                elif cmd == "pause":
                    p.anchor()
                    p.status = "paused"
                    await ws.send(json.dumps({"type": "playback_changed", "status": "paused"}))
                    await ws.send(json.dumps({"type": "position_sync", "position": p.position()}))
                elif cmd in ("skip_next", "skip_prev"):
                    p.n = max(0, p.n + (1 if cmd == "skip_next" else -1))
                    p.anchor(0)
                    await ws.send(json.dumps({"type": "track_changed", "item": track(p.n)}))
                    await ws.send(json.dumps({"type": "position_sync", "position": p.position()}))
                elif cmd == "seek":
                    p.anchor(c.get("position_ms", 0))
                    await ws.send(json.dumps({"type": "position_sync", "position": p.position()}))
                elif cmd == "set_volume":
                    p.volume = c.get("volume", p.volume)
                    event(f"volume={p.volume}")
                    await ws.send(json.dumps({"type": "volume_changed", "volume": p.volume}))
                elif cmd in ("get_state", "get_queue"):
                    await ws.send(json.dumps(p.state()))
        finally:
            sessions.discard(ws)

    server = await websockets.serve(session, "127.0.0.1", 0)
    port = server.sockets[0].getsockname()[1]
    with open(f"{data}/ws.addr", "w") as f:
        f.write("127.0.0.1")
    with open(f"{data}/ws.port", "w") as f:
        f.write(str(port))
    print(f"\x1b[0m00:00:00.000 I [f:128] client expires in {DAYS} days\x1b[0m", flush=True)
    event(f"started volume={p.volume}")
    asyncio.create_task(phone())
    try:
        await asyncio.Future()
    finally:
        event("stopped")


if __name__ == "__main__":
    if "--version" in sys.argv:
        print(f"soloist {VERSION}")
        sys.exit(0)
    import signal
    signal.signal(signal.SIGTERM, lambda *_: (event("stopped"), sys.exit(0)))
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        pass
