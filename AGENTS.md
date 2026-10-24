# Alchemy Journey — agent orientation

A Slay-the-Spire-like deckbuilder with Noita-style spell assembly, in Bevy.

## Read these first, in order

1. **COMBAT.md** — the combat core's design rulings (boundary, vocabulary,
   validate/process, turn structure, multiplayer model). Amend before coding.
2. **DESIGN.md** — settled game-design decisions, roadmap, and the list of
   explicitly open questions. Do not re-litigate settled decisions; do not
   settle open ones without the owner.
3. `devlog/` — history and rationale, including honest accounts of wrong
   turns (useful context for *why* the rulings are what they are).

## Working rules

- Design first: discuss and get explicit approval before implementing
  architecture or algorithms. Unapproved implementation gets reverted.
- Green gate: `scripts/test.sh` (fmt + clippy -D warnings + test) must pass
  before committing. CI enforces it.
- The combat core (`crates/combat`) is pure Rust — no Bevy, no wall-clock,
  no networking. The dependency graph enforces this; don't weaken it.
- Commits: small, one concern each, message states the reason.
