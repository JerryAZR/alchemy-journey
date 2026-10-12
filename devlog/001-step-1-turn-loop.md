# Devlog 001 — Step 1: The Turn Loop, and a Mistake We Committed on Purpose

*A series on building a Slay-the-Spire-like deckbuilder (with Noita-style spell
assembly) in Bevy, designed and written in collaboration with an AI assistant.*

## What Step 1 is

The smallest slice that is still a *game loop* you can play:

- 11 identical "Inert Draught" potions (cost 0, do nothing, consumed when played)
- One player (80 HP), one enemy that does nothing (30 HP)
- The full turn loop: draw to 5, play cards, end turn, enemy passes, next turn
- Discard pile reshuffles into the deck when the deck runs out
- Minimal UI: colored rectangles, text, clickable cards, an End Turn button

No damage, no effects, no enemy behavior. The point was to prove the plumbing —
piles, draws, reshuffles, consumes — with something you can actually click through.

## The process: design first, loop first

Two process decisions shaped this step more than any code did.

**Design before code.** We spent several rounds on architecture before writing a
line: how Noita-style sequential spell evaluation maps to a turn-based card game,
whether potions and spells share an evaluation engine, how to keep the map
topology decoupled from node content. Some of those debates mattered a lot — for
example, we initially conflated "the engine only knows cards" (card *lifecycle*:
cost, piles, consume) with "one effect representation" (effect *structure*).
Those are separate questions; potions will be authored as simple flat effect
lists with their own invariants, but lowered into one shared IR executed by one
interpreter — the same way Rust's safety is enforced at the language level while
running on the same machine code as C.

**Loop first, not core first.** The original AI proposal was to build a complete
"combat domain core" up front — full effect vocabulary, status system, intents —
headless and unit-tested, before anything was playable. We rejected that. The
well-understood part (StS combat) isn't where the risk is; the risk is the
interface between the card engine and the future spell compiler, and designing
that interface before understanding it is guessing. So instead: playable loop
first, features one at a time, then an explicit review/refactor checkpoint once
we know what the features actually need.

## The structural mistake (committed deliberately)

Here's the honest part. The End Turn button does this:

```rust
combat.0.end_player_turn();   // fine: this is player intent
combat.0.run_enemy_turn();    // not fine: this is game flow
```

The UI layer is **driving the combat state machine**, not just forwarding player
input. You can see the symptom in the logic core: `phase` is written in three
places and read only by guards. The `EnemyTurn` state exists for a few
microseconds inside the button handler and is never observable — the UI always
renders between frames, when `phase` is always `PlayerTurn`. The state machine
is degenerate: you could delete the `EnemyTurn` variant and nothing visible
would change.

Why it's wrong, not just ugly:

- **Wrong layer owns pacing.** When the enemy turn gains real behavior (attacks,
  buffs, multiple enemies), someone must decide how it unfolds over time —
  delays, animation, sequencing. Right now that "someone" is a button handler.
- **Guards that can't guard.** `play_card` rejects plays during `EnemyTurn`,
  but the UI never lets `EnemyTurn` exist across a frame, so the guard is
  load-bearing in tests only.
- **It will get patched around.** This is how "temporary" couplings ossify:
  the next feature grows around the button handler because that's where the
  calls already are.

Why we committed it anyway: it works, it's tested, and it's a honest snapshot
of how the code actually evolved. The fix — the **intents vs. driver split**
(UI emits player intents only; a separate driver advances autonomous states like
the enemy turn) — is the next commit, and it's much easier to explain against
real code than against a hypothetical.

## Smaller course corrections (process lessons, not typos)

- **Hand-rolled RNG → `rand`.** The first implementation had a hand-written
  xorshift64 + Fisher-Yates to "keep the logic core dependency-free." Weak
  reason: Bevy has no built-in RNG, `rand`'s `StdRng::seed_from_u64` gives
  determinism anyway, and the hand-rolled shuffle had modulo bias. Lesson:
  "no dependencies" is not a virtue when the dependency is the battle-tested
  standard for exactly your problem.
- **In-UI log → terminal log.** The playtest question "did the last card draw
  happen before or after the reshuffle?" was unanswerable from a 5-line UI log
  panel. The log moved to the terminal, per-event. Lesson: for prototype
  playtesting, *observability* of the simulation beats presentation.

## What's next

1. The intents/driver split (fixing the flaw above).
2. Step 2: first real card effects — Strike/Defend-like potions — which means
   the first slice of the effect IR (`Atom`, `Sequence`, `WithTarget`) and its
   interpreter.
