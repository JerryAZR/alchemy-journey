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

- Zero cost, flat effect list, effects apply in a **predefined order**,
  single **target scope per potion** — one `TargetSpec` for the whole
  potion: `Chosen` (the player picks at play time), `Caster` (the
  drinker), or the `AllX`/`Everyone` roles once content needs them.
- Per-potion scope means "give myself 10 shield, then deal 10 damage to an
  enemy" is **not** a valid potion — effects within one potion share one
  scope. Repeated atoms (`[Damage(N), Damage(N)]`) are structurally fine;
  whether recipes may produce them is a brewing question, deferred.
- Potion authoring invariants (flat, single scope, no modifiers, no triggers)
  are enforced at the **authoring layer**, not the executor. (Rust enforces
  memory safety in the language, not by running on a different CPU than C.)
- Lowering is mechanical (`k` = the potion's atom count): `[Eval(k)]`,
  plus `BindTarget(target)` unless the target is `Chosen` (the register's
  default — no instruction needed), plus the atoms.

## The effect IR and evaluator (settled — do not re-litigate)

- **One IR, one executor.** Spells and potions share the atomic effect
  vocabulary and the evaluator.
- **Flat instruction vector + stateful sequential evaluator.** The editing
  surface is a vector, so the IR mirrors the authoring model — no tree, no
  compiler that reshapes one. The evaluator scans the vector against **live
  combat state**: no batching, no snapshot semantics; liveness is checked
  per atom application, mid-scan. "Deal N twice" is two `Damage` atoms and
  is *not* "deal 2N."

### Instructions (open types — they grow with features)

```rust
enum Atom { Damage(u32), Block(u32) }   // step 2 slice; more land with content

enum TargetSpec { Chosen, Caster, AllEnemies, AllAllies, Everyone }
// Chosen: the player's play-time pick, and the register's default —
// BindTarget(Chosen) is also the way back from any other bind.

enum Instruction {
    Eval(u32),                // this cast applies up to N atoms (default: 1)
    BindTarget(TargetSpec),   // set the bind register until replaced
    Apply(Atom),              // apply one atom to the resolved target
}
```

### Evaluation model: paused stepping (Noita-style, per-cast)

- Playing a card runs **one cast**: the scan starts at the card's cursor and
  applies atoms until N are applied (default 1; `Eval(N)` raises it for the
  current cast), the vector ends, or the wrap budget is exhausted.
- A cast is a **paused evaluation**: all eval state — cursor, pending
  modifiers, active bind — lives on the card instance and persists until
  combat end. After the last node the cursor wraps to 0 for the next cast.
  (A stepping 6-7-8 damage card is three `Apply` nodes and nothing else.)
- `unwrap: u8` on the card def, default 0: within one play — including any
  future triggered sub-casts, which share the budget — the cursor may pass
  slot 0 at most `unwrap` times. This kills free-cast infinite loops (a
  lone "deal 10, then cast the next for free" node fizzles at budget end).
- Run-persistence of eval state: out of scope (not hard, just undesigned).

### Targeting: one-rule resolution

Every atom resolves against the card's **bind register**, which defaults
to `Chosen` — the single optional target in
`Intent::PlayCard { hand_index, target: Option<UnitRef> }`. `BindTarget`
sets the register; there is no separate reset instruction and no per-node
override. (Review note: `Chosen` was briefly special-cased as "absence of
bind" with a `ClearBind` instruction — a leftover of the tree-IR framing.
Unifying it into `TargetSpec` deleted an instruction, an `Option` wrapper,
and a resolution rule, and made `BindTarget(Chosen)` expressible.)

- **Targeting is bind-or-chosen for now.** Retargeting costs slots
  (`BindTarget`), keeping targeting expansion inside the slot economy:
  "pay 2 slots to give this spell a self-shield rider" is a real tradeoff,
  like everything else in wand building. Computed spell cost charges for
  it like any component.
- **Per-node built-in targets are parked, not rejected.** A component with
  a baked-in target trades *flexibility for slot efficiency* (e.g. "apply
  a debuff to the caster, gain 1 energy": self-targeting is part of its
  identity, no bind slot spent). That's a legitimate component-design
  lever, kept as a candidate for when we want more strategic depth.
  Adding it later is backward compatible: resolution gains a rule-0
  built-in check above the two rules.
- Binds resolve roles to concrete units **at application time** (mid-scan
  liveness applies); `AllX` expands to each living unit of the side in
  stable vector order.
- "Does the upcoming cast need a target?" is a pure query: simulate the
  scan with the card's current registers; needs-chosen iff some atom would
  apply while no bind is active. The UI builds the StS drag model on it
  (target-cards drag onto a unit; others into the play region).
- Validation is strict: a missing-when-needed target rejects, and a
  supplied-but-unconsumed target rejects (silent drops hide UI bugs).

## Spell assembly (future — step 3+ of design, not scheduled)

- Assembly happens **anytime outside combat**. In-combat assembly is a
  possible future enhancement, possibly restricted.
- Effects vocabulary borrows from StS (damage, block, heal, buffs, debuffs,
  turn-level effects like retain); modifiers borrow from Noita. The concrete
  component catalog is a research task the owner will do — the engine must
  not hardcode a catalog.
- **Slot pressure is the core tension of assembly.** Every expansion of
  what a spell does — more effects, retargeting, multicast — competes for
  slots. Scarcity creates the decisions. (Mechanisms may *price* expansion
  differently — e.g. built-in targets sell flexibility for slot
  efficiency — but expansion is never free.)
- Spell evaluation is the sequential scan described in the IR section:
  ordered component list, modifiers ride into the evaluator's registers,
  one cast per play.

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
  state, win/loss exit. First IR slice: the four instructions + the stepping
  evaluator + per-card eval state, per the IR section. Enemy still does
  nothing. The BSN adoption decision (from the Bevy 0.20 feature
  evaluation): adopt opportunistically, starting with the hand row as the
  spike at the next UI-heavy step; revert freely if it fights you.
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

## Step-2 content decisions (ruled by the owner)

- **`CombatOver`, minimal**: `CombatState.outcome: Option<CombatOutcome>` set
  the moment the last enemy dies; `CombatOver { outcome }` emitted once;
  `validate` then rejects everything (`RejectReason::CombatOver`). App shows
  the outcome and returns to menu. Proper post-combat flow is designed with
  rooms & map traversal (possibly extended again with rewards).
- **Deferred with post-combat flow**: the loss path (`CombatOutcome` ships
  with `Victory` only; `Defeat` lands when enemy damage does, step 3+) and
  the run-state writeback API — neither is e2e-testable until multiple
  combats exist.
- **Starter deck**: 4× stepping strike (6-7-8 damage, 1⚡, reusable toggle,
  chosen target — hand-written instruction vectors), 4× stepping defend
  (5-6-7 block, 1⚡, reusable toggle, `[BindTarget(Caster), Apply×3]`),
  1× 20-damage potion (0⚡, Consume, chosen target), 1× 15-block potion
  (0⚡, Consume, caster) — the two true potions authored via `PotionDef`
  lowering, the stepping cards hand-written (potion lowering's `Eval(k)`
  would collapse the stepping). Enemy: Training Dummy, 30 HP.
- **Block clears at the start of the unit's own turn** (StS rule): players
  in `begin_player_turn`; the enemy-turn-start hook is placed now, inert
  until step 3.
- **Event shapes**: `DamageDealt { target, amount, blocked }`,
  `BlockGained { unit, amount }`, `UnitDied { unit }`,
  `CombatOver { outcome }`. No `source` field until statuses need
  attribution. Events are **reports** — outputs the engine already applied;
  frontends render them, nothing feeds them back in.

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
