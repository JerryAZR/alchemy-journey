//! Combat domain logic. Pure Rust: no Bevy imports, fully unit-testable.
//!
//! The Bevy UI layer reads [`CombatState`] and calls the public mutation
//! methods in response to player input. All rules live here.

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;

pub const HAND_SIZE: usize = 5;
pub const ENERGY_PER_TURN: u32 = 3;

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
#[derive(Clone, Debug)]
pub struct CardDef {
    pub name: String,
    pub cost: u32,
    pub on_play: OnPlay,
}

/// A card instance in a combat. `id` is unique per combat for debugging.
#[derive(Clone, Debug)]
pub struct Card {
    #[allow(dead_code)] // read by tests; will surface in debug UI later
    pub id: u32,
    pub def: CardDef,
}

#[derive(Clone, Debug)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    PlayerTurn,
    EnemyTurn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayError {
    NotPlayerTurn,
    InvalidHandIndex,
    NotEnoughEnergy,
}

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
    /// Human-readable combat log, oldest first. Printed to the terminal
    /// by the UI layer as new entries appear.
    pub log: Vec<String>,
    rng: StdRng,
}

impl CombatState {
    /// Starts combat: shuffles the deck and begins turn 1.
    pub fn new(mut deck: Vec<Card>, player: Unit, enemy: Unit, seed: u64) -> Self {
        // Seeded RNG: deterministic per seed, so combats are reproducible.
        // (StdRng's algorithm may change across `rand` versions; if we ever
        // need replays stable across builds, pin a specific ChaCha*Rng.)
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
            log: Vec::new(),
            rng,
        };
        state.log("Combat begins.".to_string());
        state.draw_to_hand_size();
        state
    }

    /// Total cards across all piles; must stay constant (conservation check).
    /// Used by tests; will surface in a debug overlay later.
    #[allow(dead_code)]
    pub fn total_cards(&self) -> usize {
        self.deck.len() + self.hand.len() + self.discard.len() + self.consumed.len()
    }

    pub fn play_card(&mut self, hand_index: usize) -> Result<(), PlayError> {
        if self.phase != Phase::PlayerTurn {
            return Err(PlayError::NotPlayerTurn);
        }
        let Some(card) = self.hand.get(hand_index) else {
            return Err(PlayError::InvalidHandIndex);
        };
        if self.energy < card.def.cost {
            return Err(PlayError::NotEnoughEnergy);
        }
        let card = self.hand.remove(hand_index);
        self.energy -= card.def.cost;
        // Step 2: evaluate the card's effect IR here.
        match card.def.on_play {
            OnPlay::Reusable => {
                self.log(format!("Played {}.", card.def.name));
                self.discard.push(card);
            }
            OnPlay::Consume => {
                self.log(format!("Played {} (consumed).", card.def.name));
                self.consumed.push(card);
            }
        }
        Ok(())
    }

    /// Ends the player turn: hand is discarded, phase flips to the enemy.
    pub fn end_player_turn(&mut self) {
        if self.phase != Phase::PlayerTurn {
            return;
        }
        let held = std::mem::take(&mut self.hand);
        for card in held {
            self.discard.push(card);
        }
        self.phase = Phase::EnemyTurn;
        self.log("Player ends turn.".to_string());
    }

    /// Runs the (currently inert) enemy turn, then starts the next player turn.
    pub fn run_enemy_turn(&mut self) {
        if self.phase != Phase::EnemyTurn {
            return;
        }
        // Step 3: enemy intents execute here.
        self.log(format!("{} does nothing.", self.enemy.name));
        self.turn += 1;
        self.phase = Phase::PlayerTurn;
        self.energy = ENERGY_PER_TURN;
        self.log(format!("Turn {} begins.", self.turn));
        self.draw_to_hand_size();
    }

    /// Draws until the hand is full or no cards remain to draw.
    /// Reshuffles the discard pile into the deck when the deck runs out.
    /// Each draw is logged individually so the draw/reshuffle order is
    /// unambiguous in the combat log.
    fn draw_to_hand_size(&mut self) {
        while self.hand.len() < HAND_SIZE {
            if self.deck.is_empty() {
                if self.discard.is_empty() {
                    break;
                }
                let mut pile = std::mem::take(&mut self.discard);
                pile.shuffle(&mut self.rng);
                let count = pile.len();
                self.deck = pile;
                self.log(format!(
                    "Deck empty — shuffled {count} discarded card(s) into the deck."
                ));
            }
            let card = self.deck.pop().expect("deck is non-empty");
            self.log(format!("Drew {} #{}.", card.def.name, card.id));
            self.hand.push(card);
        }
    }

    fn log(&mut self, entry: String) {
        self.log.push(entry);
    }
}

