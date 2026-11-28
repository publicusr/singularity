#!/usr/bin/env python3
"""Record a mouse wheel's or touchpad's raw reports in this terminal, with their timing (#32, D86).

usage: python3 tools/wheel-record.py <out.txt> [phase ...]

Run it in the terminal you use Singularity in (Ghostty, kitty, …), full screen or not. For each phase (default:
`notches spin slow flick`) it asks you to scroll, logs every wheel report the terminal sends, and moves on when you press
Enter; q stops early. The file has the same format as `crates/client/tests/fixtures/wheel-touchpad-ghostty.txt`: one
report per line, `<phase> <ms since the phase's first report> <up|down|left|right>`. Nothing leaves the machine, it
makes no sound, and the terminal is put back as it was when it ends (also on Ctrl+C).
"""
import os, re, select, sys, termios, time, tty

PROMPTS = {
    "notches": "turn the mouse wheel slowly, one notch at a time, about ten notches",
    "spin": "spin the mouse wheel fast, down then up",
    "slow": "touchpad: a slow two-finger drag",
    "flick": "touchpad: one fast flick",
    "nudges": "touchpad: five tiny one-row nudges",
}
# SGR mouse reports: button code, column, row, press (M) or release (m)
REPORT = re.compile(rb"\x1b\[<(\d+);(\d+);(\d+)([Mm])")
WHEEL = {64: "up", 65: "down", 66: "left", 67: "right"}


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    out, phases = sys.argv[1], sys.argv[2:] or ["notches", "spin", "slow", "flick", "nudges"]
    fd = sys.stdin.fileno()
    saved = termios.tcgetattr(fd)
    lines = []
    try:
        tty.setraw(fd)
        # mouse reporting (button events, SGR encoding): the wheel arrives as buttons 64–67
        os.write(sys.stdout.fileno(), b"\x1b[?1000h\x1b[?1006h")
        for phase in phases:
            say(f"\r\n{phase}: {PROMPTS.get(phase, 'scroll')}; Enter when done (q stops)\r\n")
            first, buf, done = None, b"", False
            while not done:
                r, _, _ = select.select([fd], [], [], 0.5)
                if not r:
                    continue
                data = os.read(fd, 4096)
                now = time.monotonic()
                buf += data
                for m in REPORT.finditer(buf):
                    code = int(m.group(1)) & ~(4 | 8 | 16)  # without Shift, Alt, Ctrl
                    if m.group(4) == b"M" and code in WHEEL:
                        first = now if first is None else first
                        lines.append(f"{phase} {(now - first) * 1000:.1f} {WHEEL[code]}")
                # keep a report cut in half by the read for the next one
                tail = buf.rfind(b"\x1b")
                buf = buf[tail:] if tail >= 0 and not REPORT.match(buf[tail:]) else b""
                rest = REPORT.sub(b"", data)
                if b"q" in rest or b"\x03" in rest:
                    phases = []
                    break
                done = b"\r" in rest or b"\n" in rest
            say(f"  {sum(l.startswith(phase + ' ') for l in lines)} reports\r\n")
            if not phases:
                break
    finally:
        os.write(sys.stdout.fileno(), b"\x1b[?1006l\x1b[?1000l")
        termios.tcsetattr(fd, termios.TCSADRAIN, saved)
    with open(out, "w") as f:
        f.write(f"# Wheel reports recorded with tools/wheel-record.py ({os.environ.get('TERM_PROGRAM') or os.environ.get('TERM', '?')}).\n")
        f.write("# One report per line: phase, ms since the phase's first report, direction.\n")
        f.write("".join(l + "\n" for l in lines))
    print(f"{len(lines)} reports in {out}")


def say(s):
    os.write(sys.stdout.fileno(), s.encode())


if __name__ == "__main__":
    main()
