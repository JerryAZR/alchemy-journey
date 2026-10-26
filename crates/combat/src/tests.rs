//! Tests for the combat core. Every interaction goes through the
//! `validate`/`process` door: these tests are also the proof that a full
//! game can be driven programmatically with no UI and no test-side game
//! state.

use super::*;

fn reusable_card(id: u32, cost: u32) -> Card {
    Card::new(
        id,
        CardDef {
            name: format!("Test Card {id}"),
            cost,
            on_play: OnPlay::Reusable,
            effects: vec![],
            unwrap: 0,
        },
    )
}

fn consume_card(id: u32, cost: u32) -> Card {
    Card::new(
        id,
        CardDef {
            name: format!("Test Potion {id}"),
            cost,
            on_play: OnPlay::Consume,
            effects: vec![],
            unwrap: 0,
        },
    )
}

/// Builds a single card around a hand-written instruction vector.
fn program_card(effects: Vec<Instruction>, unwrap: u8) -> Card {
    Card::new(
        0,
        CardDef {
            name: "Program".to_string(),
            cost: 0,
            on_play: OnPlay::Reusable,
            effects,
            unwrap,
        },
    )
}

/// A stepping strike: 6, 7, 8 damage on successive casts, then wraps.
fn stepping_strike(id: u32) -> Card {
    Card::new(
        id,
        CardDef {
            name: format!("Strike {id}"),
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
}

/// A stepping defend: 5, 6, 7 block to the caster on successive casts.
fn stepping_defend(id: u32) -> Card {
    Card::new(
        id,
        CardDef {
            name: format!("Defend {id}"),
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
}

/// A true potion: 20 damage to one chosen unit, then consumed.
fn firebomb(id: u32) -> Card {
    Card::new(
        id,
        CardDef {
            name: format!("Firebomb {id}"),
            cost: 0,
            on_play: OnPlay::Consume,
            effects: PotionDef {
                target: TargetSpec::Chosen,
                effects: vec![Atom::Damage(20)],
            }
            .lower(),
            unwrap: 0,
        },
    )
}

fn run_player(id: PlayerId, deck: Vec<Card>) -> PlayerRunState {
    PlayerRunState {
        id,
        name: format!("Player {}", id.0),
        hp: 80,
        max_hp: 80,
        deck,
    }
}

fn combat_with(deck: Vec<Card>, enemies: &[EnemyDef]) -> CombatState {
    CombatState::start(&[run_player(LOCAL_PLAYER, deck)], enemies, 42).0
}

fn starter_with(deck: Vec<Card>) -> CombatState {
    let enemies = [EnemyDef {
        name: "E".to_string(),
        hp: 30,
    }];
    combat_with(deck, &enemies)
}

/// Two players, three enemies — proves the concurrent-turn structure.
fn coop_combat() -> CombatState {
    let players = [
        run_player(PlayerId(0), (0..11).map(|i| reusable_card(i, 0)).collect()),
        run_player(PlayerId(1), (11..22).map(|i| reusable_card(i, 0)).collect()),
    ];
    let enemies = [
        EnemyDef {
            name: "Rat".to_string(),
            hp: 10,
        },
        EnemyDef {
            name: "Bat".to_string(),
            hp: 10,
        },
        EnemyDef {
            name: "Slime".to_string(),
            hp: 10,
        },
    ];
    CombatState::start(&players, &enemies, 42).0
}

fn play(state: &mut CombatState, actor: PlayerId, hand_index: usize) -> Vec<Event> {
    play_at(state, actor, hand_index, None)
}

fn play_at(
    state: &mut CombatState,
    actor: PlayerId,
    hand_index: usize,
    target: Option<UnitRef>,
) -> Vec<Event> {
    let action = StampedIntent {
        actor,
        intent: Intent::PlayCard { hand_index, target },
    };
    state
        .validate(action)
        .expect("test plays only legal actions");
    state.process(action)
}

fn end_turn(state: &mut CombatState, actor: PlayerId) -> Vec<Event> {
    let action = StampedIntent {
        actor,
        intent: Intent::EndTurn,
    };
    state.validate(action).expect("test ends only legally");
    state.process(action)
}

fn validate_play(
    state: &CombatState,
    actor: PlayerId,
    hand_index: usize,
    target: Option<UnitRef>,
) -> Result<(), RejectReason> {
    state.validate(StampedIntent {
        actor,
        intent: Intent::PlayCard { hand_index, target },
    })
}

fn validate_end_turn(state: &CombatState, actor: PlayerId) -> Result<(), RejectReason> {
    state.validate(StampedIntent {
        actor,
        intent: Intent::EndTurn,
    })
}

fn event_kind(event: &Event) -> &'static str {
    match event {
        Event::CombatBegan => "CombatBegan",
        Event::CardPlayed { .. } => "CardPlayed",
        Event::TurnEnded { .. } => "TurnEnded",
        Event::HandDiscarded { .. } => "HandDiscarded",
        Event::EnemyTurnPassed { .. } => "EnemyTurnPassed",
        Event::TurnBegan { .. } => "TurnBegan",
        Event::CardDrawn { .. } => "CardDrawn",
        Event::DeckReshuffled { .. } => "DeckReshuffled",
        Event::DamageDealt { .. } => "DamageDealt",
        Event::BlockGained { .. } => "BlockGained",
        Event::UnitDied { .. } => "UnitDied",
        Event::CombatOver { .. } => "CombatOver",
    }
}

fn event_kinds(events: &[Event]) -> Vec<&'static str> {
    events.iter().map(event_kind).collect()
}

#[test]
fn combat_start_copies_run_state_in() {
    let (s, events) = starter_combat(42);
    let player = s.player(LOCAL_PLAYER).unwrap();
    assert_eq!(s.turn, 1);
    assert_eq!(s.enemies.len(), 1);
    assert!(s.outcome.is_none());
    assert_eq!(player.unit.hp, 80);
    assert_eq!(player.energy, ENERGY_PER_TURN);
    assert_eq!(player.hand.len(), HAND_SIZE);
    assert_eq!(player.deck.len(), 10 - HAND_SIZE);
    assert_eq!(player.total_cards(), 10);
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::CardDrawn { .. }))
            .count(),
        HAND_SIZE
    );
}

