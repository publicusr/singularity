"""Drive the real `singularity` binary in a pseudo-terminal and screenshot it (pyte + PIL).

usage: venv/bin/python shot.py <scenario> <outdir>
Starts its own singularityd (--fake-engine --no-catalogue) on a short test socket and kills only that.
"""
import json, base64, fcntl, os, pty, re, select, shutil, struct, subprocess, sys, termios, time, zlib
import pyte
from PIL import Image, ImageDraw, ImageFont

ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "../.."))
BIN = f"{ROOT}/target/debug"
COLS, ROWS, CW, CH = int(os.environ.get("PTY_COLS", 140)), int(os.environ.get("PTY_ROWS", 40)), 10, 20
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
        # pull kitty graphics APCs out before pyte sees them (it would print their payload); each image goes where
        # pyte's cursor is when its APC arrives, as in kitty
        data = self.pending + data
        self.pending = b""
        i = 0
        while i < len(data):
            j = data.find(b"\x1b_G", i)
            if j < 0:
                # a read can end inside "\x1b_G": keep that tail for the next one, or pyte prints the APC as text
                tail = next((n for n in (2, 1) if data.endswith(b"\x1b_G"[:n])), 0)
                self.stream.feed(data[i:len(data) - tail])
                self.pending = data[len(data) - tail:]
                return
            k = data.find(b"\x1b\\", j)
            if k < 0:
                self.stream.feed(data[i:j])
                self.pending = data[j:]
                return
            self.stream.feed(data[i:j])
            self.cup = (self.screen.cursor.y, self.screen.cursor.x)
            self.apc(data[j + 3:k].decode(), b"")
            i = k + 2

    def apc(self, body, before):
        ctrl, _, payload = body.partition(";")
        kv = dict(p.split("=", 1) for p in ctrl.split(",") if "=" in p)
        self.apc_log.append(kv.get("a", "") + ":" + ",".join(f"{k}={v}" for k, v in kv.items() if k in "ipcrzxywh"))
        a = kv.get("a")
        if a in ("t", "T") or (a is None and self.upload):
            if a in ("t", "T"):
                self.upload = (int(kv["i"]), int(kv["s"]), int(kv["v"]), [], kv.get("o") == "z", kv.get("f", "24"))
                if a == "T":
                    row, col = self.cup
                    self.placed[(int(kv["i"]), int(kv.get("p", "1")))] = (col, row, int(kv["c"]), int(kv["r"]), None, int(kv.get("z", "0")))
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
            # x,y,w,h: only that part of the image (a cover partly scrolled out of its Pane)
            crop = tuple(int(kv[k]) for k in "xywh") if all(k in kv for k in "xywh") else None
            self.placed[(int(kv["i"]), int(kv["p"]))] = (col, row, int(kv["c"]), int(kv["r"]), crop, int(kv.get("z", "0")))
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
        # as kitty layers them: the default background, images under cell backgrounds (z < -1073741824), cells with
        # their own background, images over backgrounds (z=-1), then text
        img = Image.new("RGB", (COLS * CW, ROWS * CH), "black")
        def paste(below):
            for (iid, _), (cx, cy, c, r, crop, z) in self.placed.items():
                if iid in self.images and (z < -1073741824) == below:
                    src = self.images[iid]
                    if crop:
                        x, y, w, h = crop
                        src = src.crop((x, y, x + w, y + h))
                    img.paste(src.resize((c * CW, r * CH)), (cx * CW, cy * CH))
        paste(True)
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
                # a cell with the default background lets images under it show; kitty decides by colour, so an
                # explicit black (its own default here) counts as default too
                if (c.bg != "default" and c.bg.lower() != "000000") or c.reverse:
                    try:
                        d.rectangle([x * CW, y * CH, (x + 1) * CW - 1, (y + 1) * CH - 1], fill=bg)
                    except ValueError:
                        pass
                if c.data.strip():
                    cells.append((x, y, c.data, fg))
        paste(False)
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


def home_scroll(t, out):
    """#27: Home by lines: rows cut at the top and bottom keep the visible part of their covers."""
    t.pump(1.5)
    x, y = t.find("Made For You") or t.find("Daily Mix 1")
    for n in range(1, 4):
        mouse(t, 65, x, y + 2, wait=0.3)  # one notch: 2 lines
        t.shot(f"{out}/{n}-scrolled-{2 * n}-lines.png")
    crops = [l for l in t.apc_log if l.startswith("p:") and "y=" in l]
    print(f"cropped placements: {len(crops)}; e.g. {crops[:1]}")
    t.send(b"q", 0.5)


