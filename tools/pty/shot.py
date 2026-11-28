"""Drive the real `singularity` binary in a pseudo-terminal and screenshot it (pyte + PIL).

usage: venv/bin/python shot.py <scenario> <outdir>
Starts its own singularityd (--fake-engine --no-catalogue) on a short test socket and kills only that.
"""
import json, base64, fcntl, os, pty, re, select, shutil, socket, struct, subprocess, sys, termios, threading, time, zlib
import pyte
from PIL import Image, ImageDraw, ImageFont

ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "../.."))
# PTY_BIN: other binaries (a release build with symbols, for profiling); PTY_WRAP: start the Client under a tool
# ("heaptrack -o out/heap"), split on spaces
BIN = os.environ.get("PTY_BIN") or f"{ROOT}/target/debug"
WRAP = os.environ.get("PTY_WRAP", "").split()
COLS, ROWS, CW, CH = int(os.environ.get("PTY_COLS", 140)), int(os.environ.get("PTY_ROWS", 40)), 10, 20
# the tiling scenario wants room for the whole Singularity layout
if sys.argv[1:2] in (["tiling"], ["tiling-fast"], ["tiling-paths"], ["pane-menu"], ["paneless"], ["arrange"], ["arrange-keys"], ["arrange-fixes"], ["new-design"]):
    COLS, ROWS = int(os.environ.get("PTY_COLS", 160)), int(os.environ.get("PTY_ROWS", 48))
# the wheel everywhere: short Panes, so everything holds more than fits
if sys.argv[1:2] == ["wheel-all"]:
    COLS, ROWS = int(os.environ.get("PTY_COLS", 120)), int(os.environ.get("PTY_ROWS", 26))


def first_font(*paths):
    """The first of these fonts that exists (Linux paths first, then macOS's)."""
    for p in paths:
        p = os.path.expanduser(p)
        if os.path.exists(p):
            return ImageFont.truetype(p, 16)
    raise SystemExit(f"none of these fonts is installed: {paths}")


BRAILLE = first_font("/usr/share/fonts/google-noto/NotoSansSymbols2-Regular.ttf", "/System/Library/Fonts/Apple Braille.ttf")
FONT = first_font("/usr/share/fonts/nerd-fonts/DejaVuSansMono/DejaVuSansMNerdFontMono-Regular.ttf",
                  "~/Library/Fonts/DejaVuSansMNerdFontMono-Regular.ttf", "~/Library/Fonts/HackNerdFontMono-Regular.ttf",
                  "/System/Library/Fonts/Menlo.ttc")


# lyrics in other scripts (#146): (first, last code point, a font that has them). Used only for a character FONT
# lacks, and only where the file exists (elsewhere it's drawn in FONT, as a box), so other shots stay as they were.
SCRIPTS = [(0x0900, 0x097F, "/usr/share/fonts/google-noto-vf/NotoSansDevanagari[wght].ttf"),
           (0x0E00, 0x0E7F, "/usr/share/fonts/google-noto-vf/NotoSansThai[wght].ttf"),
           (0x3000, 0x9FFF, "/usr/share/fonts/google-noto-sans-mono-cjk-vf-fonts/NotoSansMonoCJK-VF.ttc")]
LOADED = {}


def lacks(ch):
    """Whether FONT has no glyph for `ch` (it draws the same box as for a code point no font has)."""
    def drawn(c):
        im = Image.new("L", (40, 24))
        ImageDraw.Draw(im).text((0, 0), c, font=FONT, fill=255)
        return im.tobytes()
    if "box" not in LOADED:
        LOADED["box"] = drawn("\U0010FFFD")
    return drawn(ch) == LOADED["box"]


def font_for(ch):
    o = ord(ch[0])
    if 0x2800 <= o <= 0x28FF:
        return BRAILLE
    for lo, hi, path in SCRIPTS:
        if lo <= o <= hi and os.path.exists(path) and lacks(ch[0]):
            if path not in LOADED:
                LOADED[path] = ImageFont.truetype(path, 16)
            return LOADED[path]
    return FONT


# test sockets (short paths) go here: $XDG_RUNTIME_DIR on Linux, an owner-only dir in /tmp where there is none (macOS)
RUNTIME = os.environ.get("XDG_RUNTIME_DIR") or f"/tmp/sgl-{os.getuid()}"
os.makedirs(RUNTIME, mode=0o700, exist_ok=True)


class Term:
    def __init__(self, env):
        self.screen = pyte.Screen(COLS, ROWS)
        self.stream = pyte.ByteStream(self.screen)
        pid, fd = pty.fork()
        if pid == 0:
            if WRAP:
                os.execvpe(WRAP[0], [*WRAP, f"{BIN}/singularity"], env)
            os.execve(f"{BIN}/singularity", ["singularity"], env)
        self.pid, self.fd = pid, fd
        self.pending = b""
        self.images = {}      # kitty image id -> PIL image
        self.placed = {}      # (id, pid) -> (col, row, cols, rows)
        self.upload = None    # (id, w, h, chunks) while a chunked transmission is in flight
        self.apc_log = []
        self.cup = (0, 0)  # last cursor move seen in the output (row, col), across reads
        self.osc22 = []  # every pointer shape asked for (OSC 22), in order; "" is a reset to the terminal's own
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
        self.osc22 += [m.decode() for m in re.findall(rb"\x1b\]22;([^\x1b\x07]*)", data)]
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
                # RGBA images (the karaoke ball) keep their alpha: pasted through it, as kitty blends them
                self.images[iid] = Image.frombytes("RGBA" if f == "32" else "RGB", (w, h), raw)
                self.upload = None
        elif a == "p":
            row, col = self.cup
            # x,y,w,h: only that part of the image (a cover partly scrolled out of its Pane)
            crop = tuple(int(kv[k]) for k in "xywh") if all(k in kv for k in "xywh") else None
            self.placed[(int(kv["i"]), int(kv["p"]))] = (col, row, int(kv["c"]), int(kv["r"]), crop, int(kv.get("z", "0")))
        elif a == "d":
            if kv.get("d") == "i" and "p" in kv:
                self.placed.pop((int(kv["i"]), int(kv["p"])), None)
            elif kv.get("d") in ("I", "i"):
                # without a placement id: all of that image's placements (d=i keeps the image, d=I frees it too)
                for k in [k for k in self.placed if k[0] == int(kv["i"])]:
                    self.placed.pop(k)
            elif kv.get("d") in ("A", "a"):
                self.placed.clear()

    def send(self, b, wait=0.15):
        try:
            os.write(self.fd, b)
        except OSError:
            # the Client has quit (Esc with nothing open quits too): the closing keys of a scenario have nothing to do
            if os.waitpid(self.pid, os.WNOHANG) != (0, 0):
                return
            raise
        self.pump(wait)

    def text(self):
        try:
            return "\n".join(self.screen.display)
        except IndexError:
            # pyte's own trips over an empty cell no wide character covers (a mark drawn over one, #146)
            return "\n".join("".join(self.screen.buffer[y][x].data for x in range(COLS)) for y in range(ROWS))

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
            # lower z first, as kitty stacks them (glow's field sits one under its cover)
            for (iid, _), (cx, cy, c, r, crop, z) in sorted(self.placed.items(), key=lambda kv: kv[1][5]):
                if iid in self.images and (z < -1073741824) == below:
                    src = self.images[iid]
                    if crop:
                        x, y, w, h = crop
                        src = src.crop((x, y, x + w, y + h))
                    src = src.resize((c * CW, r * CH))
                    img.paste(src, (cx * CW, cy * CH), src if src.mode == "RGBA" else None)
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
            font = font_for(ch)
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
        # the shelf's covers on screen too, not just its heading at the bottom edge
        if (f := t.find("Recommended Stations")) and f[1] <= hy + 12:
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
    x, y = t.find("♥  Liked Songs")
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


def home_search(t, out):
    """D68–D72: Home's search bar on top, shortcuts without empty slots, a one-Card Shelf as a spotlight; the bar
    opens Search in place, and the library has no Search row."""
    t.pump(1.5)
    t.shot(f"{out}/0-home.png")
    print("library Search row gone:", t.find("⌕  Search") is None, "· bar:", t.find("⌕ Search") is not None)
    x, y = t.find("Made For You") or t.find("Daily Mix 1")
    for _ in range(12):
        if t.find("New release from"):
            break
        mouse(t, 65, x, y + 2, wait=0.3)
    for _ in range(3):  # the whole spotlight in view
        mouse(t, 65, x, y + 2, wait=0.3)
    t.shot(f"{out}/1-spotlight.png")
    bx, by = t.find("⌕ Search")
    mouse(t, 0, bx + 4, by)
    mouse(t, 0, bx + 4, by, release=True, wait=0.6)
    t.shot(f"{out}/2-search-open.png")
    t.send(b"mix", 1.2)
    t.shot(f"{out}/3-search-typed.png")
    t.send(b"\x1b", 0.3); t.send(b"\x1b", 0.3)
    t.shot(f"{out}/4-back-home.png")
    t.send(b"q", 0.5)


def history(t, out):
    """#28/#43: ← → at the right of the main Pane's title: open a list, back to Home, forward again."""
    t.pump(1.0)
    x, y = t.find("♥  Liked Songs")
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


def jam(t, out):
    """D64: the device picker's Jam row: start one (`j`), its invite link, QR and guest, the bar's Jam chip; end it."""
    t.pump(1.0)
    t.send(b"d", 0.8)
    t.shot(f"{out}/0-picker.png")
    print("start row:", t.find("Start a Jam on Singularity") is not None)
    t.send(b"j", 1.0)
    t.shot(f"{out}/1-jam.png")
    print("jam row:", t.find("Jam on Singularity · 1 guest") is not None, "link:", t.find("socialsession/FakeJamToken123") is not None,
          "qr:", t.find("█▀▀▀▀▀█") is not None, "guest:", t.find("Guest Friend") is not None)
    t.send(b"\x1b", 0.5)
    print("bar chip:", t.find("◎ Jam · 1") is not None)
    t.shot(f"{out}/2-bar.png")
    t.send(b"d", 0.6)
    t.send(b"j", 0.4)  # asks again
    t.send(b"j", 1.0)
    t.shot(f"{out}/3-ended.png")
    print("ended:", t.find("Start a Jam on Singularity") is not None)
    t.send(b"\x1b", 0.4)


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
    """#26, D86: the wheel scrolls a track list (also over artist/album names) and the library; the highlight stays."""
    t.pump(1.0)
    x, y = t.find("♥  Liked Songs")
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


def wheel_burst(t, out):
    """Ghostty sends three wheel reports per notch in one write: each burst moves the list one row (D86: the
    highlight stays, so Enter plays the first song)."""
    t.pump(1.0)
    x, y = t.find("♥  Liked Songs")
    mouse(t, 0, x + 2, y)
    mouse(t, 0, x + 2, y, release=True, wait=0.8)
    rx, ry = t.find("Fake Track 001")
    for _ in range(2):
        t.send(f"\x1b[<65;{rx + 3};{ry + 1}M".encode() * 3, 0.1)  # one notch
    t.pump(0.4)
    t.shot(f"{out}/0-two-notches.png")
    t.send(b"\r", 1.0)  # play the highlighted song: the first (the wheel left it there)
    t.shot(f"{out}/1-playing.png")
    t.send(b" ", 0.3)
    t.send(b"q", 0.5)


def touchpad(t, out):
    """D86 (#32): touchpad-like bursts of wheel reports against single mouse-wheel notches over Liked Songs. The
    reports go as Ghostty sends them (the owner's recording, `crates/client/tests/fixtures/wheel-touchpad-ghostty.txt`):
    five wheel notches (three reports in one write each), a fast flick (eleven reports in 44 ms, some in one write), a
    slow drag back up (ten reports 30 to 400 ms apart); then the pointer over a row highlights it and a click picks it.
    Prints the first song shown and the highlighted one after each; `<n>-<what>.png`."""
    t.pump(1.0)
    x, y = t.find("♥  Liked Songs")
    mouse(t, 0, x + 2, y)
    mouse(t, 0, x + 2, y, release=True, wait=0.8)
    rx, ry = t.find("Fake Track 001")

    def report(code, k=1):
        return f"\x1b[<{code};{rx + 3};{ry + 4}M".encode() * k

    def rows():
        # the songs on screen, top to bottom, and the one whose background differs from the others' (the highlight)
        shown = [(yy, line) for yy, line in enumerate(t.screen.display) if "Fake Track" in line]
        bgs = [t.screen.buffer[yy][rx + 2].bg for yy, _ in shown]
        lit = [line for (yy, line), bg in zip(shown, bgs) if bgs.count(bg) == 1]
        name = lambda line: line[line.find("Fake Track"):][:14]
        return name(shown[0][1]) if shown else None, name(lit[0]) if lit else None

    def step(i, what):
        t.pump(0.4)
        t.shot(f"{out}/{i}-{what}.png")
        first, lit = rows()
        print(f"{i} {what}: first shown {first!r}, highlighted {lit!r}")

    step(0, "start")
    for _ in range(5):
        t.send(report(65, 3), 0.04)  # a wheel notch
    step(1, "five-wheel-notches")
    # the flick, with its gaps (ms): two at once, three at once, two close together, then four
    for k, gap in [(2, 23.7), (3, 6.7), (1, 0.1), (1, 12.8), (1, 0.4), (1, 0.1), (2, 0)]:
        t.send(report(65, k), gap / 1000)
    step(2, "fast-flick")
    for gap in [66.9, 49.7, 133.7, 49.6, 34.2, 50.0, 83.3, 116.7, 100.1, 0]:
        t.send(report(64), gap / 1000)
    step(3, "slow-drag-up")
    # the pointer over the sixth row: it lights; a click there picks it (a second click plays it)
    shown = [yy for yy, line in enumerate(t.screen.display) if "Fake Track" in line]
    hy = shown[5]
    t.send(f"\x1b[<35;{rx + 3};{hy + 1}M".encode(), 0.3)
    step(4, "hover-sixth-row")
    mouse(t, 0, rx + 2, hy)
    mouse(t, 0, rx + 2, hy, release=True, wait=0.3)
    mouse(t, 0, rx + 2, hy)
    mouse(t, 0, rx + 2, hy, release=True, wait=1.0)
    step(5, "double-click-plays-it")
    print("playing:", next((l.strip()[:60] for l in t.screen.display[-4:] if "Fake Track" in l), None))
    t.send(b" ", 0.3)
    t.send(b"q", 0.5)


# library | home | lyrics over queue, on a short screen (wheel-all)
WHEEL_LAYOUT = """name = "Wheel"
design = "singularity"
tree = { split = { axis = "horizontal", first = { cells = 28 }, a = { slot = "library" }, b = { split = { axis = "horizontal", first = { ratio = 0.6 }, a = { slot = "home" }, b = { split = { axis = "vertical", first = { ratio = 0.5 }, a = { slot = "lyrics" }, b = { slot = "queue" } } } } } } }
"""


def wheel_all(t, out):
    """The wheel over everything that holds more than fits, on a short screen (120×26 by default): Home's Shelves, the
    library, a track list, an artist page (popular tracks, then its Shelves), search (recent searches, then All, Songs,
    Artists, Podcasts and Audiobooks), a Show page, Stats, Settings, synced lyrics (held, then following again), the
    queue, Karaoke (nothing: the line being sung) and top this week. Each place: a shot before and after a few notches,
    each notch three reports as Ghostty sends them; `<n>-<place>-0.png` / `-1.png`."""
    # where the Panes are (WHEEL_LAYOUT): the body runs from row 1 to five rows above the bottom
    top, bottom = 1, ROWS - 5
    lib, home, right = (2, 26), (30, 81), (85, COLS - 2)
    mid = (top + bottom) // 2
    n = [0]

    def notches(x, y, k, down=True):
        for _ in range(k):
            t.send(f"\x1b[<{65 if down else 64};{x + 1};{y + 1}M".encode() * 3, 0.06)
        t.pump(0.4)

    def pane(x, y):
        # the Pane's cells (by WHEEL_LAYOUT), so the player bar's clock doesn't count as moving
        cols = next(c for c in (lib, home, right) if c[0] - 2 <= x <= c[1] + 1)
        rows = (top, bottom) if cols != right else ((top, mid) if y < mid else (mid, bottom))
        # (colours too: a highlight that moves changes no text)
        return [(c.data, c.fg, c.bg) for y in range(*rows) for x in range(*cols) for c in [t.screen.buffer[y][x]]]

    def place(name, x, y, k, down=True, settle=0.0):
        t.pump(settle)
        t.shot(f"{out}/{n[0]:02}-{name}-0.png")
        before = pane(x, y)
        notches(x, y, k, down)
        t.shot(f"{out}/{n[0]:02}-{name}-1.png")
        print(f"{n[0]:02} {name}: {'moved' if pane(x, y) != before else 'nothing moved'}")
        n[0] += 1

    def find_in(s, cols):
        for y, line in enumerate(t.screen.display[:bottom]):
            x = line.find(s, cols[0], cols[1])
            if x >= 0:
                return x, y
        return None

    def click(x, y, wait=0.8):
        mouse(t, 0, x, y); mouse(t, 0, x, y, release=True, wait=wait)

    t.pump(1.0)
    place("home-shelves", home[0] + 10, mid, 4)
    place("library", lib[0] + 4, mid, 6)
    # Liked Songs: play its first song (lyrics and a queue for the Panes on the right), wheel down the list
    x, y = t.find("♥  Liked Songs")
    click(x + 2, y)
    t.send(b"\r", 1.0)
    place("track-list", home[0] + 10, mid, 12)
    # an artist page from a row's artist link
    x, y = find_in("Nova Drift", home)
    click(x + 1, y, 1.2)
    rx, ry = find_in("Fake Track", home) or (home[0] + 6, top + 5)
    place("artist-popular", rx + 2, ry, 6)
    place("artist-shelves", home[0] + 10, bottom - 3, 4)
    # search: the recent searches (seeded), then results under each chip
    t.send(b"\x0c", 0.8)  # Ctrl+L
    place("search-recent", home[0] + 10, top + 6, 8)
    t.send(b"al", 1.5)  # (the fake Catalogue: eight songs, and Cards whose names have "al" in them)
    rx, ry = find_in("al song", home) or (home[0] + 6, top + 5)
    place("search-all-songs", rx + 2, ry, 3)
    place("search-all-shelves", home[0] + 10, bottom - 3, 4)
    t.send(b"\t", 0.6)
    place("search-songs", home[0] + 10, mid, 7)
    t.send(b"\t\t", 0.6)
    place("search-albums", home[0] + 10, mid, 4)
    t.send(b"\t", 0.6)
    place("search-playlists", home[0] + 10, mid, 4)
    t.send(b"\t", 0.8)
    rx, ry = find_in("Episode 1", home) or (home[0] + 6, top + 5)
    place("search-podcasts-episodes", rx + 2, ry, 3)
    # a Show page from the Podcasts chip
    at = find_in("Fake Sh", home)
    if at:
        click(at[0] + 1, at[1], 1.2)
        place("show-page", home[0] + 10, mid, 12)
    # Stats and Settings
    t.send(b"\x0b", 0.3); t.send(b"stats", 0.3); t.send(b"\r", 2.0)
    place("stats", home[0] + 10, mid, 8)
    t.send(b"\x0b", 0.3); t.send(b"settings", 0.3); t.send(b"\r", 0.8)
    place("settings", home[0] + 10, mid, 8)
    t.send(b"\x1b", 0.6)
    # the Widgets on the right: synced lyrics are held where the wheel puts them, then follow the song again
    lyr = (right[0] + 6, top + 4)
    place("lyrics-synced", *lyr, 4)
    t.pump(5.5)
    t.shot(f"{out}/{n[0] - 1:02}-lyrics-synced-2-following-again.png")
    place("queue", right[0] + 6, bottom - 3, 6)
    swap_in(t, out, "tail -f lyrics.log", "karaoke")
    place("karaoke-synced", *lyr, 3, settle=0.5)
    swap_in(t, out, "karaoke", "top artists")
    x, y = t.find("top artists")
    click(x + 1, y)  # its tracks: more of them than fit
    place("top-this-week", *lyr, 4, settle=0.5)
    t.send(b" ", 0.3)
    t.send(b"q", 0.5)


