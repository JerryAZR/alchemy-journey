//! Combat domain core. See COMBAT.md at the repository root for the rulings
//! this crate enforces. In short: this crate owns all combat truth, depends
//! on nothing Bevy, accepts only user actions through its `validate` /
//! `process` pair, and reports every consequence as events. Resolution is
//! synchronous and total; between actions the state rests only at input
//! boundaries.
//!
//! Data tiers (see the design discussion):
//! - Tier 1, definitions: [`CardDef`], [`EnemyDef`], [`PotionDef`] — static
//!   data.
//! - Tier 2, persistent run state: [`PlayerRunState`] — HP and deck, no
//!   combat mechanics. Combat borrows it at start and writes outcomes back
//!   at end (writeback is deferred until multiple combats exist; see
//!   DESIGN.md).
//! - Tier 3, transient combat state: [`CombatPlayer`], [`CombatState`].

use rand::SeedableRng;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;

pub const HAND_SIZE: usize = 5;
pub const ENERGY_PER_TURN: u32 = 3;

/// The local player in a single-player combat is always id 0.
pub const LOCAL_PLAYER: PlayerId = PlayerId(0);

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

// ---------------------------------------------------------------------------
// Wire vocabulary (provisional — COMBAT.md Ruling 2)
// ---------------------------------------------------------------------------

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

/// How a combat ended. Only `Victory` exists in step 2: the loss path is
/// deferred until enemy damage exists and multiple combats make it
/// e2e-testable (DESIGN.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombatOutcome {
    Victory,
}

// ---------------------------------------------------------------------------
// The effect IR (DESIGN.md): flat instruction vector + stateful sequential
// evaluator. Open types — they grow with features.
// ---------------------------------------------------------------------------

/// The atomic effect vocabulary, shared by potions and spells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Atom {
    Damage(u32),
    Block(u32),
}

/// A target *role*, resolved to concrete units at application time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetSpec {
    Caster,
    AllEnemies,
    AllAllies,
    Everyone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Instruction {
    /// This cast applies up to N atoms (the default is 1). Potions lower
    /// with their exact atom count; "evaluate all" needs no special case.
    Eval(u32),
    /// Redirect the following atoms until cleared or replaced.
    BindTarget(TargetSpec),
    /// Return to the chosen-target default.
    ClearBind,
    /// Apply one atom to the resolved target.
    Apply(Atom),
}

/// Potion authoring type. The invariants (flat, single scope, predefined
/// order) are enforced here, at the authoring layer, not in the executor.
#[derive(Clone, Debug, PartialEq)]
pub struct PotionDef {
    pub scope: Scope,
    pub effects: Vec<Atom>,
}

/// Per-potion target scope. `OneSide`/`Everyone` are deferred variants —
/// no content needs them yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// The drinker.
    Caster,
    /// One chosen unit; the player picks the target at play time.
    OneUnit,
}

impl PotionDef {
    /// Mechanical lowering into the IR (DESIGN.md). `Eval(k)` with the exact
    /// atom count makes the whole vector one cast; `Caster` scope prepends a
    /// bind; `OneUnit` leaves atoms on the chosen-target default.
    pub fn lower(&self) -> Vec<Instruction> {
        let mut out = vec![Instruction::Eval(self.effects.len() as u32)];
        if self.scope == Scope::Caster {
            out.push(Instruction::BindTarget(TargetSpec::Caster));
        }
        out.extend(self.effects.iter().copied().map(Instruction::Apply));
        out
    }
}

/// The plan of one cast, produced by pure simulation of the scan: which
/// instructions execute, where the cursor rests, the bind register at
/// rest, and whether any applied atom consumes the chosen target. Shared
/// by execution and the needs-target query so the two can never diverge.
struct CastPlan {
    steps: Vec<usize>,
    resting_cursor: usize,
    final_bind: Option<TargetSpec>,
    needs_chosen: bool,
}

