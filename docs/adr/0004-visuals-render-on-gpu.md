# Pixel-tier visuals render on the GPU, with a CPU fallback

Pixel-tier visuals (the black hole first) are computed with wgpu compute shaders off-screen, read
back, and handed to the terminal through kitty's shared-memory transfer (`t=s`, no zlib/base64).
A music app stays open all day, and in the prototype the CPU tracer cost 4–6 cores for one
full-resolution black hole. Measured on the owner's machine (Ryzen 7 7800X3D, RX 7900 GRE, kitty
window 2520×1350 px): **Inferno at full resolution 626% CPU on the CPU path, 14% on the GPU path.**
The remaining ~14% is terminal transfer, layout and audio analysis. The CPU tracer stays as a
fallback (no adapter, `SINGULARITY_CPU=1`). The cheapest looks (pixel art at ¼ resolution) are
affordable there.

## Consequences

- Visual code is written twice (WGSL + Rust) unless the CPU fallback is limited to the cheap looks.
  Decide when building the Visual engine.
- Apple Silicon (M2 Air, 8 GB) gets Metal through wgpu. Expected fine, but it needs a measurement in the macOS spike.
- Shared-memory segments are swept after 2 s if the terminal never consumed them (tmux, other
  terminals), so a misbehaving terminal can't leak RAM. Non-kitty terminals use the direct transfer path.