# the fake account's made-up personal values (SINGULARITY_FAKE_PERSONAL=1, crates/daemon/src/catalogue/fake.rs), a
# recent search and a play in the real listening log, and an engine.device of the owner's choosing: demo mode must
# draw none of them (D90)
DEMO_SECRETS = ["zq-fakeuser", "Zara Quillfeather", "Quillfeather", "Bodhi Friendly", "zarabox", "Zara's Pixel", "Zara's Kitchen",
                "Zara's Secret Diary", "Private Stuff", "ZqSecretJamToken42", "socialsession", "zara secret search",
                "Secretartist Zq", "Zara's Desk", "Zara's Secret Song", "Zara's Queued Secret", "Secret Album"]


def demo_secrets():
    """DEMO_SECRETS, plus this computer's and user's own names (looked up here, never written down)."""
    found = {s: s for s in DEMO_SECRETS}
    for label, value in (("the computer's name", open("/etc/hostname").read().strip() if os.path.exists("/etc/hostname") else ""),
                         ("the user's name", os.environ.get("USER", "")), ("the home folder", os.environ.get("HOME", ""))):
        if len(value) > 2:
            found[label] = value
    return found


def demo_check(out):
    """In demo mode, every screen dump in `out` searched for each secret: prints what showed, and fails if anything did."""
    if os.environ.get("SINGULARITY_DEMO") != "1" and os.environ.get("PTY_DEMO") != "1":
        return
    seen = {}
    for f in sorted(os.listdir(out)):
        if f.endswith(".txt"):
            text = open(f"{out}/{f}").read().lower()
            for label, s in demo_secrets().items():
                if s.lower() in text:
                    seen.setdefault(label, []).append(f)
    print("demo check:", {k: v[:3] for k, v in seen.items()} or "no secret in any screen")
    if seen:
        sys.exit(f"demo mode drew personal values: {sorted(seen)}")


