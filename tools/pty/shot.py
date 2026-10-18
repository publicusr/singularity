"""Drive the real `singularity` binary in a pseudo-terminal and screenshot it (pyte + PIL).

usage: venv/bin/python shot.py <scenario> <outdir>
Starts its own singularityd (--fake-engine --no-catalogue) on a short test socket and kills only that.
"""
import base64, fcntl, os, pty, re, select, shutil, struct, subprocess, sys, termios, time, zlib
import pyte
from PIL import Image, ImageDraw, ImageFont

ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "../.."))
BIN = f"{ROOT}/target/debug"
COLS, ROWS, CW, CH = 140, 40, 10, 20
BRAILLE = ImageFont.truetype("/usr/share/fonts/google-noto/NotoSansSymbols2-Regular.ttf", 16)
FONT = ImageFont.truetype("/usr/share/fonts/nerd-fonts/DejaVuSansMono/DejaVuSansMNerdFontMono-Regular.ttf", 16)


class Term:
    def __init__(self, env):
        self.screen = pyte.Screen(COLS, ROWS)
        self.stream = pyte.ByteStream(self.screen)
        pid, fd = pty.fork()
        if pid == 0:
            os.execve(f"{BIN}/singularity", ["singularity"], env)
        self.pid, self.fd = pid, fd
        self.pending = b""
        self.images = {}      # kitty image id -> PIL image
        self.placed = {}      # (id, pid) -> (col, row, cols, rows)
        self.upload = None    # (id, w, h, chunks) while a chunked transmission is in flight
        self.apc_log = []
        self.cup = (0, 0)  # last cursor move seen in the output (row, col), across reads
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, COLS * CW, ROWS * CH))

    def pump(self, secs):
        end = time.time() + secs
        while time.time() < end:
            r, _, _ = select.select([self.fd], [], [], 0.05)
            if r:
                try:
                    data = os.read(self.fd, 65536)
                except OSError:
                    return
                self.feed(data)

    def feed(self, data):
        # pull kitty graphics APCs out before pyte sees them (it would print their payload)
        data = self.pending + data
        self.pending = b""
        out = bytearray()
        i = 0
        while i < len(data):
            j = data.find(b"\x1b_G", i)
            if j < 0:
                out += data[i:]
                break
            k = data.find(b"\x1b\\", j)
            if k < 0:
                out += data[i:j]
                self.pending = data[j:]
                break
            # a placement's position: the cursor move right before it
            pre = data[i:j]
            out += pre
            m = re.findall(rb"\x1b\[(\d+);(\d+)H", pre)
            if m:
                self.cup = (int(m[-1][0]) - 1, int(m[-1][1]) - 1)
            self.apc(data[j + 3:k].decode(), bytes(out))
            i = k + 2
        self.stream.feed(bytes(out))

    def apc(self, body, before):
        ctrl, _, payload = body.partition(";")
        kv = dict(p.split("=", 1) for p in ctrl.split(",") if "=" in p)
        self.apc_log.append(kv.get("a", "") + ":" + ",".join(f"{k}={v}" for k, v in kv.items() if k in "ipcr"))
        a = kv.get("a")
        if a in ("t", "T") or (a is None and self.upload):
            if a in ("t", "T"):
                self.upload = (int(kv["i"]), int(kv["s"]), int(kv["v"]), [], kv.get("o") == "z", kv.get("f", "24"))
                if a == "T":
                    row, col = self.cup
                    self.placed[(int(kv["i"]), int(kv.get("p", "1")))] = (col, row, int(kv["c"]), int(kv["r"]))
            self.upload[3].append(payload)
            if kv.get("m", "0") == "0":
                iid, w, h, chunks, z, f = self.upload
                raw = base64.b64decode("".join(chunks))
                if z:
                    raw = zlib.decompress(raw)
                self.images[iid] = Image.frombytes("RGBA" if f == "32" else "RGB", (w, h), raw).convert("RGB")
                self.upload = None
        elif a == "p":
            row, col = self.cup
            self.placed[(int(kv["i"]), int(kv["p"]))] = (col, row, int(kv["c"]), int(kv["r"]))
        elif a == "d":
            if kv.get("d") == "i":
                self.placed.pop((int(kv["i"]), int(kv["p"])), None)
            elif kv.get("d") == "I":
                for k in [k for k in self.placed if k[0] == int(kv["i"])]:
                    self.placed.pop(k)
            elif kv.get("d") in ("A", "a"):
                self.placed.clear()

    def send(self, b, wait=0.15):
        os.write(self.fd, b)
        self.pump(wait)

    def text(self):
        return "\n".join(self.screen.display)

    def find(self, s):
        for y, line in enumerate(self.screen.display):
            x = line.find(s)
            if x >= 0:
                return x, y
        return None

    def shot(self, path):
        # like kitty with z=-1: backgrounds, then images, then text on top
        img = Image.new("RGB", (COLS * CW, ROWS * CH), "black")
        d = ImageDraw.Draw(img)
        def col(v, dflt):
            if v == "default":
                return dflt
            return "#" + v if len(v) == 6 and all(ch in "0123456789abcdefABCDEF" for ch in v) else v
        cells = []
        for y in range(ROWS):
            row = self.screen.buffer[y]
            for x in range(COLS):
                c = row[x]
                fg, bg = col(c.fg, "#d6d6d6"), col(c.bg, "#000000")
                if c.reverse:
                    fg, bg = bg, fg
                try:
                    d.rectangle([x * CW, y * CH, (x + 1) * CW - 1, (y + 1) * CH - 1], fill=bg)
                except ValueError:
                    pass
                if c.data.strip():
                    cells.append((x, y, c.data, fg))
        for (iid, _), (cx, cy, c, r) in self.placed.items():
            if iid in self.images:
                img.paste(self.images[iid].resize((c * CW, r * CH)), (cx * CW, cy * CH))
        for x, y, ch, fg in cells:
            font = BRAILLE if 0x2800 <= ord(ch[0]) <= 0x28FF else FONT
            try:
                d.text((x * CW, y * CH + 1), ch, font=font, fill=fg)
            except ValueError:
                d.text((x * CW, y * CH + 1), ch, font=font, fill="#d6d6d6")
        img.save(path)
        with open(path.replace(".png", ".txt"), "w") as f:
            f.write(self.text())


