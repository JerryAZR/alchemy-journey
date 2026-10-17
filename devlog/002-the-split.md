# Devlog 002 — The Split: Boundaries, Black Boxes, and Talking the AI Out of Over-Engineering

*A series on building a Slay-the-Spire-like deckbuilder (with Noita-style spell
assembly) in Bevy, designed and written in collaboration with an AI assistant.*

## What this round was

Step 1's working-but-flawed turn loop got split across a hard boundary:

- **`crates/combat` (`alchemy-combat`)** — a pure-Rust crate owning all combat
  truth. Depends on `rand`, nothing else. The dependency graph itself forbids
  Bevy, wall-clock time, and networking from leaking into rules.
- **`src/driver.rs`** — the host role: collects player intents, stamps
  identity, validates, feeds approved actions to the engine.
- **Two frontends** — the Bevy UI (a rebuildable projection + intent
  submission) and a terminal printer that renders the event stream. Both ride
  the same stream; neither touches combat state.

The codebase went in doc-first: `COMBAT.md` records the rulings, reviewed and
amended before code. That process is the reason this devlog exists — the
doc review rounds are where the design actually happened.

## The rules that survived

**User actions in, events out — and nothing else crosses.** The protocol
accepts only user actions (`PlayCard`, `EndTurn`). Everything else — the card
leaving hand, effects applying, discards, the enemy turn, turn-start draws —
is *machinery*: the engine's own job, never an input. This sounds obvious
stated flatly, but the pre-split code had the UI's End Turn button calling
`run_enemy_turn()` — a button handler driving game flow.

**Synchronous, total resolution.** One action goes in; the engine runs *all*
of its consequences before returning, emitting an ordered event stream.
Between actions, the engine rests only at input boundaries — states where
it's waiting for a player. There is no `advance()` door, no ticking, no
pacing inside the core. Pacing (animations, reveals, delays) is entirely a
frontend concern over its own event cursor.

**Validation is a gate, not a return code.** `validate() -> Result<()>` is the
host's read-only check; `process()` applies approved actions. Rejection isn't
an event — the canonical stream contains only things that *happened*, and a
rejection is a notice routed to the submitter alone.

**Three data tiers, each referencing only downward:** definitions (`CardDef`,
`EnemyDef` — static data, later RON) → persistent run state (`PlayerRunState`:
HP and deck, no hand/block/energy) → transient combat state (piles, energy,
end-turn flags). Combat *borrows* persistent state at start and will write
outcomes back at end — it never owns it. This is also when `CombatState`
stopped assuming 1v1: `players: Vec<CombatPlayer>`, `enemies: Vec<Unit>`,
concurrent turns that end when *all* players end them, host-sequenced
resolution.

## The mistakes — all mine, all structural

This round's honest theme: the AI kept reaching for textbook distributed-
systems patterns, and the human kept cutting them back to what this game
actually needs. Four times:

**1. The `advance()` pump.** My first fix for "UI drives game flow" gave the
core a parameterless `advance()` that callers would pump on a timer — callers
own pacing, core owns sequencing. The human killed it in one sentence: *"if
advance is going to be called eventually, then timing is pointless to the
backend."* The enemy beat I was scheduling is a property of *watching* the
game, not of the game. Resolution became synchronous; the pump and its whole
class of caller-side protocol knowledge evaporated.

**2. Connection-as-identity.** For multiplayer intent stamping, I proposed the
server derive player identity from the connection — framed as a "security
insight" against spoofing. The human: *"What 'malicious attacker' are you
expecting?"* It's a co-op game with friends; there is no adversary. Worse,
connection-as-identity is one-player-per-connection by construction — it
silently kills couch-internet co-op (two players on one couch, one machine,
two remote friends). Identity travels in the payload; the lobby owns
assignment. I then re-justified the same design with "bugs, not attackers"
reasoning, which the human also dismantled — we write both ends, so design
around real requirements, not hypothetical self-inflicted bugs.

**3. Plan/apply event sourcing.** Asked how host-approved actions should
reach engines, I proposed splitting the core into `plan` (host computes a
fat, self-contained event stream, RNG included) and `apply` (everyone applies
events mechanically). The human's counter-model is simpler and better:
**lockstep over host-approved actions**. The wire carries one approved action
per intent; every engine expands the consequences *locally* and identically,
from seeds broadcast once at start. My `Vec<Event>`-on-the-wire shape was
solving a problem — guests can't run machinery — that seeded determinism
removes. As a bonus, the thin wire needs no event redaction design, no
self-contained-event discipline, no second mutation path.

**4. Determinism over-corrections.** Listing lockstep invariants, I banned
floats and hashed containers wholesale. Both wrong, both corrected: IEEE-754
basic operations *are* deterministic (same inputs, same sequence, same
result — otherwise it's a spec violation); the real hazard is transcendental
functions, which aren't specified bit-exactly and differ across platform
libms. And hashed containers are fine for keyed access — the invariant is
that *iteration order must not affect outcomes* (worse: `HashMap`'s default
seed is per-process, so iteration order varies across runs of the same
binary).

## The pattern worth naming

Each mistake had the same shape: I imported a pattern that's correct *in its
home domain* (adversarial networks, event-sourced servers, deterministic
physics engines) and applied it to a game that doesn't live there. The human's
corrections weren't domain expertise I lacked — they were simpler readings of
the actual requirements. The working protocol for this collaboration is
sharpening into: the AI proposes, the human interrogates the *premises*, and
anything that can't justify itself against the concrete game gets cut.

Also worth naming: the single most productive design probe this round was a
yes/no question — *"can you drive a full game, programmatically, without UI,
without the testbench maintaining any game state?"* Answering it honestly
exposed that the caller had to know the transition graph (a structural flaw)
and that combat couldn't end (a feature gap). Good self-tests find seams;
great ones find missing walls.

## Where we stand

`cargo test --workspace`: 12 tests, including `two_engines_stay_in_lockstep` —
two engines, same seed, same approved-action stream, identical events and
state. That test is the CI tripwire for the entire multiplayer premise before
a single socket exists. The game plays identically to step 1; everything
changed under the hood.

## What's next

Step 2: real card effects. Strike/Defend-like potions, damage and block,
lethal damage and combat termination (the feature gap the self-test exposed),
and the first slice of the effect IR that spells will eventually compile to:
`Atom`, `Sequence`, `WithTarget`, and the interpreter that walks them.
