"""First run in a pty: a real Daemon (real SoloistEngine with a stand-in binary) with no key, no keyring, scratch dirs."""
import os, stat, subprocess, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import shot
here = os.path.dirname(os.path.abspath(__file__))
fresh = f"{here}/fresh"
sock = f"{os.environ['XDG_RUNTIME_DIR']}/sgl-wiz-{os.getpid()}.sock"
env = dict(os.environ, SINGULARITY_SOCKET=sock, TERM="xterm-256color", SINGULARITY_NO_KEYRING="1",
           XDG_DATA_HOME=f"{fresh}/data", XDG_CONFIG_HOME=f"{fresh}/config", XDG_CACHE_HOME=f"{fresh}/cache", XDG_STATE_HOME=f"{fresh}/state")
for k in ("KITTY_WINDOW_ID", "TERM_PROGRAM", "GHOSTTY_RESOURCES_DIR", "TMUX", "SOLOIST_API_KEY"):
    env.pop(k, None)
out = sys.argv[1]; os.makedirs(out, exist_ok=True)
d = subprocess.Popen([f"{shot.BIN}/singularityd", "--fake-catalogue", f"{shot.ROOT}/crates/client/tests/fixtures/home.json"], env=env, stderr=open(f"{out}/daemon.log", "w"))
try:
    time.sleep(1.0)
    t = shot.Term(env)
    t.pump(2.0)
    t.shot(f"{out}/0-wizard.png")
    t.send(b"\x1b[200~spak_TESTKEY_not_real\x1b[201~", 0.3)  # bracketed paste
    t.shot(f"{out}/1-key-pasted.png")
    t.send(b"\r", 3.0)
    t.shot(f"{out}/2-after-key.png")
    keyfile = f"{fresh}/config/singularity/secrets/soloist"
    st = os.stat(keyfile)
    print("key file mode", oct(stat.S_IMODE(st.st_mode)), "content ok", open(keyfile).read() == "spak_TESTKEY_not_real")
    print("screen shows key?", "TESTKEY" in t.text())
    t.send(b"\x1b", 0.3)
    t.send(b"\x0b", 0.3); t.send(b"health", 0.3); t.send(b"\r", 1.5)
    t.shot(f"{out}/3-health.png")
    t.send(b"\x1b", 0.3); t.send(b"q", 0.5)
finally:
    d.terminate(); d.wait(5)
