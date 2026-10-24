# DESIGN.md — Game Design Decisions (settled state)

A Slay-the-Spire-like deckbuilder with Noita-style spell assembly, in Bevy
(latest). This document records **settled design decisions** so they don't
need re-deriving. Open questions are listed explicitly at the bottom — do not
settle them by accident.

Combat architecture has its own doc: see **COMBAT.md** (read it first; it
governs the core/app boundary, validate/process, lockstep multiplayer model).
Process rules (design-first, no unapproved implementation) are in the global
AGENTS.md. History and rationale live in `devlog/`.

## The game

- Players collect **spell components** and assemble them into **spells**
  (Noita-style). Assembled spells become reusable cards with a computed cost.
- Players collect **ingredients** and brew **potions**. Potions are
  non-reusable, zero-cost cards; playing one removes it from the deck
  permanently (run-level removal, not combat-level).
- Standard StS combat: turn-based, energy per turn, draw to hand size,
  visible enemy intents, block clears. Don't deviate without discussion.
- Run-based meta-structure. **Map topology is decoupled from node content**:
  the map is a graph of typed nodes; what a node does is resolved by separate
  systems. StS-style DAG and free-roam-with-fog-of-war must both remain
  possible. (Not started; step 4+ material.)

## The card model

- The combat engine knows only **cards**: `{ cost, effects, on_play }` where
  `on_play ∈ { Reusable, Consume }`. "Potion" vs "spell" is **provenance**
  (brewed vs. assembled), not an engine distinction.
- Potions sit **in the deck**, same as spells — you reach into the bag and
  draw scrolls and vials alike. Played potions go to the `consumed` pile and
  are removed from the run deck at combat end.
- A potion's **reusable toggle** exists as a testing aid (step 2): a potion
  marked reusable behaves like a spell mechanically. Do not read design
  meaning into it.
- Card ids are **run-persistent** (assigned when a card enters the run deck),
  so consumed potions can be removed at run level.

## Potions (step 2 builds the first of these)

- Zero cost, flat effect list, effects apply in a **predefined order**, single
  **target scope per potion**: `{ one unit | one side | everyone }`.
- Per-potion scope means "give myself 10 shield, then deal 10 damage to an
  enemy" is **not** a valid potion — effects within one potion share one
  scope. Repeated atoms (`[Damage(N), Damage(N)]`) are structurally fine;
  whether recipes may produce them is a brewing question, deferred.
- Potion authoring invariants (flat, single scope, no modifiers, no triggers)
  are enforced at the **authoring layer**, not the executor. (Rust enforces
  memory safety in the language, not by running on a different CPU than C.)

## The effect IR (the decision that took the longest — do not re-litigate)

- **One IR, one tree-walking interpreter.** Spells and potions share the
  atomic effect vocabulary and the executor.
- **Two compilers, one executor:** potions lower mechanically from their
  authoring type `{ scope, effects }` into the IR; spells compile from
  sequential Noita-style evaluation into the same IR.
- **The interpreter walks the tree against live combat state.** No batching,
  no snapshot semantics: conditionals ("deal 3N; if target alive, deal N to
  yourself") and liveness checks are evaluated mid-walk. "Deal N twice" is
  two `Damage` atoms and is *not* "deal 2N."
- The IR is an **open type that grows with features**. Step 2 adds only what
  Strike/Defend need: atoms (Damage, Block), `Sequence`, and target binding
  (`WithTarget`). Modifiers/multicast/triggers/conditionals are added when
  spell assembly is designed, not before.
- Spell effects will express a superset of potion effects: multiple effects,
  multiple targets, mixed scopes.

## Spell assembly (future — step 3+ of design, not scheduled)

- Assembly happens **anytime outside combat**. In-combat assembly is a
  possible future enhancement, possibly restricted.
- Effects vocabulary borrows from StS (damage, block, heal, buffs, debuffs,
  turn-level effects like retain); modifiers borrow from Noita. The concrete
  component catalog is a research task the owner will do — the engine must
  not hardcode a catalog.
- Spell evaluation is **sequential** (Noita-style): ordered component list,
  modifiers wrap following effects, produces an effect tree for the
  interpreter.

## Methodology

- **Loop-first**: playable e2e at every step; logic lands in the pure core
  crate with unit tests over the event stream; UI is a projection.
- **After step 4 (buffs/debuffs) there is an explicit review checkpoint**:
  evaluate whether the combat engine needs a refactor/redesign *before*
  designing spell assembly against it.
- Green gate: `scripts/test.sh` (fmt + clippy -D warnings + test). CI runs it.

## Roadmap (completed steps struck through)

- ~~Step 1 — turn loop skeleton: do-nothing potions, inert enemy, draw /
  discard / reshuffle / consume, energy, quit-to-menu exit.~~
- ~~The split — core crate, intents/events, validate/process, three data
  tiers, concurrent-turn shape, lockstep vocabulary.~~
- **Step 2 (next) — simple effects**: Strike/Defend-like cards (potions with
  the reusable toggle), damage, block, lethal damage, `CombatOver` terminal
  state, win/loss exit. First IR slice: atoms + `Sequence` + `WithTarget` +
  interpreter. Enemy still does nothing. The BSN adoption decision (from the
  Bevy 0.20 feature evaluation): adopt opportunistically, starting with the
  hand row as the spike at the next UI-heavy step; revert freely if it
  fights you.
- Step 3 — enemy intent: data-driven per-enemy action patterns attached to
  `EnemyDef` (scripted sequences; not hand-written state machines).
- Step 4 — buffs/debuffs, with and without duration; turn hooks in the
  phased round cascade (`player_turn_end` / `enemy_turn` / `begin_player_turn`
  already have marked hook sites).
- Step 5 — **review checkpoint**, then spell assembly design.
- Later: potions/brewing, run structure + map, multiplayer.

## Working agreements (from the devlogs — recorded here so they don't
require reading history)