def mouse(t, code, x, y, release=False, wait=0.08):
    t.send(f"\x1b[<{code};{x + 1};{y + 1}{'m' if release else 'M'}".encode(), wait)


def home(t, out):
    t.pump(1.0)
    t.shot(f"{out}/0-home.png")
    hx, hy = t.find("┌ home")
    for _ in range(3):
        mouse(t, 65, hx + 5, hy + 3)  # wheel down: one row each
    t.pump(0.6)
    t.shot(f"{out}/1-scrolled.png")
    for _ in range(12):
        if t.find("Recommended Stations"):
            break
        mouse(t, 65, hx + 5, hy + 3)
        t.pump(0.2)
    x, y = t.find("Recommended Stations")
    mouse(t, 2, x + 3, y + 3)  # right-click a Card
    t.pump(0.4)
    t.shot(f"{out}/2-menu.png")
    t.send(b"\x1b", 0.3)
    mouse(t, 0, x + 3, y + 3)  # left-click: play it
    mouse(t, 0, x + 3, y + 3, release=True, wait=0.8)
    t.shot(f"{out}/3-played.png")
    t.send(b"q", 0.5)
    with open(f"{out}/apc.log", "w") as f:
        f.write("\n".join(t.apc_log))


def library(t, out):
    t.pump(1.0)
    x, y = t.find("♥ Liked Songs")
    mouse(t, 0, x + 2, y)
    mouse(t, 0, x + 2, y, release=True, wait=0.8)
    t.shot(f"{out}/0-liked.png")
    hx, hy = t.find("Title")
    mouse(t, 0, hx + 1, hy)
    mouse(t, 0, hx + 1, hy, release=True, wait=0.3)
    mouse(t, 0, hx + 1, hy)  # second click: descending
    mouse(t, 0, hx + 1, hy, release=True, wait=0.5)
    t.shot(f"{out}/1-sorted-desc.png")
    t.send(b"/", 0.2)
    t.send(b"glass", 0.4)
    t.shot(f"{out}/2-filtering.png")
    t.send(b"\r", 0.3)
    rx, ry = t.find("Fake Track")
    for _ in range(2):
        mouse(t, 0, rx + 2, ry)
        mouse(t, 0, rx + 2, ry, release=True, wait=0.1)
    t.pump(0.8)
    t.shot(f"{out}/3-played.png")
    t.send(b"\x1b", 0.3)
    t.send(b"\x1b", 0.5)  # clear filter, then back to Home
    t.shot(f"{out}/4-back-home.png")
    t.send(b"q", 0.5)


