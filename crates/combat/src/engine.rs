//! The engine — what happens. Every mutation of [`CombatState`] lives in
//! this module: the `validate`/`process` door (COMBAT.md Ruling 3), the
//! state-mutating half of the evaluator, and the round cascade.

use rand::SeedableRng;
use rand::seq::SliceRandom;

use crate::effect::{Atom, TargetSpec};
use crate::effect::{Instruction, cast_needs_target, plan_cast};
use crate::state::Card;
use crate::state::{
    CombatOutcome, CombatPlayer, CombatState, EnemyDef, EnemyId, OnPlay, PlayerId, PlayerRunState,
    Unit, UnitRef,
};
use crate::wire::{Event, Intent, RejectReason, StampedIntent};
use crate::{ENERGY_PER_TURN, HAND_SIZE};

impl CombatState {
    /// Starts a combat from persistent player state and enemy definitions:
    /// HP is copied in, each player's deck is shuffled in, opening hands are
    /// drawn. Deterministic per seed. (StdRng's algorithm may change across
    /// `rand` versions; for replays stable across builds, pin a ChaCha*Rng.)
    ///
    /// Multiplayer: this is the only way to construct a combat, and every
    /// engine calls it with identical broadcast inputs (run states, enemy
    /// defs, seed). No other construction path may exist.
    pub fn start(
        players: &[PlayerRunState],
        enemies: &[EnemyDef],
        seed: u64,
    ) -> (Self, Vec<Event>) {
        let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
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
            outcome: None,
            rng,
        };
        let mut events = vec![Event::CombatBegan];
        // Combat starts by running the player turn-start phase for turn 1.
        state.begin_player_turn(&mut events);
        (state, events)
    }

    pub fn player(&self, id: PlayerId) -> Option<&CombatPlayer> {
        self.players.iter().find(|p| p.id == id)
    }

    /// Looks up a unit by identity. Dead units are found too (corpses stay).
    pub fn unit(&self, target: UnitRef) -> Option<&Unit> {
        match target {
            UnitRef::Player(id) => self.player(id).map(|p| &p.unit),
            UnitRef::Enemy(EnemyId(i)) => self.enemies.get(i as usize),
        }
    }

    fn unit_mut(&mut self, target: UnitRef) -> Option<&mut Unit> {
        match target {
            UnitRef::Player(id) => self
                .players
                .iter_mut()
                .find(|p| p.id == id)
                .map(|p| &mut p.unit),
            UnitRef::Enemy(EnemyId(i)) => self.enemies.get_mut(i as usize),
        }
    }

    /// The host gate (COMBAT.md Ruling 3): a read-only legality check.
    /// Engines never process actions that haven't passed this.
    pub fn validate(&self, action: StampedIntent) -> Result<(), RejectReason> {
        if self.outcome.is_some() {
            return Err(RejectReason::CombatOver);
        }
        let Some(player) = self.player(action.actor) else {
            return Err(RejectReason::UnknownPlayer);
        };
        match action.intent {
            Intent::PlayCard { hand_index, target } => {
                if player.has_ended_turn {
                    return Err(RejectReason::AlreadyEndedTurn);
                }
                let Some(card) = player.hand.get(hand_index) else {
                    return Err(RejectReason::InvalidHandIndex);
                };
                if player.energy < card.def.cost {
                    return Err(RejectReason::NotEnoughEnergy);
                }
                // Strict targeting (DESIGN.md): a missing-when-needed target
                // rejects, and a supplied-but-unconsumed target rejects.
                match (cast_needs_target(card), target) {
                    (true, None) => Err(RejectReason::TargetRequired),
                    (false, Some(_)) => Err(RejectReason::TargetNotUsed),
                    (true, Some(t)) => match self.unit(t) {
                        Some(unit) if unit.is_alive() => Ok(()),
                        _ => Err(RejectReason::InvalidTarget),
                    },
                    (false, None) => Ok(()),
                }
            }
            Intent::EndTurn => {
                if player.has_ended_turn {
                    Err(RejectReason::AlreadyEndedTurn)
                } else {
                    Ok(())
                }
            }
        }
    }

    /// The engine's one input door: processes an *approved* action,
    /// resolving all machinery synchronously and returning every consequence
    /// as events, in order. Callers must have validated the action (the host
    /// did, before broadcasting); a failure here in a receiving engine means
    /// bug or desync, so it is a debug assertion, not a rejection.
    ///
    /// Multiplayer: every engine runs this on the same approved-action
    /// stream and must derive identical state and events (lockstep).
    pub fn process(&mut self, action: StampedIntent) -> Vec<Event> {
        debug_assert!(
            self.validate(action).is_ok(),
            "process called with an illegal action: {action:?}"
        );
        let mut events = Vec::new();
        let index = self
            .players
            .iter()
            .position(|p| p.id == action.actor)
            .expect("validated action has a known actor");
        match action.intent {
            Intent::PlayCard { hand_index, target } => {
                self.resolve_play_card(action.actor, index, hand_index, target, &mut events);
            }
            Intent::EndTurn => self.resolve_end_turn(action.actor, index, &mut events),
        }
        events
    }

    fn resolve_play_card(
        &mut self,
        actor: PlayerId,
        player_index: usize,
        hand_index: usize,
        target: Option<UnitRef>,
        events: &mut Vec<Event>,
    ) {
        let mut card = self.players[player_index].hand.remove(hand_index);
        self.players[player_index].energy -= card.def.cost;
        // The echo reports the card as it was played (pre-cast eval state).
        events.push(Event::CardPlayed {
            actor,
            card: card.clone(),
        });
        self.run_cast(actor, &mut card, target, events);
        let player = &mut self.players[player_index];
        match card.def.on_play {
            OnPlay::Reusable => player.discard.push(card),
            OnPlay::Consume => player.consumed.push(card),
        }
    }

    /// Executes one cast of `card`: runs the planned scan against live
    /// state, then writes the paused-evaluation registers (cursor, bind)
    /// back onto the card instance (DESIGN.md).
    fn run_cast(
        &mut self,
        caster: PlayerId,
        card: &mut Card,
        chosen: Option<UnitRef>,
        events: &mut Vec<Event>,
    ) {
        let plan = plan_cast(card);
        let mut bind = card.bind;
        for &step in &plan.steps {
            match card.def.effects[step] {
                Instruction::Eval(_) => {}
                Instruction::BindTarget(spec) => bind = Some(spec),
                Instruction::ClearBind => bind = None,
                Instruction::Apply(atom) => {
                    let targets: Vec<UnitRef> = match bind {
                        Some(spec) => self.resolve_spec(spec, caster),
                        None => chosen.into_iter().collect(),
                    };
                    for target in targets {
                        self.apply_atom(atom, target, events);
                    }
                    // Victory is checked after every atom application; a
                    // card that ends the combat fizzles mid-scan.
                    if self.check_victory(events) {
                        break;
                    }
                }
            }
        }
        card.cursor = plan.resting_cursor;
        card.bind = plan.final_bind;
    }

    /// Resolves a target role to concrete units: living units only, in
    /// stable vec order (allies before enemies for `Everyone`).
    fn resolve_spec(&self, spec: TargetSpec, caster: PlayerId) -> Vec<UnitRef> {
        let enemies = || {
            (0..self.enemies.len())
                .map(|i| UnitRef::Enemy(EnemyId(i as u32)))
                .filter(|&r| self.unit(r).is_some_and(Unit::is_alive))
        };
        let allies = || {
            self.players
                .iter()
                .map(|p| UnitRef::Player(p.id))
                .filter(|&r| self.unit(r).is_some_and(Unit::is_alive))
        };
        match spec {
            TargetSpec::Caster => vec![UnitRef::Player(caster)],
            TargetSpec::AllEnemies => enemies().collect(),
            TargetSpec::AllAllies => allies().collect(),
            TargetSpec::Everyone => allies().chain(enemies()).collect(),
        }
    }

    /// Applies one atom to one unit. Mid-scan liveness: a target that died
    /// earlier in the same scan is skipped.
    fn apply_atom(&mut self, atom: Atom, target: UnitRef, events: &mut Vec<Event>) {
        let Some(unit) = self.unit_mut(target) else {
            return;
        };
        if !unit.is_alive() {
            return;
        }
        match atom {
            Atom::Damage(n) => {
                let blocked = unit.block.min(n);
                unit.block -= blocked;
                let amount = n - blocked;
                unit.hp = unit.hp.saturating_sub(amount);
                events.push(Event::DamageDealt {
                    target,
                    amount,
                    blocked,
                });
                if unit.hp == 0 {
                    events.push(Event::UnitDied { unit: target });
                }
            }
            Atom::Block(n) => {
                unit.block += n;
                events.push(Event::BlockGained {
                    unit: target,
                    amount: n,
                });
            }
        }
    }

    /// Sets the outcome the moment the last enemy dies. Returns whether the
    /// combat is over (the caller stops the scan).
    fn check_victory(&mut self, events: &mut Vec<Event>) -> bool {
        if self.outcome.is_none() && self.enemies.iter().all(|e| !e.is_alive()) {
            self.outcome = Some(CombatOutcome::Victory);
            events.push(Event::CombatOver {
                outcome: CombatOutcome::Victory,
            });
        }
        self.outcome.is_some()
    }

    /// A player asks to end their turn. The round cascade runs only once
    /// every player has ended (concurrent turns — COMBAT.md Ruling 5).
    /// Legality (not already ended) is established by `validate`.
    fn resolve_end_turn(&mut self, actor: PlayerId, player_index: usize, events: &mut Vec<Event>) {
        self.players[player_index].has_ended_turn = true;
        events.push(Event::TurnEnded { actor });

        // 1. The turn can only end if no player is still active.
        if self.players.iter().any(|p| !p.has_ended_turn) {
            return;
        }
        self.player_turn_end(events);
        self.enemy_turn(events);
        self.turn += 1;
        self.begin_player_turn(events);
    }

    /// 2. Player turn-end: discard hands. (Step 4: end-of-turn player
    ///    statuses tick here.)
    fn player_turn_end(&mut self, events: &mut Vec<Event>) {
        for player in &mut self.players {
            let held = std::mem::take(&mut player.hand);
            let count = held.len();
            player.discard.extend(held);
            if count > 0 {
                events.push(Event::HandDiscarded {
                    player: player.id,
                    count,
                });
            }
        }
    }

    /// 3–5. Enemy turn: clear stale block (StS rule — a unit's block clears
    ///    at the start of its own turn; inert until enemies can gain block,
    ///    step 3+), then each enemy acts one by one, then turn-end events.
    fn enemy_turn(&mut self, events: &mut Vec<Event>) {
        // 3. Enemy turn-start: block clears.
        for enemy in &mut self.enemies {
            enemy.block = 0;
        }
        for index in 0..self.enemies.len() {
            // 4. Enemy actions, taken one by one.
            self.enemy_action(index, events);
        }
        // 5. Enemy turn-end events: none yet.
    }

    /// Step 3 placeholder: execute one enemy's action graph.
    fn enemy_action(&mut self, enemy_index: usize, events: &mut Vec<Event>) {
        events.push(Event::EnemyTurnPassed {
            enemy: self.enemies[enemy_index].name.clone(),
        });
    }

    /// 6. Player turn-start: announce, reset energy and end-turn flags,
    ///    clear block (StS rule: a unit's block clears at the start of its
    ///    own turn), then draw. Also runs for turn 1 at combat start.
    ///    (Step 4: start-of-turn player statuses tick here.)
    fn begin_player_turn(&mut self, events: &mut Vec<Event>) {
        events.push(Event::TurnBegan { turn: self.turn });
        for index in 0..self.players.len() {
            let player = &mut self.players[index];
            player.energy = ENERGY_PER_TURN;
            player.has_ended_turn = false;
            player.unit.block = 0;
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