- **Prefer mature existing solutions over hand-rolling**, unless you can
  articulate why hand-rolling is necessary. "Fewer dependencies" alone is
  not an articulation. Legitimate justifications: it's our core domain (the
  combat engine is hand-rolled because it *is* the product), a hard
  constraint no library meets, or control the library can't give.
- **Don't import patterns from domains this game doesn't live in.** Co-op
  with friends has no adversary; a turn-based card game is not an
  event-sourced server. Patterns must justify themselves against *this*
  game's concrete requirements. (Four corrections in devlog 002 all had
  this shape.)
- **A guard that can never observe its guarded state is load-bearing in
  tests only** (the degenerate state machine lesson, devlog 001): states
  that exist only for microseconds inside a call stack are not states.
  Step 2's `CombatOver` must be a real resting state.
- **For prototype playtesting, observability beats presentation**: event
  ordering and pile movements must be answerable from the terminal log.

## Step-2 decisions pending the owner (found by a fresh-eyes doc review)

The architecture docs fully constrain *how* step 2 plugs in; these *content*
decisions are not yet made. The next agent must get owner rulings before
implementing:

1. **Targeting model & `Intent` shape.** Does `PlayCard` grow a target
   field? Or does targeting use the `ChooseOption`-style decision point
   (core rests awaiting a choice)? Or auto-target the only enemy while
   there's exactly one? What is a target in the IR (enemy index? unit
   identity? side?) — mind that removing dead enemies from the vec
   invalidates indices mid-sequence.
2. **`CombatOver` representation and post-terminal behavior.** Resting
   state (per the degenerate-state-machine lesson), event shape, and
   whether `validate` rejects everything afterwards (new `RejectReason`).
   App-side win/loss display and exit path.
3. **Run-state writeback API** at combat end (HP back to `PlayerRunState`;
   remove `consumed` card ids from the run deck — confirm that removal is
   in scope for step 2), and what HP-on-loss means with no run layer yet.
4. **Is the loss path in scope when unreachable in play?** (Inert enemy,
   no self-damage card.) Implement-and-test via unit construction anyway,
   or defer?
5. **Starter content numbers**: how many Strike/Defend-like cards, costs,
   values, enemy HP; and whether shipped starter cards are Consume or
   Reusable.
6. **Block-clear timing** (StS: at the start of the unit's own turn) —
   confirm hook placement in `begin_player_turn`; does enemy block clearing
   land now or with step 3?
7. **New event shapes** (`DamageDealt` / `BlockGained` / `UnitDied` /
   `CombatOver`): amounts pre/post block, actor fields — propose, get
   ruling.

## Open questions (explicitly NOT settled)

- Whether raw spell components are themselves playable as cards. (Current
  lean: no — components are inventory; deck holds only assembled spells and
  potions. Not binding.)
- Whether assembled spells can be used as components (nesting). (Lean: not
  in the core; possible later as an opt-in component type. Not binding.)
- When/how brewing happens (stations? anywhere? ingredient sources), and
  potion recipe structure.
- In-combat assembly: shape and restrictions.
- Map shape per run (DAG vs free-roam) — the architecture supports both;
  the game choice is open.
- Dead-player semantics details in co-op (current encoding: "all living
  players ended"; flagged in code comments, logic lands with damage).