def like(t, out):
    """#22: the ♥ in the player bar, `l`, unliking from a row's menu, Follow on an artist page."""
    t.pump(1.0)
    x, y = t.find("♥ Liked Songs")
    mouse(t, 0, x + 2, y); mouse(t, 0, x + 2, y, release=True, wait=0.8)
    rx, ry = t.find("Fake Track 001")
    for _ in range(2):
        mouse(t, 0, rx + 2, ry)
        mouse(t, 0, rx + 2, ry, release=True, wait=0.1)
    t.pump(1.0)
    t.send(b" ", 0.5)  # pause, so the screen holds still
    t.shot(f"{out}/0-playing-liked.png")
    t.send(b"l", 0.8)  # unlike what's playing: it leaves Liked Songs at once
    t.shot(f"{out}/1-unliked.png")
    t.send(b"l", 0.8)  # and back on top
    t.shot(f"{out}/2-liked-again.png")
    rx, ry = t.find("Fake Track 003")
    mouse(t, 2, rx + 2, ry, wait=0.4)  # right-click
    t.shot(f"{out}/3-menu.png")
    t.send(b"\x1b[B" * 3, 0.1)
    t.send(b"\r", 0.8)
    t.shot(f"{out}/4-removed.png")
    ax, ay = t.find("Nova Drift")
    mouse(t, 0, ax + 1, ay); mouse(t, 0, ax + 1, ay, release=True, wait=0.8)
    t.shot(f"{out}/5-artist.png")
    fx, fy = t.find("+ Follow")
    mouse(t, 0, fx + 1, fy); mouse(t, 0, fx + 1, fy, release=True, wait=0.6)
    t.shot(f"{out}/6-following.png")
    t.send(b"q", 0.5)


def private_bus(out):
    """A private D-Bus without service activation, so playerctl and the Daemon never touch the desktop's bus."""
    conf = f"{out}/bus.conf"
    # socket paths must be short: under $XDG_RUNTIME_DIR, removed with the bus
    sockdir = f"{os.environ['XDG_RUNTIME_DIR']}/sgl-bus-{os.getpid()}"
    os.makedirs(sockdir, mode=0o700, exist_ok=True)
    with open(conf, "w") as f:
        f.write(f"""<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig><type>session</type><listen>unix:dir={sockdir}</listen><auth>EXTERNAL</auth>
<policy context="default"><allow send_destination="*" eavesdrop="true"/><allow eavesdrop="true"/><allow own="*"/></policy></busconfig>""")
    bus = subprocess.Popen(["dbus-daemon", f"--config-file={conf}", "--nofork", "--print-address=1"], stdout=subprocess.PIPE, text=True)
    return bus, bus.stdout.readline().strip()


def mpris(t, out, env):
    """#21: playerctl drives the Daemon; the Client shows each change."""
    def ctl(*args):
        r = subprocess.run(["playerctl", "-p", "singularity", *args], env=env, capture_output=True, text=True, timeout=5)
        line = f"$ playerctl {' '.join(args)}\n{r.stdout}{r.stderr}"
        print(line.rstrip())
        with open(f"{out}/playerctl.log", "a") as f:
            f.write(line)
        return r.stdout.strip()
    t.pump(1.0)
    t.send(b" ", 1.0)
    ctl("status")
    ctl("metadata")
    ctl("play-pause"); t.pump(0.6)
    t.shot(f"{out}/0-paused-by-playerctl.png")
    ctl("status")
    ctl("position", "60"); t.pump(0.6)
    ctl("volume", "0.3"); t.pump(0.6)
    ctl("loop", "Track"); t.pump(0.6)
    t.shot(f"{out}/1-seek-volume-loop.png")
    ctl("next"); t.pump(0.6)
    ctl("play-pause"); t.pump(0.6)
    ctl("metadata", "--format", "{{status}}: {{title}} {{duration(position)}}/{{duration(mpris:length)}} vol {{volume}} loop {{loop}}")
    t.shot(f"{out}/2-next-playing.png")
    t.send(b" ", 0.4)
    t.send(b"q", 0.5)