#[test]
fn playing_a_potion_consumes_it() {
    let mut s = starter_with((0..11).map(|i| consume_card(i, 0)).collect());
    let events = play(&mut s, LOCAL_PLAYER, 0);
    assert!(matches!(events.as_slice(), [Event::CardPlayed { .. }]));
    let player = s.player(LOCAL_PLAYER).unwrap();
    assert_eq!(player.hand.len(), 4);
    assert_eq!(player.consumed.len(), 1);
    assert_eq!(player.discard.len(), 0);
    assert_eq!(player.total_cards(), 11);
}

#[test]
fn playing_a_reusable_card_sends_it_to_discard() {
    let mut s = starter_with((0..11).map(|i| reusable_card(i, 0)).collect());
    play(&mut s, LOCAL_PLAYER, 2);
    let player = s.player(LOCAL_PLAYER).unwrap();
    assert_eq!(player.discard.len(), 1);
    assert_eq!(player.consumed.len(), 0);
}

#[test]
fn illegal_intents_fail_validation() {
    let mut s = starter_with((0..11).map(|i| reusable_card(i, 1)).collect());
    assert_eq!(
        validate_play(&s, LOCAL_PLAYER, 99, None),
        Err(RejectReason::InvalidHandIndex)
    );
    // 3 energy, cards cost 1: three plays succeed, the fourth is rejected.
    for _ in 0..3 {
        assert!(matches!(
            play(&mut s, LOCAL_PLAYER, 0).as_slice(),
            [Event::CardPlayed { .. }]
        ));
    }
    assert_eq!(
        validate_play(&s, LOCAL_PLAYER, 0, None),
        Err(RejectReason::NotEnoughEnergy)
    );
}

#[test]
fn intents_from_unknown_players_fail_validation() {
    let (s, _) = starter_combat(42);
    assert_eq!(
        validate_end_turn(&s, PlayerId(99)),
        Err(RejectReason::UnknownPlayer)
    );
    assert_eq!(s.turn, 1);
}

