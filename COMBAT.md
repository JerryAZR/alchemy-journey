# COMBAT.md — Combat Core Design Rulings

The combat system is split across a hard boundary: a pure-Rust core crate that
owns all combat truth, and app-layer Bevy plugins that drive it and render it.
This document records the rulings for that boundary. Amend before coding.

## Ruling 1 — The core is a crate, and it is pure

`crates/combat` (`alchemy-combat`) owns combat state and rules. It depends on
`rand` and nothing Bevy. The crate boundary exists so that the dependency graph
itself forbids UI, wall-clock time, and networking from creeping into combat
logic — modules would only enforce this by convention.

Everything observable about a combat is derivable from its inputs: the initial
state, the RNG seed, and the intent sequence. The core is deterministic given
those.

## Ruling 2 — The vocabulary is data

```rust
struct PlayerId(u32);

/// As submitted by a player. Carries no identity.
enum Intent { PlayCard { hand_index: usize }, EndTurn }

/// As consumed by the core: identity stamped at submission.
struct StampedIntent { actor: PlayerId, intent: Intent }

enum Event {
    // Action echoes carry the actor:
    CardPlayed      { actor: PlayerId, /* card */ },
    TurnEnded       { actor: PlayerId },
    IntentRejected  { actor: PlayerId, reason: RejectReason },
    // Autonomous events carry no actor — no one "did" them:
    TurnBegan       { turn: u32 },
    CardsDrawn      { /* ... */ },
    DeckReshuffled  { count: usize },
    PhaseChanged    { phase: Phase },
}
```

Intents and events are plain data: indices, not pointers or entity IDs. They
will derive `Serialize`/`Deserialize` when the wire exists, and not before.

## Ruling 3 — One input door, and it accepts only user actions

`step(StampedIntent) -> Vec<Event>` is the only input door, and the protocol
accepts **user actions only**. Consequences are never input:

| user action (input) | machinery (the core's own job) |
|---|---|
| `PlayCard { hand_index }` | card leaving hand, effects applied, discard/consume |
| `EndTurn` | turn ending, hand discard, enemy turn, next turn start, turn-start draw |

An action's resolution happens inside `step` and is reported as events.
Machinery that no player initiates (the enemy turn, turn-start draws) advances
via `advance() -> Vec<Event>` — **parameterless, a pure function of state**,
one beat of machinery per call, safe no-op (empty stream) when nothing is
pending. The caller cannot call it wrong. Whether machinery is pending is
queryable state (`needs_advance()`, derivable from `phase`).

The transition graph lives wholly in the core. Callers own pacing — *when* to
pump `advance()` — never sequencing, i.e. never *what follows what*. A driver
that pumps whenever `needs_advance()` holds, with no other knowledge, is
correct by construction.

There is no rejection return value. An illegal intent produces an
`IntentRejected` event; every call yields an event stream, and outcomes are
read from it. Side effects travel only as events.

## Ruling 4 — The app layer is policy; the core is mechanism

The core never decides *when* `advance()` runs. Pacing (e.g. "the enemy turn
begins 0.5s after the player ends theirs") is policy and lives in the
app-layer driver. A future multiplayer server is a different driver over the
same core.

App-layer structure, as composed Bevy plugins:

- **Driver plugin** — owns the intent queue (one drain point: UI submits bare
  `Intent`s, the drain stamps the local `PlayerId` and calls `step`), owns the
  pending-event buffer (one drain point, fanning out to frontends), owns
  pacing timers.
- **UI plugin** — a frontend: renders a projection of `CombatState` and submits
  intents. It never mutates combat state directly and never calls
  `advance_autonomous`. The projection is rebuildable from state at any time
  (today: rebuilt on every change).

Rendering events to human-readable text (the terminal combat log) is one
function in the app layer — the single home for that mapping. The terminal
printer is a second frontend riding the same event stream as the UI.

## Ruling 5 — Multiplayer is a constraint, not a feature (yet)

No networking is built now. But the vocabulary is shaped so networking doesn't
require redesign:

- **A connection controls a set of players** (common case: one). Identity
  travels in the intent payload as `actor`, because one connection may
  legitimately speak for several local players (couch co-op over the internet).
- Player-slot assignment belongs to a lobby/join layer that doesn't exist yet.
  When it does, a server validating `actor ∈ connection's players` is a
  fail-loud bug tripwire, not a security boundary — we write both ends.
- `CombatState` carries `active_player: PlayerId`; `step` rejects intents from
  anyone else with `IntentRejected { reason: NotYourTurn }`. The game remains
  single-player; only the vocabulary admits that players exist.

## Explicitly deferred

Animations, replays (enabled by determinism + events, unbuilt), serde derives,
networking, multiple player units, turn order among players.

Combat termination (win/loss, a terminal `CombatOver` phase) is a known
*feature* gap — deliberately so: it arrives with damage in step 2. It is not a
structural gap; the machinery above already supports a terminal resting state.