/// Simulates one cast of `card` without touching any state (DESIGN.md:
/// paused-stepping evaluation). A cast scans from the cursor, applying
/// atoms until the budget (default 1, raised by `Eval`) is spent, the
/// vector ends, or the wrap budget (`def.unwrap` — passing slot 0 counts)
/// is exhausted. Reaching the vector end rests the cursor at 0, which is
/// what makes the next cast cycle.
fn plan_cast(card: &Card) -> CastPlan {
    let effects = &card.def.effects;
    let mut plan = CastPlan {
        steps: Vec::new(),
        resting_cursor: 0,
        final_bind: card.bind,
        needs_chosen: false,
    };
    let mut budget: u32 = 1;
    let mut applied: u32 = 0;
    let mut wraps_left = card.def.unwrap;
    let mut i = card.cursor.min(effects.len());
    loop {
        if i >= effects.len() {
            if wraps_left == 0 {
                break; // vector end: cast stops; cursor rests at 0
            }
            wraps_left -= 1;
            i = 0;
            continue;
        }
        match effects[i] {
            Instruction::Eval(n) => {
                budget = n;
                plan.steps.push(i);
            }
            Instruction::BindTarget(spec) => {
                plan.final_bind = Some(spec);
                plan.steps.push(i);
            }
            Instruction::ClearBind => {
                plan.final_bind = None;
                plan.steps.push(i);
            }
            Instruction::Apply(_) => {
                if applied >= budget {
                    break; // budget spent: cursor rests on this unconsumed instruction
                }
                applied += 1;
                if plan.final_bind.is_none() {
                    plan.needs_chosen = true;
                }
                plan.steps.push(i);
            }
        }
        i += 1;
    }
    plan.resting_cursor = if i >= effects.len() { 0 } else { i };
    plan
}

/// Does the upcoming cast of this card consume the chosen target? A pure
/// query the UI builds its targeting flow on (DESIGN.md).
pub fn cast_needs_target(card: &Card) -> bool {
    plan_cast(card).needs_chosen
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
    /// Within one play, the cursor may pass slot 0 at most this many times
    /// (shared across any future triggered sub-casts). Default 0: a cast
    /// never wraps; the cursor only cycles *between* casts.
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
    /// The active target bind, if a past cast left one.
    pub bind: Option<TargetSpec>,
}

impl Card {
    pub fn new(id: u32, def: CardDef) -> Self {
        Self {
            id,
            def,
            cursor: 0,
            bind: None,
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
    /// The enemy group. One today; commonly several. Corpses are never
    /// removed, keeping `EnemyId` index-stable.
    pub enemies: Vec<Unit>,
    /// Set the moment the combat is decided; a real resting state
    /// (DESIGN.md: no transient terminal states). `validate` rejects all
    /// actions once this is `Some`.
    pub outcome: Option<CombatOutcome>,
    rng: StdRng,
}

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

/// The step-2 combat (DESIGN.md): 4× stepping strike (6-7-8) and 4× stepping
/// defend (5-6-7) — reusable-toggle potions with hand-written instruction
/// vectors — plus two true potions authored via `PotionDef` lowering. One
/// inert dummy, 30 HP.
pub fn starter_combat(seed: u64) -> (CombatState, Vec<Event>) {
    let strike = |id: u32| {
        Card::new(
            id,
            CardDef {
                name: "Strike Draught".to_string(),
                cost: 1,
                on_play: OnPlay::Reusable,
                effects: vec![
                    Instruction::Apply(Atom::Damage(6)),
                    Instruction::Apply(Atom::Damage(7)),
                    Instruction::Apply(Atom::Damage(8)),
                ],
                unwrap: 0,
            },
        )
    };
    let defend = |id: u32| {
        Card::new(
            id,
            CardDef {
                name: "Ward Draught".to_string(),
                cost: 1,
                on_play: OnPlay::Reusable,
                effects: vec![
                    Instruction::BindTarget(TargetSpec::Caster),
                    Instruction::Apply(Atom::Block(5)),
                    Instruction::Apply(Atom::Block(6)),
                    Instruction::Apply(Atom::Block(7)),
                ],
                unwrap: 0,
            },
        )
    };
    let potion = |id: u32, name: &str, def: PotionDef| {
        Card::new(
            id,
            CardDef {
                name: name.to_string(),
                cost: 0,
                on_play: OnPlay::Consume,
                effects: def.lower(),
                unwrap: 0,
            },
        )
    };
    let deck: Vec<Card> = (0..4)
        .map(strike)
        .chain((4..8).map(defend))
        .chain([potion(
            8,
            "Firebomb Flask",
            PotionDef {
                scope: Scope::OneUnit,
                effects: vec![Atom::Damage(20)],
            },
        )])
        .chain([potion(
            9,
            "Stoneskin Flask",
            PotionDef {
                scope: Scope::Caster,
                effects: vec![Atom::Block(15)],
            },
        )])
        .collect();
    let player = PlayerRunState {
        id: LOCAL_PLAYER,
        name: "Alchemist".to_string(),
        hp: 80,
        max_hp: 80,
        deck,
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
