# Owner app tests

The owner tests the installed app once a spec's tickets are merged into its integration branch, and again for the
fixes. They are not an engineer: write for what they see on screen.

## The checklist

- Install the integration branch (`cargo install` of `crates/daemon` and `crates/client` through `run-hidden`), say
  which commit, and that a restart replaces the running Daemon.
- Number every item by section (A1, A2 … B1 …) so the owner answers by number. Save a copy in `../private/` for ticking off.
- Each item says **what to do and what they should see** ("drag a title onto another Pane: the two swap, nothing else
  moves"), never what the code does. Name things the way the app shows them. Explain any term once, in the item.
- Mark items that are judgement calls (a feel, a look) and the open questions they settle.

## After the report

- Sort the report into: bugs to fix before the merge, answers to the owner's questions, and later tickets. Put new
  decisions as numbered questions with a recommendation.
- The **re-test list** has only what changed since the last test, numbered R1, R2 …
- List separately every call you made yourself while fixing, so the owner can overrule it.