def config(t, out, env):
    """#20: the files are written on first run; edits apply live; the palette's toggle edits config.toml."""
    cdir = f"{env['XDG_CONFIG_HOME']}/singularity"
    t.pump(1.0)
    t.send(b"?", 0.5)
    t.shot(f"{out}/0-help-standard.png")
    t.send(b"\x1b", 0.3)
    keys = open(f"{cdir}/keys.toml").read()
    with open(f"{cdir}/keys.toml", "w") as f:
        f.write(keys.replace('preset = "standard"', 'preset = "vim"').replace('# like = "f"', 'like = "f"\nshuffle = "f"'))
    t.pump(2.0)
    t.shot(f"{out}/1-conflict-toast.png")
    t.send(b"?", 0.5)
    t.shot(f"{out}/2-help-vim.png")
    t.send(b"\x1b", 0.3)
    x, y = t.find("♥ Liked Songs")
    mouse(t, 0, x + 2, y); mouse(t, 0, x + 2, y, release=True, wait=0.8)
    t.send(b"jjj", 0.4)
    t.shot(f"{out}/3-vim-j.png")
    t.send(b"\x0b", 0.3); t.send(b"notif", 0.3); t.send(b"\r", 0.8)
    t.shot(f"{out}/4-notifications-on.png")
    print(open(f"{cdir}/config.toml").read())
    t.send(b"\x1b", 0.3); t.send(b"\x1b", 0.3); t.send(b"q", 0.5)


def designs(t, out):
    """#18: each design through the picker (live preview), a track playing; Cockpit's palettes with t."""
    t.pump(1.0)
    t.send(b" ", 1.5)  # play: the meters and the Now playing panel have something to show
    names = ["singularity", "event-horizon", "classic", "cockpit", "terminal"]
    t.send(b"\x0b", 0.3); t.send(b"design", 0.3); t.send(b"\r", 0.6)
    for i, name in enumerate(names):
        if i:
            t.send(b"\x1b[B", 1.2)  # ↓ previews the next design
        t.shot(f"{out}/{i}-{name}.png")
    t.send(b"\r", 0.5)  # keep Terminal
    t.send(b"\x0b", 0.3); t.send(b"design", 0.3); t.send(b"\r", 0.5)
    t.send(b"\x1b[A", 0.8)  # back up to Cockpit
    t.send(b"\r", 0.6)
    for k in range(2):
        t.send(b"t", 0.8)
        t.shot(f"{out}/5-cockpit-t{k + 1}.png")
    t.send(b" ", 0.4)
    t.send(b"q", 0.5)


def layouts(t, out, env):
    """#19: swap Widgets (v, ◂ ▸), drag a divider, save the layout, and find it again in a new Client."""
    t.pump(1.0)
    t.send(b" ", 1.0)
    x, y = t.find("fft")
    mouse(t, 0, x + 1, y + 3); mouse(t, 0, x + 1, y + 3, release=True, wait=0.3)  # focus the fft Pane
    t.send(b"v", 0.6)  # fft → levels
    lx, ly = t.find("loudness")
    row = t.screen.display[ly]
    sx = row.index("▸", lx)
    mouse(t, 0, sx, ly); mouse(t, 0, sx, ly, release=True, wait=0.6)  # loudness → next free one
    # drag the library's right edge 12 columns right
    gx = t.screen.display[5].index("││") + 1
    mouse(t, 0, gx, 12); mouse(t, 32, gx + 6, 12); mouse(t, 32, gx + 12, 12); mouse(t, 0, gx + 12, 12, release=True, wait=0.5)
    t.shot(f"{out}/0-changed.png")
    t.send(b"\x0b", 0.3); t.send(b"save layout", 0.3); t.send(b"\r", 0.4)
    t.send(b"Wide library", 0.3)
    t.shot(f"{out}/1-naming.png")
    t.send(b"\r", 0.6)
    t.shot(f"{out}/2-saved.png")
    before = [l for l in t.screen.display[1:44]]
    t.send(b" ", 0.3)
    t.send(b"q", 0.8)
    t2 = Term(env)
    t2.pump(2.5)
    t2.shot(f"{out}/3-new-client.png")
    after = [l for l in t2.screen.display[1:44]]
    # compare the Pane frames (titles and borders), not the moving contents
    # compare where the Pane frames are: box-drawing characters left of the well (its frame sags, the borders near it
    # bend and the scope draws braille, all moving with the music; contents and toasts move too)
    frame = lambda ls: [re.sub(r"[^─│┌┐└┘├┤┬┴┼╭╮╯╰]", " ", l[:80]) for l in ls[:33]]
    print("same layout after restart:", frame(before) == frame(after))
    t2.send(b"\x0b", 0.3); t2.send(b"design", 0.3); t2.send(b"\r", 0.6)
    t2.shot(f"{out}/4-picker.png")
    t2.send(b"\x1b", 0.3); t2.send(b"q", 0.5)


