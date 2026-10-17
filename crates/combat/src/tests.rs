//! Tests for the combat core. Every interaction goes through `step`:
//! these tests are also the proof that a full game can be driven
//! programmatically with no UI and no test-side game state.

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

fn run_player(id: PlayerId, deck: Vec<Card>) -> PlayerRunState {
    PlayerRunState {
        id,
        name: format!("Player {}", id.0),
        hp: 80,
        max_hp: 80,
        deck,
    }
}

fn starter_with(deck: Vec<Card>) -> CombatState {
    let enemies = [EnemyDef {
        name: "E".to_string(),
        hp: 30,
    }];
    CombatState::start(&[run_player(LOCAL_PLAYER, deck)], &enemies, 42).0
}

/// Two players, three enemies — proves the concurrent-turn structure.
fn coop_combat() -> CombatState {
    let players = [
        run_player(PlayerId(0), (0..11).map(|i| reusable_card(i, 0)).collect()),
        run_player(PlayerId(1), (11..22).map(|i| reusable_card(i, 0)).collect()),
    ];
    let enemies = [
        EnemyDef { name: "Rat".to_string(), hp: 10 },
        EnemyDef { name: "Bat".to_string(), hp: 10 },
        EnemyDef { name: "Slime".to_string(), hp: 10 },
    ];
    CombatState::start(&players, &enemies, 42).0
}

fn play(state: &mut CombatState, actor: PlayerId, hand_index: usize) -> Vec<Event> {
    state.step(StampedIntent {
        actor,
        intent: Intent::PlayCard { hand_index },
    })
}

fn end_turn(state: &mut CombatState, actor: PlayerId) -> Vec<Event> {
    state.step(StampedIntent {
        actor,
        intent: Intent::EndTurn,
    })
}

fn event_kind(event: &Event) -> &'static str {
    match event {
        Event::CombatBegan => "CombatBegan",
        Event::CardPlayed { .. } => "CardPlayed",
        Event::IntentRejected { .. } => "IntentRejected",
        Event::TurnEnded { .. } => "TurnEnded",
        Event::HandDiscarded { .. } => "HandDiscarded",
        Event::EnemyTurnPassed { .. } => "EnemyTurnPassed",
        Event::TurnBegan { .. } => "TurnBegan",
        Event::CardDrawn { .. } => "CardDrawn",
        Event::DeckReshuffled { .. } => "DeckReshuffled",
    }
}

#[test]
fn combat_start_copies_run_state_in() {
    let (s, events) = starter_combat(42);
    let player = s.player(LOCAL_PLAYER).unwrap();
    assert_eq!(s.turn, 1);
    assert_eq!(s.enemies.len(), 1);
    assert_eq!(player.unit.hp, 80);
    assert_eq!(player.energy, ENERGY_PER_TURN);
    assert_eq!(player.hand.len(), HAND_SIZE);
    assert_eq!(player.deck.len(), 11 - HAND_SIZE);
    assert_eq!(player.total_cards(), 11);
    assert_eq!(
        events.iter().filter(|e| matches!(e, Event::CardDrawn { .. })).count(),
        HAND_SIZE
    );
}

#[test]
fn playing_a_potion_consumes_it() {
    let (mut s, _) = starter_combat(42);
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
fn illegal_intents_are_rejected_as_events() {
    let mut s = starter_with((0..11).map(|i| reusable_card(i, 1)).collect());
    let events = play(&mut s, LOCAL_PLAYER, 99);
    assert!(matches!(
        events.as_slice(),
        [Event::IntentRejected { reason: RejectReason::InvalidHandIndex, .. }]
    ));
    // 3 energy, cards cost 1: three plays succeed, the fourth is rejected.
    for _ in 0..3 {
        assert!(matches!(play(&mut s, LOCAL_PLAYER, 0).as_slice(), [Event::CardPlayed { .. }]));
    }
    let events = play(&mut s, LOCAL_PLAYER, 0);
    assert!(matches!(
        events.as_slice(),
        [Event::IntentRejected { reason: RejectReason::NotEnoughEnergy, .. }]
    ));
}

#[test]
fn intents_from_unknown_players_are_rejected() {
    let (mut s, _) = starter_combat(42);
    let events = end_turn(&mut s, PlayerId(99));
    assert!(matches!(
        events.as_slice(),
        [Event::IntentRejected {
            actor: PlayerId(99),
            reason: RejectReason::UnknownPlayer,
            ..
        }]
    ));
    assert_eq!(s.turn, 1);
}

#[test]
fn end_turn_runs_the_full_round_synchronously() {
    let (mut s, _) = starter_combat(42);
    let events = end_turn(&mut s, LOCAL_PLAYER);
    let kinds: Vec<&'static str> = events.iter().map(event_kind).collect();
    assert_eq!(
        kinds,
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
    assert_eq!(player.deck.len(), 1);
    assert_eq!(player.total_cards(), 11);
}

#[test]
fn reshuffle_happens_mid_draw_and_the_event_stream_proves_it() {
    let (mut s, _) = starter_combat(42);
    end_turn(&mut s, LOCAL_PLAYER); // turn 2: discard 5, draw 5 (deck 6 -> 1)
    let events = end_turn(&mut s, LOCAL_PLAYER); // turn 3: draw 1, reshuffle 10, draw 4
    let kinds: Vec<&'static str> = events.iter().map(event_kind).collect();
    let pos = |k| kinds.iter().position(|x| *x == k).unwrap();
    assert!(pos("DeckReshuffled") > pos("CardDrawn"));
    let draws_after =
        kinds.iter().skip(pos("DeckReshuffled")).filter(|k| **k == "CardDrawn").count();
    assert_eq!(draws_after, 4);
    let player = s.player(LOCAL_PLAYER).unwrap();
    assert_eq!(player.hand.len(), 5);
    assert_eq!(player.deck.len(), 6);
}

#[test]
fn consumed_cards_are_never_redrawn() {
    let (mut s, _) = starter_combat(42);
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
    assert!(matches!(play(&mut s, PlayerId(1), 0).as_slice(), [Event::CardPlayed { .. }]));

    // Player 0 is locked out until the next turn.
    let events = play(&mut s, PlayerId(0), 0);
    assert!(matches!(
        events.as_slice(),
        [Event::IntentRejected { reason: RejectReason::AlreadyEndedTurn, .. }]
    ));

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

fn event_kinds(events: &[Event]) -> Vec<&'static str> {
    events.iter().map(event_kind).collect()
}
