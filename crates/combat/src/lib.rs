//! Combat domain core. See COMBAT.md at the repository root for the rulings
//! this crate enforces. In short: this crate owns all combat truth, depends
//! on nothing Bevy, accepts only user actions through `step`, and reports
//! every consequence as events. Resolution is synchronous and total; between
//! `step` calls the state rests only at input boundaries.
//!
//! Data tiers (see the design discussion):
//! - Tier 1, definitions: [`CardDef`], [`EnemyDef`] — static data.
//! - Tier 2, persistent run state: [`PlayerRunState`] — HP and deck, no
//!   combat mechanics. Combat borrows it at start and writes outcomes back
//!   at end (writeback arrives with combat termination in step 2).
//! - Tier 3, transient combat state: [`CombatPlayer`], [`CombatState`].

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;

pub const HAND_SIZE: usize = 5;
pub const ENERGY_PER_TURN: u32 = 3;

/// The local player in a single-player combat is always id 0.
pub const LOCAL_PLAYER: PlayerId = PlayerId(0);

/// Opaque player identity. Deliberately *not* a collection index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlayerId(pub u32);

// ---------------------------------------------------------------------------
// Wire vocabulary (provisional — COMBAT.md Ruling 2)
// ---------------------------------------------------------------------------

/// As submitted by a player. Carries no identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    PlayCard { hand_index: usize },
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
}

/// Everything that happened as a consequence of a `step` call, in order.
/// Action echoes carry the actor; per-player machinery carries the player
/// it happened to; shared machinery carries no identity.
///
/// Note: this stream is currently omniscient (e.g. `CardDrawn` reveals the
/// card). Per-player event visibility for multiplayer clients is deferred.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    CombatBegan,
    CardPlayed { actor: PlayerId, card: Card },
    IntentRejected { actor: PlayerId, intent: Intent, reason: RejectReason },
    TurnEnded { actor: PlayerId },
    HandDiscarded { player: PlayerId, count: usize },
    EnemyTurnPassed { enemy: String },
    TurnBegan { turn: u32 },
    CardDrawn { player: PlayerId, card: Card },
    DeckReshuffled { player: PlayerId, count: usize },
}

// ---------------------------------------------------------------------------
// Tier 1: definitions
// ---------------------------------------------------------------------------

/// What happens to a card after it is played.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnPlay {
    /// Returns to the discard pile, can be drawn again.
    /// Unused until step 2, when the reusable toggle lands.
    #[allow(dead_code)]
    Reusable,
    /// Permanently removed from the deck when played (potions).
    /// Note: within a single combat the card sits in `consumed`; the
    /// run-level removal happens when run structure lands (step 4+).
    Consume,
}

/// Static card definition. Step 1 has no effects yet; the effect IR
/// lands in step 2.
#[derive(Clone, Debug, PartialEq)]
pub struct CardDef {
    pub name: String,
    pub cost: u32,
    pub on_play: OnPlay,
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
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub id: u32,
    pub def: CardDef,
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

    /// Unused until step 2, when damage lands.
    #[allow(dead_code)]
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
    /// advances when all players have ended. (Step 2: only *living*
    /// players count; death doesn't exist yet.)
    pub has_ended_turn: bool,
}

impl CombatPlayer {
    /// Total cards across this player's piles; must stay constant.
    /// Used by tests; will surface in a debug overlay later.
    #[allow(dead_code)]
    pub fn total_cards(&self) -> usize {
        self.deck.len() + self.hand.len() + self.discard.len() + self.consumed.len()
    }
}

pub struct CombatState {
    pub turn: u32,
    /// Every human player, facing the enemy group together.
    pub players: Vec<CombatPlayer>,
    /// The enemy group. One today; commonly several.
    pub enemies: Vec<Unit>,
    rng: StdRng,
}

impl CombatState {
    /// Starts a combat from persistent player state and enemy definitions:
    /// HP is copied in, each player's deck is shuffled in, opening hands are
    /// drawn. Deterministic per seed. (StdRng's algorithm may change across
    /// `rand` versions; for replays stable across builds, pin a ChaCha*Rng.)
    pub fn start(
        players: &[PlayerRunState],
        enemies: &[EnemyDef],
        seed: u64,
    ) -> (Self, Vec<Event>) {
        let mut rng = StdRng::seed_from_u64(seed);
        let players: Vec<CombatPlayer> = players
            .iter()
            .map(|p| {
                let mut deck = p.deck.clone();
                deck.shuffle(&mut rng);
                CombatPlayer {
                    id: p.id,
                    unit: Unit {
                        name: p.name.clone(),
                        hp: p.hp,
                        max_hp: p.max_hp,
                        block: 0,
                    },
                    energy: ENERGY_PER_TURN,
                    deck,
                    hand: Vec::new(),
                    discard: Vec::new(),
                    consumed: Vec::new(),
                    has_ended_turn: false,
                }
            })
            .collect();
        let enemies = enemies.iter().map(|d| Unit::new(&d.name, d.hp)).collect();
        let mut state = Self {
            turn: 1,
            players,
            enemies,
            rng,
        };
        let mut events = vec![Event::CombatBegan, Event::TurnBegan { turn: 1 }];
        for index in 0..state.players.len() {
            state.draw_to_hand_size(index, &mut events);
        }
        (state, events)
    }