def history(t, out):
    """#28/#43: ← → at the right of the main Pane's title: open a list, back to Home, forward again."""
    t.pump(1.0)
    x, y = t.find("♥ Liked Songs")
    mouse(t, 0, x + 2, y)
    mouse(t, 0, x + 2, y, release=True, wait=0.8)
    t.shot(f"{out}/0-list.png")
    bx, by = t.find("← →")
    mouse(t, 0, bx, by)
    mouse(t, 0, bx, by, release=True, wait=0.6)
    t.shot(f"{out}/1-back.png")
    print("back on Home:", t.find("Made For You") is not None)
    fx, fy = t.find("← →")
    mouse(t, 0, fx + 2, fy)
    mouse(t, 0, fx + 2, fy, release=True, wait=0.6)
    t.shot(f"{out}/2-forward.png")
    print("forward to the list:", t.find("250 songs") is not None)
    t.send(b"q", 0.5)


def meters(t, out):
    """#29: the loudness and levels meters follow the volume (the fake Engine's audio is post-volume, like Soloist's)."""
    t.send(b"\x0b", 0.3); t.send(b"design", 0.3); t.send(b"\r", 0.6)
    for _ in range(3):
        t.send(b"\x1b[B", 0.3)  # Cockpit: loudness and levels side by side
    t.send(b"\r", 0.8)
    t.send(b" ", 3.0)
    t.shot(f"{out}/0-volume-full.png")
    vx, vy = t.find("vol")
    for _ in range(110):
        mouse(t, 65, vx + 8, vy, wait=0.01)  # wheel down on the volume bar: -1 % each, to 0
    for _ in range(10):
        mouse(t, 64, vx + 8, vy, wait=0.01)  # and up to 10 %
    t.pump(3.0)
    t.shot(f"{out}/1-volume-10.png")
    t.send(b" ", 0.4)
    t.send(b"q", 0.5)


def wheel(t, out):
    """#26: the wheel moves the highlight over a track list (also over artist/album names) and in the library."""
    t.pump(1.0)
    x, y = t.find("♥ Liked Songs")
    mouse(t, 0, x + 2, y)
    mouse(t, 0, x + 2, y, release=True, wait=0.8)
    ax, ay = t.find("Nova Drift")  # an artist link in the first row
    for _ in range(6):
        mouse(t, 65, ax + 1, ay, wait=0.05)  # wheel down
    t.pump(0.4)
    t.shot(f"{out}/0-wheel-over-artist.png")
    for _ in range(40):
        mouse(t, 65, ax + 1, ay + 5, wait=0.02)
    t.pump(0.4)
    t.shot(f"{out}/1-wheel-past-the-fold.png")
    lx, ly = t.find("Daily Mix 1")
    for _ in range(4):
        mouse(t, 65, lx, ly, wait=0.05)
    t.pump(0.4)
    t.shot(f"{out}/2-library.png")
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


def idle(t, out):
    """#17: the palette's preview, at 0.5 s (breaking out) and 5 s (drifting); a key wakes it and is swallowed."""
    t.send(b" ", 1.5)  # play, so the disk and the meters move
    t.apc_log.clear()
    t.send(b"\x0b", 0.3); t.send(b"show it now", 0.3); t.send(b"\r", 0.5)
    t.shot(f"{out}/0-half-second.png")
    t.pump(4.5)
    t.shot(f"{out}/1-five-seconds.png")
    # (Visual id n goes up as 2n / 2n+1, double-buffered: the well is 2/3, the free hole 4/5)
    free = [l for l in t.apc_log if l.startswith("T:") and l.split("i=")[1].split(",")[0] in ("4", "5")]
    print("apc kinds:", sorted(set(l.split(":")[0] + ":" + ",".join(p for p in l.split(":", 1)[1].split(",") if p[:1] in "iz") for l in t.apc_log)))
    print(f"free-hole frames: {len(free)}; first {free[:1]}; well empty: {t.find('the well is empty') is not None}")
    t.send(b" ", 0.6)  # wakes only: the track keeps playing
    t.shot(f"{out}/2-awake.png")
    print(f"after wake: placed {sorted(t.placed)} (no 4/5), well empty: {t.find('the well is empty') is not None}")
    t.send(b"q", 0.5)


