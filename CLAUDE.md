# Working agreements

- Planning runs on the grilling method (`.claude/skills/grilling`): rounds over the design tree in
  `docs/DESIGN-TREE.md`, numbered questions with a ➡️ recommendation each, worded so "yes" accepts.
  Look facts up yourself; put decisions to the owner and wait.
- Do not start implementing until the owner confirms the design-tree frontier is empty and the
  understanding is shared. Spikes in Phase 1 need explicit approval too.
- After each round: update `docs/DESIGN-TREE.md` (status, outcome, round log), write resolved terms
  to `GLOSSARY.md` immediately, and offer an ADR only for decisions that are hard to reverse,
  surprising, and a real trade-off.
- `docs/SPEC.md` is a draft until it is rewritten from the grilling conversation.
- Spotify's platform rules change often. Re-verify facts in `docs/research/` that are more than ~3 months old before
  building on them.
- Never commit secrets (Soloist API key, tokens).