#[test]
fn end_turn_runs_the_full_round_synchronously() {
    let (mut s, _) = starter_combat(42);
    let events = end_turn(&mut s, LOCAL_PLAYER);
    assert_eq!(
        event_kinds(&events),
        [
            "TurnEnded",
            "HandDiscarded",
            "EnemyTurnPassed",
            "TurnBegan",
            "CardDrawn",
            "CardDrawn",
            "CardDrawn",
            "CardDrawn",
            "CardDrawn",
        ]
    );
    let player = s.player(LOCAL_PLAYER).unwrap();
    assert_eq!(s.turn, 2);
    assert_eq!(player.energy, ENERGY_PER_TURN);
    assert!(!player.has_ended_turn);
    assert_eq!(player.hand.len(), 5);
    assert_eq!(player.deck.len(), 0);
    assert_eq!(player.total_cards(), 10);
}

#[test]
fn reshuffle_happens_mid_draw_and_the_event_stream_proves_it() {
    let mut s = starter_with((0..11).map(|i| reusable_card(i, 0)).collect());
    end_turn(&mut s, LOCAL_PLAYER); // turn 2: discard 5, draw 5 (deck 6 -> 1)
    let events = end_turn(&mut s, LOCAL_PLAYER); // turn 3: draw 1, reshuffle 10, draw 4
    let kinds = event_kinds(&events);
    let pos = |k| kinds.iter().position(|x| *x == k).unwrap();
    assert!(pos("DeckReshuffled") > pos("CardDrawn"));
    let draws_after = kinds
        .iter()
        .skip(pos("DeckReshuffled"))
        .filter(|k| **k == "CardDrawn")
        .count();
    assert_eq!(draws_after, 4);
    let player = s.player(LOCAL_PLAYER).unwrap();
    assert_eq!(player.hand.len(), 5);
    assert_eq!(player.deck.len(), 6);
}

#[test]
fn consumed_cards_are_never_redrawn() {
    let mut s = starter_with((0..11).map(|i| consume_card(i, 0)).collect());
    // A full game driven programmatically: play everything, end turn, repeat.
    for _ in 0..5 {
        while !s.player(LOCAL_PLAYER).unwrap().hand.is_empty() {
            play(&mut s, LOCAL_PLAYER, 0);
        }
        end_turn(&mut s, LOCAL_PLAYER);
    }
    let player = s.player(LOCAL_PLAYER).unwrap();
    assert_eq!(player.consumed.len(), 11);
    assert_eq!(player.hand.len(), 0);
    assert_eq!(player.deck.len(), 0);
    assert_eq!(player.discard.len(), 0);
}

#[test]
fn deterministic_seed_reproduces_state_and_events() {
    let (state_a, events_a) = starter_combat(7);
    let (state_b, events_b) = starter_combat(7);
    let ids = |s: &CombatState| -> Vec<u32> {
        let p = s.player(LOCAL_PLAYER).unwrap();
        p.hand.iter().chain(p.deck.iter()).map(|c| c.id).collect()
    };
    assert_eq!(ids(&state_a), ids(&state_b));
    assert_eq!(events_a, events_b);
}

// --- Step 2: effects, stepping, targeting, combat end ---

/// The Eval quota bug: a cast whose quota is spent must stop immediately —
/// scanning on into an Eval must not retroactively extend the spent cast.
/// First cast of [Damage(1), Eval(2), Damage(1), Damage(1)] is 1 damage.
#[test]
fn cast_stops_when_its_quota_is_spent() {
    let card = Card::new(
        0,
        CardDef {
            name: "Early Eval".to_string(),
            cost: 0,
            on_play: OnPlay::Reusable,
            effects: vec![
                Instruction::Apply(Atom::Damage(1)),
                Instruction::Eval(2),
                Instruction::Apply(Atom::Damage(1)),
                Instruction::Apply(Atom::Damage(1)),
            ],
            unwrap: 0,
        },
    );
    let mut s = starter_with(vec![card]);
    play_at(&mut s, LOCAL_PLAYER, 0, Some(UnitRef::Enemy(EnemyId(0))));
    assert_eq!(s.enemies[0].hp, 29);
}

