# Singularity

A terminal Spotify client for Linux and macOS that feels like the desktop app, plays lossless audio,
and looks spectacular: a Home feed, Made For You, full library, mouse *and* keyboard, btop-grade themes, and
built-in audio-reactive visuals (yes, including a black hole).

**Status:** Phase 0, planning. The only code is the throwaway design prototype in `prototypes/design-directions/`.

## Docs

| Doc | What's in it |
|-----|--------------|
| [docs/SPEC.md](docs/SPEC.md) | Problem, solution, user stories, (proposed) implementation decisions |
| [docs/DESIGN-TREE.md](docs/DESIGN-TREE.md) | Every open decision, its dependencies, and the grilling round log |
| [docs/ROADMAP.md](docs/ROADMAP.md) | Phased plan, spikes first |
| [docs/research/2026-10-platform-constraints.md](docs/research/2026-10-platform-constraints.md) | What Spotify does and doesn't allow in 2026, with sources |
| [docs/ideas/visuals-and-widgets.md](docs/ideas/visuals-and-widgets.md) | Brainstorm of visuals, widgets and layout modes |
| [GLOSSARY.md](GLOSSARY.md) | The project's vocabulary (Engine, Catalogue, Shelf, Widget, Pane, …) |
| [docs/adr/](docs/adr/) | Hard-to-reverse decisions: Soloist Engine, private-API Catalogue, Daemon + Client, GPU visuals |
| [docs/research/2026-10-private-api-risk.md](docs/research/2026-10-private-api-risk.md) | How risky the private API is for a one-person client |
| [docs/research/2026-10-macos.md](docs/research/2026-10-macos.md) | How macOS support works (official app as Engine) |
| [prototypes/design-directions/](prototypes/design-directions/) | Five runnable design directions with a GPU black hole (`cargo run --release`) |

## Process

Planning follows Matt Pocock's [skills](https://github.com/mattpocock/skills):
`grill-with-docs` (interview in rounds over a design tree, write glossary + ADRs as we go), then
`to-spec`, then tickets, then implementation. The grilling skills are vendored in `.claude/skills/`,
so `/grill-with-docs` works in any Claude Code session opened here.
