"""#25: a Daemon too old to know `Shutdown` (played by SINGULARITY_IGNORE_SHUTDOWN) is replaced anyway by a newer Client.
Scratch dirs, no keyring, no tap, no D-Bus, no LocalStore, a dead CDN: the replacement (a real Daemon the Client starts)
can't reach the owner's Soloist, account, audio or bus. Kills only what this script or its Client started."""
import os, socket, struct, subprocess, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import shot

out = os.path.abspath(sys.argv[1]); os.makedirs(out, exist_ok=True)
sock = f"{os.environ['XDG_RUNTIME_DIR']}/sgl-st-{os.getpid()}.sock"
env = dict(os.environ, SINGULARITY_SOCKET=sock, TERM="xterm-256color", COLORTERM="truecolor", SINGULARITY_NO_KEYRING="1",
           SINGULARITY_TAP="off", SINGULARITY_DBUS="off", SINGULARITY_DATA="off", SINGULARITY_SOLOIST_CDN="http://127.0.0.1:9",
           XDG_DATA_HOME=f"{out}/data", XDG_CONFIG_HOME=f"{out}/config", XDG_CACHE_HOME=f"{out}/cache", XDG_STATE_HOME=f"{out}/state")
for k in ("KITTY_WINDOW_ID", "TERM_PROGRAM", "GHOSTTY_RESOURCES_DIR", "TMUX", "SOLOIST_API_KEY", "DBUS_SESSION_BUS_ADDRESS"):
    env.pop(k, None)

def peer_pid():
    s = socket.socket(socket.AF_UNIX)
    try:
        s.connect(sock)
        return struct.unpack("3i", s.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))[0]
    except OSError:
        return None
    finally:
        s.close()

old = subprocess.Popen([f"{shot.BIN}/singularityd", "--fake-engine", "--no-catalogue"], env=dict(env, SINGULARITY_IGNORE_SHUTDOWN="1"),
                       stderr=open(f"{out}/old-daemon.log", "w"))
new_pid = None
try:
    time.sleep(3.0)
    os.utime(f"{shot.BIN}/singularityd")  # "rebuilt" after the old one started
    t0 = time.time()
    t = shot.Term(env)
    while time.time() - t0 < 15 and old.poll() is None:
        t.pump(0.5)
    print(f"old Daemon gone: {old.poll() is not None} (signal {-old.returncode if old.returncode else None}) after {time.time() - t0:.1f}s")
    t.pump(4.0)
    new_pid = peer_pid()
    print(f"a new Daemon answers: pid {new_pid} (old was {old.pid})")
    t.shot(f"{out}/0-after-replace.png")
    print("Client connected:", "Not connected" not in t.text() and "connecting…" not in t.text())
    t.send(b"q", 0.5)
finally:
    if old.poll() is None:
        old.terminate(); old.wait(3)
    # the replacement was started by this script's Client
    if new_pid and new_pid != old.pid:
        os.kill(new_pid, 15)