#[test]
fn potion_lowering_is_mechanical() {
    let caster = PotionDef {
        target: TargetSpec::Caster,
        effects: vec![Atom::Block(5), Atom::Block(3)],
    }
    .lower();
    assert_eq!(
        caster,
        [
            Instruction::Eval(2),
            Instruction::BindTarget(TargetSpec::Caster),
            Instruction::Apply(Atom::Block(5)),
            Instruction::Apply(Atom::Block(3)),
        ]
    );
    let chosen = PotionDef {
        target: TargetSpec::Chosen,
        effects: vec![Atom::Damage(9)],
    }
    .lower();
    assert_eq!(
        chosen,
        [Instruction::Eval(1), Instruction::Apply(Atom::Damage(9))]
    );
}

/// Grouping semantics (DESIGN.md): Eval(N) makes the next N *blocks* one
/// block, and groups nest multiplicatively.
/// Eval(2) - Eval(2) - D - D - Eval(2) - D - D  =  Eval(2)[Eval(2)[D,D], Eval(2)[D,D]].
#[test]
fn eval_groups_nest_multiplicatively() {
    let d = || Instruction::Apply(Atom::Damage(1));
    let card = program_card(
        vec![
            Instruction::Eval(2),
            Instruction::Eval(2),
            d(),
            d(),
            Instruction::Eval(2),
            d(),
            d(),
        ],
        0,
    );
    let mut s = starter_with(vec![card]);
    play_at(&mut s, LOCAL_PLAYER, 0, Some(UnitRef::Enemy(EnemyId(0))));
    assert_eq!(s.enemies[0].hp, 30 - 4);
}

/// Eval(2) - D - Eval(2) - D - D  =  Eval(2)[D, Eval(2)[D, D]]: 3 damage.
#[test]
fn eval_groups_compose_sequentially() {
    let d = || Instruction::Apply(Atom::Damage(1));
    let card = program_card(
        vec![Instruction::Eval(2), d(), Instruction::Eval(2), d(), d()],
        0,
    );
    let mut s = starter_with(vec![card]);
    play_at(&mut s, LOCAL_PLAYER, 0, Some(UnitRef::Enemy(EnemyId(0))));
    assert_eq!(s.enemies[0].hp, 30 - 3);
}

/// The wrap budget lets a cast pass slot 0, re-reading instructions —
/// including Eval, which groups again on the second lap.
/// [Eval(3), D6, D7] with unwrap 1: one cast applies 6+7, wraps, 6+7 again.
#[test]
fn unwrap_budget_allows_one_lap_past_slot_zero() {
    let card = program_card(
        vec![
            Instruction::Eval(3),
            Instruction::Apply(Atom::Damage(6)),
            Instruction::Apply(Atom::Damage(7)),
        ],
        1,
    );
    let mut s = starter_with(vec![card]);
    play_at(&mut s, LOCAL_PLAYER, 0, Some(UnitRef::Enemy(EnemyId(0))));
    assert_eq!(s.enemies[0].hp, 30 - 26);
    // The second cast starts over: another 26, which is lethal.
    end_turn(&mut s, LOCAL_PLAYER); // discard, reshuffle, redraw the same card
    play_at(&mut s, LOCAL_PLAYER, 0, Some(UnitRef::Enemy(EnemyId(0))));
    assert_eq!(s.outcome, Some(CombatOutcome::Victory));
}

/// A one-card deck plays the same instance every turn, so its cursor and
/// bind can be observed stepping across casts.
fn one_card_loop(card: Card) -> CombatState {
    starter_with(vec![card])
}

#[test]
fn stepping_strike_cycles_and_wraps() {
    let mut s = one_card_loop(stepping_strike(0));
    let enemy = Some(UnitRef::Enemy(EnemyId(0)));
    for (turn, expected) in [(1, 6), (2, 7), (3, 8), (4, 6)] {
        assert_eq!(s.turn, turn);
        let events = play_at(&mut s, LOCAL_PLAYER, 0, enemy);
        assert!(events.contains(&Event::DamageDealt {
            target: UnitRef::Enemy(EnemyId(0)),
            amount: expected,
            blocked: 0,
        }));
        assert_eq!(
            s.enemies[0].hp,
            30 - [6, 7, 8, 6][..turn as usize].iter().sum::<u32>()
        );
        end_turn(&mut s, LOCAL_PLAYER); // discard, reshuffle, redraw the same card
    }
}

