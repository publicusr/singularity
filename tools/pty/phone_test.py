"""#T0005 in a pty: a phone (any other Spotify app on Connect) changing playback must be followed, not undone.

A real Daemon (the real SoloistEngine) runs a stand-in Soloist build from scratch dirs and a fake CDN, like
`update_test.py`. The stand-in can act as a phone on Connect: it changes the track by itself (a file in its data dir),
or it comes back from a restart already playing a track (as when Spotify hands a restarted device the track another
app started).

Phase 1 (`remote`): a new build is staged while a track plays; the phone then plays a song, and queues one and skips
to it. The Daemon must not restart Soloist for the update at those moments (the restart would put its remembered
track back) and must send no play/pause/seek/queue command at all.

Phase 2 (`resume`): a new build is staged; our own Next is the quiet moment, so Soloist restarts into it. The new
build comes back already playing the phone's track. The Daemon must not put the track it remembered back.

    venv/bin/python phone_test.py out-phone
"""
import http.server, json, os, shutil, socket, struct, subprocess, sys, tarfile, threading, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import shot

here = os.path.dirname(os.path.abspath(__file__))
out = os.path.abspath(sys.argv[1])
shutil.rmtree(out, ignore_errors=True)
os.makedirs(out)
LAST_MODIFIED = "Fri, 16 Oct 2026 18:10:28 GMT"


def build(version, days):
    """A tarball holding a stand-in `soloist` (as the CDN ships it: binary, CHANGELOG, licenses)."""
    d = f"{out}/build-{version}"
    os.makedirs(d, exist_ok=True)
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


build_v1, build_v2 = build("v1", 90), build("v2", 90)
# what the fake CDN serves right now (a Run sets it)
serving = {"etag": '"v1"', "file": build_v1, "last_modified": LAST_MODIFIED}


class CDN(http.server.BaseHTTPRequestHandler):
    """Serves whatever `serving` says: the Daemon's updater checks it every 2 s in these runs."""

    def answer(self):
        body = open(serving["file"], "rb").read()
        self.send_response(200)
        self.send_header("ETag", serving["etag"])
        self.send_header("Last-Modified", serving["last_modified"])
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        return body

    def do_HEAD(self):
        self.answer()

    def do_GET(self):
        self.wfile.write(self.answer())

    def log_message(self, fmt, *args):
        pass


cdn = http.server.ThreadingHTTPServer(("127.0.0.1", 0), CDN)
threading.Thread(target=cdn.serve_forever, daemon=True).start()


class Run:
    """One Daemon with its own scratch dirs, a stand-in Soloist installed (v1) and the fake CDN serving v1."""

    def __init__(self, name, login_track=None):
        global serving
        serving = {"etag": '"v1"', "file": build_v1, "last_modified": LAST_MODIFIED}
        self.out = f"{out}/{name}"
        self.scratch = f"{self.out}/scratch"
        self.events = f"{self.out}/soloist-events.log"
        self.inst = f"{self.scratch}/data/singularity/soloist"
        os.makedirs(self.inst)
        with tarfile.open(build_v1) as t:
            t.extractall(self.inst, filter="tar")
        json.dump({"etag": '"v1"', "last_modified": LAST_MODIFIED, "version": "soloist v1"}, open(f"{self.inst}/build.json", "w"))
        self.sock = f"{shot.RUNTIME}/sgl-phone-{name}-{os.getpid()}.sock"
        self.env = dict(os.environ, SINGULARITY_ENGINE="soloist", SINGULARITY_SOLOIST="local", SINGULARITY_START_APP="0", SINGULARITY_SOCKET=self.sock, TERM="xterm-256color", COLORTERM="truecolor", SINGULARITY_NO_KEYRING="1",
                        SOLOIST_API_KEY="not-a-real-key", SINGULARITY_TAP="off", SINGULARITY_DBUS="off", SINGULARITY_PIXEL="0",
                        SINGULARITY_SOLOIST_CDN=f"http://127.0.0.1:{cdn.server_address[1]}/soloist_release_x86_64.tar.gz",
                        SINGULARITY_SOLOIST_CHECK_SECS="2", FAKE_SOLOIST_EVENTS=self.events,
                        XDG_DATA_HOME=f"{self.scratch}/data", XDG_CONFIG_HOME=f"{self.scratch}/config", XDG_CACHE_HOME=f"{here}/xdgcache", XDG_STATE_HOME=f"{self.scratch}/state")
        if login_track is not None:
            self.env["FAKE_SOLOIST_STATE_AT_LOGIN"] = str(login_track)
        for k in ("KITTY_WINDOW_ID", "TERM_PROGRAM", "GHOSTTY_RESOURCES_DIR", "TMUX"):
            self.env.pop(k, None)
        self.soloist_data = f"{self.scratch}/data/singularity/soloist-state/data"
        self.last = None
        self.lines = []
        self.daemon = None
        self.term = None

    def start(self):
        self.daemon = subprocess.Popen([f"{shot.BIN}/singularityd", "--fake-catalogue", f"{shot.ROOT}/crates/client/tests/fixtures/home.json"], env=self.env, stderr=open(f"{self.out}/daemon.log", "w"))
        time.sleep(1.5)
        self.term = shot.Term(self.env)
        self.term.pump(2.0)

    def watch(self):
        """A second connection that sends the commands and keeps the Daemon's last playback event."""
        self.client = Client(self.sock, self)
        threading.Thread(target=self.client.reader, daemon=True).start()

    def command(self, **c):
        self.client.send({"t": "command", **c})

    def phone(self, what):
        open(f"{self.soloist_data}/remote-{what}", "w").close()

    def stage(self):
        """The CDN starts offering v2; wait until the Daemon has it staged (verifies `--version` first)."""
        serving.update(etag='"v2"', file=build_v2, last_modified="Fri, 17 Oct 2026 18:10:28 GMT")
        for _ in range(40):
            if os.path.exists(f"{self.inst}.new/build.json"):
                return True
            time.sleep(0.25)
        return False

    def sent(self, since=0.0):
        """The commands the stand-in got after `since` (a timestamp in the events file)."""
        cmds = []
        for line in open(self.events):
            parts = line.split(" ", 3)
            if len(parts) < 4 or float(parts[0]) < since:
                continue
            if parts[3].startswith("cmd "):
                cmds.append(json.loads(parts[3][4:]))
        return cmds

    def starts(self):
        return [l.split(" ", 3)[1] for l in open(self.events) if " started " in l]

    def stop(self):
        try:
            self.client.close()
        except Exception:
            pass
        if self.term is not None:
            # Esc quits (nothing is open)
            try:
                self.term.send(b"\x1b", 0.3)
            except OSError:
                pass
        if self.daemon is not None:
            self.daemon.terminate()
            self.daemon.wait(10)
        if os.path.exists(self.sock):
            os.unlink(self.sock)


