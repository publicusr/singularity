"""A stand-in Soloist for the update test (`update_test.py`): enough of the WebSocket protocol for the SoloistEngine
(auth, playback state, play/pause/next/seek/volume, `--initial-volume`), plus `--version` and the "client expires in N
days" log line.

`update_test.py` copies this into builds with VERSION and DAYS filled in and a shebang to the harness's venv. Every
start and stop is appended to $FAKE_SOLOIST_EVENTS, so the test can tell when a build ran."""
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
    def __init__(self):
        self.n, self.status, self.pos, self.ts, self.volume, self.loaded = 0, "idle", 0, 0, 40, False

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


async def main():
    import websockets
    data = sys.argv[sys.argv.index("--data-dir") + 1]
    os.makedirs(data, exist_ok=True)
    with open(f"{data}/soloist.pid", "w") as f:
        f.write(str(os.getpid()))
    p = Player()
    if "--initial-volume" in sys.argv:
        p.volume = int(sys.argv[sys.argv.index("--initial-volume") + 1])

    async def session(ws):
        await ws.send(json.dumps({"type": "auth_state", "logged_in": True, "is_active": True}))
        await ws.send(json.dumps(p.state()))
        async for raw in ws:
            c = json.loads(raw)
            cmd = c.get("command")
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

    server = await websockets.serve(session, "127.0.0.1", 0)
    port = server.sockets[0].getsockname()[1]
    with open(f"{data}/ws.addr", "w") as f:
        f.write("127.0.0.1")
    with open(f"{data}/ws.port", "w") as f:
        f.write(str(port))
    print(f"\x1b[0m00:00:00.000 I [f:128] client expires in {DAYS} days\x1b[0m", flush=True)
    event(f"started volume={p.volume}")
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