#[test]
fn stepping_defend_binds_the_caster_across_casts() {
    let mut s = one_card_loop(stepping_defend(0));
    for expected in [5, 6, 7] {
        // The bind set in the first cast governs every later cast: the card
        // never needs a chosen target.
        assert!(!cast_needs_target(&s.player(LOCAL_PLAYER).unwrap().hand[0]));
        let events = play(&mut s, LOCAL_PLAYER, 0);
        assert!(events.contains(&Event::BlockGained {
            unit: UnitRef::Player(LOCAL_PLAYER),
            amount: expected,
        }));
        assert_eq!(s.player(LOCAL_PLAYER).unwrap().unit.block, expected);
        end_turn(&mut s, LOCAL_PLAYER); // block clears at the next turn start
        assert_eq!(s.player(LOCAL_PLAYER).unwrap().unit.block, 0);
    }
}

#[test]
fn block_absorbs_damage_before_hp() {
    let mut s = starter_with(vec![stepping_strike(0)]);
    s.enemies[0].block = 5; // test-only construction
    let events = play_at(&mut s, LOCAL_PLAYER, 0, Some(UnitRef::Enemy(EnemyId(0))));
    assert!(events.contains(&Event::DamageDealt {
        target: UnitRef::Enemy(EnemyId(0)),
        amount: 1,
        blocked: 5,
    }));
    assert_eq!(s.enemies[0].hp, 29);
    assert_eq!(s.enemies[0].block, 0);
}

#[test]
fn enemy_block_clears_at_the_enemy_turn_start() {
    let mut s = starter_with(vec![reusable_card(0, 0)]);
    s.enemies[0].block = 12; // test-only construction
    end_turn(&mut s, LOCAL_PLAYER);
    assert_eq!(s.enemies[0].block, 0);
}

#[test]
fn lethal_damage_ends_the_combat() {
    let mut s = combat_with(
        vec![firebomb(0)],
        &[EnemyDef {
            name: "E".to_string(),
            hp: 20,
        }],
    );
    let events = play_at(&mut s, LOCAL_PLAYER, 0, Some(UnitRef::Enemy(EnemyId(0))));
    assert_eq!(
        event_kinds(&events),
        ["CardPlayed", "DamageDealt", "UnitDied", "CombatOver"]
    );
    assert_eq!(s.outcome, Some(CombatOutcome::Victory));
    // The terminal state rejects everything.
    assert_eq!(
        validate_end_turn(&s, LOCAL_PLAYER),
        Err(RejectReason::CombatOver)
    );
    assert_eq!(
        validate_play(&s, LOCAL_PLAYER, 0, None),
        Err(RejectReason::CombatOver)
    );
}

#[test]
fn victory_fizzles_the_rest_of_the_card() {
    let killer = Card::new(
        0,
        CardDef {
            name: "Overkill".to_string(),
            cost: 0,
            on_play: OnPlay::Consume,
            effects: vec![
                Instruction::Eval(2),
                Instruction::Apply(Atom::Damage(99)),
                Instruction::BindTarget(TargetSpec::Caster),
                Instruction::Apply(Atom::Block(10)),
            ],
            unwrap: 0,
        },
    );
    let mut s = combat_with(
        vec![killer],
        &[EnemyDef {
            name: "E".to_string(),
            hp: 20,
        }],
    );
    let events = play_at(&mut s, LOCAL_PLAYER, 0, Some(UnitRef::Enemy(EnemyId(0))));
    assert_eq!(s.outcome, Some(CombatOutcome::Victory));
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::BlockGained { .. }))
    );
    assert_eq!(s.player(LOCAL_PLAYER).unwrap().unit.block, 0);
}

