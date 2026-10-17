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

## Ruling 2 — The vocabulary is data, and it is provisional

The lists below are the *current* vocabulary, not a frozen contract. If an
event or intent turns out to be shaped wrong, change it; if a new one is
needed, add it. What is stable is the *kind* of thing they are — plain data,
indices not pointers — not any particular variant list.

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

The engine's input door is a pair:

```rust
/// The host gate: read-only legality check.
fn validate(&self, action: StampedIntent) -> Result<(), RejectReason>

/// What every engine runs on an approved action: applies it and emits the
/// locally-derived consequence stream (action echo + machinery events).
fn process(&mut self, action: StampedIntent) -> Vec<Event>
```

The protocol accepts **user actions only**. Consequences are never input:

| user action (input) | machinery (the core's own job) |
|---|---|
| `PlayCard { hand_index }` | card leaving hand, effects applied, discard/consume |
| `EndTurn` | turn ending, hand discard, enemy turn, next turn start, turn-start draw |

Resolution is **synchronous and total**: `process` runs all machinery
consequences of an action — draws, discards, the enemy turn, the next turn
start — before returning, and the event stream reports everything in order.
If the machinery would run eventually anyway, its timing is meaningless to
the core, so the core runs it immediately.

Between actions the core rests **only at input boundaries**: states
where it awaits a user action. Today that is one state (the local player's
free action). Future decision points ("choose one of three cards") are also
input boundaries: the decision is itself a user action (e.g.
`ChooseOption { .. }`), never a machinery callback. There is no `advance()`
door and no pacing in the core: it consumes actions as soon as they land.

Rejection is not part of the canonical stream: `validate` returns
`Err(RejectReason)` and the host notifies only the rejected submitter.
The event stream contains only things that actually happened.

## Ruling 4 — The app layer is policy; the core is mechanism

The core is a black box: callers submit actions, receive events, and may pull
state at any time. How a frontend *presents* what happened — instantly, with
delays, with animations — is entirely frontend policy over its own event
cursor; the core never knows. Input gating during presentation (e.g. ignoring
clicks while the enemy-turn animation plays) is likewise frontend policy:
the core may already be resting at the next input boundary, and that is fine.
A future multiplayer server is a different driver over the same core.

App-layer structure, as composed Bevy plugins:

- **Driver plugin** — owns the intent queue (one drain point: UI submits bare
  `Intent`s, the drain stamps the local `PlayerId`, `validate`s, and
  `process`es), owns the pending-event buffer and the rejection-notice buffer
  (one drain point each, fanning out to frontends).
- **UI plugin** — a frontend: renders a projection of `CombatState` and the
  event stream, and submits intents. It never mutates combat state directly.
  The projection is rebuildable from state at any time (today: rebuilt on
  every change).

Rendering events to human-readable text (the terminal combat log) is one
function in the app layer — the single home for that mapping. The terminal
printer is a second frontend riding the same event stream as the UI.

## Ruling 5 — Multiplayer is a constraint, not a feature (yet)

No networking is built now. The target model is **lockstep over host-approved
actions**, and the vocabulary is shaped so it doesn't require redesign:

- **Two channels, one topology.** An intent channel (players → host) and an
  action channel (host → everyone, including the host's own loopback). Each
  is a network port or an in-memory queue — single player is the in-memory
  case, and local play rides the same path as network play.
- **The host validates; everyone processes.** The host listens on the intent
  channel, `validate`s, and broadcasts approved actions. Engines never act on
  raw intents — only on approved actions from the action channel. Every
  engine expands the machinery cascade *locally* via `process`, so the wire
  stays thin (one approved action per intent; derived events never cross it).
- **Rejection is a notice**, host → rejectee only.

Determinism invariants (the lockstep premise; broadcast seeds once at start):

1. **Pin the RNG algorithm** when multiplayer ships (`ChaCha8Rng` or similar,
   stable across `rand` versions and platforms).
2. **Floating point is fine** — IEEE-754 basic operations are deterministic
   (same inputs, same sequence, same result). What rules math must avoid is
   transcendental functions (`sin`, `pow`, `exp`, ...), which are not
   specified bit-exactly and differ across platform libms.
3. **Iteration order must not affect outcomes.** Keyed access on hashed
   containers is always fine. `HashMap`'s default `RandomState` is seeded
   per process, so iteration order varies even across local runs —
   order-sensitive iteration sorts or uses `BTreeMap`.
4. **Approved actions are the only external input** — no wall-clock, no
   external reads in the core (Ruling 1).

Accepted tradeoff: **no hidden information.** Every engine simulates
everything, so hands and deck orders are knowable by every client — fine for
co-op with friends (and unenforceable with modded clients anyway).

Desync tripwire: any engine may re-`validate` an approved action; a failure
means bug or desync — fail loud.

- **Turns are concurrent; there is no turn order among players.** During a
  shared turn, every player may play cards at any time. A turn ends when
  *all* players have chosen to end it; ending is per-player and locks that
  player out of further plays until the next turn. The core tracks who has
  ended; the round machinery (enemy turn, draws) resolves inside the action
  that completes the set.
- **Concurrent resolution order is decided by the host.** Simultaneous card
  plays are sequenced by the host's intent channel (arrival order); the
  host's approved-action stream is the single truth every engine processes.
  Resolution order is therefore deterministic and replayable.
- **A connection controls a set of players** (common case: one). Identity
  travels in the intent payload as `actor`, because one connection may
  legitimately speak for several local players (couch co-op over the internet).
- Player-slot assignment belongs to a lobby/join layer that doesn't exist yet.
  When it does, a server validating `actor ∈ connection's players` is a
  fail-loud bug tripwire, not a security boundary — we write both ends.
- The game remains single-player; only the vocabulary admits that players
  exist. An action from a player who has ended their turn fails `validate`
  (`AlreadyEndedTurn`), which single-player never triggers mid-turn.

## Explicitly deferred

Animations, replays (enabled by determinism + events, unbuilt), serde derives,
networking, multiple player units, the lobby/join layer.

Combat termination (win/loss, a terminal `CombatOver` phase) is a known
*feature* gap — deliberately so: it arrives with damage in step 2. It is not a
structural gap; the machinery above already supports a terminal resting state.