class Client:
    """One Client connection: sends commands, remembers the Daemon's playback events."""

    def __init__(self, sock, run):
        self.run = run
        self.s = socket.socket(socket.AF_UNIX)
        self.s.connect(sock)
        self.send({"t": "hello", "protocol": 1, "client": "phone-test"})

    def send(self, m):
        b = json.dumps(m).encode()
        self.s.sendall(struct.pack(">I", len(b)) + b)

    def reader(self):
        try:
            while True:
                head = self.recv(4)
                if head is None:
                    return
                n = struct.unpack(">I", head)[0]
                body = self.recv(n)
                if body is None:
                    return
                m = json.loads(body)
                if m.get("t") == "event" and m.get("ev") == "playback":
                    self.run.last = m
        except OSError:
            return

    def recv(self, n):
        b = b""
        while len(b) < n:
            c = self.s.recv(n - len(b))
            if not c:
                return None
            b += c
        return b

    def close(self):
        try:
            self.s.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        self.s.close()


def track(run):
    item = (run.last or {}).get("item") or {}
    return (item.get("uri") or "", item.get("name") or "")


ok = True


def check(what, got, want):
    global ok
    good = got == want
    ok &= good
    print(f"{'ok  ' if good else 'FAIL'} {what}: {got!r}" + ("" if good else f" (wanted {want!r})"))


# ── phase 1: a phone changes the track while a build is staged ───────────────────────────────────────────────
r = Run("remote")
try:
    r.start()
    r.watch()
    r.command(cmd="play", uri="spotify:track:fake0")
    time.sleep(1.5)
    check("the Daemon's own play", track(r), ("spotify:track:fake0", "Fake Track 001"))
    staged = r.stage()
    r.term.pump(1.0)
    check("a new build is staged while a track plays", staged, True)
    at = time.time()
    r.phone("play-1")
    r.term.pump(2.5)
    check("the phone's song plays", track(r), ("spotify:track:fake1", "Fake Track 002"))
    r.phone("queue-2")
    r.term.pump(2.5)
    check("the queued song the phone skipped to plays", track(r), ("spotify:track:fake2", "Fake Track 003"))
    r.term.shot(f"{out}/1-phone-song.png")
    check("Soloist was not restarted over the phone's changes", r.starts(), ["v1"])
    check("the Daemon sent no command that changes playback", [c for c in r.sent(at) if c.get("command") not in ("get_queue", "get_state")], [])
    r.term.pump(2.0)
    check("still the phone's song a while later", track(r), ("spotify:track:fake2", "Fake Track 003"))
finally:
    r.stop()

# ── phase 2: the restarted build comes back playing the phone's track ────────────────────────────────────────
r = Run("resume", login_track=2)
try:
    r.start()
    r.watch()
    r.command(cmd="play", uri="spotify:track:fake0")
    time.sleep(1.5)
    staged = r.stage()
    r.term.pump(1.0)
    check("a new build is staged", staged, True)
    # our own next: the quiet moment the update takes
    r.command(cmd="next")
    time.sleep(1.0)
    check("our next reached the player", [c.get("command") for c in r.sent(0) if c.get("command") != "get_queue"], ["play", "skip_next"])
    at = time.time()
    r.term.pump(5.0)
    check("the new build took over", r.starts(), ["v1", "v2"])
    check("the Daemon did not put the remembered track back", [c for c in r.sent(at) if c.get("command") in ("play", "seek")], [])
    check("the track the player came back with stands", track(r), ("spotify:track:fake2", "Fake Track 003"))
    r.term.shot(f"{out}/2-resume-dropped.png")
finally:
    r.stop()

cdn.shutdown()
print("all good" if ok else "SOMETHING FAILED")
sys.exit(0 if ok else 1)