#[test]
fn targeting_validation_is_strict() {
    let s = starter_with(vec![stepping_strike(0), stepping_defend(1)]);
    let enemy = Some(UnitRef::Enemy(EnemyId(0)));
    // Find the strike in hand (order is shuffled; both are present).
    let strike_idx = s.player(LOCAL_PLAYER).unwrap().hand[0]
        .def
        .name
        .starts_with("Strike");
    let (strike, defend) = if strike_idx { (0, 1) } else { (1, 0) };
    assert_eq!(
        validate_play(&s, LOCAL_PLAYER, strike, None),
        Err(RejectReason::TargetRequired)
    );
    assert_eq!(
        validate_play(&s, LOCAL_PLAYER, strike, Some(UnitRef::Enemy(EnemyId(99)))),
        Err(RejectReason::InvalidTarget)
    );
    assert_eq!(
        validate_play(&s, LOCAL_PLAYER, defend, enemy),
        Err(RejectReason::TargetNotUsed)
    );
    assert_eq!(validate_play(&s, LOCAL_PLAYER, strike, enemy), Ok(()));

    // A corpse is not a valid target. Two enemies: kill one, target it.
    let mut s = combat_with(
        vec![firebomb(0), stepping_strike(1)],
        &[
            EnemyDef {
                name: "A".to_string(),
                hp: 10,
            },
            EnemyDef {
                name: "B".to_string(),
                hp: 30,
            },
        ],
    );
    play_at(&mut s, LOCAL_PLAYER, 0, Some(UnitRef::Enemy(EnemyId(0))));
    assert_eq!(
        validate_play(&s, LOCAL_PLAYER, 0, Some(UnitRef::Enemy(EnemyId(0)))),
        Err(RejectReason::InvalidTarget)
    );
    assert_eq!(
        validate_play(&s, LOCAL_PLAYER, 0, Some(UnitRef::Enemy(EnemyId(1)))),
        Ok(())
    );
}

#[test]
fn all_enemies_binds_expand_to_living_units_only() {
    let wave = Card::new(
        0,
        CardDef {
            name: "Shockwave".to_string(),
            cost: 0,
            on_play: OnPlay::Consume,
            effects: vec![
                Instruction::Eval(1),
                Instruction::BindTarget(TargetSpec::AllEnemies),
                Instruction::Apply(Atom::Damage(5)),
            ],
            unwrap: 0,
        },
    );
    let mut s = combat_with(
        vec![firebomb(0), wave],
        &[
            EnemyDef {
                name: "A".to_string(),
                hp: 10,
            },
            EnemyDef {
                name: "B".to_string(),
                hp: 30,
            },
            EnemyDef {
                name: "C".to_string(),
                hp: 30,
            },
        ],
    );
    play_at(&mut s, LOCAL_PLAYER, 0, Some(UnitRef::Enemy(EnemyId(0)))); // kill A
    let events = play(&mut s, LOCAL_PLAYER, 0); // shockwave: bound, no chosen target
    let hits: Vec<UnitRef> = events
        .iter()
        .filter_map(|e| match e {
            Event::DamageDealt { target, .. } => Some(*target),
            _ => None,
        })
        .collect();
    // The corpse is skipped; B and C are hit in stable vec order.
    assert_eq!(
        hits,
        [UnitRef::Enemy(EnemyId(1)), UnitRef::Enemy(EnemyId(2))]
    );
}

// --- Concurrent-turn semantics (2 players, 3 enemies) ---

#[test]
fn the_round_advances_only_when_all_players_have_ended() {
    let mut s = coop_combat();
    assert_eq!(s.enemies.len(), 3);

    // Player 0 ends: no cascade, turn holds, player 1 unaffected.
    let events = end_turn(&mut s, PlayerId(0));
    assert_eq!(event_kinds(&events), ["TurnEnded"]);
    assert_eq!(s.turn, 1);
    assert!(s.player(PlayerId(0)).unwrap().has_ended_turn);
    assert!(!s.player(PlayerId(1)).unwrap().has_ended_turn);

    // Player 1 can still act while player 0 has ended.
    assert!(matches!(
        play(&mut s, PlayerId(1), 0).as_slice(),
        [Event::CardPlayed { .. }]
    ));

    // Player 0 is locked out until the next turn.
    assert_eq!(
        validate_play(&s, PlayerId(0), 0, None),
        Err(RejectReason::AlreadyEndedTurn)
    );

    // Player 1 ends: the round cascade runs — three enemies pass, both
    // players reset and redraw, flags clear.
    let events = end_turn(&mut s, PlayerId(1));
    let kinds = event_kinds(&events);
    assert_eq!(kinds.iter().filter(|k| **k == "EnemyTurnPassed").count(), 3);
    assert_eq!(kinds.iter().filter(|k| **k == "HandDiscarded").count(), 2);
    assert_eq!(kinds.iter().filter(|k| **k == "CardDrawn").count(), 10);
    assert_eq!(s.turn, 2);
    for id in [PlayerId(0), PlayerId(1)] {
        let p = s.player(id).unwrap();
        assert!(!p.has_ended_turn);
        assert_eq!(p.energy, ENERGY_PER_TURN);
        assert_eq!(p.hand.len(), HAND_SIZE);
        assert_eq!(p.total_cards(), 11);
    }
}

