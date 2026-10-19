"""#66: the setup wizard and health check with the Spotify app as Engine (D51), in a pty. Never touches the real app
(SINGULARITY_START_APP=0) or the owner's login (scratch dirs, no keyring). Three Daemons in turn:
no Catalogue (the app can't be driven), a real but logged-out Catalogue (log in first), the fake Catalogue (logged in:
waiting for the app on Connect, and the health check)."""
import os, subprocess, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import shot
out = os.path.abspath(sys.argv[1]); os.makedirs(out, exist_ok=True)
sock = f"{shot.RUNTIME}/sgl-appw-{os.getpid()}.sock"
env = dict(os.environ, SINGULARITY_ENGINE="spotify-app", SINGULARITY_START_APP="0", SINGULARITY_SOCKET=sock, TERM="xterm-256color",
           SINGULARITY_NO_KEYRING="1", SINGULARITY_PIXEL="0", SINGULARITY_PIXEL_MOUSE="0", SINGULARITY_DATA="off",
           XDG_DATA_HOME=f"{out}/data", XDG_CONFIG_HOME=f"{out}/config", XDG_CACHE_HOME=f"{out}/cache", XDG_STATE_HOME=f"{out}/state")
for k in ("KITTY_WINDOW_ID", "TERM_PROGRAM", "GHOSTTY_RESOURCES_DIR", "TMUX", "SOLOIST_API_KEY"):
    env.pop(k, None)


def phase(name, args, health=False):
    d = subprocess.Popen([f"{shot.BIN}/singularityd", *args], env=env, stderr=open(f"{out}/{name}-daemon.log", "w"))
    try:
        time.sleep(0.8)
        t = shot.Term(env)
        t.pump(2.0)
        t.shot(f"{out}/{name}.png")
        print(f"== {name}")
        print("\n".join(l.strip()[:120] for l in t.screen.display if any(w in l for w in ("Spotify app", "Log in", "Connect", "Can't play", "You need", "Audio check", "›"))))
        if health:
            t.send(b"\x1b", 0.3)
            t.send(b"\x0b", 0.3); t.send(b"health", 0.3); t.send(b"\r", 1.0)
            t.shot(f"{out}/{name}-health.png")
            print("health:", [l.strip()[:110] for l in t.screen.display if any(w in l for w in ("Engine", "Soloist key", "Spotify login"))])
        t.send(b"\x1b", 0.3); t.send(b"q", 0.5)
    finally:
        d.terminate(); d.wait(5)


phase("0-no-catalogue", ["--no-catalogue"])
phase("1-logged-out", [])
phase("2-waiting-on-connect", ["--fake-catalogue", f"{shot.ROOT}/crates/client/tests/fixtures/home.json"], health=True)