def main():
    scenario, out = sys.argv[1], sys.argv[2]
    os.makedirs(out, exist_ok=True)
    sock = f"{os.environ['XDG_RUNTIME_DIR']}/sgl-{os.getpid()}.sock"
    env = dict(os.environ, SINGULARITY_SOCKET=sock, TERM="xterm-256color", COLORTERM="truecolor",
               # scratch config and state: test runs never write the owner's files
               XDG_CONFIG_HOME=f"{os.path.abspath(out)}/config", XDG_STATE_HOME=f"{os.path.abspath(out)}/state")
    for k in ("KITTY_WINDOW_ID", "TERM_PROGRAM", "GHOSTTY_RESOURCES_DIR", "TMUX"):
        env.pop(k, None)
    if scenario == "pixel":
        env["SINGULARITY_PIXEL_MOUSE"] = "1"
    cat = ["--no-catalogue"]
    if scenario.startswith("well"):
        env["SINGULARITY_PIXEL"] = "1" if scenario in ("well-pixels", "well-hover") else "0"
    if scenario.startswith("home") or scenario in ("library", "queue", "lyrics", "artist", "search", "like", "config", "designs", "layouts"):
        here = os.path.dirname(os.path.abspath(__file__))
        env["XDG_CACHE_HOME"] = f"{here}/xdgcache"
        env["SINGULARITY_PIXEL"] = "1" if scenario in ("home-pixels", "designs") else "0"
        cat = ["--fake-catalogue", f"{ROOT}/crates/client/tests/fixtures/home.json"]
    bus = None
    if scenario == "mpris":
        bus, addr = private_bus(out)
        env["SINGULARITY_DBUS"] = env["DBUS_SESSION_BUS_ADDRESS"] = addr
    d = subprocess.Popen([f"{BIN}/singularityd", "--fake-engine", *cat], env=env, stdout=subprocess.DEVNULL, stderr=open(f"{out}/daemon.log", "w"))
    try:
        time.sleep(0.4)
        t = Term(env)
        t.pump(1.5)
        if scenario.startswith("home"):
            return home(t, out)
        if scenario == "library":
            return library(t, out)
        if scenario == "like":
            return like(t, out)
        if scenario == "mpris":
            return mpris(t, out, env)
        if scenario == "config":
            return config(t, out, env)
        if scenario == "designs":
            return designs(t, out)
        if scenario == "layouts":
            return layouts(t, out, env)
        if scenario == "artist":
            t.pump(1.0)
            x, y = t.find("♥ Liked Songs")
            mouse(t, 0, x + 2, y); mouse(t, 0, x + 2, y, release=True, wait=0.8)
            ax, ay = t.find("Nova Drift")
            mouse(t, 0, ax + 1, ay); mouse(t, 0, ax + 1, ay, release=True, wait=0.8)
            t.shot(f"{out}/0-artist.png")
            t.send(b"\x1b", 0.5)
            t.shot(f"{out}/1-back.png")
            t.send(b"q", 0.5)
            return
        if scenario == "search":
            t.pump(1.0)
            t.send(b"\x0c", 0.3)  # Ctrl+L
            t.send(b"mix", 1.2)
            t.shot(f"{out}/0-search.png")
            t.send(b"q", 0.1)
            t.send(b"\x1b", 0.2); t.send(b"\x1b", 0.2); t.send(b"\x1b", 0.3)
            t.send(b"q", 0.5)
            return
        if scenario == "lyrics":
            t.pump(1.0)
            t.send(b" ", 0.5)
            t.send(b"\x1b[1;2C" * 6, 3.0)  # Shift+→ ×6: 30 s in
            t.shot(f"{out}/0-lyrics.png")
            t.send(b"q", 0.5)
            return
        if scenario == "queue":
            t.pump(1.0)
            x, y = t.find("♥ Liked Songs")
            mouse(t, 0, x + 2, y); mouse(t, 0, x + 2, y, release=True, wait=0.8)
            for row in (3, 5):
                rx, ry = t.find(f"Fake Track 00{row}")
                mouse(t, 2, rx + 2, ry, wait=0.3)  # right-click
                t.send(b"\x1b[B", 0.1)  # down: "Play next"
                t.send(b"\r", 0.8)
            t.shot(f"{out}/0-queued.png")
            t.send(b"q", 0.5)
            return
        if scenario == "well-hover":
            t.pump(1.0)
            t.apc_log.clear()
            x, y = t.find("gravity well")
            t0 = time.time()
            n = 0
            while time.time() - t0 < 1.5:
                # any-motion reports (button 35 = no button, moved), sweeping across the well
                mouse(t, 35, x + (n % 40), y + 3 + (n % 8), wait=0.003)
                n += 1
            el = time.time() - t0
            ups = [l for l in t.apc_log if l.startswith("T:")]
            frees = [l for l in t.apc_log if l.startswith("d:")]
            print(f"{n} motion events in {el:.2f}s -> {len(ups)} hole frames ({len(ups)/el:.0f}/s), {len(frees)} frees; ids {sorted(set(l.split('i=')[1].split(',')[0] for l in ups))}")
            t.send(b"q", 0.5)
            return
        if scenario.startswith("well"):
            t.send(b" ", 2.0)
            t.shot(f"{out}/0-well.png")
            t.send(b"v", 1.0)
            t.shot(f"{out}/1-next-look.png")
            t.send(b"\x0b", 0.6)  # Ctrl+K: palette over the well → cells there
            t.shot(f"{out}/2-palette.png")
            t.send(b"\x1b", 0.3)
            t.send(b"q", 0.5)
            return
        if scenario == "audio":
            t.send(b" ", 2.5)  # play: the fake Engine's groove feeds the tap
            t.shot(f"{out}/0-playing.png")
            t.send(b" ", 2.0)  # pause: bars fall and stop
            t.shot(f"{out}/1-paused.png")
            t.send(b"q", 0.5)
            return
        t.send(b" ", 0.6)  # play
        t.send(b" ", 0.4)  # pause, so positions stay put
        t.shot(f"{out}/0-start.png")
        # the seek bar: the row with the time labels; its first cell follows the "0:0x" label
        y = next(i for i, l in enumerate(t.screen.display) if "3:20" in l)
        line = t.screen.display[y]
        bx = line.index("0:0") + 6
        if scenario == "cells":
            mouse(t, 0, bx + 20, y)
            for dx in range(1, 8):
                mouse(t, 32, bx + 20 + dx, y)
            t.pump(0.3)
            t.shot(f"{out}/1-dragging.png")
            mouse(t, 0, bx + 27, y, release=True, wait=0.5)
            t.shot(f"{out}/2-released.png")
            for _ in range(3):
                mouse(t, 64, bx + 5, y)  # wheel up: +1 s each
            t.pump(0.6)
            t.shot(f"{out}/3-wheel.png")
            t.send(b"\x1b[1;2D", 0.6)  # Shift+Left: -1 s
            t.shot(f"{out}/4-shift-left.png")
        else:
            # pixel mode: aim for 1:37 of the 3:20 fake track with a sub-cell pointer
            width = line.index("3:20") - 1 - bx
            f = 97 / 200
            px = int(round((bx + 0.5 + f * (width - 1)) * CW))
            py = y * CH + CH // 2
            t.send(f"\x1b[<0;{px + 1};{py + 1}M".encode(), 0.1)
            t.send(f"\x1b[<32;{px + 1};{py + 1}M".encode(), 0.3)
            t.shot(f"{out}/1-dragging.png")
            t.send(f"\x1b[<0;{px + 1};{py + 1}m".encode(), 0.5)
            t.shot(f"{out}/2-released.png")
        t.send(b"q", 0.5)
    finally:
        d.terminate()
        d.wait(3)
        if bus:
            bus.terminate()
            bus.wait(3)
            shutil.rmtree(f"{os.environ['XDG_RUNTIME_DIR']}/sgl-bus-{os.getpid()}", ignore_errors=True)


if __name__ == "__main__":
    main()