def demo_setup(env, out):
    """The scratch files the demo walk needs: a real recent search, a play by a secret artist in the real log, a
    device name of the owner's own choosing."""
    os.makedirs(f"{env['XDG_STATE_HOME']}/singularity", exist_ok=True)
    open(f"{env['XDG_STATE_HOME']}/singularity/recent-searches.json", "w").write(json.dumps(["zara secret search"]))
    os.makedirs(f"{env['XDG_CONFIG_HOME']}/singularity", exist_ok=True)
    open(f"{env['XDG_CONFIG_HOME']}/singularity/config.toml", "w").write("schema = 6\n[engine]\ndevice = \"Zara's Desk\"\n")
    env["SINGULARITY_DATA"] = f"{os.path.abspath(out)}/data/singularity"
    seed_listening(env["SINGULARITY_DATA"])
    import sqlite3
    db = sqlite3.connect(f"{env['SINGULARITY_DATA']}/listening.db")
    now = int(time.time() * 1000)
    db.execute("INSERT INTO plays (started_ms, day, track_uri, name, artists, album, context_uri, duration_ms, played_ms, device) VALUES (?,?,?,?,?,?,?,?,?,?)",
               (now - 3_600_000, (now - 3_600_000) // 86_400_000, "spotify:track:secret", "Secret Song", json.dumps(["Secretartist Zq"]), "Secret Album", None, 200_000, 9_000_000, "Zara's Pixel"))
    db.commit()
    db.close()


def demo(t, out, env):
    """D90: demo mode on the fake account with personal values (`SINGULARITY_FAKE_PERSONAL=1`) and a personal song left
    playing (`SINGULARITY_FAKE_CARRYOVER=1`, #128): Home, the library, a playlist and one of its songs played, Search
    (recent searches, results), the device picker and a Jam, Stats, Settings, the health check, Cockpit's stream
    Widget, a toast from a refused playlist. Every drawn screen is searched for each secret; prints what showed.
    `PTY_DEMO=0` runs the same walk without the switch (the secrets must show there)."""
    secrets = demo_secrets()
    on = env.get("SINGULARITY_DEMO") == "1"
    seen = {}
    n = [0]

    def look(name, wait=0.0):
        t.pump(wait)
        t.shot(f"{out}/{n[0]:02}-{name}.png")
        screen = t.text()
        for label, s in secrets.items():
            if s.lower() in screen.lower():
                seen.setdefault(label, []).append(name)
        marks = [m for m in ("DEMO", "Premium", "Invite link hidden in demo mode", "Make a folder called Demo", "Guest 1", "you", "a friend") if m in screen]
        print(f"{n[0]:02} {name}: {', '.join(marks) or '-'}")
        n[0] += 1

    def palette(what, wait=1.0):
        t.send(b"\x0b", 0.3)
        t.send(what.encode(), 0.3)
        t.send(b"\r", wait)

    t.pump(2.5)
    look("home")
    if on == any("Secret Song" in l for l in t.screen.display):
        sys.exit(f"the carried-over song is {'drawn' if on else 'missing'}")
    t.send(b"\x1b[B", 0.2)  # a key, so the Home view has a selection; then the shelves below
    for _ in range(6):
        t.send(b"\x1b[B", 0.1)
    look("home-scrolled", 0.5)
    at = t.find("Fake Mixtape")
    if at:
        mouse(t, 0, at[0] + 2, at[1]); mouse(t, 0, at[0] + 2, at[1], release=True, wait=1.0)
        look("playlist")
        # (#128) a Demo playlist's song played from here: the player shows it (the carried-over one it didn't)
        sx = t.find("Fake Track 003")
        if sx:
            for _ in range(2):  # double-click plays it
                mouse(t, 0, sx[0] + 2, sx[1], wait=0.03); mouse(t, 0, sx[0] + 2, sx[1], release=True, wait=0.05)
            look("playing-demo-song", 1.5)
            if not any("Track 2" in l and "—" in l for l in t.screen.display):
                sys.exit("the Demo playlist's song played from here isn't shown")
            # next: the song queued in the normal session plays, under the Demo playlist; demo mode still hides it
            t.send(b"n", 0.2)
            look("queued-secret-plays", 1.0)
            if on and not any("now-playing --follow  # nothing playing" in l for l in t.screen.display):
                sys.exit("the song queued in the normal session is shown")
            t.send(b"n", 0.2)
            look("next-demo-song", 1.0)
            if not any("Track 1" in l and "—" in l for l in t.screen.display):
                sys.exit("the Demo playlist's next song isn't shown")
        mouse(t, 2, at[0] + 2, at[1], wait=0.4)
        look("library-menu")
        t.send(b"\x1b", 0.2)
        # an artist page from a row's artist link: its Shelves hold the private playlist too
        ax = t.find("Nova Drift")
        if ax:
            mouse(t, 0, ax[0] + 1, ax[1]); mouse(t, 0, ax[0] + 1, ax[1], release=True, wait=1.2)
            look("artist")
            for _ in range(8):
                t.send(f"\x1b[<65;{ax[0] + 2};{ax[1] + 4}M".encode() * 3, 0.1)
            look("artist-shelves", 0.4)
    at = t.find("Road Trip")
    if at:
        mouse(t, 0, at[0] + 2, at[1]); mouse(t, 0, at[0] + 2, at[1], release=True, wait=1.0)
        look("friends-playlist")
    t.send(b"\x0c", 0.8)  # Ctrl+L: Search, empty
    look("search-recent")
    t.send(b"mix", 1.5)
    look("search-results")
    t.send(b"\x1b", 0.3); t.send(b"\x1b", 0.3)
    t.send(b"d", 0.8)
    look("devices")
    t.send(b"j", 1.0)
    look("jam")
    t.send(b"\x1b", 0.4)
    look("jam-chip")
    palette("stats", 2.0)
    look("stats")
    palette("settings")
    look("settings")
    t.send(b"\x1b", 0.4)
    palette("health check")
    look("health", 0.5)
    t.send(b"\x1b", 0.4)
    palette("new playlist")
    t.send(b"failing list", 0.3)
    t.send(b"\r", 1.0)
    look("toast")
    palette("design", 0.6)
    for _ in range(3):
        t.send(b"\x1b[B", 0.3)  # Cockpit: the stream and listening Widgets
    t.send(b"\r", 1.0)
    # (a song plays already: the Demo playlist's, or without the switch the carried-over one)
    look("cockpit-playing", 2.0)
    print("demo mode" if on else "without the switch", "· secrets drawn:", {k: v[:3] for k, v in seen.items()} or "none")
    if on and seen:
        sys.exit(f"demo mode drew personal values: {sorted(seen)}")
    t.send(b"q", 0.5)


def popup_wheel(t, out):
    """Wheel events never reach the Panes behind a popup: Devices and Settings are opened over an open list, and a
    notch (three reports, as Ghostty sends) goes inside the popup, on its border and outside it, over the library,
    the list, the queue and the bars. The highlights behind must not move; the popup's own must."""
    t.pump(1.0)
    x, y = t.find("♥  Liked Songs")
    mouse(t, 0, x + 2, y)
    mouse(t, 0, x + 2, y, release=True, wait=0.8)
    rx, ry = t.find("Fake Track 001")
    mouse(t, 0, rx + 2, ry)
    mouse(t, 0, rx + 2, ry, release=True, wait=0.3)

    def notch(px, py, down=True):
        t.send(f"\x1b[<{65 if down else 64};{px + 1};{py + 1}M".encode() * 3, 0.1)

    def look():
        # the highlight is a background: the library column and the list's name column, as colours
        b = t.screen.buffer
        return [(yy, b[yy][2].bg, b[yy][2].reverse, b[yy][rx + 1].bg, b[yy][rx + 1].reverse) for yy in range(3, ROWS - 4)]

    before = look()
    t.send(b"d", 0.8)
    t.shot(f"{out}/0-devices.png")
    px, py = t.find("Play on")
    spots = [("inside", px + 3, py + 2), ("border", px - 1, py + 1), ("top border", px + 3, py - 1), ("off the edge", px + 60, py + 1),
             ("library", 2, 12), ("list", rx + 3, ry + 4), ("corner", COLS - 2, ROWS - 6), ("bar", COLS // 2, ROWS - 3)]
    for name, sx, sy in spots:
        for down in (True, True, False):
            notch(min(sx, COLS - 1), min(sy, ROWS - 1), down)
    t.shot(f"{out}/1-devices-wheeled.png")
    t.send(b"\x1b", 0.6)
    t.shot(f"{out}/2-devices-closed.png")
    print("devices: background unchanged:", look() == before)
    # Settings: a page of the main Pane, which keeps the wheel when the pointer is off it
    t.send(b"\x0b", 0.3); t.send(b"settings", 0.3); t.send(b"\r", 0.8)
    t.shot(f"{out}/3-settings.png")
    page0 = [l for l in t.screen.display[3:ROWS - 4]]
    lib0 = [r[:3] for r in look()]
    for name, sx, sy in [("library", 2, 12), ("library low", 4, ROWS - 8), ("bar", COLS // 2, ROWS - 3)] * 2:
        for down in (True, True, True):
            notch(sx, sy, down)
    t.shot(f"{out}/4-settings-wheeled.png")
    lib1 = [r[:3] for r in look()]
    print("settings: library unchanged:", lib0 == lib1, "| page scrolled:", page0 != list(t.screen.display[3:ROWS - 4]))
    t.send(b"\x1b", 0.6)

    def chosen(title):
        # the popup's highlighted row: below its title, the one whose background isn't the popup's
        tx, ty = t.find(title)
        b = t.screen.buffer
        rows = list(range(ty + 1, min(ty + 20, ROWS - 1)))
        bgs = [b[yy][tx + 1].bg for yy in rows]
        usual = max(set(bgs), key=bgs.count)
        return next((t.screen.display[yy][tx:tx + 28].strip(" ›") for yy, c in zip(rows, bgs) if c != usual), None)

    def wheel_popup(title, shot, spots_down, spots_up):
        px, py = t.find(title)
        first = chosen(title)
        bg = look()
        for sx, sy in spots_down(px, py):
            notch(min(sx, COLS - 1), min(sy, ROWS - 1))
        t.shot(f"{out}/{shot}-down.png")
        down = chosen(title)
        for sx, sy in spots_up(px, py):
            notch(min(sx, COLS - 1), min(sy, ROWS - 1), down=False)
        t.shot(f"{out}/{shot}-up.png")
        print(f"{title}: {first!r} → wheel down → {down!r} → up → {chosen(title)!r} | background unchanged:", look() == bg)

    # "add a pane": right-click a Pane's title → the Split row (→ lit) → an empty Pane and the Widget picker on it
    at = t.find("┌ ~/library") or t.find("~/library")
    mouse(t, 2, at[0] + 3, at[1]); mouse(t, 2, at[0] + 3, at[1], release=True, wait=0.4)
    t.shot(f"{out}/5-title-menu.png")
    t.send(b"\x1b[B", 0.2)  # past Replace widget…
    t.send(b"\r", 0.6)  # Split →
    t.shot(f"{out}/6-widget-list.png")
    # three notches: inside, on the border, far off it; then two back, over the library and the bar
    wheel_popup("Choose a widget", "7-widget-list", lambda px, py: [(px + 4, py + 3), (px - 2, py + 2), (2, ROWS - 6)],
                lambda px, py: [(2, 12), (COLS // 2, ROWS - 3)])
    t.send(b"\x1b", 0.5)
    # the first edit of the built-in Design asks for a name once the picker is gone: Esc keeps it unnamed
    if t.find("Keep your changes"):
        t.send(b"\x1b", 0.5)
    # the palette: a long list that scrolls with its selection
    t.send(b"\x0b", 0.6)
    wheel_popup("Command palette", "8-palette", lambda px, py: [(px + 4, py + 4)] * 4 + [(2, 12)] * 16 + [(COLS - 2, 2)] * 4,
                lambda px, py: [(px + 4, py + 4)] * 2)
    t.send(b"\x1b", 0.5)
    t.send(b"q", 0.5)


def like(t, out):
    """#22: the ♥ in the player bar, `l`, unliking from a row's menu, Follow on an artist page."""
    t.pump(1.0)
    x, y = t.find("♥  Liked Songs")
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
    mx, my = t.find("Remove from Liked Songs")  # by name: the menu grew (Add to playlist…, #53)
    mouse(t, 0, mx + 1, my); mouse(t, 0, mx + 1, my, release=True, wait=0.8)
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
    # socket paths must be short: under RUNTIME, removed with the bus
    sockdir = f"{RUNTIME}/sgl-bus-{os.getpid()}"
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
    x, y = t.find("♥  Liked Songs")
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


def seed_backup(data, short, account="fake"):
    """A scratch library backup dated today, the fake Catalogue's account's (`account`): whole, or (#144) with Liked
    Songs short beside its whole copy and a playlist saved marked incomplete."""
    os.makedirs(f"{data}/backups", exist_ok=True)
    b = {"schema": 1, "taken_ms": int(time.time() * 1000), "account": account, "liked": [], "liked_total": 0, "followed": [],
         "playlists": [{"uri": "spotify:playlist:fake", "name": "Fake List", "tracks": [], "total": 0}]}
    if short:
        b["liked_short"] = {"taken_ms": b["taken_ms"], "incomplete": "only 0 of its 3 tracks came back", "total": 3, "tracks": []}
        b["playlists"][0]["incomplete"] = "the page at 0 came back without its tracks (Something went wrong)"
    open(f"{data}/backups/library-{time.strftime('%Y-%m-%d')}.json", "w").write(json.dumps(b))


def backup_notice(t, out):
    """#144: Settings with the newest backup short (PTY_BACKUP=whole: whole; =other: another account's); the screen's
    text goes beside the shot. The backup task reads the backup a couple of seconds after start."""
    t.pump(3.5)
    t.send(b"\x0b", 0.3); t.send(b"settings", 0.3); t.send(b"\r", 1.0)
    t.shot(f"{out}/settings.png")
    open(f"{out}/screen.txt", "w").write("\n".join(t.screen.display))
    print("backup line:", [l.strip() for l in t.screen.display if "backup" in l.lower()])
    t.send(b"q", 0.5)


def settings(t, out, env):
    """#57: the Settings page from config.toml: a click flips a setting, → steps the design, a typed number, an edit
    made in the file shows on the page; the file keeps its comments."""
    cfg = f"{env['XDG_CONFIG_HOME']}/singularity/config.toml"
    t.pump(1.0)
    t.send(b"\x0b", 0.3); t.send(b"settings", 0.3); t.send(b"\r", 0.8)
    t.shot(f"{out}/0-settings.png")
    x, y = t.find("transparent")
    mouse(t, 0, x + 1, y); mouse(t, 0, x + 1, y, release=True, wait=0.8)  # a click flips it
    t.shot(f"{out}/1-transparent-on.png")
    t.send(b"\x1b[A", 0.1); t.send(b"\x1b[A", 0.1)  # up to design
    t.send(b"\x1b[C", 1.0)  # →: the next design
    t.shot(f"{out}/2-next-design.png")
    for _ in range(7):
        t.send(b"\x1b[B", 0.1)  # down to unfocused_fps
    t.send(b"\r", 0.2); t.send(b"24", 0.3)
    t.shot(f"{out}/3-typing.png")
    t.send(b"\r", 0.8)
    text = open(cfg).read()
    print("file:", [l for l in text.splitlines() if l.startswith(("design", "transparent", "unfocused_fps"))], "| comments kept:", text.count("#"))
    # an edit in the file shows within a second
    open(cfg, "w").write(text.replace("minutes = 3", "minutes = 7"))
    t.pump(1.6)
    t.send(b"\x1b[F", 0.4)  # End: the last setting
    t.shot(f"{out}/4-edited-in-the-file.png")
    print("page:", [l.strip()[:90] for l in t.screen.display if "minutes" in l or "unfocused_fps" in l][:3])
    # #58: the keys tab, by a click; a binding added in keys.toml shows within a second
    x, y = t.find("keys.toml")
    mouse(t, 0, x + 1, y); mouse(t, 0, x + 1, y, release=True, wait=0.6)
    t.shot(f"{out}/5-keys.png")
    kp = f"{env['XDG_CONFIG_HOME']}/singularity/keys.toml"
    keys = open(kp).read().replace("# like = \"f\"", "stats = \"Ctrl+T\"\nnope = \"x\"")
    open(kp, "w").write(keys)
    t.pump(1.6)
    for _ in range(41):
        mouse(t, 65, x + 4, y + 6, wait=0.02)  # the wheel, down to Stats
    t.pump(0.4)
    t.shot(f"{out}/6-keys-edited.png")
    print("keys:", [l.strip()[30:120] for l in t.screen.display if "^T" in l or "nope" in l][:3])
    t.send(b"\x1b", 0.4)  # Esc: back
    t.send(b"q", 0.5)


def show(t, out):
    """#59/#60: a show from search, followed into the library: its episodes with progress, one played; older ones load near the end; an episode
    card opens its show at that episode."""
    t.pump(1.0)
    t.send(b"\x0c", 0.3); t.send(b"fake", 1.2)  # Ctrl+L, search
    t.shot(f"{out}/0-search.png")
    # #61: the Podcasts chip lists episodes above the shows
    cx, cy = t.find(" Podcasts ")
    mouse(t, 0, cx + 2, cy); mouse(t, 0, cx + 2, cy, release=True, wait=0.6)
    t.shot(f"{out}/0b-podcasts-chip.png")
    print("episodes:", [l.strip()[30:120] for l in t.screen.display if "min left" in l or "played" in l][:2])
    x, y = t.find("Fake Sh")
    mouse(t, 0, x + 1, y); mouse(t, 0, x + 1, y, release=True, wait=1.0)
    t.shot(f"{out}/1-show.png")
    # #60: Follow puts it in the library
    fx, fy = t.find("♡ Follow")
    mouse(t, 0, fx + 1, fy); mouse(t, 0, fx + 1, fy, release=True, wait=1.2)
    t.shot(f"{out}/1b-followed.png")
    print("library row:", [l[:30].strip() for l in t.screen.display if "Fake Show" in l[:32]][:1], "| chip:", bool(t.find("♥ Following")))
    x, y = t.find("Episode 129")
    for _ in range(2):  # double-click plays it
        mouse(t, 0, x + 2, y, wait=0.03); mouse(t, 0, x + 2, y, release=True, wait=0.05)
    t.pump(1.5)
    t.shot(f"{out}/2-playing.png")
    print("now playing:", [l.strip()[:80] for l in t.screen.display if "now-playing" in l][:1])
    t.send(b"\x1b[F", 1.0)  # End: older episodes load
    t.send(b"\x1b[F", 0.6)
    t.shot(f"{out}/3-end.png")
    print("last rows:", [l.strip()[30:90] for l in t.screen.display if "Episode 1:" in l or "Episode 2:" in l][:2])
    t.send(b"\x1b", 0.6)  # back to search
    x, y = t.find("Episode…")
    mouse(t, 0, x + 1, y); mouse(t, 0, x + 1, y, release=True, wait=1.5)
    t.shot(f"{out}/4-episode-card.png")
    t.send(b"q", 0.5)


def book(t, out):
    """#62: a book under the Audiobooks chip, its page (chapters, progress, Spotify's access line), a chapter plays."""
    t.pump(1.0)
    t.send(b"\x0c", 0.3); t.send(b"fake", 1.2)  # Ctrl+L, search
    cx, cy = t.find(" Audiobooks ")
    mouse(t, 0, cx + 2, cy); mouse(t, 0, cx + 2, cy, release=True, wait=0.6)
    t.shot(f"{out}/0-audiobooks-chip.png")
    print("card:", [l.strip()[30:110] for l in t.screen.display if "Listen" in l][:1])
    x, y = t.find("The Lon")
    mouse(t, 0, x + 1, y); mouse(t, 0, x + 1, y, release=True, wait=1.0)
    t.shot(f"{out}/1-book.png")
    print("header:", [l.strip()[30:130] for l in t.screen.display if "chapters" in l or "Audiobook ·" in l][:2])
    x, y = t.find("Chapter 2: The Long Orbit")
    for _ in range(2):  # double-click plays it
        mouse(t, 0, x + 2, y, wait=0.03); mouse(t, 0, x + 2, y, release=True, wait=0.05)
    t.pump(1.5)
    t.shot(f"{out}/2-playing.png")
    print("playing chapter 2:", any("♪" in l and "Chapter 2" in l for l in t.screen.display))
    t.send(b"q", 0.5)


def queue_wheel(t, out):
    """The wheel over the queue scrolls it one line a notch and leaves its highlight (D86; as in the lists, #26)."""
    t.pump(1.0)
    x, y = t.find("♥  Liked Songs")
    mouse(t, 0, x + 2, y); mouse(t, 0, x + 2, y, release=True, wait=0.8)
    rx, ry = t.find("Fake Track 002")
    for _ in range(2):  # double-click plays from here: the list's next songs show as "Next from"
        mouse(t, 0, rx + 2, ry, wait=0.03); mouse(t, 0, rx + 2, ry, release=True, wait=0.05)
    t.pump(1.5)
    for row in range(3, 12):  # queue nine more, so the list is longer than the Pane
        rx, ry = t.find(f"Fake Track {row:03}")
        mouse(t, 2, rx + 2, ry, wait=0.3)  # right-click → Play next
        t.send(b"\x1b[B", 0.1); t.send(b"\r", 0.6)
    qx, qy = t.find("┌ queue")
    for n, steps in enumerate((3, 4, 30)):
        for _ in range(steps):
            mouse(t, 65, qx + 4, qy + 4, wait=0.05)  # wheel down over the queue
        t.pump(0.4)
        t.shot(f"{out}/{n}-down-{steps}.png")
    print("selected:", [l.strip()[:60] for l in t.screen.display if "Described" in l][:12])
    mouse(t, 64, qx + 4, qy + 4, wait=0.3)  # wheel up: one back
    t.shot(f"{out}/3-up-1.png")
    t.send(b"q", 0.5)


def queue_lift(t, out):
    """#50: play from Liked Songs (the fake Engine adds the list's next songs as "Next from"), queue one song, then
    drag a "Next from" song up onto it: it becomes queued there."""
    t.pump(1.0)
    x, y = t.find("♥  Liked Songs")
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


def playlist_add(t, out):
    """#53: Add to playlist… from a track's menu: the picker filtered as you type, a duplicate asked first, a new
    playlist made with the song in it."""
    t.pump(1.0)

    def add_from_first_row():
        x, y = t.find("Fake Track 001")
        mouse(t, 2, x + 1, y, wait=0.4)
        mx, my = t.find("Add to playlist")
        mouse(t, 0, mx + 1, my); mouse(t, 0, mx + 1, my, release=True, wait=0.4)

    x, y = t.find("♥  Liked Songs")
    mouse(t, 0, x + 2, y); mouse(t, 0, x + 2, y, release=True, wait=1.0)
    add_from_first_row()
    t.shot(f"{out}/0-picker.png")
    px, py = t.find("New playlist")
    mouse(t, 65, px, py, wait=0.3)  # wheel down over the picker
    t.shot(f"{out}/0b-wheeled.png")
    print("wheel picks:", [l.split("›")[-1].strip()[:20] for l in t.screen.display if "› " in l and "Mixtape" in l][:1], "| list behind still on row 1:", any("› " not in l and " 1  Fake Track 001" in l for l in t.screen.display))
    mouse(t, 64, px, py, wait=0.3)  # and back up
    t.send(b"mix", 0.3)
    t.shot(f"{out}/1-filtered.png")
    print("filtered:", [l.strip()[:60] for l in t.screen.display if "New playlist" in l or "Fake Mixtape" in l and "›" in l][:3])
    t.send(b"\x1b[B\r", 1.0)  # Down, Enter: Fake Mixtape, which has it
    t.shot(f"{out}/2-already.png")
    print("asks:", [l.strip()[-80:] for l in t.screen.display if "Already in" in l][:1])
    add_from_first_row()
    t.send(b"mix\x1b[B\r", 1.0)
    print("anyway:", [l.strip()[-40:] for l in t.screen.display if "Added to" in l][:1])
    add_from_first_row()
    t.send(b"Gym", 0.3)
    t.send(b"\r", 1.2)
    t.shot(f"{out}/3-new.png")
    print("new:", [l.strip()[-40:] for l in t.screen.display if "Added to Gym" in l][:1], "| in sidebar:", any("Gym" in l[:30] for l in t.screen.display))
    x, y = t.find("Gym")
    mouse(t, 0, x + 1, y); mouse(t, 0, x + 1, y, release=True, wait=1.0)
    t.shot(f"{out}/4-gym.png")
    print("Gym holds:", [l.strip()[:60] for l in t.screen.display[3:8] if "Added" in l or "songs" in l])
    t.send(b"q", 0.5)


def playlist_remove(t, out):
    """#54: Remove from this playlist (menu) and the Delete key on your own playlist; nothing offered on others'."""
    t.pump(1.0)
    x, y = t.find("Fake Mixtape")
    mouse(t, 0, x + 1, y); mouse(t, 0, x + 1, y, release=True, wait=1.0)
    count = lambda: [l.strip()[:50] for l in t.screen.display[:3] if "songs" in l]
    print("before:", count())
    x, y = t.find("Fake Track 002")
    mouse(t, 2, x + 1, y, wait=0.4)
    t.shot(f"{out}/0-menu.png")
    mx, my = t.find("Remove from this playlist")
    mouse(t, 0, mx + 1, my); mouse(t, 0, mx + 1, my, release=True, wait=0.8)
    t.shot(f"{out}/1-removed.png")
    print("after menu:", count(), "| 002 gone:", not any("Fake Track 002" in l and "Removed" not in l for l in t.screen.display), "| toast:", [l.strip()[-30:] for l in t.screen.display if "Removed" in l][:1])
    t.send(b"\x1b[3~", 0.8)  # Delete: the selected row
    t.shot(f"{out}/2-deleted-key.png")
    print("after Delete:", count())
    x, y = t.find("Daily Mix 1")
    mouse(t, 0, x + 1, y); mouse(t, 0, x + 1, y, release=True, wait=1.0)
    x, y = t.find("Fake Track 001")
    mouse(t, 2, x + 1, y, wait=0.4)
    print("their menu offers Remove:", any("Remove from this" in l for l in t.screen.display))
    t.send(b"q", 0.5)


def playlist_move(t, out):
    """#55: drag a row of your own playlist to a new place (preview while dragging), Alt+↓, and a sorted view saying why
    it can't."""
    t.pump(1.0)
    x, y = t.find("Fake Mixtape")
    mouse(t, 0, x + 1, y); mouse(t, 0, x + 1, y, release=True, wait=1.0)
    order = lambda: [l.split("Fake Track ")[1][:3] for l in t.screen.display if "Fake Track " in l][:6]
    print("before:", order())
    src, dst = t.find("Fake Track 001"), t.find("Fake Track 004")
    mouse(t, 0, src[0] + 2, src[1])
    mouse(t, 32, src[0] + 2, src[1] + 1, wait=0.15)
    mouse(t, 32, dst[0] + 2, dst[1], wait=0.3)
    t.shot(f"{out}/0-dragging.png")
    print("preview:", order())
    mouse(t, 0, dst[0] + 2, dst[1], release=True, wait=1.0)
    t.shot(f"{out}/1-dropped.png")
    print("dropped:", order())
    t.send(b"\x1b[1;3B", 0.8)  # Alt+Down: the selected row one place down
    print("Alt+Down:", order())
    hx, hy = t.find("Title")
    mouse(t, 0, hx + 1, hy); mouse(t, 0, hx + 1, hy, release=True, wait=0.4)
    t.send(b"\x1b[1;3A", 0.5)  # Alt+Up in a sorted view
    t.shot(f"{out}/2-sorted.png")
    print("sorted says:", [l.strip()[-60:] for l in t.screen.display if "Clear the sort" in l][:1])
    t.send(b"q", 0.5)


def playlist_cover(t, out):
    """#56: Change cover… on your own playlist: a picture dropped (a pasted file:// path), its square preview, the upload;
    then Remove cover."""
    import os
    from PIL import Image
    pic = os.path.join(os.path.abspath(out), "my cover.png")
    w, h = 900, 600
    im = Image.new("RGB", (w, h))
    im.putdata([(255, 0, 0) if x < 150 else (x * 255 // w, y * 255 // h, 160) for y in range(h) for x in range(w)])
    im.save(pic)
    t.pump(1.0)
    x, y = t.find("Fake Mixtape")
    mouse(t, 0, x + 1, y); mouse(t, 0, x + 1, y, release=True, wait=1.0)
    t.shot(f"{out}/0-sidebar.png")
    mouse(t, 2, x + 1, y, wait=0.4)
    mx, my = t.find("Change cover")
    mouse(t, 0, mx + 1, my); mouse(t, 0, mx + 1, my, release=True, wait=0.4)
    t.shot(f"{out}/0-form.png")
    uri = "file://" + pic.replace(" ", "%20")
    t.send(b"\x1b[200~" + uri.encode() + b"\x1b[201~", 1.0)  # what a terminal sends for a dropped file
    t.shot(f"{out}/1-preview.png")
    print("preview:", [l.strip()[-50:] for l in t.screen.display if "KB JPEG" in l or "No file" in l or "Not a picture" in l][:1])
    t.send(b"\r", 1.0)
    t.shot(f"{out}/2-uploaded.png")
    print("upload:", [l.strip()[-50:] for l in t.screen.display if "Uploading" in l or "Cover changed" in l or "Couldn't" in l][:1])
    mouse(t, 2, x + 1, y, wait=0.4)
    mx, my = t.find("Remove cover")
    mouse(t, 0, mx + 1, my); mouse(t, 0, mx + 1, my, release=True, wait=0.8)
    print("remove:", [l.strip()[-50:] for l in t.screen.display if "Putting back" in l or "Cover changed" in l or "Couldn't" in l][:1])
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
    x, y = t.find("♥  Liked Songs")
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


BIG_LAYOUT = """name = "Big widgets"
design = "singularity"
tree = { split = { axis = "vertical", first = { cells = 5 },
    a = { slot = "home" },
    b = { split = { axis = "vertical", first = { ratio = 0.5 },
        a = { split = { axis = "horizontal", first = { ratio = 0.333 }, a = { slot = "stream" },
            b = { split = { axis = "horizontal", first = { ratio = 0.5 }, a = { slot = "top" }, b = { slot = "listening" } } } } },
        b = { split = { axis = "horizontal", first = { ratio = 0.333 }, a = { slot = "today" },
            b = { split = { axis = "horizontal", first = { ratio = 0.5 }, a = { slot = "topweek" }, b = { slot = "queue" } } } } } } } } }
"""


def seed_today(data):
    """A morning's plays today (fake tracks) on top of `seed_listening`, so "today" and "top this week" have this week."""
    import random, sqlite3
    db = sqlite3.connect(f"{data}/listening.db")
    rnd = random.Random(11)
    now = time.time()
    off = time.localtime(now).tm_gmtoff
    day = int((now + off) // 86400)
    start = (day * 86400 - off) + 7 * 3600
    for i in range(18):
        a = rnd.choice(["Nova Drift", "Mira Vale", "Lumen Fields", "Glass Harbour", "Saffron Echo", "The Quiet Orbit"])
        n = rnd.randint(1, 12)
        t = min(start + i * 600, now - 60)
        db.execute("INSERT INTO plays (started_ms, day, track_uri, name, artists, album, duration_ms, played_ms, device) VALUES (?,?,?,?,?,?,?,?,?)",
                   (int(t * 1000), day, f"spotify:track:fake{n}", f"Fake Track {n:03}", f'["{a}"]', f"{a} · Album {n % 3 + 1}", 200000, rnd.randint(90, 200) * 1000, "here"))
    db.commit()
    db.close()


RESIZE_LAYOUT = """name = "Wide"
design = "cockpit"
theme = "ember"
tree = { split = { axis = "vertical", first = { ratio = 0.43548387 },
    a = { split = { axis = "horizontal", first = { ratio = 0.25714287 },
        a = { slot = "marquee" },
        b = { split = { axis = "horizontal", first = { ratio = 0.5 }, a = { slot = "scope" }, b = { slot = "hole" } } },
    } },
    b = { split = { axis = "vertical", first = { ratio = 0.3 },
        a = { split = { axis = "horizontal", first = { ratio = 0.7080292 },
            a = { split = { axis = "horizontal", first = { ratio = 0.5 }, a = { slot = "fft" }, b = { slot = "nowplaying" } } },
            b = { slot = "starfield" },
        } },
        b = { split = { axis = "horizontal", first = { ratio = 0.52 },
            a = { slot = "home" },
            b = { split = { axis = "horizontal", first = { ratio = 0.5 }, a = { slot = "lyrics" }, b = { slot = "library" } } },
        } },
    } },
} }
"""


def resize(t, out):
    """The owner's 2026-11-02 report: Home can't be made narrower. A full-screen-sized Cockpit edit (Home | lyrics |
    library along the bottom): drag the Home | lyrics border left, then the border above Home up; a shot after each,
    and Home's size printed. Also the lyrics Pane dragged by its title onto Home's right edge (a size-only drop: refused,
    and the bar should say how to resize). Run at PTY_COLS=264 PTY_ROWS=68."""
    t.pump(1.5)
    t.shot(f"{out}/0-start.png")
    def home_rect():
        x, y = t.find("\u250c home") or t.find("\u256d home")
        row = "".join(t.screen.buffer[y][i].data for i in range(t.screen.columns))
        right = next(i for i in range(x + 1, len(row)) if row[i] in "\u2510\u256e")
        col = [t.screen.buffer[r][x].data for r in range(t.screen.lines)]
        bottom = next(r for r in range(y + 1, t.screen.lines) if col[r] in "\u2514\u2570")
        return x, y, right, bottom
    x, y, right, bottom = home_rect()
    print("home before:", right - x + 1, "x", bottom - y + 1)
    # the border between Home and lyrics: Home's right edge, halfway down
    gx, gy = right, (y + bottom) // 2
    mouse(t, 35, gx, gy, wait=0.4)  # hover: the border lights up
    t.shot(f"{out}/0b-hover.png")
    print("pointer on the border:", t.osc22[-1:])
    mouse(t, 35, gx - 6, gy, wait=0.4)
    print("pointer off it:", t.osc22[-2:])
    mouse(t, 35, gx, gy, wait=0.3)
    mouse(t, 0, gx, gy)
    for i in range(1, 9):
        mouse(t, 32, gx - 4 * i, gy, wait=0.1)
    mouse(t, 0, gx - 32, gy, release=True, wait=0.6)
    t.shot(f"{out}/1-narrower.png")
    # off the border it was dragged to (it sits 32 columns left now)
    mouse(t, 35, gx - 40, gy, wait=0.4)
    print("pointer after the drag, moved off:", t.osc22[-2:])
    x, y, right, bottom = home_rect()
    print("home after the side border:", right - x + 1, "x", bottom - y + 1)
    # the border above Home (its title row is the move handle; the Pane above's bottom border row is the grip)
    gx, gy = x + 10, y - 1
    mouse(t, 0, gx, gy)
    for i in range(1, 6):
        mouse(t, 32, gx, gy - 2 * i, wait=0.1)
    mouse(t, 0, gx, gy - 10, release=True, wait=0.6)
    t.shot(f"{out}/2-taller.png")
    x, y, right, bottom = home_rect()
    print("home after the top border:", right - x + 1, "x", bottom - y + 1)
    # the lyrics Pane by its title onto Home's right edge: the same arrangement, only sizes: refused
    lx, ly = t.find("lyrics")
    mouse(t, 0, lx + 1, ly)
    for i in range(1, 7):
        mouse(t, 32, lx + 1 - (lx - right + 3) * i // 6, ly + 6 * i // 6, wait=0.15)
    t.pump(0.4)
    mouse(t, 0, right - 2, ly + 6, release=True, wait=0.8)
    t.shot(f"{out}/3-title-drop.png")
    print("hint shown:", bool(t.find("drag the border between two Panes")))
    # (Ghostty ignores an empty OSC 22: a reset names `text` first)
    print("pointer shapes asked for:", t.osc22)
    t.send(b"q", 0.5)
    t.pump(0.5)
    print("pointer at exit:", t.osc22[-2:])


def space_startup(t, out, env):
    """The owner (2026-11-05): Space pressed fast as the app launches left the bar showing paused while it played.
    Space is mashed while the Client is still connecting and starting, then the bar is compared with a second Client's
    (a fresh snapshot: the player's real state), then Space pressed a few more times, each compared again."""
    for gap in (0.05, 0.1, 0.1, 0.2, 0.15, 0.3, 0.1):
        t.send(b" ", gap)
    t.pump(2.5)
    t.shot(f"{out}/0-after-mashing.png")
    def shown(term):
        # (❚❚ while playing, ▶ otherwise; "▶|" is always there)
        return "playing" if term.find("\u275a\u275a") else "not playing"
    b = Term(env)
    b.pump(1.5)
    b.shot(f"{out}/0-second-client.png")
    print("after mashing: this client", shown(t), "| a fresh client (the player)", shown(b))
    ok = shown(t) == shown(b)
    for i in range(1, 5):
        t.send(b" ", 0.05 + 0.1 * i)
        t.pump(2.0)
        b.pump(0.3)
        print(f"press {i}: this client", shown(t), "| the player", shown(b))
        ok &= shown(t) == shown(b)
    t.shot(f"{out}/1-end.png")
    print("in step:", ok)
    b.send(b"q", 0.3)
    t.send(b"q", 0.5)


# every Widget a Slot can show, and the sizes (inside the frame) the `sizes` scenario shoots each at
SIZES_WIDGETS = ["lyrics", "queue", "nowplaying", "stream", "top", "listening", "today", "topweek", "marquee", "glow",
                 "fft", "loudness", "levels", "scope", "rain", "starfield", "library", "home"]
SIZES = {"tiny": (10, 2), "small": (20, 6), "narrow": (16, 30), "wide": (110, 4), "medium": (46, 16),
         "large": (90, 34), "huge": (256, 64)}


def sizes_layout(subject, w, h):
    """A layout with `subject` in a Pane of exactly w×h inside its frame (top left), Home (or the library) beside it
    and the rest of the four every Design keeps in a row underneath."""
    core = ["home", "library", "queue", "lyrics"]
    filler = "library" if subject == "home" else "home"
    rest = [c for c in core if c not in (subject, filler)]
    def row(names):
        if len(names) == 1:
            return f'{{ slot = "{names[0]}" }}'
        return f'{{ split = {{ axis = "horizontal", first = {{ ratio = {1 / len(names):.4f} }}, a = {{ slot = "{names[0]}" }}, b = {row(names[1:])} }} }}'
    top = f'{{ split = {{ axis = "horizontal", first = {{ cells = {w + 2} }}, a = {{ slot = "{subject}" }}, b = {{ slot = "{filler}" }} }} }}'
    return f'name = "Sizes"\ndesign = "singularity"\ntree = {{ split = {{ axis = "vertical", first = {{ cells = {h + 2} }}, a = {top}, b = {row(rest)} }} }}\n'


def sizes(env, out):
    """Every Widget (SIZES_WIDGETS) in a Pane of each size in SIZES (inside the frame: tiny 10×2 up to huge 256×64),
    a track playing on a healthy fake stream, about 30 s in. One Client per shot, the terminal sized to fit; shots are
    `<size>-<widget>.png`. PTY_WIDGETS / PTY_SIZES (comma-separated; a name or `<w>x<h>`) pick a subset. Also the mini-player at a few
    terminal sizes (`mini-<w>x<h>.png`; PTY_MINI=0 skips them). PTY_PIXEL=1 shoots on the pixel tier; PTY_SEEKS
    (comma-separated counts of 5 s seeks, default 6) shoots several moments of the song, `<size>-<widget>-<secs>s.png`;
    PTY_TRACK=2 plays the fake Catalogue's second track (the karaoke song) instead of the first."""
    global COLS, ROWS
    widgets = os.environ.get("PTY_WIDGETS", ",".join(SIZES_WIDGETS)).split(",")
    seeks = [int(n) for n in os.environ.get("PTY_SEEKS", "6").split(",")]
    second = os.environ.get("PTY_TRACK", "1") == "2"
    names = os.environ.get("PTY_SIZES", ",".join(SIZES)).split(",")
    cfg = f"{env['XDG_CONFIG_HOME']}/singularity"
    os.makedirs(f"{cfg}/layouts", exist_ok=True)
    started = False
    def client():
        t = Term(env)
        t.pump(1.5)
        return t
    def stop(t):
        t.send(b"q", 0.3)
        try:
            os.kill(t.pid, 9)
        except ProcessLookupError:
            pass
        os.waitpid(t.pid, 0)
        os.close(t.fd)
    for name in names:
        # a size's name, or any `<w>x<h>`
        w, h = SIZES[name] if name in SIZES else (int(n) for n in name.split("x"))
        # room for Home beside it and a row of Panes under it, and never so small the mini-player takes over
        COLS, ROWS = max(90, w + 2 + 14), max(30, h + 2 + 12)
        for widget in widgets:
            open(f"{cfg}/layouts/sizes.toml", "w").write(sizes_layout(widget, w, h))
            open(f"{cfg}/config.toml", "w").write('schema = 6\n[ui]\ndesign = "singularity"\nlayout = "sizes"\n')
            t = client()
            if not started:
                # play from Liked Songs (the queue gets a context), so the meters, the stream and lyrics have something
                x, y = t.find("Liked Songs") or (0, 0)
                mouse(t, 0, x + 2, y); mouse(t, 0, x + 2, y, release=True, wait=0.8)
                t.send(b"\r", 0.8)
                started = True
            # every shot at the same moment of a song: back a track and on again (both start it from 0), then 30 s in
            t.send(b"b", 0.2); t.send(b"n", 0.4)
            if second:
                t.send(b"n", 0.4)
            done = 0
            for n in seeks:
                t.send(b"\x1b[1;2C" * (n - done), 0.6)  # Shift+→ ×n
                done = n
                t.pump(1.0 if widget not in ("stream", "top", "loudness") else 3.5)
                tag = f"-{n * 5}s" if len(seeks) > 1 else ""
                t.shot(f"{out}/{name}-{widget}{tag}.png")
            stop(t)
    if os.environ.get("PTY_MINI", "1") == "1":
        open(f"{cfg}/config.toml", "w").write('schema = 6\n')
        for COLS, ROWS in [(20, 1), (36, 2), (49, 6), (49, 13), (24, 13)]:
            t = client()
            t.shot(f"{out}/mini-{COLS}x{ROWS}.png")
            stop(t)


DESIGNS = ["singularity", "event-horizon", "classic", "cockpit", "terminal", "karaoke"]
TERMS = ["80x24", "120x36", "160x48", "220x60", "264x68"]


def design_sizes(env, out):
    """Every built-in Design in its own Panes at several terminal sizes, a track playing about 30 s in, no popup. One
    Client per shot, `<design>-<cols>x<rows>.png`. PTY_DESIGNS / PTY_TERMS (comma-separated) pick a subset."""
    global COLS, ROWS
    designs = os.environ.get("PTY_DESIGNS", ",".join(DESIGNS)).split(",")
    terms = os.environ.get("PTY_TERMS", ",".join(TERMS)).split(",")
    cfg = f"{env['XDG_CONFIG_HOME']}/singularity"
    os.makedirs(cfg, exist_ok=True)
    started = False
    for term in terms:
        COLS, ROWS = (int(n) for n in term.split("x"))
        for design in designs:
            open(f"{cfg}/config.toml", "w").write(f'schema = 6\n[ui]\ndesign = "{design}"\n')
            t = Term(env)
            t.pump(1.5)
            if not started:
                t.send(b" ", 0.8)  # play
                started = True
            t.send(b"b", 0.2); t.send(b"n", 0.4)  # every shot at the same moment: track 2 from 0, then 30 s in
            t.send(b"\x1b[1;2C" * 6, 0.6)
            t.pump(2.0)
            t.shot(f"{out}/{design}-{term}.png")
            t.send(b"q", 0.3)
            try:
                os.kill(t.pid, 9)
            except ProcessLookupError:
                pass
            os.waitpid(t.pid, 0)
            os.close(t.fd)


# the contact sheet (#36, D87): every Widget, lyrics and Karaoke first; Pane sizes are the whole Pane, frame included
SHEET_WIDGETS = ["lyrics", "karaoke", "home", "library", "queue", "nowplaying", "well", "hole", "stream", "top",
                 "listening", "today", "topweek", "marquee", "glow", "fft", "loudness", "levels", "scope", "rain",
                 "starfield"]
SHEET_SIZES = {"tiny": (20, 6), "narrow-tall": (24, 30), "wide-short": (100, 8), "medium": (60, 20), "big": (120, 40),
               "full": None}
# the full screen of the sheet, and the full-terminal Design shots: (label, cols, rows, cell px w, cell px h)
SHEET_TERMS = [("pc", 264, 68, 10, 20), ("laptop-full", 180, 54, 16, 34), ("laptop-half", 90, 50, 16, 34)]
SHEET_FULL = (180, 54)


def set_cell_px(w, h):
    """The terminal's cell size in pixels (what the pty reports, so the pixel tier draws to it) and fonts to match."""
    global CW, CH, FONT, BRAILLE
    CW, CH = w, h
    size = round(h * 0.8)
    FONT = ImageFont.truetype(FONT.path, size)
    BRAILLE = ImageFont.truetype(BRAILLE.path, size)


def pane_rect(t, want, widget):
    """The top-left Pane's rect in cells (x, y, w, h), read off its frame in the screen. The well's frame sags (no
    corners to read): `want` (w, h) under the top bar, narrowed as its shape rule narrows it (4:3 in pixels)."""
    rows = t.screen.display
    y0 = next((y for y, line in enumerate(rows) if line[:1] in "╭┌╔┏"), None)
    if widget == "well" or y0 is None:
        w, h = want
        shaped = round((h - 2) * CH / CW * 4 / 3) + 2
        # (alone on the screen there's no neighbour to give the width to)
        return (0, 1, shaped if w > shaped + shaped // 20 and w < COLS else w, h)
    top = rows[y0]
    x1 = next((x for x in range(1, len(top)) if top[x] in "╮┐╗┓"), None)
    y1 = next((y for y in range(y0 + 1, len(rows)) if rows[y][:1] in "╰└╚┗"), None)
    if x1 is None or y1 is None:
        return (0, y0, *want)
    return (0, y0, x1 + 1, y1 - y0 + 1)


def bar_track(t):
    """Which of the fake Catalogue's tracks the player bar (the last rows) shows, or None."""
    for line in t.screen.display[-4:]:
        m = re.search(r"Track (\d+)", line)
        if m:
            return int(m[1])
    return None


def bar_secs(t):
    """Where the song is, as the player bar shows it (the time before the song's length, 3:20), or None."""
    for line in t.screen.display[-4:]:
        m = re.search(r"(\d+):(\d\d)\s.*3:20", line)
        if m:
            return int(m[1]) * 60 + int(m[2])
    return None


def sheet_font(size):
    return ImageFont.truetype(FONT.path, size)


def compose_sheet(path, title, cells):
    """One image of captioned cells, packed in rows: `cells` is [(caption, PIL image)], in order."""
    cap, gap, pad, maxw = 30, 28, 32, 3900
    font = sheet_font(20)
    # each cell as wide as its picture or its caption
    span = lambda c, im: max(im.width, int(font.getlength(c)))
    rows, row, x = [], [], 0
    for c, im in cells:
        if row and x + span(c, im) > maxw:
            rows.append(row)
            row, x = [], 0
        row.append((c, im))
        x += span(c, im) + gap
    if row:
        rows.append(row)
    width = max(sum(span(c, im) + gap for c, im in r) - gap for r in rows) + 2 * pad
    height = 80 + sum(cap + max(im.height for _, im in r) + gap for r in rows) + pad
    sheet = Image.new("RGB", (width, height), "#202024")
    d = ImageDraw.Draw(sheet)
    d.text((pad, 22), title, font=sheet_font(34), fill="#f0f0f0")
    y = 80
    for r in rows:
        x = pad
        for c, im in r:
            d.text((x, y + 2), c, font=font, fill="#c8c8d0")
            sheet.paste(im, (x, y + cap))
            d.rectangle([x - 1, y + cap - 1, x + im.width, y + cap + im.height], outline="#55555f")
            x += span(c, im) + gap
        y += cap + max(im.height for _, im in r) + gap
    sheet.save(path)


SHEET_HTML = """<!doctype html>
<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Size contact sheet</title>
<style>
:root { color-scheme: dark; --bg: #18181b; --fg: #e8e8ec; --dim: #9a9aa6; --good: #3fa56b; --bad: #d0473f; }
body { background: var(--bg); color: var(--fg); font: 15px/1.45 system-ui, sans-serif; margin: 0 16px 64px; }
h1 { font-size: 22px; margin: 20px 0 4px; } h2 { font-size: 18px; margin: 36px 0 8px; border-top: 1px solid #333; padding-top: 16px; }
p { color: var(--dim); max-width: 70ch; }
nav a { color: var(--dim); margin-right: 10px; }
.cells { display: flex; flex-wrap: wrap; gap: 18px; align-items: flex-start; }
figure { margin: 0; } figcaption { font: 13px ui-monospace, monospace; color: var(--dim); margin-bottom: 4px; display: flex; gap: 6px; align-items: center; }
figure img { display: block; max-width: min(100%, 92vw); outline: 1px solid #444; image-rendering: auto; }
figure.good img { outline: 3px solid var(--good); } figure.bad img { outline: 3px solid var(--bad); }
button { font: 12px system-ui; background: #2a2a30; color: var(--fg); border: 1px solid #444; border-radius: 4px; cursor: pointer; padding: 1px 7px; }
figure.good button.g { background: var(--good); } figure.bad button.b { background: var(--bad); }
textarea { width: 100%; max-width: 900px; height: 10em; background: #101013; color: var(--fg); border: 1px solid #444; font: 13px ui-monospace, monospace; }
</style></head><body>
<h1>Size contact sheet</h1>
<p>Every Widget alone in a Pane at six sizes (the whole Pane, frame included), on the cell tier and the pixel tier,
then each built-in Design at the PC's size and an emulated 13-inch laptop (Retina cells). Mark each cell good or bad;
the marks are kept in this browser and listed at the bottom to copy. Click an image for full size.</p>
<nav>NAV</nav>
BODY
<h2 id="marks">Marks</h2>
<p>Bad cells (with a note if you add one by clicking "bad" again):</p>
<textarea id="out" readonly></textarea>
<script>
const KEY = "contact-sheet-marks";
let marks = {};
try { marks = JSON.parse(localStorage.getItem(KEY) || "{}"); } catch (e) {}
function save() { try { localStorage.setItem(KEY, JSON.stringify(marks)); } catch (e) {} show(); }
function show() {
  document.querySelectorAll("figure[data-id]").forEach(f => {
    const m = marks[f.dataset.id];
    f.classList.toggle("good", !!m && m.v === "good");
    f.classList.toggle("bad", !!m && m.v === "bad");
  });
  const bad = Object.entries(marks).filter(([, m]) => m.v === "bad").map(([id, m]) => id + (m.note ? ": " + m.note : ""));
  document.getElementById("out").value = bad.join("\\n");
}
document.querySelectorAll("figure[data-id]").forEach(f => {
  f.querySelector("button.g").onclick = () => { marks[f.dataset.id] = { v: "good" }; save(); };
  f.querySelector("button.b").onclick = () => {
    const was = marks[f.dataset.id];
    const note = was && was.v === "bad" ? prompt("What's wrong here?", was.note || "") : "";
    marks[f.dataset.id] = { v: "bad", note: note || (was && was.note) || "" }; save();
  };
});
show();
</script></body></html>
"""


def sheet_html(sheet, add):
    """index.html for the sheet: every Widget's cells in order, then the Designs. `add` is {file: caption} of this
    run's shots, kept in the sheet's captions.json with the earlier runs' (a run of a few Widgets updates their cells)."""
    path = f"{sheet}/captions.json"
    captions = json.load(open(path)) if os.path.exists(path) else {}
    captions.update(add)
    json.dump(captions, open(path, "w"), indent=0, ensure_ascii=False)
    def fig(src):
        cid = captions[src]
        return (f'<figure data-id="{cid}"><figcaption>{cid} <button class="g">good</button> <button class="b">bad</button>'
                f'</figcaption><a href="{src}"><img src="{src}" loading="lazy" alt="{cid}"></a></figure>')
    order = {f"{w}-{n}-{t}": i for i, (w, n, t) in enumerate((w, n, t) for w in SHEET_WIDGETS for n in SHEET_SIZES for t in ("cells", "pixels"))}
    body, nav = [], []
    for w in SHEET_WIDGETS:
        cells = sorted((f for f in captions if f.startswith(f"cells/{w}-")), key=lambda f: order.get(f[6:-4], len(order)))
        if cells:
            nav.append(f'<a href="#{w}">{w}</a>')
            body.append(f'<h2 id="{w}">{w}</h2><p><a href="{w}.png">the whole sheet</a></p><div class="cells">'
                        + "".join(fig(f) for f in cells) + "</div>")
    designs = [f for f in captions if f.startswith("designs/")]
    if designs:
        nav.append('<a href="#designs">Designs</a>')
        body.append('<h2 id="designs">Designs, full terminal</h2><div class="cells">' + "".join(fig(f) for f in designs) + "</div>")
    nav.append('<a href="#marks">marks</a>')
    open(f"{sheet}/index.html", "w").write(SHEET_HTML.replace("NAV", " ".join(nav)).replace("BODY", "\n".join(body)))


SHEET_QUEUE = 60  # more than the full screen's queue shows


def fill_queue(env, n):
    """Queue `n` made-up songs over the Daemon's socket, as a Client would (the fake Catalogue names
    `spotify:track:song<i>` with a title, artist, album, length and cover each, #59)."""
    s = socket.socket(socket.AF_UNIX)
    s.connect(env["SINGULARITY_SOCKET"])
    # (drain what the Daemon sends: it writes its whole snapshot before reading anything)
    threading.Thread(target=lambda: [None for _ in iter(lambda: s.recv(65536), b"")], daemon=True).start()
    msgs = [{"t": "hello", "protocol": 1, "client": "contact-sheet"}]
    msgs += [{"t": "command", "cmd": "add_to_queue", "uri": f"spotify:track:song{i}"} for i in range(n)]
    for m in msgs:
        b = json.dumps(m).encode()
        s.sendall(struct.pack(">I", len(b)) + b)
    time.sleep(1.0)
    s.shutdown(socket.SHUT_RDWR)
    s.close()


def contact_sheet(env, out):
    """The size contact sheet (#36, D87): every Widget (SHEET_WIDGETS, lyrics and Karaoke first) alone in a Pane of
    each size in SHEET_SIZES (the whole Pane: tiny 20×6 up to big 120×40, and the full 180×54 screen) on the cell and
    the pixel tier, the fake Catalogue's second track (synced, word-timed lyrics) playing about 10 s in; one Client per
    shot. Then every built-in Design full-terminal at SHEET_TERMS (the PC's 264×68, a 13-inch laptop's 180×54 full
    screen and 90×50 half-screen window with Retina-sized 16×34 px cells) on both tiers. Raw shots go to `out`; the
    sheet (one captioned image per Widget, each cell on its own, the Design shots and an index.html to mark them in)
    goes to PTY_SHEET (default `<out>/sheet` when `out` is an ignored `out-*` folder, else
    `tools/pty/out-contact-sheet/sheet`); set it outside the repo to keep the sheet. PTY_WIDGETS /
    PTY_SIZES / PTY_TIERS (cells,pixels) / PTY_DESIGNS pick a subset; PTY_WIDGETS= skips the Widgets, PTY_DESIGNS= the
    Designs."""
    global COLS, ROWS
    widgets = [w for w in os.environ.get("PTY_WIDGETS", ",".join(SHEET_WIDGETS)).split(",") if w]
    sizes = os.environ.get("PTY_SIZES", ",".join(SHEET_SIZES)).split(",")
    tiers = os.environ.get("PTY_TIERS", "cells,pixels").split(",")
    designs = [d for d in os.environ.get("PTY_DESIGNS", ",".join(DESIGNS)).split(",") if d]
    # by default under the output folder when it is an ignored `out-*` one, else under tools/pty/out-contact-sheet: the
    # sheet's index.html and captions.json must never land untracked in the repo
    here = os.path.dirname(os.path.abspath(__file__))
    base = out if os.path.basename(os.path.abspath(out)).startswith("out-") else f"{here}/out-contact-sheet"
    sheet = os.environ.get("PTY_SHEET") or f"{base}/sheet"
    os.makedirs(f"{sheet}/cells", exist_ok=True)
    os.makedirs(f"{sheet}/designs", exist_ok=True)
    cfg = f"{env['XDG_CONFIG_HOME']}/singularity"
    os.makedirs(f"{cfg}/layouts", exist_ok=True)
    # a full queue, so the queue Widget is judged as it looks in use (#59)
    fill_queue(env, SHEET_QUEUE)
    started = [False]

    def run(tier, setup, shoot):
        """One Client on `tier`: set up its config, play track 2 from 0 (started once), seek, shoot, stop."""
        setup()
        t = Term(dict(env, SINGULARITY_PIXEL="1" if tier == "pixels" else "0"))
        t.pump(1.5)
        if not started[0]:
            t.send(b" ", 0.8)  # play (track 1 of the fake Catalogue)
            started[0] = True
        # every shot at about the same moment: track 2 (n/b until the player bar says so; each starts it from 0), then
        # 5 s seeks (Shift+→/←) until the bar is a few seconds short of 12 s, then wait for it
        for _ in range(6):
            n = bar_track(t)
            if n == 2:
                break
            t.send(b"n" if n is None or n < 2 else b"b", 0.6)
        for _ in range(8):
            at = bar_secs(t)
            if at is None or 0 <= 12 - at < 5:
                break
            t.send(b"\x1b[1;2C" if at < 12 else b"\x1b[1;2D", 0.6)
        t.pump(max(1.0, 12 - (at or 12)))
        shoot(t)
        t.send(b"q", 0.3)
        try:
            os.kill(t.pid, 9)
        except ProcessLookupError:
            pass
        os.waitpid(t.pid, 0)
        os.close(t.fd)

    set_cell_px(10, 20)
    for widget in widgets:
        cells, figs = [], []
        for name in sizes:
            for tier in tiers:
                if SHEET_SIZES.get(name, 0) is None:
                    COLS, ROWS = SHEET_FULL
                    layout = f'name = "Sizes"\ndesign = "singularity"\ntree = {{ slot = "{widget}" }}\n'
                    want = (SHEET_FULL[0], SHEET_FULL[1] - 5)  # under the top bar, over the player bar
                else:
                    # a size's name, or any `<w>x<h>`
                    w, h = SHEET_SIZES[name] if name in SHEET_SIZES else (int(n) for n in name.split("x"))
                    # room for Home beside it and a row of Panes under it, and never so small the mini-player takes over
                    COLS, ROWS = max(90, w + 16), max(30, h + 12)
                    layout = sizes_layout(widget, w - 2, h - 2)
                    want = (w, h)
                def setup():
                    open(f"{cfg}/layouts/sizes.toml", "w").write(layout)
                    open(f"{cfg}/config.toml", "w").write('schema = 6\n[ui]\ndesign = "singularity"\nlayout = "sizes"\n')
                raw = f"{out}/{widget}-{name}-{tier}.png"
                got = []
                def shoot(t):
                    t.shot(raw)
                    got.append(pane_rect(t, want, widget))
                run(tier, setup, shoot)
                x, y, w, h = got[0]
                crop = Image.open(raw).crop((x * CW, y * CH, (x + w) * CW, (y + h) * CH))
                cid = f"{widget} · {name} {w}×{h} · {tier}"
                fname = f"cells/{widget}-{name}-{tier}.png"
                crop.save(f"{sheet}/{fname}")
                cells.append((cid, crop))
                figs.append((cid, fname))
                print(cid, flush=True)
        compose_sheet(f"{sheet}/{widget}.png", widget, cells)
        sheet_html(sheet, {f: c for c, f in figs})
    shots = []
    for label, cols, rows, cw, ch in SHEET_TERMS:
        set_cell_px(cw, ch)
        COLS, ROWS = cols, rows
        for design in designs:
            for tier in tiers:
                fname = f"designs/{design}-{label}-{cols}x{rows}-{tier}.png"
                def setup():
                    open(f"{cfg}/config.toml", "w").write(f'schema = 6\n[ui]\ndesign = "{design}"\n')
                raw = f"{out}/{design}-{label}-{tier}.png"
                run(tier, setup, lambda t: t.shot(raw))
                shutil.copy(raw, f"{sheet}/{fname}")
                shots.append((f"{design} · {label} {cols}×{rows} @{cw}×{ch}px · {tier}", fname))
                print(shots[-1][0], flush=True)
    set_cell_px(10, 20)
    sheet_html(sheet, {f: c for c, f in shots})
    print(f"sheet: {sheet}/index.html")


def widgets_large(t, out):
    """The tiling Widgets (stream, top, listening, today, top this week) each in a big Pane of a saved layout, a
    track playing on a healthy fake stream; then top this week switched to tracks. Try PTY_COLS=240 PTY_ROWS=70 (Panes
    about 80×32) and 140×40 (about 46×17)."""
    t.send(b" ", 0.5)  # play: the stream and the processes have something to show
    t.send(b"\x0b", 0.3); t.send(b"design", 0.3); t.send(b"\r", 0.6)
    for _ in range(5):
        t.send(b"\x1b[B", 0.3)  # past the five designs to the saved layout
    t.send(b"\r", 1.0)
    t.pump(20.0)  # a graph's worth of seconds
    t.shot(f"{out}/0-widgets.png")
    print("panes:", [n for n in ("stream", "top · system", "listening", "today", "top artists") if t.find(n)])
    x, y = t.find("top artists")
    mouse(t, 0, x + 1, y); mouse(t, 0, x + 1, y, release=True, wait=0.8)
    t.shot(f"{out}/1-top-tracks.png")
    t.send(b" ", 0.3)
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
    names = ["singularity", "event-horizon", "classic", "cockpit", "terminal", "karaoke"]
    t.send(b"\x0b", 0.3); t.send(b"design", 0.3); t.send(b"\r", 0.6)
    for i, name in enumerate(names):
        if i:
            t.send(b"\x1b[B", 1.2)  # ↓ previews the next design
        t.shot(f"{out}/{i}-{name}.png")
    t.send(b"\r", 0.5)  # keep Karaoke
    t.send(b"\x0b", 0.3); t.send(b"design", 0.3); t.send(b"\r", 0.5)
    t.send(b"\x1b[A", 0.4); t.send(b"\x1b[A", 0.8)  # back up to Cockpit
    t.send(b"\r", 0.6)
    for k in range(2):
        t.send(b"t", 0.8)
        t.shot(f"{out}/6-cockpit-t{k + 1}.png")
    t.send(b" ", 0.4)
    t.send(b"q", 0.5)


def karaoke(t, out):
    """The Karaoke design and Widget: track 2 of the fake Catalogue sings (timed and estimated lines, an instrumental
    gap, a wrapping line). Shots at moments of the song: the count-in, the ball mid-hop, a line sung out, an estimated
    line, the gap, its count-in, the long line. Run it at a few sizes (PTY_COLS/PTY_ROWS); `karaoke-cells` has no
    pixel tier (the ball is a `●` in cells)."""
    t.pump(1.0)
    t.send(b" ", 0.3)  # play track 1
    t.send(b"n", 0.0)  # track 2, from 0 s
    t0 = time.time()
    def at(secs, name):
        t.pump(max(0.0, t0 + secs - time.time()))
        t.shot(f"{out}/{name}.png")
        print(f"{name}: {time.time() - t0:.2f} s")
    at(1.6, "0-count-in")
    at(3.35, "1-first-words")
    at(3.62, "2-mid-hop")
    at(4.1, "3-mid-hop-2")
    at(6.2, "4-sung-out")
    at(8.0, "5-estimated")
    t.send(b"\x1b[1;2C", 0.0)  # Shift+→: seek 5 s on, ~13 s (a bare → moves Home's highlight)
    t0 -= 5
    at(16.5, "6-gap")
    at(19.6, "7-gap-count-in")
    at(24.0, "8-long-line")
    t.send(b" ", 0.6)  # pause: the ball holds still
    t.shot(f"{out}/9-paused.png")
    t.send(b"q", 0.5)


def themes(t, out):
    """The shared palettes: the theme picker (scrolling list, live preview), then every new one with `t` on
    Singularity and on Cockpit (the black hole and the meters in the theme's colours)."""
    names = ["ember", "sunset", "terracotta", "honey", "rose", "peach", "sandstone", "paper", "lagoon", "moss"]
    t.pump(1.0)
    t.send(b" ", 2.0)
    t.send(b"\x0b", 0.5)
    t.shot(f"{out}/0-palette.png")  # every label whole, none cut off
    t.send(b"palette", 0.4)
    t.shot(f"{out}/0-palette-old-word.png")  # the old word still finds "Next theme" and "Pick a theme"
    t.send(b"\x1b", 0.4)
    t.send(b"\x0b", 0.3); t.send(b"pick a theme", 0.3); t.send(b"\r", 0.6)
    t.shot(f"{out}/0-picker.png")
    for _ in range(12):
        t.send(b"\x1b[B", 0.3)  # past the built-ins: the list scrolls
    t.shot(f"{out}/0-picker-scrolled.png")
    t.send(b"\x1b", 0.5)
    for design, own, tag in [(b"design", 0, "singularity"), (b"design", 3, "cockpit")]:
        t.send(b"\x0b", 0.3); t.send(design, 0.3); t.send(b"\r", 0.6)
        for _ in range(5):
            t.send(b"\x1b[A", 0.2)
        for _ in range(own):
            t.send(b"\x1b[B", 0.2)
        t.send(b"\r", 2.0)
        # `t` goes through the design's own palettes first
        for _ in range({"singularity": 1, "cockpit": 5}[tag] - 1):
            t.send(b"t", 0.3)
        for i, n in enumerate(names):
            t.send(b"t", 1.0)
            t.shot(f"{out}/{tag}-{i + 1:02d}-{n}.png")
    t.send(b" ", 0.4)
    t.send(b"q", 0.5)


def readme(t, out):
    """The README's pictures (docs/media/): each design with a track playing and no popup, then the Jam picker."""
    t.pump(1.0)
    t.send(b" ", 2.5)
    for i, name in enumerate(["singularity", "event-horizon", "classic", "cockpit", "terminal"]):
        t.send(b"\x0b", 0.3); t.send(b"design", 0.3); t.send(b"\r", 0.6)
        for _ in range(4):
            t.send(b"\x1b[A", 0.2)  # the picker opens on the current design: back to the top first (↑ stops there)
        for _ in range(i):
            t.send(b"\x1b[B", 0.2)
        t.send(b"\r", 2.5)  # keep it; the visuals settle
        t.shot(f"{out}/{i}-{name}.png")
    t.send(b"\x0b", 0.3); t.send(b"design", 0.3); t.send(b"\r", 0.6)
    for _ in range(4):
        t.send(b"\x1b[A", 0.2)
    t.send(b"\r", 1.5)  # back to Singularity
    t.send(b"d", 0.6); t.send(b"j", 1.5)
    t.shot(f"{out}/5-jam.png")
    print("jam:", t.find("Guest Friend") is not None)
    t.send(b"\x1b", 0.4); t.send(b"d", 0.6); t.send(b"j", 0.4); t.send(b"j", 0.8); t.send(b"\x1b", 0.4)
    t.send(b" ", 0.4)
    t.send(b"q", 0.5)


def mini(t, out):
    """#47: a tiny terminal is a mini-player; play, then show it. (Restored: it went with the waveform scenario, D46.)"""
    t.pump(1.0)
    t.send(b" ", 2.5)
    t.shot(f"{out}/0-mini.png")
    print("\n".join(l.rstrip() for l in t.screen.display))
    t.send(b" ", 0.3)
    t.send(b"q", 0.5)


def gesture(t, out):
    """D89/#104: a seek-bar drag whose release is never heard (a bare move: it lets go, seeking where it was let go of),
    a divider held while Ctrl+K opens the palette (the move is kept: Ctrl+Z takes it back, Q103 a), and Ctrl+Z while a
    divider is held (it takes back that drag, not the edit before it, Q103 b)."""
    t.pump(1.0)
    t.send(b" ", 1.0)

    def dump(name):
        t.shot(f"{out}/{name}.png")
        with open(f"{out}/{name}.txt", "w") as f:
            f.write("\n".join(t.screen.display))

    def grip():
        return t.screen.display[5].index("││") + 1

    # Q103 (a): Ctrl+K while a divider is held, the release in the palette, Esc, Ctrl+Z
    gx = grip()
    mouse(t, 0, gx, 12); mouse(t, 32, gx + 4, 12); mouse(t, 32, gx + 8, 12, wait=0.3)
    t.send(b"\x0b", 0.5)
    dump("02-palette-over-held-divider")
    mouse(t, 0, gx + 8, 12, release=True, wait=0.3)
    t.send(b"\x1b", 0.5)
    dump("03-palette-closed")
    t.send(b"\x1a", 0.6)
    dump("04-ctrl-z")
    # Q103 (b): one divider move let go, then a second held while Ctrl+Z, then its release
    gx = grip()
    mouse(t, 0, gx, 12); mouse(t, 32, gx + 6, 12); mouse(t, 0, gx + 6, 12, release=True, wait=0.5)
    dump("05-first-move")
    gx = grip()
    mouse(t, 0, gx, 12); mouse(t, 32, gx + 5, 12); mouse(t, 32, gx + 10, 12, wait=0.3)
    dump("06-second-held")
    t.send(b"\x1a", 0.6)
    dump("07-ctrl-z-while-held")
    mouse(t, 0, gx + 10, 12, release=True, wait=0.5)
    dump("08-released")
    t.send(b"\x1a", 0.6)
    dump("09-ctrl-z-again")
    # last (on main a seek drag left held swallows the next press): the seek bar, the player row's run of ━╸─
    y = max(i for i, l in enumerate(t.screen.display) if re.search(r"\d:\d\d  [━╸─]", l))
    x = re.search(r"\d:\d\d  ([━╸─])", t.screen.display[y]).start(1)
    mouse(t, 0, x + 6, y); mouse(t, 32, x + 20, y); mouse(t, 32, x + 40, y, wait=0.4)
    dump("10-seek-mid-drag")
    # the release went to another window: the next we hear is a bare move
    mouse(t, 35, x + 10, y - 12, wait=0.8)
    dump("11-seek-unheard-move")
    # a click on a Pane's body afterwards
    mouse(t, 0, x + 10, y - 12); mouse(t, 0, x + 10, y - 12, release=True, wait=0.8)
    dump("12-click-after")
    t.send(b"q", 0.5)


def layouts(t, out, env):
    """#19: swap Widgets (v, ◂ ▸), drag a divider, save the layout, and find it again in a new Client."""
    t.pump(1.0)
    t.send(b" ", 1.0)
    x, y = t.find("fft")
    mouse(t, 0, x + 1, y + 3); mouse(t, 0, x + 1, y + 3, release=True, wait=0.3)  # focus the fft Pane
    t.send(b"v", 0.6)  # fft → levels
    lx, ly = t.find("loudne")  # the title is cut short ("loudne…") since the arrows moved into it (#43)
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


def delete_design(t, out, env):
    """#80, Q151: two Designs of your own saved; in the Design picker, Delete on the one on screen deletes it at once
    (the picker stays, without it, on Singularity, saying `Deleted "Other" · u Undo`), `u` brings it back, Delete again
    and a click on Undo too; Esc. Ctrl+K → Delete design… deletes "Other" at once (the bar offers Ctrl+Z), Ctrl+Z brings
    it back. Then the picker on a built-in row (its hint without Del). Prints the layouts folder after each step."""
    files = lambda: sorted(os.listdir(f"{env['XDG_CONFIG_HOME']}/singularity/layouts"))
    t.pump(1.0)
    for name in (b"Mine", b"Other"):
        t.send(b"\x0b", 0.3); t.send(b"save design", 0.3); t.send(b"\r", 0.4)
        t.send(name, 0.3); t.send(b"\r", 0.6)
    print("saved:", files())
    t.send(b"\x0b", 0.3); t.send(b"pick a design", 0.3); t.send(b"\r", 0.6)
    t.shot(f"{out}/0-picker-on-own.png")
    t.send(b"\x1b[3~", 0.8)
    t.shot(f"{out}/1-deleted-undo-line.png")
    print("after Del:", files())
    t.send(b"u", 0.8)
    t.shot(f"{out}/2-undone.png")
    print("after u:", files())
    t.send(b"\x7f", 0.8)  # Backspace, a Mac's delete key
    x, y = t.find("· u Undo")
    mouse(t, 0, x + 2, y); mouse(t, 0, x + 2, y, release=True, wait=0.8)
    t.shot(f"{out}/3-clicked-undo.png")
    print("after Backspace and a click on Undo:", files())
    t.send(b"\x1b", 0.6)
    t.send(b"\x0b", 0.3); t.send(b"delete design", 0.3)
    t.shot(f"{out}/4-palette.png")
    t.send(b"\r", 0.8)
    t.shot(f"{out}/5-deleted-bar.png")
    print("after the palette's:", files())
    t.send(b"\x1a", 0.8)  # Ctrl+Z
    t.shot(f"{out}/6-ctrl-z-back.png")
    print("after Ctrl+Z:", files())
    t.send(b"\x0b", 0.3); t.send(b"pick a design", 0.3); t.send(b"\r", 0.6)
    t.send(b"\x1b[H", 0.6)  # Home: Singularity, built in
    t.shot(f"{out}/7-picker-built-in.png")
    t.send(b"\x1b", 0.6)
    print("at the end:", files())
    t.send(b"q", 0.5)


def tiling(t, out, env):
    """D87, a drag moves contents: drag a Pane's title onto another Pane (a shot while the pointer is still down:
    nothing has moved, the Pane under the pointer is lit with "swap with …" and the Widget's chip follows the pointer;
    another after the release: the two Widgets swapped, every outline where it was), a release on its own Pane and over the top bar (nothing happens), Esc cancelling a drag, a close with the x,
    a split from the right-click menu with the Widget list, and Arrange mode's keys (Enter picks up, ↑ aims, Enter swaps,
    + - size). Run it at
    PTY_COLS=160 PTY_ROWS=48 (the default here)."""
    # (the pixel tier's first frames and the fake Catalogue take a while: start once they're there)
    t.pump(6.0)
    t.send(b" ", 0.5)

    def title_at(title):
        # a spot on the title bar, left of the buttons
        # (a Pane the hole bends has braille in front of its title: look for the words alone)
        at = t.find("\u250c " + title)
        if at:
            return at[0] + 3, at[1]
        x, y = t.find(title)
        return x + 2, y

    def pull(title, to, shot=None, steps=6, hold=False):
        x0, y0 = title_at(title)
        mouse(t, 0, x0, y0)
        for i in range(1, steps + 1):
            mouse(t, 32, x0 + (to[0] - x0) * i // steps, y0 + (to[1] - y0) * i // steps, wait=0.12)
        t.pump(0.3)
        if shot:
            t.shot(f"{out}/{shot}")
        if not hold:
            mouse(t, 0, to[0], to[1], release=True, wait=0.5)

    t.shot(f"{out}/00-start.png")
    # 1. onto the queue: the queue lit, "swap with queue", the chip by the pointer; released, the two swap
    qx, qy = t.find("\u250c queue")
    pull("stream", (qx + 25, qy + 6), "01-swap-drag.png", hold=True)
    print("mid-drag:", "label shown" if t.find("swap with queue") else "NO LABEL", "·", "chip shown" if t.find("\u283f stream") else "NO CHIP")
    mouse(t, 0, qx + 25, qy + 6, release=True, wait=0.5)
    t.shot(f"{out}/02-swap-done.png")
    print("after the swap: stream where the queue was" if t.find("\u250c stream") == (qx, qy) else f"stream at {t.find(chr(0x250c) + ' stream')}, the queue was at {(qx, qy)}")
    # 2. lyrics onto Home, a small Pane onto a big one: they swap, the floor plan stays
    hx, hy = t.find("\u250c home")
    pull("tail -f lyrics.log", (hx + 30, hy + 10), "04-onto-home-drag.png")
    t.shot(f"{out}/05-onto-home-done.png")
    # 3. fft let go on its own Pane, then over the top bar: nothing happens either time
    fx, fy = title_at("fft")
    pull("fft", (fx + 4, fy + 4), "06-own-pane-drag.png")
    pull("fft", (60, 0), "07-top-bar-drag.png")
    t.shot(f"{out}/08-nothing-moved.png")
    # 4. Esc cancels a drag
    x0, y0 = title_at("home")
    mouse(t, 0, x0, y0)
    for i in range(1, 5):
        mouse(t, 32, x0 + 8 * i, y0 + 3 * i, wait=0.12)
    t.pump(0.3)
    t.shot(f"{out}/09-esc-held.png")
    t.send(b"\x1b", 0.4)
    mouse(t, 0, x0 + 32, y0 + 12, release=True, wait=0.4)
    t.shot(f"{out}/10-esc-after.png")
    # 5. close the loudness Pane with its x
    lx, ly = t.find("loudness")
    row = t.screen.display[ly]
    cx = row.index("\u00d7", lx)
    t.shot(f"{out}/11-before-close.png")
    mouse(t, 0, cx, ly); mouse(t, 0, cx, ly, release=True, wait=0.6)
    t.shot(f"{out}/12-closed.png")
    # 6. split: right-click a title, the Split row (the second; → lit), pick a Widget for the new empty Pane
    x0, y0 = title_at("~/library")
    mouse(t, 2, x0, y0); mouse(t, 2, x0, y0, release=True, wait=0.4)
    t.shot(f"{out}/13-title-menu.png")
    t.send(b"\x1b[B", 0.2)
    t.send(b"\r", 0.5)
    t.shot(f"{out}/14-widget-list.png")
    t.send(b"\x1b[B", 0.2)
    t.send(b"\r", 0.6)
    t.shot(f"{out}/15-split-done.png")
    # 7. Arrange mode's keys: the queue (where stream was, bottom left) focused with a click on its body; Enter picks it
    # up, ↑ aims at the Pane above it, Enter swaps; + + grow its Pane; Esc is Done (its name prompt kept unnamed)
    t.send(b"m", 0.4)
    qx, qy = t.find("\u250c queue")
    mouse(t, 0, qx + 6, qy + 3); mouse(t, 0, qx + 6, qy + 3, release=True, wait=0.4)
    t.send(b"\r", 0.3)
    t.send(b"\x1b[A", 0.4)
    t.shot(f"{out}/16-picked-up-aimed.png")
    lab = t.find("swap with ")
    print("aimed:", t.screen.display[lab[1]][lab[0]:lab[0] + 24].strip() if lab else "NO LABEL", "·", "chip shown" if t.find("\u283f queue") else "NO CHIP")
    t.send(b"\r", 0.4)
    t.shot(f"{out}/17-swapped-up.png")
    print("swapped: the queue moved up from", (qx, qy), "to", t.find("\u250c queue"))
    t.send(b"+", 0.3); t.send(b"+", 0.3)
    t.shot(f"{out}/18-grown.png")
    t.send(b"\x1b", 0.5)
    t.send(b"\x1b", 0.5)
    # 8. the design list has your design, and the file is on disk
    t.send(b"\x0b", 0.3); t.send(b"design", 0.3); t.send(b"\r", 0.6)
    t.shot(f"{out}/19-picker.png")
    t.send(b"\x1b", 0.3)
    import glob
    files = sorted(glob.glob(f"{env['XDG_CONFIG_HOME']}/singularity/layouts/*.toml"))
    print("layout files:", [os.path.basename(f) for f in files])
    # 9. Reset design
    t.send(b"\x0b", 0.3); t.send(b"reset design", 0.3); t.send(b"\r", 0.6)
    t.shot(f"{out}/20-reset.png")
    t.send(b"q", 0.5)


def arrange(t, out):
    """D87, #41: an arrange session. stream's title dragged onto the Tray (shot mid-drag, the Tray lit), dropped
    (its Pane left empty, the Tray, the legend, Done, each Pane's ⊞ and ×); the queue split with its ⊞ and the picker
    left with Esc (an empty Pane); stream dragged from the Tray into it; Home set aside (Done says why it can't) and
    put back; Done asks for the name (kept unnamed with Esc), and the app looks as before. The player bar stays over
    the Tray while arranging (#69). Prints what each step left. 160×48, cell tier, an empty output folder."""
    t.pump(1.0)
    t.send(b" ", 0.5)
    tray_y = ROWS - 3

    def title_at(title):
        x, y = t.find("┌ " + title)
        return x + 3, y

    def drag(frm, to, shot=None):
        (x, y), (tx, ty) = frm, to
        mouse(t, 0, x, y)
        mouse(t, 32, x + 3, max(1, y - 2), wait=0.15)
        mouse(t, 32, tx, ty, wait=0.4)
        if shot:
            t.shot(f"{out}/{shot}")
        mouse(t, 0, tx, ty, release=True, wait=0.6)

    def on_row(s, y, after=0):
        x = t.screen.display[y].find(s, after)
        return (x, y) if x >= 0 else None

    def legend():
        return t.screen.display[ROWS - 1].strip()

    t.shot(f"{out}/00-start.png")
    # 1. stream's title onto the Tray: the mode is on while it's held, the Tray lit
    drag(title_at("stream"), (COLS // 2, tray_y), shot="01-over-the-tray.png")
    t.shot(f"{out}/02-in-the-tray.png")
    print("in the Tray:", bool(on_row("⠿ stream", tray_y)), "· legend:", legend())
    # (#69, Q39: the player bar stays, over the Tray)
    print("player bar over the Tray:", bool(on_row("▶|", tray_y - 3)))
    # 2. the queue's ⊞: a new empty Pane and the picker; Esc leaves it empty
    qx, qy = t.find("┌ queue")
    x, _ = on_row("⊞", qy, qx)
    mouse(t, 0, x, qy); mouse(t, 0, x, qy, release=True, wait=0.6)
    t.shot(f"{out}/03-split-picker.png")
    t.send(b"\x1b", 0.6)
    t.shot(f"{out}/04-empty-pane.png")
    # 3. stream from the Tray into the new empty Pane (beside the queue)
    ex, ey = next((line.find("empty", qx + 20), y) for y, line in enumerate(t.screen.display) if y > qy and line.find("empty", qx + 20) >= 0)
    sx, _ = on_row("⠿ stream", tray_y)
    drag((sx + 3, tray_y), (ex + 2, ey), shot="05-from-the-tray.png")
    t.shot(f"{out}/06-filled.png")
    # 4. Home into the Tray: Done is unavailable and says why; then back into its empty Pane
    hx, hy = t.find("┌ home") or t.find("┌ ")
    drag((hx + 3, hy), (COLS // 2, tray_y))
    t.shot(f"{out}/07-home-in-the-tray.png")
    print("Home set aside · legend:", legend())
    t.send(b"\x1b", 0.4)
    print("Esc with Home in the Tray keeps arranging:", "ARRANGE" in legend())
    ex, ey = next((line.find("empty"), y) for y, line in enumerate(t.screen.display) if "empty" in line and y < tray_y - 1)
    hx, _ = on_row("⠿ home", tray_y)
    drag((hx + 3, tray_y), (ex + 2, ey))
    t.shot(f"{out}/08-home-back.png")
    # 5. Done: the name prompt (something changed); Esc keeps it as "Singularity (edited)"
    dx, _ = on_row(" Done ", ROWS - 1)
    mouse(t, 0, dx + 2, ROWS - 1); mouse(t, 0, dx + 2, ROWS - 1, release=True, wait=0.6)
    t.shot(f"{out}/09-done-asks.png")
    print("Done asks for a name:", bool(t.find("Keep your changes as a design")))
    t.send(b"\x1b", 0.8)
    t.shot(f"{out}/10-after.png")
    print("after Done: no Tray:", not t.find(" Tray ") and "ARRANGE" not in legend(), "· bottom:", legend()[:60])
    t.send(b"q", 0.5)


def arrange_keys(t, out):
    """D87, #43: Arrange mode from the keyboard alone. `m` and the legend; arrows move the focus; Enter picks the
    queue up (its Pane lit, the chip), ↑ aims at Home ("swap with …"), Enter swaps; Esc puts a pick-up back; `t` sets
    the focused Widget aside in the Tray; Home picked up and aimed at that empty Pane ("put here"); ↓ onto the Tray,
    Enter, ↑ and Enter take its Widget out into a Pane (undone with Ctrl+Z); `s` (the legend
    waits for the arrow) then → splits (the picker; Esc leaves the new Pane empty); `x` closes it; `-` `+` move a line;
    Esc is Done (the name prompt; Esc keeps it unnamed) and the app looks as before. Prints what each step left.
    160×48, cell tier, an empty output folder."""
    t.pump(1.0)
    t.send(b" ", 0.5)
    UP, DOWN, LEFT, RIGHT = b"\x1b[A", b"\x1b[B", b"\x1b[D", b"\x1b[C"

    def legend():
        return t.screen.display[ROWS - 1].strip()

    def at(title):
        return t.find("┌ " + title)

    def step(name, keys, wait=0.4):
        for k in keys:
            t.send(k, wait)
        t.shot(f"{out}/{name}.png")

    t.shot(f"{out}/00-start.png")
    before = list(t.screen.display)
    step("01-arrange-mode", [b"m"])
    print("legend:", legend())
    # 1. the focus: from Home down to the queue
    step("02-focus-down", [DOWN])
    # 2. the queue picked up, aimed at Home above it, swapped
    q, h = at("queue"), at("home")
    step("03-picked-up", [b"\r"])
    print("picked up: chip", bool(t.find("⠿ queue")), "· legend:", legend())
    step("04-aimed-up", [UP])
    print("aimed: label", bool(t.find("swap with home")))
    step("05-dropped", [b"\r"])
    print("swapped:", at("queue") == h and at("home") == q)
    # 3. picked up again and put back with Esc: still arranging
    step("06-esc-put-back", [b"\r", DOWN, b"\x1b"])
    print("Esc put it back:", at("queue") == h, "· still arranging:", "ARRANGE" in legend())
    # 4. the focus down to Home (where the queue was), → to its neighbour, which `t` sets aside in the Tray
    step("07-focus-home", [DOWN])
    step("08-t-to-tray", [RIGHT, b"t"])
    print("after t, the Tray row:", t.screen.display[ROWS - 3].strip()[:100])
    # 5. Home picked up, aimed at the empty Pane, "put here", then Esc
    step("09-home-aimed-at-empty", [LEFT, b"\r", RIGHT])
    print("put here:", bool(t.find("put here")))
    step("10-esc", [b"\x1b"])
    # 5b. the Tray by keyboard: ↓ from Home (a bottom Pane) onto the Tray's Widget, Enter picks it up, ↑ aims at the
    # Pane above it, Enter puts it there (that Pane's Widget into the Tray); Ctrl+Z takes it back (the focus on Home)
    step("10a-tray-focus", [DOWN])
    step("10b-from-tray-aimed", [b"\r", UP])
    lab = t.find("swap with ") or t.find("put here")
    print("from the Tray, aimed:", t.screen.display[lab[1]][lab[0]:lab[0] + 24].strip() if lab else "NO LABEL")
    step("10c-from-tray-dropped", [b"\r"])
    print("after the drop, the Tray row:", t.screen.display[ROWS - 3].strip()[:100])
    step("10d-undone", [b"\x1a"])
    # 6. s waits for the arrow (the legend says so); → splits: the picker; Esc leaves the new Pane empty
    step("11-s-waits", [b"s"])
    print("after s, legend:", legend())
    step("12-split-picker", [RIGHT], wait=0.6)
    step("13-split-empty", [b"\x1b"], wait=0.6)
    # 7. x closes the focused (new, empty) Pane; - + move Home's line
    step("14-x-closed", [b"x"])
    step("15-shrunk", [b"-", b"-"])
    step("16-grown", [b"+", b"+"])
    # 8. Esc is Done: the name prompt (something changed); Esc keeps it unnamed, and the app looks as before
    step("17-done-asks", [b"\x1b"], wait=0.6)
    print("Done asks for a name:", bool(t.find("Keep your changes as a design")))
    step("18-after", [b"\x1b"], wait=0.8)
    print("after Done: no Tray:", not t.find(" Tray ") and "ARRANGE" not in legend(), "· bottom:", legend()[:60])
    t.send(b"q", 0.5)


def arrange_fixes(t, out):
    """The owner's Arrange-mode test (#41, #43): the chip by a dragged title over small pointer moves (pixel mouse
    reports, as Ghostty sends them: prints the chip's cell against the pointer's for each move); the queue picked up
    with Enter and aimed ↓ into the Tray (the chip in the Tray's row, where it would land); `=` `-` `]` `[` on the
    focused Pane's borders (prints the sizes); `w` opens "+ Widget"'s picker; `m` leaves Arrange mode. Pixel tier and
    pixel mouse, 160x48; start from an empty output folder."""
    t.pump(6.0)
    t.send(b" ", 0.5)
    UP, DOWN = b"\x1b[A", b"\x1b[B"
    t.shot(f"{out}/00-start.png")

    def px(code, x, y, release=False, wait=0.05):
        # pixel reports (SGR-Pixels), 1-based
        t.send(f"\x1b[<{code};{int(x) + 1};{int(y) + 1}{'m' if release else 'M'}".encode(), wait)

    def legend():
        return t.screen.display[ROWS - 1].strip()

    # 1. a title held and wiggled a few pixels at a time, up, down and sideways: the chip keeps its place by the pointer
    sx, sy = t.find("┌ stream")
    px(0, (sx + 3.5) * CW, (sy + 0.5) * CH)
    cx, cy = 60.3 * CW, 22.5 * CH
    px(32, cx, cy, wait=1.0)
    offsets = []

    def settled(want):
        # (under load a frame can come late: wait up to 3 s for the chip to get where this move puts it)
        at = None
        for _ in range(30):
            t.pump(0.1)
            at = t.find("⠿ stream")
            if at == want:
                break
        return at
    for i in range(48):
        dx = [0, 3, -3, 6, -2, 1][i % 6]
        dy = [0, 7, 14, 4, -6, -12, -3, 9][i % 8]
        x, y = cx + i * 1.7 + dx, cy + dy
        px(32, x, y, wait=0.05)
        at = settled((int(x // CW) + 3, int(y // CH)))
        offsets.append(None if not at else (at[0] - int(x // CW), at[1] - int(y // CH)))
        if i == 20:
            t.shot(f"{out}/01-chip-mid-wiggle.png")
    print("chip - pointer (cells) over", len(offsets), "moves:", sorted(set(offsets), key=str))
    t.send(b"\x1b", 1.0)
    px(0, cx, cy, release=True, wait=1.0)
    t.send(b"\x1b", 1.5)
    # 2. the keyboard: m (the focus on the stream's Pane, where the drag left it: a bottom one), Enter picks it up, ↓
    # aims past the bottom of the Panes into the Tray
    t.send(b"m", 1.0)
    t.shot(f"{out}/02-arrange.png")
    print("legend:", legend())
    t.send(b"\r", 0.4)
    t.shot(f"{out}/03-picked-up.png")
    t.send(DOWN, 0.4)
    t.shot(f"{out}/04-aimed-into-tray.png")
    at = t.find("⠿ stream")
    print("aimed into the Tray: chip at", at, "· the Tray's frame rows", ROWS - 4, "to", ROWS - 2, "· in its inner row:", bool(at) and at[1] == ROWS - 3)
    t.send(b"\r", 0.4)
    t.shot(f"{out}/05-set-aside.png")
    print("after Enter, the Tray row:", t.screen.display[ROWS - 3].strip()[:100])
    # ↓ onto it in the Tray and Enter picks it up from there: its chip on its own place in the Tray's row
    t.send(DOWN, 0.3)
    t.send(b"\r", 0.4)
    t.shot(f"{out}/05b-picked-up-from-tray.png")
    at = t.find("⠿ stream")
    print("picked up from the Tray: chip at", at, "· the Tray's frame:", t.screen.display[ROWS - 4].strip()[:40])
    t.send(b"\x1b", 0.3)
    t.send(b"\x1a", 0.4)
    # 3. Home focused (a click on it): `=` grows like `+`, `-` shrinks; `]` taller, `[` shorter. Prints where the
    # titles of Home and the queue under it are
    hx, hy = t.find("home")
    px(0, (hx + 20) * CW, (hy + 8) * CH)
    px(0, (hx + 20) * CW, (hy + 8) * CH, release=True, wait=0.3)

    def where():
        return {w: t.find(w) for w in ("┌ queue", "┌ home")}
    for name, keys in [("06-eq", b"=="), ("07-minus", b"--"), ("08-taller", b"]]"), ("09-shorter", b"[["), ("10-plus", b"+")]:
        before = where()
        for k in keys:
            t.send(bytes([k]), 0.3)
        t.shot(f"{out}/{name}.png")
        print(name, "titles (x, y):", before, "->", where())
    # 4. `w`: the "+ Widget" picker, from a Pane
    t.send(b"w", 0.5)
    t.shot(f"{out}/11-w-picker.png")
    print("after w:", [l.strip()[:70] for l in t.screen.display if "Widget" in l or "widget" in l][:3])
    t.send(b"\x1b", 0.4)
    # 5. `m` leaves Arrange mode as Esc does (something changed: the name prompt; Esc keeps it unnamed)
    t.send(b"m", 0.6)
    t.shot(f"{out}/12-m-done.png")
    print("m: name prompt:", bool(t.find("Keep your changes as a design")), "· legend:", legend()[:60])
    t.send(b"\x1b", 0.6)
    t.shot(f"{out}/13-after.png")
    print("after: arranging:", "ARRANGE" in legend())
    t.send(b"q", 0.5)


def new_design(t, out, env):
    """D87, #42: a whole New design… build. Ctrl+K → New design…, Esc at once (the Design as before), then again: one
    empty Pane filling the screen and Home in the Tray; the Tray's "+ Widget" (the picker, the queue picked into the
    Tray); the Pane's ⊞ split, the picker left with Esc; Home and the queue dragged from the Tray into the two Panes;
    Done's name prompt, a name; the saved file. Prints what each step left. 160×48, cell tier, an empty output folder."""
    t.pump(1.0)
    t.send(b" ", 0.5)
    tray_y = ROWS - 3
    layouts = f"{env['XDG_CONFIG_HOME']}/singularity/layouts"

    def on_row(s, y, after=0):
        x = t.screen.display[y].find(s, after)
        return (x, y) if x >= 0 else None

    def legend():
        return t.screen.display[ROWS - 1].strip()

    def click(x, y, wait=0.6):
        mouse(t, 0, x, y); mouse(t, 0, x, y, release=True, wait=wait)

    def drag(frm, to):
        (x, y), (tx, ty) = frm, to
        mouse(t, 0, x, y)
        mouse(t, 32, x + 3, max(1, y - 2), wait=0.15)
        mouse(t, 32, tx, ty, wait=0.4)
        mouse(t, 0, tx, ty, release=True, wait=0.6)

    def palette(word):
        t.send(b"\x0b", 0.3); t.send(word.encode(), 0.3); t.send(b"\r", 0.8)

    def empties():
        return [(line.find("empty"), y) for y, line in enumerate(t.screen.display) if 0 < y < tray_y - 3 and "empty" in line]

    t.shot(f"{out}/00-start.png")
    before = list(t.screen.display)
    # 1. New design…, Esc before anything is placed: the Design as it was
    palette("new design")
    t.shot(f"{out}/01-blank.png")
    print("blank: empty Panes:", len(empties()), "· Home in the Tray:", bool(on_row("⠿ home", tray_y)), "· legend:", legend())
    t.send(b"\x1b", 0.8)
    t.shot(f"{out}/01b-esc.png")
    titles = lambda d: [x for x in ("┌ ~/library", "┌ home", "┌ queue", "┌ stream", "gravity well", "┌ loudness") if any(x in l for l in d)]
    print("Esc at once: the same Panes as before:", titles(t.screen.display) == titles(before), titles(before), "· no Tray:", "ARRANGE" not in legend(), "· layouts written:", os.listdir(layouts) if os.path.isdir(layouts) else [])
    # 2. again; "+ Widget" on the Tray: the picker; the queue goes into the Tray
    palette("new design")
    ax, _ = on_row("+ Widget", tray_y - 1)
    click(ax + 2, tray_y - 1)
    t.shot(f"{out}/02-tray-picker.png")
    t.send(b"queue", 0.4)
    t.send(b"\r", 0.6)
    t.shot(f"{out}/03-queue-in-the-tray.png")
    print("in the Tray:", [w for w in ("⠿ home", "⠿ queue") if on_row(w, tray_y)])
    # 3. the floor plan first: the Pane's ⊞ splits it; Esc leaves the new Pane empty
    x, y = on_row("⊞", 1)
    click(x, y)
    t.send(b"\x1b", 0.6)
    t.shot(f"{out}/04-split.png")
    print("after the split: empty Panes:", len(empties()))
    # 4. then the contents: Home into the first empty Pane, the queue into the other
    lx, ly = empties()[0]
    hx, _ = on_row("⠿ home", tray_y)
    drag((hx + 3, tray_y), (lx + 2, ly))
    t.shot(f"{out}/05-home-placed.png")
    lx, ly = empties()[0]
    qx, _ = on_row("⠿ queue", tray_y)
    drag((qx + 3, tray_y), (lx + 2, ly))
    t.shot(f"{out}/06-filled.png")
    print("filled: empty Panes:", len(empties()), "· legend:", legend())
    # 5. Done: the name prompt; a name saves it
    dx, _ = on_row(" Done ", ROWS - 1)
    click(dx + 2, ROWS - 1)
    t.shot(f"{out}/07-name.png")
    print("Done asks for a name:", bool(t.find("Save design as")))
    t.send(b"Built", 0.4)
    t.send(b"\r", 0.8)
    t.shot(f"{out}/08-saved.png")
    path = f"{layouts}/built.toml"
    print("after: no Tray:", "ARRANGE" not in legend(), "· saved:", os.path.exists(path))
    if os.path.exists(path):
        print(open(path).read())
    t.send(b"q", 0.5)


def pane_menu(t, out):
    """D87, #37: the Pane menu and the Widget picker. A right-click on a title (Replace widget… first), the picker
    (Widgets not on screen, then the on-screen ones dimmed below a line), typing to filter, a replace; a right-click on
    a body; a click on the Widget's name, a pick from the on-screen part (a swap); Home's menu and the `w` key on Home.
    #39: the Split row's arrows (↓ then Esc: an empty Pane; ← then a pick; ↑ from a body; → from the keyboard), a
    `needs W × H` row in the picker, the palette's Split up until it's refused. Prints each menu's items and what each
    Pane shows. 160×48, cell tier, an empty output folder."""
    t.pump(1.0)
    t.send(b" ", 0.5)

    def title_at(title):
        at = t.find("\u250c " + title)
        if at:
            return at[0] + 2, at[1]
        x, y = t.find(title)
        return x, y

    def menu_items():
        # the menu's rows: between its rounded corners
        at = next(((x, y) for y, line in enumerate(t.screen.display) for x in [line.find("\u256d")] if x >= 0), None)
        if not at:
            return None
        x, y = at
        items = []
        for line in t.screen.display[y + 1:]:
            if line[x] != "\u2502":
                break
            items.append(line[x + 1:line.index("\u2502", x + 1)].strip())
        return items

    def titles():
        return [w for w in ("fft", "stream", "queue", "starfield", "karaoke", "loudness", "tail -f lyrics.log") if t.find("\u250c " + w) or t.find(w + " ")]

    t.shot(f"{out}/00-start.png")
    # 1. right-click fft's title: its menu
    x, y = title_at("fft")
    mouse(t, 2, x + 6, y); mouse(t, 2, x + 6, y, release=True, wait=0.4)
    t.shot(f"{out}/01-title-menu.png")
    print("fft's menu:", menu_items())
    # 2. Replace widget…: the picker
    t.send(b"\r", 0.5)
    t.shot(f"{out}/02-picker.png")
    # 3. typed: filtered
    t.send(b"star", 0.4)
    t.shot(f"{out}/03-picker-typed.png")
    t.send(b"\r", 0.6)
    # the first edit of a built-in Design asks for a name: Esc keeps it unnamed
    t.shot(f"{out}/04-replaced-naming.png")
    t.send(b"\x1b", 0.5)
    t.shot(f"{out}/05-replaced.png")
    print("after replacing fft:", titles())
    # 4. right-click the lyrics' body: the same menu
    lx, ly = title_at("tail -f lyrics.log")
    mouse(t, 2, lx + 10, ly + 6); mouse(t, 2, lx + 10, ly + 6, release=True, wait=0.4)
    t.shot(f"{out}/06-body-menu.png")
    print("lyrics body menu:", menu_items())
    t.send(b"\x1b", 0.4)
    # 5. a click on the Widget's name in stream's title: the picker; the wheel down into the on-screen part
    sx, sy = title_at("stream")
    mouse(t, 0, sx + 1, sy); mouse(t, 0, sx + 1, sy, release=True, wait=0.5)
    t.shot(f"{out}/07-name-click-picker.png")
    px, py = t.find("Replace stream")
    for _ in range(40):
        t.send(f"\x1b[<65;{px + 6};{py + 4}M".encode(), 0.03)
    t.pump(0.4)
    t.shot(f"{out}/08-picker-on-screen.png")
    t.send(b"\x1b[A" * 3, 0.4)
    t.shot(f"{out}/09-picker-on-screen-up.png")
    t.send(b"\r", 0.6)
    t.shot(f"{out}/10-swapped.png")
    print("after the swap:", titles())
    # 6. Home's menu: no Replace widget; `w` on Home says why
    hx, hy = title_at("home")
    mouse(t, 2, hx + 8, hy); mouse(t, 2, hx + 8, hy, release=True, wait=0.4)
    t.shot(f"{out}/11-home-menu.png")
    print("home's menu:", menu_items())
    t.send(b"\x1b", 0.3)
    mouse(t, 0, hx + 8, hy + 3); mouse(t, 0, hx + 8, hy + 3, release=True, wait=0.3)
    t.send(b"w", 0.4)
    t.shot(f"{out}/12-home-w.png")
    # 7. `w` on the queue: Widgets off screen first, as on any Pane but Home's (D87, #38)
    qx, qy = title_at("queue")
    mouse(t, 0, qx + 12, qy); mouse(t, 0, qx + 12, qy, release=True, wait=0.3)
    t.send(b"w", 0.5)
    t.shot(f"{out}/13-queue-picker.png")
    t.send(b"\x1b", 0.3)

    # 8. (#39) split: the Split row's arrows, each making an empty Pane with the Widget picker on it
    def menu_on(title, dx=8):
        x, y = title_at(title)
        mouse(t, 2, x + dx, y); mouse(t, 2, x + dx, y, release=True, wait=0.4)

    def click_arrow(arrow):
        at = t.find("Split  \u2190")
        x = t.screen.display[at[1]].index(arrow, at[0])
        mouse(t, 0, x, at[1]); mouse(t, 0, x, at[1], release=True, wait=0.6)

    def panes():
        return sum(line.count("\u250c") for line in t.screen.display)

    menu_on("stream")
    t.shot(f"{out}/14-split-row.png")
    print("stream's menu:", menu_items())
    n = panes()
    click_arrow("\u2193")  # ↓
    t.shot(f"{out}/15-split-down-picker.png")
    print("split ↓: the picker is up:", bool(t.find("Choose a widget")), "| hint:", bool(t.find("Esc leaves it empty")))
    t.send(b"\x1b", 0.5)
    t.shot(f"{out}/16-left-empty.png")
    print("after Esc: Panes", n, "→", panes(), "| labelled empty:", any(" empty " in line for line in t.screen.display))
    # ← on the queue: pick by typing
    menu_on("queue", 12)
    click_arrow("\u2190")  # ←
    t.send(b"scope", 0.4)
    t.shot(f"{out}/17-split-left-typed.png")
    t.send(b"\r", 0.6)
    t.shot(f"{out}/18-split-left-filled.png")
    print("after ← and a pick:", titles(), "| osc shown:", bool(t.find("osc")))
    # ↑ on the lyrics' body
    lx, ly = title_at("tail -f lyrics.log")
    mouse(t, 2, lx + 10, ly + 6); mouse(t, 2, lx + 10, ly + 6, release=True, wait=0.4)
    click_arrow("\u2191")  # ↑
    t.shot(f"{out}/19-split-up-picker.png")
    t.send(b"\x1b", 0.5)
    # → from the keyboard: the Split row, ← → light an arrow, Enter
    menu_on("starfi")  # (its title may be cut short by now)
    t.send(b"\x1b[B", 0.2)
    t.send(b"\x1b[D", 0.3)  # → lit → ← lit
    t.shot(f"{out}/20-split-row-key-lit.png")
    t.send(b"\x1b[C", 0.3)  # → again
    t.send(b"\r", 0.6)
    t.shot(f"{out}/21-split-right-picker.png")
    # a Widget too big for the new Pane says what it needs
    t.send(b"kara", 0.4)
    t.shot(f"{out}/22-picker-needs.png")
    print("karaoke row says:", next((line.strip() for line in t.screen.display if "karaoke" in line and "needs" in line), None))
    t.send(b"\x1b", 0.5)
    # the palette: Split up on the focused Pane, until there's no room
    said = None
    for i in range(24):
        t.send(b"\x0b", 0.4); t.send(b"split up", 0.3); t.send(b"\r", 0.6)
        if t.find("Choose a widget"):
            t.send(b"\x1b", 0.4)
            continue
        said = next((line.strip() for line in t.screen.display if "No room for a new Pane" in line), None)
        break
    t.shot(f"{out}/23-split-refused.png")
    print("refused:", said)
    t.send(b"q", 0.5)


def paneless(t, out):
    """D87, #38: the library and the queue without a Pane. Three songs queued from Liked Songs; "Your Library" and
    "Queue" from the palette focus their Panes; both Panes closed with their ×; then the same entries open them as
    popups: the library's (a playlist opened from it into Home's Pane) and the queue's (a song dragged to the top).
    Prints the queue's order before and after. 160×48, cell tier, an empty output folder."""
    t.pump(1.0)
    x, y = t.find("♥  Liked Songs")
    mouse(t, 0, x + 2, y); mouse(t, 0, x + 2, y, release=True, wait=0.8)
    for row in (3, 5, 7):
        rx, ry = t.find(f"Fake Track 00{row}")
        mouse(t, 2, rx + 2, ry, wait=0.3)  # right-click
        t.send(b"\x1b[B", 0.1)  # down: "Play next"
        t.send(b"\r", 0.8)

    def palette(what):
        t.send(b"\x0b", 0.3)  # Ctrl+K
        t.send(what.encode(), 0.3)
        t.send(b"\r", 0.6)

    def close(title):
        # the × at the right end of the Pane's title (the first after its name: its corner may be a neighbour's)
        x, y = t.find("┌ " + title)
        cx = t.screen.display[y].index("×", x)
        mouse(t, 0, cx, y); mouse(t, 0, cx, y, release=True, wait=0.6)

    def order():
        rows = sorted((at[1], n) for n in ("fake2", "fake4", "fake6") if (at := t.find(f"Described {n} ·")))
        return [n for _, n in rows]

    palette("your library")
    t.shot(f"{out}/00-library-pane-focused.png")
    close("~/library")
    # the first edit of a built-in Design asks for a name: Esc keeps it unnamed
    t.send(b"\x1b", 0.5)
    close("queue")
    t.shot(f"{out}/01-without-library-and-queue.png")
    # the library as a popup
    palette("your library")
    t.shot(f"{out}/02-library-popup.png")
    t.send(b"\x1b[B", 0.3)
    t.shot(f"{out}/03-library-popup-down.png")
    t.send(b"\r", 0.8)
    t.shot(f"{out}/04-opened-in-home.png")
    # the queue as a popup: drag the last queued song (fake2) onto the first (fake6)
    palette("queue")
    t.shot(f"{out}/05-queue-popup.png")
    print("queue in the popup:", order())
    (sx, sy), (dx, dy) = t.find("Described fake2 ·"), t.find("Described fake6 ·")
    mouse(t, 0, sx + 2, sy)
    mouse(t, 32, sx + 2, sy - 1, wait=0.15)
    mouse(t, 32, dx + 2, dy, wait=0.3)
    t.shot(f"{out}/06-dragging.png")
    mouse(t, 0, dx + 2, dy, release=True, wait=1.0)
    t.shot(f"{out}/07-dropped.png")
    print("after the drag:", order())
    t.send(b"\x1b", 0.4)
    t.shot(f"{out}/08-closed.png")
    t.send(b"q", 0.5)


def tiling_fast(t, out, env):
    """Fast drags at `visuals.motion_fps = 240`: a fast sweep across ~5 Panes moves nothing but the highlight (on the
    Pane under the pointer at once) and the chip; Esc ends it; a drop on a short wide Pane swaps. The Client logs every
    frame to `frames.log` (start µs, rate, tick/draw/diff/flush µs): the frame intervals during the drag are printed.
    Pixel tier, 160x48; start from an empty output folder."""
    t.pump(1.0)
    t.send(b" ", 0.5)
    t.shot(f"{out}/00-start.png")
    hx, hy = t.find("┌ home")
    hx += 3
    mouse(t, 0, hx, hy, wait=0.05)
    # a sweep at ~4 ms a report over the library, the queue, lyrics, the well and back: nothing rearranges, the
    # highlight is on whatever Pane the pointer is over
    path = [(hx, hy), (12, 14), (31, 33), (140, 30), (120, 10), (60, 38)]
    shots = 0
    for (x0, y0), (x1, y1) in zip(path, path[1:]):
        for i in range(1, 21):
            mouse(t, 32, x0 + (x1 - x0) * i // 20, y0 + (y1 - y0) * i // 20, wait=0.004)
        t.shot(f"{out}/01-sweep-{shots}.png")
        shots += 1
    # it rests: the same, nothing waits for a pause
    t.pump(0.5)
    t.shot(f"{out}/02-paused.png")
    # Esc ends the drag: nothing changed
    t.send(b"\x1b", 0.5)
    mouse(t, 0, 60, 38, release=True, wait=0.5)
    t.shot(f"{out}/03-put-back.png")
    # the shortest wide Pane: dropped on its top row, it swaps
    sx, sy = t.find("┌ stream")
    mouse(t, 0, sx + 3, sy, wait=0.05)
    for lx, ly in [(t.find("┌ loudness") or (0, 0))]:
        for i in range(1, 11):
            mouse(t, 32, sx + 3 + (lx + 10 - sx - 3) * i // 10, sy + (ly + 1 - sy) * i // 10, wait=0.01)
        t.pump(0.4)
        t.shot(f"{out}/04-over-short-wide.png")
        mouse(t, 0, lx + 10, ly + 1, release=True, wait=0.6)
    t.shot(f"{out}/05-dropped.png")
    t.send(b"\x1b", 0.3)
    t.send(b"q", 0.5)
    rows = [list(map(int, l.split())) for l in open(f"{out}/frames.log") if l.strip()]
    fast = [r for r in rows if r[1] == 240]
    gaps = sorted((b[0] - a[0]) / 1000 for a, b in zip(fast, fast[1:]) if b[0] - a[0] < 50_000)
    work = sorted(sum(r[2:]) / 1000 for r in fast)
    if gaps:
        p = lambda v, q: v[min(len(v) - 1, int(len(v) * q))]
        print(f"240 fps frames: {len(fast)}; interval ms mean {sum(gaps) / len(gaps):.2f} p95 {p(gaps, .95):.2f} max {gaps[-1]:.2f}; "
              f"work ms mean {sum(work) / len(work):.2f} p95 {p(work, .95):.2f} max {work[-1]:.2f}")


def tiling_paths(t, out, env):
    """#21's pointer paths in the real Client, under D87 (a drag moves contents): a fling across Panes let go at once
    drops on the Pane under the release (Home and the library swap); a pause on the queue then a flick to the library
    let go at once drops on the library too, the Pane the pointer is over when it comes up; a release the Client never
    heard (the pointer comes back with no button down) changes nothing and the next click works. Prints where Home and
    the stream end each time. Pixel tier, 160x48; start from an empty output folder."""
    t.pump(1.0)
    t.send(b" ", 0.5)
    t.shot(f"{out}/00-start.png")
    hx, hy = t.find("┌ home")
    qx, qy = t.find("┌ queue")
    lx, ly = t.find("┌ ~/library")

    def glide(x0, y0, x1, y1, n, wait):
        for i in range(1, n + 1):
            mouse(t, 32, x0 + (x1 - x0) * i // n, y0 + (y1 - y0) * i // n, wait=wait)

    # 1. a fling over the queue to the library, let go at once: Home goes into the library's Pane, the library into Home's
    mouse(t, 0, hx + 3, hy, wait=0.05)
    glide(hx + 3, hy, qx + 10, qy + 5, 10, 0.004)
    glide(qx + 10, qy + 5, lx + 5, ly + 5, 10, 0.004)
    t.shot(f"{out}/01-fling-over-library.png")
    mouse(t, 0, lx + 5, ly + 5, release=True, wait=0.6)
    t.shot(f"{out}/02-fling-dropped.png")
    print("fling: home", "in the library's Pane" if t.find("┌ home") == (lx, ly) else f"at {t.find('┌ home')}, the library's Pane is at {(lx, ly)}")
    t.send(b"\x1b", 0.4)  # the naming prompt: keep it unnamed
    # 2. rest over the queue (lit, "swap with queue"), then a flick back to Home's old Pane let go at once: it drops
    # where the pointer comes up, not where it rested
    hx2, hy2 = t.find("┌ home")
    mouse(t, 0, hx2 + 3, hy2, wait=0.05)
    glide(hx2 + 3, hy2, qx + 10, qy + 5, 15, 0.01)
    t.pump(0.4)
    t.shot(f"{out}/03-resting-over-queue.png")
    glide(qx + 10, qy + 5, hx + 10, hy + 5, 6, 0.004)
    mouse(t, 0, hx + 10, hy + 5, release=True, wait=0.6)
    t.shot(f"{out}/04-flicked-dropped.png")
    print("flick after a pause: home", "back in its own Pane" if t.find("┌ home") == (hx, hy) else f"at {t.find('┌ home')}")
    # 3. picked up, then the pointer moves with no button down: its release went elsewhere
    sx, sy = t.find("┌ stream")
    mouse(t, 0, sx + 3, sy, wait=0.05)
    glide(sx + 3, sy, 60, 20, 10, 0.01)
    t.pump(0.3)
    mouse(t, 35, 61, 21, wait=0.6)
    t.shot(f"{out}/05-unheard-release.png")
    print("unheard release: stream", "where it was" if t.find("┌ stream") == (sx, sy) else f"at {t.find('┌ stream')}")
    lx, ly = t.find("┌ ~/library") or t.find("┌ home")
    mouse(t, 0, lx + 3, ly, wait=0.05)
    mouse(t, 0, lx + 3, ly, release=True, wait=0.5)
    t.shot(f"{out}/06-click-after.png")
    t.send(b"q", 0.5)


def main():
    scenario, out = sys.argv[1], sys.argv[2]
    os.makedirs(out, exist_ok=True)
    sock = f"{RUNTIME}/sgl-{os.getpid()}.sock"
    # (never the real Spotify app: a Daemon a Client starts by itself gets these too)
    env = dict(os.environ, SINGULARITY_ENGINE="soloist", SINGULARITY_SOLOIST="local", SINGULARITY_START_APP="0", SINGULARITY_SOCKET=sock, TERM="xterm-256color", COLORTERM="truecolor",
               # scratch config and state: test runs never write the owner's files
               XDG_CONFIG_HOME=f"{os.path.abspath(out)}/config", XDG_STATE_HOME=f"{os.path.abspath(out)}/state",
               XDG_DATA_HOME=f"{os.path.abspath(out)}/data",
               # never the owner's Spotify login (keyring) or session bus (MPRIS); `mpris` gets a private bus below
               SINGULARITY_NO_KEYRING="1", SINGULARITY_NOWPLAYING="off")
    for k in ("KITTY_WINDOW_ID", "TERM_PROGRAM", "GHOSTTY_RESOURCES_DIR", "TMUX"):
        env.pop(k, None)
    # big text in the font these shots draw the cells with, as a terminal's own font (#53)
    env.setdefault("SINGULARITY_BIG_FONT", FONT.getname()[0])
    if scenario in ("pixel", "arrange-fixes"):
        env["SINGULARITY_PIXEL_MOUSE"] = "1"
    cat = ["--no-catalogue"]
    if scenario.startswith("idle"):
        env["SINGULARITY_PIXEL"] = "1" if scenario == "idle-pixels" else "0"
    if scenario.startswith("well"):
        env["SINGULARITY_PIXEL"] = "1" if scenario in ("well-pixels", "well-hover") else "0"
    if scenario.startswith("home") or scenario in ("demo", "library", "queue", "lyrics", "artist", "search", "search-recent", "search-timing", "like", "config", "designs", "readme", "layouts", "tiling", "tiling-fast", "tiling-paths", "listening", "wheel", "home-scroll", "history", "meters", "jam", "stats", "stats-widgets", "ambient", "arrows", "popup", "stalls", "queue-lift", "queue-wheel", "gesture", "wheel-burst", "touchpad", "popup-wheel", "wheel-all", "settings", "backup-notice", "show", "book", "mini", "playlists", "playlist-details", "playlist-add", "playlist-remove", "playlist-move", "playlist-cover", "themes", "widgets-large", "karaoke", "karaoke-cells", "sizes", "design-sizes", "pane-menu", "contact-sheet", "paneless", "arrange-fixes"):
        here = os.path.dirname(os.path.abspath(__file__))
        env["XDG_CACHE_HOME"] = f"{here}/xdgcache"
        env["SINGULARITY_PIXEL"] = "1" if scenario in ("home-pixels", "designs", "readme", "home-scroll", "ambient", "popup", "tiling", "tiling-fast", "tiling-paths", "resize", "karaoke", "arrange-fixes") else "0"
        if scenario == "design-sizes":
            env["SINGULARITY_PIXEL"] = os.environ.get("PTY_PIXEL", "1")
        if scenario == "sizes":
            env["SINGULARITY_PIXEL"] = os.environ.get("PTY_PIXEL", "0")
        # kitty's layer under cell backgrounds (#41); the harness composites it as kitty does
        env["SINGULARITY_BELOW_BG"] = "1" if scenario in ("popup", "ambient") else "0"
        cat = ["--fake-catalogue", f"{ROOT}/crates/client/tests/fixtures/home.json"]
    if scenario == "listening":
        env["SINGULARITY_DATA"] = f"{os.path.abspath(out)}/data/singularity"
        env["XDG_DOWNLOAD_DIR"] = f"{os.path.abspath(out)}/downloads"
        env["SINGULARITY_BACKUP_CHECK_SECS"] = "2"
        os.makedirs(env["XDG_DOWNLOAD_DIR"], exist_ok=True)
        seed_listening(env["SINGULARITY_DATA"])
    if scenario == "backup-notice":
        env["SINGULARITY_DATA"] = f"{os.path.abspath(out)}/data/singularity"
        seed_backup(env["SINGULARITY_DATA"], os.environ.get("PTY_BACKUP") != "whole", "someone" if os.environ.get("PTY_BACKUP") == "other" else "fake")
        env["SINGULARITY_BACKUP_CHECK_SECS"] = "2"
    if scenario == "stalls":
        env["SINGULARITY_FAKE_STALLS"] = "1"
    if scenario in ("stats", "stats-widgets", "widgets-large", "sizes", "wheel-all", "contact-sheet"):
        env["SINGULARITY_DATA"] = f"{os.path.abspath(out)}/data/singularity"
        seed_listening(env["SINGULARITY_DATA"])
    if scenario in ("widgets-large", "sizes", "contact-sheet"):
        env["SINGULARITY_FAKE_STALLS"] = "fine"
        seed_today(env["SINGULARITY_DATA"])
        os.makedirs(f"{env['XDG_CONFIG_HOME']}/singularity/layouts", exist_ok=True)
        if scenario == "widgets-large":
            open(f"{env['XDG_CONFIG_HOME']}/singularity/layouts/big.toml", "w").write(BIG_LAYOUT)
    if scenario == "wheel-all":
        os.makedirs(f"{env['XDG_CONFIG_HOME']}/singularity/layouts", exist_ok=True)
        open(f"{env['XDG_CONFIG_HOME']}/singularity/layouts/wheel.toml", "w").write(WHEEL_LAYOUT)
        open(f"{env['XDG_CONFIG_HOME']}/singularity/config.toml", "w").write('schema = 6\n[ui]\ndesign = "singularity"\nlayout = "wheel"\n')
        # recent searches, more than a short Pane shows
        os.makedirs(f"{env['XDG_STATE_HOME']}/singularity", exist_ok=True)
        open(f"{env['XDG_STATE_HOME']}/singularity/recent-searches.json", "w").write(json.dumps([f"recent query {i}" for i in range(12)]))
    if scenario == "search-recent":
        os.makedirs(f"{env['XDG_STATE_HOME']}/singularity", exist_ok=True)
        open(f"{env['XDG_STATE_HOME']}/singularity/recent-searches.json", "w").write(json.dumps(["mix", "orbit"]))
    if scenario == "resize":
        os.makedirs(f"{env['XDG_CONFIG_HOME']}/singularity/layouts", exist_ok=True)
        open(f"{env['XDG_CONFIG_HOME']}/singularity/layouts/wide.toml", "w").write(RESIZE_LAYOUT)
        open(f"{env['XDG_CONFIG_HOME']}/singularity/config.toml", "w").write('schema = 6\n[ui]\ndesign = "cockpit"\ntheme = "ember"\nlayout = "wide"\n')
    if scenario.startswith("karaoke"):
        os.makedirs(f"{env['XDG_CONFIG_HOME']}/singularity", exist_ok=True)
        open(f"{env['XDG_CONFIG_HOME']}/singularity/config.toml", "w").write('schema = 6\n[ui]\ndesign = "karaoke"\n')
    if scenario == "tiling-fast":
        os.makedirs(f"{env['XDG_CONFIG_HOME']}/singularity", exist_ok=True)
        open(f"{env['XDG_CONFIG_HOME']}/singularity/config.toml", "w").write("schema = 6\n[visuals]\nmotion_fps = 240\n")
        env["SINGULARITY_FRAME_LOG"] = f"{os.path.abspath(out)}/frames.log"
    # demo mode (D90): `demo` walks every page with the switch on (PTY_DEMO=0: off); PTY_DEMO=1 turns it on for any
    # other scenario (`wheel-all`), both on the fake account with made-up personal values
    if scenario == "demo" or os.environ.get("PTY_DEMO") == "1":
        env["SINGULARITY_FAKE_PERSONAL"] = "1"
        # (PTY_DEMO=0: no switch at all; the Client refuses a "0" since #167)
        if os.environ.get("PTY_DEMO") == "0":
            env.pop("SINGULARITY_DEMO", None)
        else:
            env["SINGULARITY_DEMO"] = "1"
    if scenario == "demo":
        # the song left playing from a normal session (#128): hidden until a Demo playlist plays
        env["SINGULARITY_FAKE_CARRYOVER"] = "1"
        demo_setup(env, out)
    elif os.environ.get("PTY_DEMO") == "1":
        # another scenario in demo mode: Liked Songs in the Demo set, and the seeded recent searches in demo mode's file
        os.makedirs(f"{env['XDG_CONFIG_HOME']}/singularity", exist_ok=True)
        open(f"{env['XDG_CONFIG_HOME']}/singularity/demo.toml", "w").write("liked = true\n")
        recent = f"{env['XDG_STATE_HOME']}/singularity/recent-searches.json"
        if os.path.exists(recent):
            shutil.copy(recent, recent.replace(".json", "-demo.json"))
    bus = None
    if scenario == "mpris":
        bus, addr = private_bus(out)
        # MPRIS on the private bus (the Daemon's session bus here), for a fake Engine too
        env["DBUS_SESSION_BUS_ADDRESS"] = addr
        env["SINGULARITY_NOWPLAYING"] = "on"
    d = subprocess.Popen([f"{BIN}/singularityd", "--fake-engine", *cat], env=env, stdout=subprocess.DEVNULL, stderr=open(f"{out}/daemon.log", "w"))
    try:
        time.sleep(0.4)
        if d.poll() is not None:
            # a Client would start a Daemon of its own: stop here instead
            sys.exit(f"the Daemon exited ({open(f'{out}/daemon.log').read().strip().splitlines()[-1:]}); built with `cargo build -p singularityd --features fake`?")
        if scenario == "sizes":
            return sizes(env, out)
        if scenario == "design-sizes":
            return design_sizes(env, out)
        if scenario == "contact-sheet":
            return contact_sheet(env, out)
        t = Term(env)
        if scenario == "space-startup":
            return space_startup(t, out, env)
        t.pump(1.5)
        if scenario == "home-scroll":
            return home_scroll(t, out)
        if scenario == "home-search":
            return home_search(t, out)
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
        if scenario == "backup-notice":
            return backup_notice(t, out)
        if scenario == "stats":
            return stats(t, out)
        if scenario == "stats-widgets":
            return stats_widgets(t, out)
        if scenario == "widgets-large":
            return widgets_large(t, out)
        if scenario == "resize":
            return resize(t, out)
        if scenario == "ambient":
            return ambient(t, out)
        if scenario == "arrows":
            return arrows(t, out)
        if scenario == "popup":
            return popup(t, out)
        if scenario == "stalls":
            return stalls(t, out)
        if scenario == "show":
            return show(t, out)
        if scenario == "book":
            return book(t, out)
        if scenario == "settings":
            return settings(t, out, env)
        if scenario == "themes":
            return themes(t, out)
        if scenario.startswith("karaoke"):
            return karaoke(t, out)
        if scenario == "queue-wheel":
            return queue_wheel(t, out)
        if scenario == "queue-lift":
            return queue_lift(t, out)
        if scenario == "gesture":
            return gesture(t, out)
        if scenario == "playlists":
            return playlists(t, out)
        if scenario == "playlist-cover":
            return playlist_cover(t, out)
        if scenario == "playlist-move":
            return playlist_move(t, out)
        if scenario == "playlist-remove":
            return playlist_remove(t, out)
        if scenario == "playlist-add":
            return playlist_add(t, out)
        if scenario == "playlist-details":
            return playlist_details(t, out)
        if scenario == "mini":
            return mini(t, out)
        if scenario == "wheel":
            return wheel(t, out)
        if scenario == "wheel-burst":
            return wheel_burst(t, out)
        if scenario == "touchpad":
            return touchpad(t, out)
        if scenario == "popup-wheel":
            return popup_wheel(t, out)
        if scenario == "wheel-all":
            wheel_all(t, out)
            return demo_check(out)
        if scenario == "demo":
            return demo(t, out, env)
        if scenario == "history":
            return history(t, out)
        if scenario == "meters":
            return meters(t, out)
        if scenario == "jam":
            return jam(t, out)
        if scenario == "config":
            return config(t, out, env)
        if scenario == "designs":
            return designs(t, out)
        if scenario == "readme":
            return readme(t, out)
        if scenario == "layouts":
            return layouts(t, out, env)
        if scenario == "delete-design":
            return delete_design(t, out, env)
        if scenario == "tiling":
            return tiling(t, out, env)
        if scenario == "paneless":
            return paneless(t, out)
        if scenario == "arrange":
            return arrange(t, out)
        if scenario == "arrange-keys":
            return arrange_keys(t, out)
        if scenario == "arrange-fixes":
            return arrange_fixes(t, out)
        if scenario == "pane-menu":
            return pane_menu(t, out)
        if scenario == "new-design":
            return new_design(t, out, env)
        if scenario == "tiling-fast":
            return tiling_fast(t, out, env)
        if scenario == "tiling-paths":
            return tiling_paths(t, out, env)
        if scenario == "artist":
            t.pump(1.0)
            x, y = t.find("♥  Liked Songs")
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
        if scenario == "search-timing":
            # #126: the Client's share of a search (the fake Catalogue answers at once): last key to results drawn
            t.pump(1.0)
            t.send(b"\x0c", 0.3)  # Ctrl+L
            times = []
            for i, q in enumerate((b"mix", b"orbit", b"nova")):
                if i:
                    t.send(b"\x1b", 0.5)  # Esc: an empty query
                for c in q[:-1]:
                    t.send(bytes([c]), 0.08)
                began = time.time()
                t.send(q[-1:], 0)
                want = f"{q.decode()} song 1"
                while not t.find(want) and time.time() - began < 5:
                    t.pump(0.005)
                times.append(round((time.time() - began) * 1000))
                print(f"“{q.decode()}”: results on screen {times[-1]} ms after the last key")
            t.shot(f"{out}/0-search.png")
            t.send(b"\x1b", 0.2); t.send(b"\x1b", 0.2); t.send(b"\x1b", 0.3)
            t.send(b"q", 0.5)
            return
        if scenario == "search-recent":
            # #8: a search, cleared with Esc, then picked again from the recent searches (Enter, then a click)
            t.pump(1.0)
            t.send(b"\x0c", 0.3)  # Ctrl+L
            t.send(b"mix", 1.2)
            t.shot(f"{out}/0-searched.png")
            t.send(b"\x1b", 0.5)
            t.shot(f"{out}/1-cleared.png")
            print("recent searches listed:", bool(t.find("Recent searches")))
            t.send(b"\r", 0.6)
            t.shot(f"{out}/2-enter.png")
            print("Enter on the recent search shows results:", bool(t.find("mix song 1")))
            t.send(b"\x1b", 0.5)
            x, y = t.find("  orbit")
            mouse(t, 0, x + 3, y); mouse(t, 0, x + 3, y, release=True, wait=0.6)
            t.shot(f"{out}/3-click.png")
            print("a click on a recent search shows results:", bool(t.find("orbit song 1")))
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
            x, y = t.find("♥  Liked Songs")
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
            # and stay still: the meters 2 s later (the black hole turns and the rain drifts by design: not compared)
            top = next(i for i, l in enumerate(t.screen.display) if "loudne" in l)
            still = t.screen.display[top:]
            t.pump(2.0)
            print("paused loudness/fft still:", still == t.screen.display[top:])
            t.send(b"\x0b", 0.3); t.send(b"design", 0.3); t.send(b"\r", 0.6)
            for _ in range(4):
                t.send(b"\x1b[B", 0.4)  # down to Terminal: osc, rain, fft
            t.send(b"\r", 1.0)
            t.send(b" ", 2.5)
            t.shot(f"{out}/2-terminal-playing.png")
            t.send(b" ", 2.0)
            t.shot(f"{out}/3-terminal-paused.png")
            rain = next(i for i, l in enumerate(t.screen.display) if "rain --audio" in l)
            x0 = t.screen.display[rain].index("rain --audio") - 3
            x1 = x0 + 38  # the rain Pane's columns, masked
            def no_rain():
                return [l[:x0] + l[x1:] for l in t.screen.display]
            before = no_rain()
            t.pump(2.0)
            t.shot(f"{out}/4-terminal-paused-later.png")
            print("Terminal paused, osc/fft still:", before == no_rain())
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
            shutil.rmtree(f"{RUNTIME}/sgl-bus-{os.getpid()}", ignore_errors=True)


if __name__ == "__main__":
    main()
