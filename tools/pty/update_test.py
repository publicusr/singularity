"""#6 in a pty: a real Daemon (real SoloistEngine) runs an expiring stand-in Soloist build (`fake_soloist.py`, 1 day
left) from scratch dirs; a fake CDN then offers a newer one. The new build must be staged while a track plays, and
taken over only at the next track change, with playback picked up. Never touches the owner's Soloist, keyring or audio.

    venv/bin/python update_test.py out-update
"""
import http.server, json, os, shutil, subprocess, sys, tarfile, threading, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import shot

here = os.path.dirname(os.path.abspath(__file__))
out = os.path.abspath(sys.argv[1])
shutil.rmtree(out, ignore_errors=True)
os.makedirs(out)
scratch = f"{out}/scratch"
events = f"{out}/soloist-events.log"
LAST_MODIFIED = "Fri, 16 Oct 2026 18:10:28 GMT"


def build(version, days):
    """A tarball holding a stand-in `soloist` (as the CDN ships it: binary, CHANGELOG, licenses)."""
    d = f"{out}/build-{version}"
    os.makedirs(d)
    src = open(f"{here}/fake_soloist.py").read().replace("@VERSION@", version).replace("@DAYS@", str(days))
    with open(f"{d}/soloist", "w") as f:
        f.write(f"#!{here}/venv/bin/python\n{src}")
    os.chmod(f"{d}/soloist", 0o555)
    for name in ("CHANGELOG.md", "THIRD_PARTY_LICENSES.txt"):
        open(f"{d}/{name}", "w").write(version)
    tgz = f"{out}/{version}.tar.gz"
    with tarfile.open(tgz, "w:gz") as t:
        for name in ("soloist", "CHANGELOG.md", "THIRD_PARTY_LICENSES.txt"):
            t.add(f"{d}/{name}", arcname=name)
    return tgz


serving = {"etag": '"v1"', "file": build("v1", 1)}
build_v2 = build("v2", 90)


class CDN(http.server.BaseHTTPRequestHandler):
    def headers_only(self):
        body = open(serving["file"], "rb").read()
        self.send_response(200)
        self.send_header("ETag", serving["etag"])
        self.send_header("Last-Modified", LAST_MODIFIED)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        return body

    def do_HEAD(self):
        self.headers_only()

    def do_GET(self):
        self.wfile.write(self.headers_only())

    def log_message(self, fmt, *args):
        with open(f"{out}/cdn.log", "a") as f:
            f.write(f"{time.time():.3f} {self.command} {serving['etag']}\n")


cdn = http.server.ThreadingHTTPServer(("127.0.0.1", 0), CDN)
threading.Thread(target=cdn.serve_forever, daemon=True).start()

# v1 is installed, as a previous update would have left it
inst = f"{scratch}/data/singularity/soloist"
os.makedirs(inst)
with tarfile.open(serving["file"]) as t:
    t.extractall(inst, filter="tar")
json.dump({"etag": '"v1"', "last_modified": LAST_MODIFIED, "version": "soloist v1"}, open(f"{inst}/build.json", "w"))

sock = f"{shot.RUNTIME}/sgl-upd-{os.getpid()}.sock"
env = dict(os.environ, SINGULARITY_SOCKET=sock, TERM="xterm-256color", COLORTERM="truecolor", SINGULARITY_NO_KEYRING="1",
           SOLOIST_API_KEY="not-a-real-key", SINGULARITY_TAP="off", SINGULARITY_DBUS="off", SINGULARITY_PIXEL="0",
           SINGULARITY_SOLOIST_CDN=f"http://127.0.0.1:{cdn.server_address[1]}/soloist_release_x86_64.tar.gz",
           SINGULARITY_SOLOIST_CHECK_SECS="2", FAKE_SOLOIST_EVENTS=events,
           XDG_DATA_HOME=f"{scratch}/data", XDG_CONFIG_HOME=f"{scratch}/config", XDG_CACHE_HOME=f"{here}/xdgcache", XDG_STATE_HOME=f"{scratch}/state")
for k in ("KITTY_WINDOW_ID", "TERM_PROGRAM", "GHOSTTY_RESOURCES_DIR", "TMUX"):
    env.pop(k, None)


def started():
    return [l.split()[1] for l in open(events)] if os.path.exists(events) else []


d = subprocess.Popen([f"{shot.BIN}/singularityd", "--fake-catalogue", f"{shot.ROOT}/crates/client/tests/fixtures/home.json"], env=env, stderr=open(f"{out}/daemon.log", "w"))
try:
    time.sleep(1.5)
    t = shot.Term(env)
    t.pump(2.0)
    # play the first track of Liked Songs
    x, y = t.find("♥  Liked Songs")
    shot.mouse(t, 0, x + 2, y); shot.mouse(t, 0, x + 2, y, release=True, wait=0.8)
    rx, ry = t.find("Fake Track 001")
    for _ in range(2):
        shot.mouse(t, 0, rx + 2, ry); shot.mouse(t, 0, rx + 2, ry, release=True, wait=0.1)
    t.pump(1.5)
    t.shot(f"{out}/0-playing-v1.png")
    print("running builds so far:", started())
    # a new build appears on the CDN while the track plays
    serving.update(etag='"v2"', file=build_v2)
    t.pump(6.0)
    staged = os.path.exists(f"{inst}.new/build.json")
    t.shot(f"{out}/1-staged-still-v1.png")
    print("staged:", staged, "| running builds:", started(), "| still playing:", "❚❚" in t.text())
    # the next track: the moment to switch
    t.send(b"n", 4.0)
    t.shot(f"{out}/2-next-track-v2.png")
    print("after next:", started(), "| installed:", json.load(open(f"{inst}/build.json"))["version"], "| playing:", "❚❚" in t.text())
    t.send(b"\x0b", 0.3); t.send(b"health", 0.3); t.send(b"\r", 2.0)
    t.shot(f"{out}/3-health.png")
    print("health:", [l.strip() for l in t.text().splitlines() if "Engine" in l][:1])
    t.send(b"\x1b", 0.3); t.send(b" ", 0.4); t.send(b"q", 0.5)
finally:
    d.terminate()
    d.wait(5)
    cdn.shutdown()
