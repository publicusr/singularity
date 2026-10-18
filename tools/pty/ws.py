"""Tiny Soloist WS helper: ws.py '<json command>' [wait_s] → prints events' (status, item uri, context uri)."""
import asyncio, json, os, sys
import websockets
D = os.path.expanduser("~/.local/share/singularity/soloist-state/data")
async def main():
    addr, port = os.environ["WS"].split(":") if "WS" in os.environ else (open(f"{D}/ws.addr").read().strip(), open(f"{D}/ws.port").read().strip())
    async with websockets.connect(f"ws://{addr}:{port}") as ws:
        if sys.argv[1] != "-":
            await ws.send(json.dumps({"type": "command", **json.loads(sys.argv[1])}))
        wait = float(sys.argv[2]) if len(sys.argv) > 2 else 1.0
        end = asyncio.get_event_loop().time() + wait
        while (left := end - asyncio.get_event_loop().time()) > 0:
            try:
                e = json.loads(await asyncio.wait_for(ws.recv(), left))
            except asyncio.TimeoutError:
                break
            if e.get("type") == "playback_state" or "item" in e:
                print(e.get("type"), e.get("status"), (e.get("item") or {}).get("uri"), (e.get("context") or {}).get("uri"), e.get("volume"))
asyncio.run(main())