    pub fn player(&self, id: PlayerId) -> Option<&CombatPlayer> {
        self.players.iter().find(|p| p.id == id)
    }

    /// The only input door (COMBAT.md Ruling 3). Accepts user actions,
    /// resolves all machinery synchronously, and returns every consequence
    /// as events, in order. Rejection is an event, never a return value.
    pub fn step(&mut self, stamped: StampedIntent) -> Vec<Event> {
        let mut events = Vec::new();
        let Some(index) = self.players.iter().position(|p| p.id == stamped.actor) else {
            events.push(Event::IntentRejected {
                actor: stamped.actor,
                intent: stamped.intent,
                reason: RejectReason::UnknownPlayer,
            });
            return events;
        };
        match stamped.intent {
            Intent::PlayCard { hand_index } => {
                self.resolve_play_card(stamped.actor, index, hand_index, &mut events);
            }
            Intent::EndTurn => self.resolve_end_turn(stamped.actor, index, &mut events),
        }
        events
    }

    fn resolve_play_card(
        &mut self,
        actor: PlayerId,
        player_index: usize,
        hand_index: usize,
        events: &mut Vec<Event>,
    ) {
        let reject = |events: &mut Vec<Event>, reason| {
            events.push(Event::IntentRejected {
                actor,
                intent: Intent::PlayCard { hand_index },
                reason,
            });
        };
        let player = &self.players[player_index];
        if player.has_ended_turn {
            reject(events, RejectReason::AlreadyEndedTurn);
            return;
        }
        let Some(card) = player.hand.get(hand_index) else {
            reject(events, RejectReason::InvalidHandIndex);
            return;
        };
        if player.energy < card.def.cost {
            reject(events, RejectReason::NotEnoughEnergy);
            return;
        }
        let player = &mut self.players[player_index];
        let card = player.hand.remove(hand_index);
        player.energy -= card.def.cost;
        // Step 2: evaluate the card's effect IR here.
        events.push(Event::CardPlayed {
            actor,
            card: card.clone(),
        });
        match card.def.on_play {
            OnPlay::Reusable => player.discard.push(card),
            OnPlay::Consume => player.consumed.push(card),
        }
    }

    fn resolve_end_turn(&mut self, actor: PlayerId, player_index: usize, events: &mut Vec<Event>) {
        if self.players[player_index].has_ended_turn {
            events.push(Event::IntentRejected {
                actor,
                intent: Intent::EndTurn,
                reason: RejectReason::AlreadyEndedTurn,
            });
            return;
        }
        self.players[player_index].has_ended_turn = true;
        events.push(Event::TurnEnded { actor });

        // Concurrent turns: the round advances only when every player has
        // ended their turn.
        if !self.players.iter().all(|p| p.has_ended_turn) {
            return;
        }

        // Enemy turn. Step 3: each enemy executes its action graph here.
        for enemy in &self.enemies {
            events.push(Event::EnemyTurnPassed {
                enemy: enemy.name.clone(),
            });
        }

        self.turn += 1;
        events.push(Event::TurnBegan { turn: self.turn });

        for index in 0..self.players.len() {
            let player = &mut self.players[index];
            player.energy = ENERGY_PER_TURN;
            player.has_ended_turn = false;
            let held = std::mem::take(&mut player.hand);
            let count = held.len();
            player.discard.extend(held);
            if count > 0 {
                events.push(Event::HandDiscarded {
                    player: player.id,
                    count,
                });
            }
            self.draw_to_hand_size(index, events);
        }
    }

    /// Draws until the player's hand is full or no cards remain to draw.
    /// Reshuffles that player's discard pile when their deck runs out,
    /// mid-draw if necessary; the event stream records the true order.
    fn draw_to_hand_size(&mut self, player_index: usize, events: &mut Vec<Event>) {
        let player = &mut self.players[player_index];
        while player.hand.len() < HAND_SIZE {
            if player.deck.is_empty() {
                if player.discard.is_empty() {
                    break;
                }
                let mut pile = std::mem::take(&mut player.discard);
                pile.shuffle(&mut self.rng);
                let count = pile.len();
                player.deck = pile;
                events.push(Event::DeckReshuffled {
                    player: player.id,
                    count,
                });
            }
            let card = player.deck.pop().expect("deck is non-empty");
            events.push(Event::CardDrawn {
                player: player.id,
                card: card.clone(),
            });
            player.hand.push(card);
        }
    }
}

/// The step-1 combat: one player with 11 do-nothing potions, one inert dummy.
pub fn starter_combat(seed: u64) -> (CombatState, Vec<Event>) {
    let player = PlayerRunState {
        id: LOCAL_PLAYER,
        name: "Alchemist".to_string(),
        hp: 80,
        max_hp: 80,
        deck: (0..11)
            .map(|i| Card {
                id: i,
                def: CardDef {
                    name: "Inert Draught".to_string(),
                    cost: 0,
                    on_play: OnPlay::Consume,
                },
            })
            .collect(),
    };
    let enemies = [EnemyDef {
        name: "Training Dummy".to_string(),
        hp: 30,
    }];
    CombatState::start(&[player], &enemies, seed)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
