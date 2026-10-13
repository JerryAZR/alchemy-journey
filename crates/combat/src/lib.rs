//! Combat domain core. See COMBAT.md at the repository root for the rulings
//! this crate enforces. In short: this crate owns all combat truth, depends
//! on nothing Bevy, accepts only user actions through `step`, and reports
//! every consequence as events. Resolution is synchronous and total; between
//! `step` calls the state rests only at input boundaries.

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;

pub const HAND_SIZE: usize = 5;
pub const ENERGY_PER_TURN: u32 = 3;

/// The local player in a single-player combat is always id 0.
pub const LOCAL_PLAYER: PlayerId = PlayerId(0);

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
    InvalidHandIndex,
    NotEnoughEnergy,
}

/// Everything that happened as a consequence of a `step` call, in order.
/// Action echoes carry the actor; machinery events carry no actor.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    CombatBegan,
    CardPlayed { actor: PlayerId, card: Card },
    IntentRejected { actor: PlayerId, intent: Intent, reason: RejectReason },
    TurnEnded { actor: PlayerId },
    HandDiscarded { count: usize },
    EnemyTurnPassed { enemy: String },
    TurnBegan { turn: u32 },
    CardDrawn { card: Card },
    DeckReshuffled { count: usize },
}

// ---------------------------------------------------------------------------
// Cards and units
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

/// A card instance in a combat. `id` is unique per combat for debugging.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub id: u32,
    pub def: CardDef,
}

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

/// Internal machinery phase. Never rests in `EnemyTurn` between `step`
/// calls: resolution is synchronous, so this is only observable mid-step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    PlayerTurn,
    EnemyTurn,
}

// ---------------------------------------------------------------------------
// Combat state
// ---------------------------------------------------------------------------

pub struct CombatState {
    pub turn: u32,
    pub phase: Phase,
    pub player: Unit,
    pub enemy: Unit,
    pub energy: u32,
    /// Draw pile; the top of the deck is the *last* element.
    pub deck: Vec<Card>,
    pub hand: Vec<Card>,
    pub discard: Vec<Card>,
    pub consumed: Vec<Card>,
    player_id: PlayerId,
    rng: StdRng,
}

impl CombatState {
    /// Starts combat: shuffles the deck, draws the opening hand, and returns
    /// the state along with the events describing combat start.
    ///
    /// Seeded RNG: deterministic per seed, so combats are reproducible.
    /// (StdRng's algorithm may change across `rand` versions; if we ever
    /// need replays stable across builds, pin a specific ChaCha*Rng.)
    pub fn new(mut deck: Vec<Card>, player: Unit, enemy: Unit, seed: u64) -> (Self, Vec<Event>) {
        let mut rng = StdRng::seed_from_u64(seed);
        deck.shuffle(&mut rng);
        let mut state = Self {
            turn: 1,
            phase: Phase::PlayerTurn,
            player,
            enemy,
            energy: ENERGY_PER_TURN,
            deck,
            hand: Vec::new(),
            discard: Vec::new(),
            consumed: Vec::new(),
            player_id: LOCAL_PLAYER,
            rng,
        };
        let mut events = vec![Event::CombatBegan, Event::TurnBegan { turn: 1 }];
        state.draw_to_hand_size(&mut events);
        (state, events)
    }

    pub fn player_id(&self) -> PlayerId {
        self.player_id
    }

    /// Total cards across all piles; must stay constant (conservation check).
    /// Used by tests; will surface in a debug overlay later.
    #[allow(dead_code)]
    pub fn total_cards(&self) -> usize {
        self.deck.len() + self.hand.len() + self.discard.len() + self.consumed.len()
    }

    /// The only input door (COMBAT.md Ruling 3). Accepts user actions,
    /// resolves all machinery synchronously, and returns every consequence
    /// as events, in order. Rejection is an event, never a return value.
    pub fn step(&mut self, stamped: StampedIntent) -> Vec<Event> {
        let mut events = Vec::new();
        if stamped.actor != self.player_id {
            events.push(Event::IntentRejected {
                actor: stamped.actor,
                intent: stamped.intent,
                reason: RejectReason::UnknownPlayer,
            });
            return events;
        }
        match stamped.intent {
            Intent::PlayCard { hand_index } => {
                self.resolve_play_card(stamped.actor, hand_index, &mut events);
            }
            Intent::EndTurn => self.resolve_end_turn(stamped.actor, &mut events),
        }
        events
    }

    fn resolve_play_card(&mut self, actor: PlayerId, hand_index: usize, events: &mut Vec<Event>) {
        debug_assert_eq!(self.phase, Phase::PlayerTurn);
        let reject = |events: &mut Vec<Event>, reason| {
            events.push(Event::IntentRejected {
                actor,
                intent: Intent::PlayCard { hand_index },
                reason,
            });
        };
        let Some(card) = self.hand.get(hand_index) else {
            reject(events, RejectReason::InvalidHandIndex);
            return;
        };
        if self.energy < card.def.cost {
            reject(events, RejectReason::NotEnoughEnergy);
            return;
        }
        let card = self.hand.remove(hand_index);
        self.energy -= card.def.cost;
        // Step 2: evaluate the card's effect IR here.
        events.push(Event::CardPlayed {
            actor,
            card: card.clone(),
        });
        match card.def.on_play {
            OnPlay::Reusable => self.discard.push(card),
            OnPlay::Consume => self.consumed.push(card),
        }
    }

    fn resolve_end_turn(&mut self, actor: PlayerId, events: &mut Vec<Event>) {
        debug_assert_eq!(self.phase, Phase::PlayerTurn);
        events.push(Event::TurnEnded { actor });
        let held = std::mem::take(&mut self.hand);
        let count = held.len();
        for card in held {
            self.discard.push(card);
        }
        if count > 0 {
            events.push(Event::HandDiscarded { count });
        }
        self.phase = Phase::EnemyTurn;
        // Step 3: enemy intents execute here.
        events.push(Event::EnemyTurnPassed {
            enemy: self.enemy.name.clone(),
        });
        self.turn += 1;
        self.phase = Phase::PlayerTurn;
        self.energy = ENERGY_PER_TURN;
        events.push(Event::TurnBegan { turn: self.turn });
        self.draw_to_hand_size(events);
    }

    /// Draws until the hand is full or no cards remain to draw.
    /// Reshuffles the discard pile into the deck when the deck runs out,
    /// mid-draw if necessary; the event stream records the true order.
    fn draw_to_hand_size(&mut self, events: &mut Vec<Event>) {
        while self.hand.len() < HAND_SIZE {
            if self.deck.is_empty() {
                if self.discard.is_empty() {
                    break;
                }
                let mut pile = std::mem::take(&mut self.discard);
                pile.shuffle(&mut self.rng);
                let count = pile.len();
                self.deck = pile;
                events.push(Event::DeckReshuffled { count });
            }
            let card = self.deck.pop().expect("deck is non-empty");
            events.push(Event::CardDrawn { card: card.clone() });
            self.hand.push(card);
        }
    }
}

/// The step-1 combat: 11 do-nothing potions, a player, and an inert dummy.
pub fn starter_combat(seed: u64) -> (CombatState, Vec<Event>) {
    let deck = (0..11)
        .map(|i| Card {
            id: i,
            def: CardDef {
                name: "Inert Draught".to_string(),
                cost: 0,
                on_play: OnPlay::Consume,
            },
        })
        .collect();
    CombatState::new(
        deck,
        Unit::new("Alchemist", 80),
        Unit::new("Training Dummy", 30),
        seed,
    )
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
