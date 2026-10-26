//! Wire vocabulary (COMBAT.md Ruling 2): intents, rejections, events —
//! the *messages* that cross engine boundaries under lockstep. Plain data,
//! indices not pointers. The identities these messages carry (`PlayerId`,
//! `UnitRef`, `CombatOutcome`, ...) are state vocabulary and live in
//! `state.rs`; this module depends on it, never the reverse.

use crate::state::{Card, CombatOutcome, PlayerId, UnitRef};

/// As submitted by a player. Carries no identity. `target` is the single
/// optional "chosen" target (DESIGN.md, two-rule target resolution).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    PlayCard {
        hand_index: usize,
        target: Option<UnitRef>,
    },
    EndTurn,
}

/// As consumed by the core: identity stamped at submission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StampedIntent {
    pub actor: PlayerId,
    pub intent: Intent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectReason {
    UnknownPlayer,
    AlreadyEndedTurn,
    InvalidHandIndex,
    NotEnoughEnergy,
    /// The combat has ended; no further actions are legal.
    CombatOver,
    /// The upcoming cast consumes the chosen target, but none was given.
    TargetRequired,
    /// A target was supplied, but the upcoming cast never consumes it.
    TargetNotUsed,
    /// The chosen target does not exist or is not alive.
    InvalidTarget,
}

/// Everything that happened as a consequence of processing an approved
/// action, in order. Action echoes carry the actor; per-player machinery
/// carries the player it happened to; shared machinery carries no identity.
///
/// This stream contains only things that *happened* — rejections are
/// host-side notices, never events (COMBAT.md Ruling 3).
///
/// Note: this stream is currently omniscient (e.g. `CardDrawn` reveals the
/// card). Under lockstep that is inherent — every engine simulates
/// everything (Ruling 5, accepted tradeoff).
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    CombatBegan,
    CardPlayed {
        actor: PlayerId,
        card: Card,
    },
    TurnEnded {
        actor: PlayerId,
    },
    HandDiscarded {
        player: PlayerId,
        count: usize,
    },
    EnemyTurnPassed {
        enemy: String,
    },
    TurnBegan {
        turn: u32,
    },
    CardDrawn {
        player: PlayerId,
        card: Card,
    },
    DeckReshuffled {
        player: PlayerId,
        count: usize,
    },
    // Effect reports: what the evaluator already applied. Outputs, never
    // inputs — frontends render them, nothing feeds them back in.
    DamageDealt {
        target: UnitRef,
        /// HP actually lost, after block absorption.
        amount: u32,
        /// How much of the hit block absorbed.
        blocked: u32,
    },
    BlockGained {
        unit: UnitRef,
        amount: u32,
    },
    UnitDied {
        unit: UnitRef,
    },
    CombatOver {
        outcome: CombatOutcome,
    },
}
