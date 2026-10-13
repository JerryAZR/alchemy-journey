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

fn combat_with(deck: Vec<Card>) -> CombatState {
    CombatState::new(deck, Unit::new("P", 80), Unit::new("E", 30), 42).0
}

fn play(state: &mut CombatState, hand_index: usize) -> Vec<Event> {
    state.step(StampedIntent {
        actor: state.player_id(),
        intent: Intent::PlayCard { hand_index },
    })
}

fn end_turn(state: &mut CombatState) -> Vec<Event> {
    state.step(StampedIntent {
        actor: state.player_id(),
        intent: Intent::EndTurn,
    })
}

#[test]
fn new_combat_draws_opening_hand() {
    let (s, events) = starter_combat(42);
    assert_eq!(s.turn, 1);
    assert_eq!(s.phase, Phase::PlayerTurn);
    assert_eq!(s.energy, ENERGY_PER_TURN);
    assert_eq!(s.hand.len(), HAND_SIZE);
    assert_eq!(s.deck.len(), 11 - HAND_SIZE);
    assert_eq!(s.total_cards(), 11);
    assert_eq!(
        events.iter().filter(|e| matches!(e, Event::CardDrawn { .. })).count(),
        HAND_SIZE
    );
}

#[test]
fn playing_a_potion_consumes_it() {
    let (mut s, _) = starter_combat(42);
    let events = play(&mut s, 0);
    assert!(matches!(events.as_slice(), [Event::CardPlayed { .. }]));
    assert_eq!(s.hand.len(), 4);
    assert_eq!(s.consumed.len(), 1);
    assert_eq!(s.discard.len(), 0);
    assert_eq!(s.total_cards(), 11);
}

#[test]
fn playing_a_reusable_card_sends_it_to_discard() {
    let mut s = combat_with((0..11).map(|i| reusable_card(i, 0)).collect());
    play(&mut s, 2);
    assert_eq!(s.discard.len(), 1);
    assert_eq!(s.consumed.len(), 0);
    assert_eq!(s.total_cards(), 11);
}

#[test]
fn illegal_intents_are_rejected_as_events() {
    let mut s = combat_with((0..11).map(|i| reusable_card(i, 1)).collect());
    // Invalid index.
    let events = play(&mut s, 99);
    assert!(matches!(
        events.as_slice(),
        [Event::IntentRejected { reason: RejectReason::InvalidHandIndex, .. }]
    ));
    // 3 energy, cards cost 1: three plays succeed, the fourth is rejected.
    assert!(matches!(play(&mut s, 0).as_slice(), [Event::CardPlayed { .. }]));
    assert!(matches!(play(&mut s, 0).as_slice(), [Event::CardPlayed { .. }]));
    assert!(matches!(play(&mut s, 0).as_slice(), [Event::CardPlayed { .. }]));
    let events = play(&mut s, 0);
    assert!(matches!(
        events.as_slice(),
        [Event::IntentRejected { reason: RejectReason::NotEnoughEnergy, .. }]
    ));
}

#[test]
fn intents_from_unknown_players_are_rejected() {
    let (mut s, _) = starter_combat(42);
    let events = s.step(StampedIntent {
        actor: PlayerId(99),
        intent: Intent::EndTurn,
    });
    assert!(matches!(
        events.as_slice(),
        [Event::IntentRejected {
            actor: PlayerId(99),
            reason: RejectReason::UnknownPlayer,
            ..
        }]
    ));
    // State untouched.
    assert_eq!(s.turn, 1);
    assert_eq!(s.hand.len(), HAND_SIZE);
}

#[test]
fn end_turn_runs_the_full_round_synchronously() {
    let (mut s, _) = starter_combat(42);
    let events = end_turn(&mut s);
    // One step: discard, enemy pass, next turn, five draws — in order.
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
    assert_eq!(s.phase, Phase::PlayerTurn);
    assert_eq!(s.turn, 2);
    assert_eq!(s.energy, ENERGY_PER_TURN);
    assert_eq!(s.hand.len(), 5);
    assert_eq!(s.deck.len(), 1);
    assert_eq!(s.total_cards(), 11);
}

#[test]
fn reshuffle_happens_mid_draw_and_the_event_stream_proves_it() {
    let (mut s, _) = starter_combat(42);
    end_turn(&mut s); // turn 2: discard 5, draw 5 (deck 6 -> 1)
    let events = end_turn(&mut s); // turn 3: draw 1, reshuffle 10, draw 4
    let kinds: Vec<&'static str> = events.iter().map(event_kind).collect();
    let pos = |k| kinds.iter().position(|x| *x == k).unwrap();
    // The reshuffle lands strictly between the draws, not before them.
    assert!(pos("DeckReshuffled") > pos("CardDrawn"));
    let draws_after: usize =
        kinds.iter().skip(pos("DeckReshuffled")).filter(|k| **k == "CardDrawn").count();
    assert_eq!(draws_after, 4);
    assert_eq!(s.hand.len(), 5);
    assert_eq!(s.deck.len(), 6);
    assert_eq!(s.total_cards(), 11);
}

#[test]
fn consumed_cards_are_never_redrawn() {
    let (mut s, _) = starter_combat(42);
    // A full game driven programmatically: play everything, end turn, repeat.
    for _ in 0..5 {
        while !s.hand.is_empty() {
            play(&mut s, 0);
        }
        end_turn(&mut s);
    }
    assert_eq!(s.consumed.len(), 11);
    assert_eq!(s.hand.len(), 0);
    assert_eq!(s.deck.len(), 0);
    assert_eq!(s.discard.len(), 0);
    assert_eq!(s.total_cards(), 11);
}

#[test]
fn deterministic_seed_reproduces_state_and_events() {
    let (state_a, events_a) = starter_combat(7);
    let (state_b, events_b) = starter_combat(7);
    let ids = |s: &CombatState| -> Vec<u32> {
        s.hand.iter().chain(s.deck.iter()).map(|c| c.id).collect()
    };
    assert_eq!(ids(&state_a), ids(&state_b));
    assert_eq!(events_a, events_b);
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