/// The step-1 combat: 11 do-nothing potions, a player, and an inert dummy.
pub fn starter_combat(seed: u64) -> CombatState {
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
mod tests {
    use super::*;

    fn reusable_card(id: u32, cost: u32) -> Card {
        Card {
            id,
            def: CardDef {
                name: format!("Test Card {id}"),
                cost,
                on_play: OnPlay::Reusable,
            },
        }
    }

    fn combat_with(deck: Vec<Card>) -> CombatState {
        CombatState::new(deck, Unit::new("P", 80), Unit::new("E", 30), 42)
    }

    #[test]
    fn new_combat_draws_opening_hand() {
        let s = starter_combat(42);
        assert_eq!(s.turn, 1);
        assert_eq!(s.phase, Phase::PlayerTurn);
        assert_eq!(s.energy, ENERGY_PER_TURN);
        assert_eq!(s.hand.len(), HAND_SIZE);
        assert_eq!(s.deck.len(), 11 - HAND_SIZE);
        assert_eq!(s.total_cards(), 11);
    }

    #[test]
    fn playing_a_potion_consumes_it() {
        let mut s = starter_combat(42);
        s.play_card(0).unwrap();
        assert_eq!(s.hand.len(), 4);
        assert_eq!(s.consumed.len(), 1);
        assert_eq!(s.discard.len(), 0);
        assert_eq!(s.total_cards(), 11);
    }

    #[test]
    fn playing_a_reusable_card_sends_it_to_discard() {
        let mut s = combat_with((0..11).map(|i| reusable_card(i, 0)).collect());
        s.play_card(2).unwrap();
        assert_eq!(s.discard.len(), 1);
        assert_eq!(s.consumed.len(), 0);
        assert_eq!(s.total_cards(), 11);
    }

    #[test]
    fn play_card_validates_phase_index_and_energy() {
        let mut s = combat_with((0..11).map(|i| reusable_card(i, 1)).collect());
        assert_eq!(s.play_card(99), Err(PlayError::InvalidHandIndex));
        // 3 energy, cards cost 1: three plays succeed, the fourth fails.
        assert!(s.play_card(0).is_ok());
        assert!(s.play_card(0).is_ok());
        assert!(s.play_card(0).is_ok());
        assert_eq!(s.play_card(0), Err(PlayError::NotEnoughEnergy));
        s.end_player_turn();
        assert_eq!(s.play_card(0), Err(PlayError::NotPlayerTurn));
    }

    #[test]
    fn end_turn_discards_hand_and_next_turn_draws() {
        let mut s = starter_combat(42);
        s.end_player_turn();
        assert_eq!(s.phase, Phase::EnemyTurn);
        assert_eq!(s.hand.len(), 0);
        assert_eq!(s.discard.len(), 5);
        s.run_enemy_turn();
        assert_eq!(s.phase, Phase::PlayerTurn);
        assert_eq!(s.turn, 2);
        assert_eq!(s.energy, ENERGY_PER_TURN);
        assert_eq!(s.hand.len(), 5);
        assert_eq!(s.deck.len(), 1);
        assert_eq!(s.total_cards(), 11);
    }

    #[test]
    fn discard_reshuffles_when_deck_runs_out() {
        let mut s = starter_combat(42);
        // Turn 1: keep all 5 cards, end turn. Turn 2 draws 5, deck has 1 left.
        s.end_player_turn();
        s.run_enemy_turn();
        // Turn 2: keep all 5, end turn. Turn 3 must reshuffle mid-draw:
        // 1 from the old deck, then 4 from the reshuffled discard.
        s.end_player_turn();
        s.run_enemy_turn();
        assert_eq!(s.turn, 3);
        assert_eq!(s.hand.len(), 5);
        assert_eq!(s.deck.len(), 6);
        assert_eq!(s.discard.len(), 0);
        assert_eq!(s.total_cards(), 11);
    }

    #[test]
    fn consumed_cards_are_never_redrawn() {
        let mut s = starter_combat(42);
        // Play every card we ever draw; with all 11 consumed the well runs dry.
        for _ in 0..5 {
            while !s.hand.is_empty() {
                s.play_card(0).unwrap();
            }
            s.end_player_turn();
            s.run_enemy_turn();
        }
        assert_eq!(s.consumed.len(), 11);
        assert_eq!(s.hand.len(), 0);
        assert_eq!(s.deck.len(), 0);
        assert_eq!(s.discard.len(), 0);
        assert_eq!(s.total_cards(), 11);
    }

    #[test]
    fn deterministic_seed_reproduces_deck_order() {
        let a = starter_combat(7);
        let b = starter_combat(7);
        let ids_a: Vec<u32> = a.hand.iter().chain(a.deck.iter()).map(|c| c.id).collect();
        let ids_b: Vec<u32> = b.hand.iter().chain(b.deck.iter()).map(|c| c.id).collect();
        assert_eq!(ids_a, ids_b);
    }
}