#[test]
fn piles_are_per_player() {
    let mut s = coop_combat();
    // Player 0 plays two cards; player 1's piles are untouched.
    play(&mut s, PlayerId(0), 0);
    play(&mut s, PlayerId(0), 0);
    assert_eq!(s.player(PlayerId(0)).unwrap().hand.len(), 3);
    assert_eq!(s.player(PlayerId(0)).unwrap().discard.len(), 2);
    assert_eq!(s.player(PlayerId(1)).unwrap().hand.len(), 5);
    assert_eq!(s.player(PlayerId(1)).unwrap().discard.len(), 0);
}

/// The lockstep premise (COMBAT.md Ruling 5): two engines given the same
/// seed and the same approved-action stream derive identical event streams
/// and identical state. The script is adaptive — built from the host's
/// public state (including the needs-target query), so it is a legal
/// approved-action stream by construction, and identical for the guest
/// precisely when lockstep holds. It also plays a real game to victory.
#[test]
fn two_engines_stay_in_lockstep() {
    let (mut host, host_start) = starter_combat(7);
    let (mut guest, guest_start) = starter_combat(7);
    assert_eq!(host_start, guest_start);

    let first_living_enemy = |s: &CombatState| {
        s.enemies
            .iter()
            .position(Unit::is_alive)
            .map(|i| UnitRef::Enemy(EnemyId(i as u32)))
    };

    for _ in 0..20 {
        if host.outcome.is_some() {
            break;
        }
        // Play every playable card this turn.
        loop {
            if host.outcome.is_some() {
                break;
            }
            let player = host.player(LOCAL_PLAYER).unwrap();
            let playable = player.hand.iter().position(|c| c.def.cost <= player.energy);
            let Some(hand_index) = playable else { break };
            let target = if cast_needs_target(&player.hand[hand_index]) {
                let Some(t) = first_living_enemy(&host) else {
                    break;
                };
                Some(t)
            } else {
                None
            };
            let action = StampedIntent {
                actor: LOCAL_PLAYER,
                intent: Intent::PlayCard { hand_index, target },
            };
            assert_eq!(host.validate(action), guest.validate(action));
            assert!(host.validate(action).is_ok());
            assert_eq!(host.process(action), guest.process(action));
        }
        if host.outcome.is_some() {
            break;
        }
        let action = StampedIntent {
            actor: LOCAL_PLAYER,
            intent: Intent::EndTurn,
        };
        assert_eq!(host.validate(action), guest.validate(action));
        assert_eq!(host.process(action), guest.process(action));
    }

    // The deck is lethal within a few turns: this is also the e2e win test.
    assert_eq!(host.outcome, Some(CombatOutcome::Victory));
    assert_eq!(host.outcome, guest.outcome);

    // Identical event streams imply identical state; spot-check the state.
    let snapshot = |s: &CombatState| {
        let p = s.player(LOCAL_PLAYER).unwrap();
        let ids = |pile: &[Card]| pile.iter().map(|c| c.id).collect::<Vec<_>>();
        (
            s.turn,
            p.energy,
            p.unit.hp,
            ids(&p.hand),
            ids(&p.deck),
            ids(&p.discard),
            ids(&p.consumed),
        )
    };
    assert_eq!(snapshot(&host), snapshot(&guest));
}