def seed_listening(data):
    """An invented listening history (26 weeks, fake tracks) in a scratch listening.db, in the Daemon's schema."""
    import random, sqlite3
    os.makedirs(data, exist_ok=True)
    db = sqlite3.connect(f"{data}/listening.db")
    db.executescript("""CREATE TABLE IF NOT EXISTS plays (id INTEGER PRIMARY KEY, started_ms INTEGER NOT NULL,
        day INTEGER NOT NULL, track_uri TEXT NOT NULL, name TEXT NOT NULL, artists TEXT NOT NULL, album TEXT,
        context_uri TEXT, duration_ms INTEGER NOT NULL, played_ms INTEGER NOT NULL, device TEXT NOT NULL);""")
    rnd = random.Random(7)
    now = time.time()
    off = time.localtime(now).tm_gmtoff
    for back in range(1, 175):
        if rnd.random() < 0.3:
            continue
        t = now - back * 86400
        day = int((t + off) // 86400)
        for _ in range(rnd.randint(1, 4 + (175 - back) // 20)):
            a = rnd.choice(["Nova Drift", "Mira Vale", "Lumen Fields"])
            n = rnd.randint(1, 12)
            # schema 1 (as #23 wrote it): no links; the Daemon migrates and fills them in from the Catalogue (#35)
            db.execute("INSERT INTO plays (started_ms, day, track_uri, name, artists, album, duration_ms, played_ms, device) VALUES (?,?,?,?,?,?,?,?,?)",
                       (int(t * 1000) + _ * 240_000, day, f"spotify:track:fake{n}", f"Fake Track {n:03}", f'["{a}"]', f"{a} · Album {n % 3 + 1}", 200000, rnd.randint(60, 200) * 1000, "here"))
    db.commit()
    db.close()


def stats(t, out):
    """#36: the Stats page from a seeded log: tabs, sizes, and a top artist opening once its link was filled in."""
    t.send(b"\x0b", 0.3); t.send(b"stats", 0.3); t.send(b"\r", 1.5)
    t.shot(f"{out}/0-week.png")
    t.send(b"\x1b[C", 1.0); t.send(b"\x1b[C", 1.0)  # → → This year
    t.shot(f"{out}/1-year.png")
    t.send(b"\x1b[C", 2.5)  # All time; the links arrive meanwhile
    t.shot(f"{out}/2-all.png")
    t.send(b"\r", 1.5)  # the first row: the top artist
    t.shot(f"{out}/3-opened.png")
    print("opened:", [l.strip() for l in t.screen.display if "Nova Drift" in l or "Mira Vale" in l or "Lumen Fields" in l][:1])
    t.send(b"\x1b", 1.0)  # back to Stats
    print("back on stats:", any("Week" in l and "Month" in l for l in t.screen.display))
    t.send(b"q", 0.5)


def swap_in(t, out, title_now, want):
    """Click the ▸ in the title row of the Pane titled `title_now` until `want` shows there."""
    at = t.find(title_now)
    if not at:
        t.shot(f"{out}/missing.png")
        raise SystemExit(f"no {title_now!r} on screen")
    x, y = at
    for _ in range(14):
        row = t.screen.display[y]
        if want in row[x - 2:]:
            return
        sx = row.find("▸", x)
        mouse(t, 0, sx, y); mouse(t, 0, sx, y, release=True, wait=0.4)


def popup(t, out):
    """#41: in kitty, images sit under cell backgrounds, so the palette covers them; nothing turns into half-blocks."""
    t.pump(1.5)
    t.shot(f"{out}/0-home.png")
    t.send(b"\x0b", 1.0)  # Ctrl+K
    t.shot(f"{out}/1-palette.png")
    print("half-blocks with the palette open:", sum(l.count("▀") for l in t.screen.display))
    print("placements:", sorted(set(l.split("z=")[1].split(",")[0] for l in t.apc_log if "z=" in l)))
    t.send(b"\x1b", 0.5)
    t.send(b"q", 0.5)


def stalls(t, out):
    """#45: the fake Engine starves (SINGULARITY_FAKE_STALLS=1): one toast says why, the stream Pane shows it."""
    t.pump(1.0)
    t.send(b" ", 0.5)
    toasts = 0
    for _ in range(80):  # 16 s: stalls every 4 s
        t.pump(0.2)
        if any("too slow" in l for l in t.screen.display):
            if toasts == 0:
                t.shot(f"{out}/0-toast.png")
                print("toast:", [l.strip()[-120:] for l in t.screen.display if "too slow" in l][:1])
            toasts += 1
        elif toasts:
            toasts = -abs(toasts)  # gone again; a second appearance would count up from here
    t.shot(f"{out}/1-pane.png")
    print("stream Pane:", [l.split("││")[0].strip("│ ").strip() for l in t.screen.display if "kB/s" in l or "60s" in l])
    print("shown once, not per stall:", toasts < 0)
    t.send(b" ", 0.3)
    t.send(b"q", 0.5)


def queue_lift(t, out):
    """#50: play from Liked Songs (the fake Engine adds the list's next songs as "Next from"), queue one song, then
    drag a "Next from" song up onto it: it becomes queued there."""
    t.pump(1.0)
    x, y = t.find("♥ Liked Songs")
    mouse(t, 0, x + 2, y); mouse(t, 0, x + 2, y, release=True, wait=0.8)
    rx, ry = t.find("Fake Track 002")
    for _ in range(2):  # double-click plays from here
        mouse(t, 0, rx + 2, ry, wait=0.03); mouse(t, 0, rx + 2, ry, release=True, wait=0.05)
    t.pump(1.5)
    rx, ry = t.find("Fake Track 007")
    mouse(t, 2, rx + 2, ry, wait=0.3)
    t.send(b"\x1b[B", 0.1); t.send(b"\r", 1.2)  # Play next
    t.shot(f"{out}/0-before.png")
    src, dst = t.find("Described ctx4 ·"), t.find("Described fake6 ·")
    if not src or not dst:
        print("rows:", [l.strip()[30:100] for l in t.screen.display if "Described" in l or "Next" in l])
        raise SystemExit("missing rows")
    mouse(t, 0, src[0] + 2, src[1])
    mouse(t, 32, src[0] + 2, src[1] - 1, wait=0.15)
    mouse(t, 32, dst[0] + 2, dst[1], wait=0.3)
    t.shot(f"{out}/1-dragging.png")
    mouse(t, 0, dst[0] + 2, dst[1], release=True, wait=1.2)
    t.shot(f"{out}/2-dropped.png")
    hx, hy = t.find("Next in queue")
    print("queue after the drop:", [t.screen.display[k][hx:hx + 30].strip() for k in range(hy, hy + 7)])
    t.send(b" ", 0.3)
    t.send(b"q", 0.5)


def playlists(t, out):
    """#51: a new playlist from the sidebar's menu opens; deleting your own asks twice; others' offer no Delete."""
    t.pump(1.0)
    x, y = t.find("Fake Mixtape")
    mouse(t, 2, x + 1, y, wait=0.4)  # right-click your own playlist
    t.shot(f"{out}/0-own-menu.png")
    print("own menu:", [l.strip() for l in t.screen.display if "Delete playlist" in l or "New playlist" in l][:2])
    t.send(b"\x1b", 0.2)
    x, y = t.find("Daily Mix 1")
    mouse(t, 2, x + 1, y, wait=0.4)  # someone else's: no Delete
    print("their menu has Delete:", any("Delete playlist" in l for l in t.screen.display))
    t.shot(f"{out}/1-their-menu.png")
    t.send(b"\x1b", 0.2)
    t.send(b"\x0b", 0.3)  # Ctrl+K
    t.send(b"new playlist", 0.3)
    t.send(b"\r", 0.3)
    t.send(b"Road trip", 0.3)
    t.shot(f"{out}/2-naming.png")
    t.send(b"\r", 1.0)
    t.shot(f"{out}/3-created.png")
    print("opened:", any("Road trip" in l for l in t.screen.display[:6]), "| toast:", [l.strip() for l in t.screen.display if "Created" in l][:1])
    t.send(b"\x0b", 0.3)
    t.send(b"delete this playlist", 0.3)
    t.send(b"\r", 0.4)
    t.shot(f"{out}/4-asks.png")
    print("asks:", [l.strip() for l in t.screen.display if "again within" in l][:1])
    t.send(b"\x0b", 0.3)
    t.send(b"delete this playlist", 0.3)
    t.send(b"\r", 1.0)
    t.shot(f"{out}/5-deleted.png")
    print("still listed:", any("Road trip" in l for l in t.screen.display))
    t.send(b"q", 0.5)


def playlist_details(t, out):
    """#52: Edit details… on your own playlist: the form, the new name and description everywhere, a refusal put back."""
    t.pump(1.0)
    x, y = t.find("Fake Mixtape")
    mouse(t, 0, x + 1, y); mouse(t, 0, x + 1, y, release=True, wait=0.8)
    print("header:", [l.strip() for l in t.screen.display[:8] if "Made up" in l][:1])
    mouse(t, 2, x + 1, y, wait=0.4)
    print("menu:", [l.strip() for l in t.screen.display if "Edit details" in l][:1])
    x2, y2 = t.find("Edit details")
    mouse(t, 0, x2 + 1, y2); mouse(t, 0, x2 + 1, y2, release=True, wait=0.4)
    t.shot(f"{out}/0-form.png")
    t.send(b"\x15", 0.1)  # Ctrl+U: clear the name
    t.send(b"Mixtape II", 0.2)
    t.send(b"\t\x15", 0.1)
    t.send(b"Songs for night drives, long ones that go on and on past the edge of the form so it wraps", 0.3)
    t.shot(f"{out}/1-typed.png")
    t.send(b"\r", 1.0)
    t.shot(f"{out}/2-saved.png")
    d = t.screen.display
    print("sidebar renamed:", any("Mixtape II" in l[:30] for l in d), "| title:", any("Mixtape II" in l for l in d[:4]), "| header:", any("Songs for night" in l for l in d[:8]))
    t.send(b"\x0b", 0.3)
    t.send(b"edit this playlist", 0.3)
    t.send(b"\r", 0.4)
    t.send(b"\x15fail", 0.2)
    t.send(b"\r", 1.0)
    t.shot(f"{out}/3-refused.png")
    d = t.screen.display
    print("refused, put back:", any("Mixtape II" in l[:30] for l in d), "| toast:", [l.strip() for l in d if "didn't take" in l][:1])
    t.send(b"q", 0.5)


def arrows(t, out):
    """The owner's report: cycling a narrow Pane onto a long title hid its ◂ ▸. Walk two Panes through every Widget."""
    t.pump(1.0)
    for start in ("stream", "fft"):
        x, y = t.find(start)
        seen = []
        for _ in range(12):
            row = t.screen.display[y]
            # the Pane's own title row, from its left corner to its right corner
            seg = row[x - 2:]
            name = seg.split("─")[0].split("◂")[0].strip()
            seen.append(name + (" ◂▸" if "◂ ▸" in seg[:len(seg)] else " (no arrows!)"))
            sx = row.find("▸", x)
            if sx < 0:
                break
            mouse(t, 0, sx, y); mouse(t, 0, sx, y, release=True, wait=0.4)
        print(start, "→", " | ".join(seen))
    t.shot(f"{out}/0-after.png")
    t.send(b"q", 0.5)


def ambient(t, out):
    """#44: starfield, marquee and glow swapped in while a song plays (pixel tier, so the glow has a cover)."""
    t.pump(1.0)
    x, y = t.find("♥ Liked Songs")
    mouse(t, 0, x + 2, y); mouse(t, 0, x + 2, y, release=True, wait=0.8)
    for row in (3, 5, 7):
        rx, ry = t.find(f"Fake Track 00{row}")
        mouse(t, 2, rx + 2, ry, wait=0.3)
        t.send(b"\x1b[B", 0.1); t.send(b"\r", 0.6)  # Play next
    t.send(b" ", 1.0)
    swap_in(t, out, "fft", "starfield")
    t.pump(1.5)
    t.shot(f"{out}/0-starfield.png")
    swap_in(t, out, "tail -f lyrics.log", "marquee")
    t.pump(1.0)
    t.shot(f"{out}/1-marquee.png")
    swap_in(t, out, "marquee", "glow")
    t.pump(1.5)
    t.shot(f"{out}/2-glow.png")
    t.send(b" ", 0.3)
    t.send(b"q", 0.5)


def stats_widgets(t, out):
    """#37: "today" and "top this week" swapped into the side Slots; a 31 s play is logged and both update."""
    swap_to = lambda now, want: swap_in(t, out, now, want)
    t.pump(1.0)
    swap_to("tail -f lyrics.log", "today")
    swap_to("today", "top artists")  # the same Slot on to "top this week"…
    swap_to("queue", "today")      # …and "today" into the queue's Slot
    t.pump(1.0)
    t.shot(f"{out}/0-widgets.png")
    before = [l for l in t.screen.display if "today" in l or "plays" in l]
    t.send(b" ", 31.5)  # play 31 s: one Play
    t.send(b"n", 2.0)   # next: the Play is logged
    t.shot(f"{out}/1-after-a-play.png")
    after = [l for l in t.screen.display if "today" in l or "plays" in l]
    print("today changed after a play:", before != after)
    print("today:", [l.strip()[:150] for l in after][:3])
    t.send(b" ", 0.4)
    t.send(b"q", 0.5)


def listening(t, out, env):
    """#23: Cockpit's listening Pane from a seeded log; a 31 s play is logged live; a backup is taken; export; delete."""
    data = env["SINGULARITY_DATA"]
    t.send(b"\x0b", 0.3); t.send(b"design", 0.3); t.send(b"\r", 0.6)
    for _ in range(3):
        t.send(b"\x1b[B", 0.4)  # down to Cockpit
    t.send(b"\r", 1.0)
    t.shot(f"{out}/0-cockpit.png")
    before = t.text()
    t.send(b" ", 31.5)  # play 31 s: one play
    t.send(b"n", 1.5)   # next: the play is logged, the Pane follows
    t.shot(f"{out}/1-after-a-play.png")
    print("pane changed after a play:", before != t.text())
    print("backups:", sorted(os.listdir(f"{data}/backups")) if os.path.isdir(f"{data}/backups") else "none")
    t.send(b" ", 0.4)  # pause
    t.send(b"\x0b", 0.3); t.send(b"export listening", 0.3); t.send(b"\r", 1.0)
    t.shot(f"{out}/2-exported.png")
    dl = env["XDG_DOWNLOAD_DIR"]
    for f in os.listdir(dl):
        st = os.stat(f"{dl}/{f}")
        doc = json.load(open(f"{dl}/{f}"))
        print(f"export {f}: mode {oct(st.st_mode & 0o777)}, {len(doc['plays'])} plays, backup liked {len((doc['library_backup'] or {}).get('liked', []))}, owned {len((doc['library_backup'] or {}).get('playlists', []))}, followed {len((doc['library_backup'] or {}).get('followed', []))}")
    print("modes: db", oct(os.stat(f"{data}/listening.db").st_mode & 0o777), "backups dir", oct(os.stat(f"{data}/backups").st_mode & 0o777) if os.path.isdir(f"{data}/backups") else "-")
    for _ in range(2):
        t.send(b"\x0b", 0.3); t.send(b"delete listening", 0.3); t.send(b"\r", 0.8)
        t.shot(f"{out}/3-delete-{_}.png")
    print("after delete: backups", os.path.isdir(f"{data}/backups"), "| pane:", [l for l in t.screen.display if "streak" in l or "nothing logged" in l][:1])
    t.send(b"q", 0.5)


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
               XDG_CONFIG_HOME=f"{os.path.abspath(out)}/config", XDG_STATE_HOME=f"{os.path.abspath(out)}/state",
               XDG_DATA_HOME=f"{os.path.abspath(out)}/data")
    for k in ("KITTY_WINDOW_ID", "TERM_PROGRAM", "GHOSTTY_RESOURCES_DIR", "TMUX"):
        env.pop(k, None)
    if scenario == "pixel":
        env["SINGULARITY_PIXEL_MOUSE"] = "1"
    cat = ["--no-catalogue"]
    if scenario.startswith("idle"):
        env["SINGULARITY_PIXEL"] = "1" if scenario == "idle-pixels" else "0"
    if scenario.startswith("well"):
        env["SINGULARITY_PIXEL"] = "1" if scenario in ("well-pixels", "well-hover") else "0"
    if scenario.startswith("home") or scenario in ("library", "queue", "lyrics", "artist", "search", "like", "config", "designs", "layouts", "listening", "wheel", "home-scroll", "history", "meters", "stats", "stats-widgets", "ambient", "arrows", "popup", "stalls", "queue-lift", "mini", "playlists", "playlist-details"):
        here = os.path.dirname(os.path.abspath(__file__))
        env["XDG_CACHE_HOME"] = f"{here}/xdgcache"
        env["SINGULARITY_PIXEL"] = "1" if scenario in ("home-pixels", "designs", "home-scroll", "ambient", "popup") else "0"
        # kitty's layer under cell backgrounds (#41); the harness composites it as kitty does
        env["SINGULARITY_BELOW_BG"] = "1" if scenario in ("popup", "ambient") else "0"
        cat = ["--fake-catalogue", f"{ROOT}/crates/client/tests/fixtures/home.json"]
    if scenario == "listening":
        env["SINGULARITY_DATA"] = f"{os.path.abspath(out)}/data/singularity"
        env["XDG_DOWNLOAD_DIR"] = f"{os.path.abspath(out)}/downloads"
        env["SINGULARITY_BACKUP_CHECK_SECS"] = "2"
        os.makedirs(env["XDG_DOWNLOAD_DIR"], exist_ok=True)
        seed_listening(env["SINGULARITY_DATA"])
    if scenario == "stalls":
        env["SINGULARITY_FAKE_STALLS"] = "1"
    if scenario in ("stats", "stats-widgets"):
        env["SINGULARITY_DATA"] = f"{os.path.abspath(out)}/data/singularity"
        seed_listening(env["SINGULARITY_DATA"])
    bus = None
    if scenario == "mpris":
        bus, addr = private_bus(out)
        env["SINGULARITY_DBUS"] = env["DBUS_SESSION_BUS_ADDRESS"] = addr
    d = subprocess.Popen([f"{BIN}/singularityd", "--fake-engine", *cat], env=env, stdout=subprocess.DEVNULL, stderr=open(f"{out}/daemon.log", "w"))
    try:
        time.sleep(0.4)
        t = Term(env)
        t.pump(1.5)
        if scenario == "home-scroll":
            return home_scroll(t, out)
        if scenario.startswith("home"):
            return home(t, out)
        if scenario == "library":
            return library(t, out)
        if scenario == "like":
            return like(t, out)
        if scenario == "mpris":
            return mpris(t, out, env)
        if scenario.startswith("idle"):
            return idle(t, out)
        if scenario == "listening":
            return listening(t, out, env)
        if scenario == "stats":
            return stats(t, out)
        if scenario == "stats-widgets":
            return stats_widgets(t, out)
        if scenario == "ambient":
            return ambient(t, out)
        if scenario == "arrows":
            return arrows(t, out)
        if scenario == "popup":
            return popup(t, out)
        if scenario == "stalls":
            return stalls(t, out)
        if scenario == "queue-lift":
            return queue_lift(t, out)
        if scenario == "playlists":
            return playlists(t, out)
        if scenario == "playlist-details":
            return playlist_details(t, out)
        if scenario == "mini":
            return mini(t, out)
        if scenario == "wheel":
            return wheel(t, out)
        if scenario == "history":
            return history(t, out)
        if scenario == "meters":
            return meters(t, out)
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
            for row in (3, 5, 7):
                rx, ry = t.find(f"Fake Track 00{row}")
                mouse(t, 2, rx + 2, ry, wait=0.3)  # right-click
                t.send(b"\x1b[B", 0.1)  # down: "Play next"
                t.send(b"\r", 0.8)
            t.shot(f"{out}/0-queued.png")
            # #32: drag the last queued song (track 3, "fake2") onto the first (track 7, "fake6"): press, move (32 = left
            # held), release
            (sx, sy), (dx, dy) = t.find("Described fake2 ·"), t.find("Described fake6 ·")
            mouse(t, 0, sx + 2, sy)
            mouse(t, 32, sx + 2, sy - 1, wait=0.15)
            mouse(t, 32, dx + 2, dy, wait=0.3)
            t.shot(f"{out}/1-dragging.png")
            mouse(t, 0, dx + 2, dy, release=True, wait=1.0)
            t.shot(f"{out}/2-dropped.png")
            order = [n for n in ("fake2", "fake4", "fake6") if t.find(f"Described {n} ·")]
            rows = sorted((t.find(f"Described {n} ·")[1], n) for n in order)
            print("queue after the drop:", [n for _, n in rows])
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
