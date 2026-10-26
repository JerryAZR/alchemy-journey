//! State structures — what exists, at every tier, plus the identity
//! vocabulary (`PlayerId`, `EnemyId`, `UnitRef`, `CombatOutcome`) that both
//! state and wire messages are expressed in. This module holds data and
//! trivial accessors only; all mutation of `CombatState` lives in
//! `engine.rs`.

use rand::rngs::StdRng;

use crate::effect::{Instruction, TargetSpec};

// ---------------------------------------------------------------------------
// Identity vocabulary (shared by state and wire)
// ---------------------------------------------------------------------------

/// Opaque player identity. Deliberately *not* a collection index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlayerId(pub u32);

/// Enemy identity. Assigned in `enemies` vec order at combat start and
/// index-stable: corpses are never removed, so the index never shifts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EnemyId(pub u32);

/// A unit by identity, not index (COMBAT.md Ruling 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitRef {
    Player(PlayerId),
    Enemy(EnemyId),
}

/// How a combat ended. Only `Victory` exists in step 2: the loss path is
/// deferred until enemy damage exists and multiple combats make it
/// e2e-testable (DESIGN.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombatOutcome {
    Victory,
}

// ---------------------------------------------------------------------------
// Tier 1: definitions
// ---------------------------------------------------------------------------

/// What happens to a card after it is played.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnPlay {
    /// Returns to the discard pile, can be drawn again.
    Reusable,
    /// Permanently removed from the deck when played (potions).
    /// Note: within a single combat the card sits in `consumed`; the
    /// run-level removal lands with the writeback (deferred; DESIGN.md).
    Consume,
}

/// Static card definition.
#[derive(Clone, Debug, PartialEq)]
pub struct CardDef {
    pub name: String,
    pub cost: u32,
    pub on_play: OnPlay,
    /// The flat effect program (DESIGN.md).
    pub effects: Vec<Instruction>,
    /// Within one play, the scan may return to the cast's starting cell at
    /// most this many times (shared across any future triggered sub-casts).
    /// Default 0: every cell is still scanned once per play — the catch-up
    /// lap back to the start is not a wrap — but nothing is re-scanned.
    pub unwrap: u8,
}

/// Static enemy definition: how an enemy is spawned. Its action graph
/// (how it behaves) attaches here in step 3 — never to the live `Unit`.
#[derive(Clone, Debug, PartialEq)]
pub struct EnemyDef {
    pub name: String,
    pub hp: u32,
}

// ---------------------------------------------------------------------------
// Tier 2: persistent run state
// ---------------------------------------------------------------------------

/// A player's run-persistent state: survives combats, carries no combat
/// mechanics (no hand, no discard, no block, no energy).
pub struct PlayerRunState {
    pub id: PlayerId,
    pub name: String,
    pub hp: u32,
    pub max_hp: u32,
    /// The run deck. Card ids are assigned at run level and are stable
    /// across combats — potion removal lands here.
    pub deck: Vec<Card>,
}

// ---------------------------------------------------------------------------
// Shared live state
// ---------------------------------------------------------------------------

/// A card instance. `id` is run-persistent (assigned when the card enters
/// the run deck), so consumed potions can be removed from the run deck.
/// `cursor` and `bind` are the paused-evaluation state (DESIGN.md): a cast
/// is a scan paused between plays, persisting until combat end.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub id: u32,
    pub def: CardDef,
    /// Where the next cast's scan starts.
    pub cursor: usize,
    /// The active target bind (defaults to `Chosen`: unbound).
    pub bind: TargetSpec,
}

impl Card {
    pub fn new(id: u32, def: CardDef) -> Self {
        Self {
            id,
            def,
            cursor: 0,
            bind: TargetSpec::Chosen,
        }
    }
}

/// Live unit state — a player avatar or an enemy. Pure combat-local numbers;
/// behavior and spawn data live in definitions.
#[derive(Clone, Debug, PartialEq)]
pub struct Unit {
    pub name: String,
    pub hp: u32,
    pub max_hp: u32,
    pub block: u32,
}

impl Unit {
    pub fn new(name: &str, hp: u32) -> Self {
        Self {
            name: name.to_string(),
            hp,
            max_hp: hp,
            block: 0,
        }
    }

    pub fn is_alive(&self) -> bool {
        self.hp > 0
    }
}

// ---------------------------------------------------------------------------
// Tier 3: transient combat state
// ---------------------------------------------------------------------------

/// One human player within a combat: their avatar plus everything
/// combat-local — energy, piles, and turn participation.
pub struct CombatPlayer {
    pub id: PlayerId,
    pub unit: Unit,
    pub energy: u32,
    /// Draw pile; the top of the deck is the *last* element.
    pub deck: Vec<Card>,
    pub hand: Vec<Card>,
    pub discard: Vec<Card>,
    pub consumed: Vec<Card>,
    /// Concurrent turns: set when this player ends their turn. The round
    /// advances when all players have ended. (Open question in DESIGN.md:
    /// whether dead players are excluded — "all living players ended" is the
    /// current lean; logic lands with damage, when death exists.)
    pub has_ended_turn: bool,
}

impl CombatPlayer {
    /// Total cards across this player's piles. A *conservation check*,
    /// not a game rule: true today because no content creates or destroys
    /// cards mid-combat (Consume is a pile, not deletion). The first
    /// card-generating effect (StS's Shivs precedent) breaks it — tests
    /// using it must then account for generated cards explicitly.
    #[cfg(test)]
    pub fn total_cards(&self) -> usize {
        self.deck.len() + self.hand.len() + self.discard.len() + self.consumed.len()
    }
}

pub struct CombatState {
    pub turn: u32,
    /// Every human player, facing the enemy group together.
    pub players: Vec<CombatPlayer>,
    /// The enemy group. One today; commonly several. Corpses are never
    /// removed, keeping `EnemyId` index-stable.
    pub enemies: Vec<Unit>,
    /// Set the moment the combat is decided; a real resting state
    /// (DESIGN.md: no transient terminal states). `validate` rejects all
    /// actions once this is `Some`.
    pub outcome: Option<CombatOutcome>,
    pub(crate) rng: StdRng,
}
