# Design-direction prototype (throwaway)

Five complete looks for Singularity, so they can be judged by eye instead of in prose. The data is fake
(invented artists, titles and lyrics) and there is no Spotify connection. **The visuals react to the
audio your PC is actually playing** (PipeWire monitor via `parec`, like cava does), so play music in any
app while you look. With silence it falls back to a built-in synthetic groove.

```sh
cargo run --release
```

Best in kitty / ghostty / wezterm (pixel tier: real-resolution black hole and covers). Any other
truecolor terminal gets the cell tier (half-blocks). Toggle with `p` to compare the two.

| # | Direction | Idea |
|---|-----------|------|
| 1 | **Event Horizon** | OLED black, cream + ice. The black hole owns the top-right quadrant; default disk look *pixel ember* (orange/red pixel art). |
| 2 | **Classic+** | The Spotify desktop app on OLED black: cover cards, "Now playing" panel with a proper spectrum. |
| 3 | **Cockpit** | btop energy on OLED. `t` cycles palettes (btop, ice, amber, violet, mono); everything recolours together. |
| 4 | **Singularity** (default) | Terminal layout + cockpit instruments around a **gravity well**: the hole has its own pane, its frame sags toward it, neighbouring borders bend toward it, and the starfield inside is lensed. Content never moves. |
| 5 | **Terminal** | Phosphor on OLED, file-tree library, matrix rain, lyrics log, calm scope. `t`: green / amber CRT / ice. |

**Idle mode:** after 3 minutes without input (or press `i` to preview), the black hole on screen breaks out of its
pane and drifts over the whole UI, bending its text. Any key or click wakes it and is swallowed. `Shift+I` (or the
palette) switches the takeover off.

**GPU:** the black hole renders with wgpu compute shaders (the adapter is shown in a toast at start) and reaches kitty
through shared memory. `SINGULARITY_CPU=1` forces the CPU tracer for comparison (much heavier).

Things to try: click everything (tabs, transport, seek bar drag, volume drag, library rows, lyric lines,
the hero visual), `Ctrl+K` / `:` for the command palette, `v` to swap the visual / disk look, `t` palette,
`i` idle preview, `?` for help.

`--snapshot DIR` renders every direction headlessly (cell dumps + PPM images of the pixel visuals).

## Credits

The black hole (`src/blackhole.rs`) is a CPU/Rust port of
[s0xDk/ghostty-blackhole](https://github.com/s0xDk/ghostty-blackhole) (MIT, © its author), which is
modelled on [Eric Bruneton's black hole shader](https://ebruneton.github.io/black_hole_shader/).
