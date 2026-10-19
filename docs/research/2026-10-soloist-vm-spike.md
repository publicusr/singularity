# Spike: Soloist in a Linux VM on macOS (D53, 2026-10-19)

Question (owner): could a lossless player like Soloist run on macOS, and would it cost less than the Spotify app
(~600 MB, 5–25 % CPU while playing)? Soloist is Linux-only, so: Soloist inside a tiny Linux VM, its audio sent to
the Daemon.

## Setup (all on the M2 Air, macOS 15.7, nothing left running)

- **VM**: `vfkit` 0.6.4 (Homebrew; Red Hat's CLI over Apple's Virtualization framework), EFI boot of Debian 13's
  arm64 `genericcloud` image, cloud-init seed (SSH key, `pulseaudio`, `libatomic1`, `socat`), 2 vCPUs.
- **Network**: `gvproxy` 0.8.9 (gvisor-tap-vsock, user-mode networking: the guest's connections are made by a normal
  Mac process). Needed: the owner runs a VPN (a `utun` default route, Mullvad-style), and the VPN's firewall drops
  the Virtualization framework's NAT (vmnet) traffic, so the guest had no internet. Through gvproxy it goes through
  the VPN like any app's traffic, and port forwards are an HTTP call.
- **Soloist**: the CDN has an arm64 build (`soloist_release_arm64.tar.gz`, 13 MB; 1.3.9.36, glibc, PulseAudio).
  It runs in the guest unchanged.

## Measured

| What | Result |
|------|--------|
| Boot to SSH | ~20 s (stock cloud image, not tuned) |
| Memory | The framework keeps every guest page it has touched, and the guest's page cache fills its RAM: **the cost is the RAM given to the VM**. Physical footprint: 1024 MB VM → 1032 MB; 320 MB → 326 MB; **192 MB → 199 MB**, with Soloist running and 72 MB still free in the guest. vfkit has a virtio balloon to give memory back. |
| Idle CPU | VM 0–0.4 %, gvproxy ~0 % |
| Audio out of the VM | PulseAudio null sink's monitor → `parec` → TCP → gvproxy → the Mac: 345 KB/s (44.1 kHz float stereo, real time) for ~0.2–0.3 % VM + 0.2 % gvproxy CPU. On the Mac side this PCM would go to cpal and the AudioTap, like librespot's: the audio Widgets would work, no capture permission. |
| Soloist, no key | Starts, ~22 MB RSS, opens its ZeroConf pairing endpoint (`/zc` on a random port) and mDNS (inside the VM only) |
| Pairing endpoint from the Mac | Reachable through a gvproxy port forward (`getInfo` answered: `deviceType` SPEAKER, `remoteName` as given). For the phone, the Mac would forward a LAN port and announce it with `dns-sd -P` (Bonjour proxy registration). |

Not measured (needs the owner's Soloist API key and a pairing from the phone): CPU and memory while streaming
lossless, and the phone actually pairing through the proxy.

## Estimate

~200–270 MB (a 192–256 MB VM + ~10–20 MB of gvproxy) and probably 1–3 % CPU while playing (Soloist's decode + the
audio hop), against the app's ~600 MB and 5–25 %. So: roughly a third to half of the app's memory, a fraction of its
CPU, lossless, standalone (no Spotify app), Widgets working. librespot stays far lighter (~26 MB) but never lossless.

## What building it would take

- Ship nothing of Spotify's: the Daemon downloads Soloist from the CDN inside the VM, as on Linux (D3 rules).
- A minimal guest image (Debian or Alpine-with-glibc, kernel + rootfs, built by a script, downloaded on first use,
  ~100–200 MB on disk), booted directly (kernel + initrd, no EFI) to cut the boot time.
- The Daemon starts and stops the VM (Virtualization framework via `objc2-virtualization`, or vfkit as a helper;
  needs the `com.apple.security.virtualization` entitlement, which ad-hoc signing allows locally) and gvproxy (or the
  same user-mode stack in-process).
- The existing SoloistEngine talks to Soloist's WebSocket through a forwarded port: almost all of its code is reused.
- Audio: a vsock or TCP stream of PCM from the guest's PulseAudio to the Daemon, played through cpal and fed to the tap.
- Pairing once from the phone: a LAN port forward + a Bonjour proxy record for Soloist's ZeroConf (with a VPN, its
  "local network sharing" must allow the phone in). Possibly the Daemon could pair Soloist itself with the Catalogue's
  login (ZeroConf `addUser`), which would drop the phone step on Linux too: unverified.
- Risks: VM memory can't be shared with the Mac the way an app's can; sleep/wake and network changes need handling;
  one more moving part to keep updated.
