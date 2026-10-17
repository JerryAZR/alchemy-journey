//! The terminal frontend: the smallest possible projection of the combat
//! event stream. Owns the single event-to-text rendering function
//! (COMBAT.md Ruling 4).

use alchemy_combat::{Event, Intent, OnPlay, RejectReason};
use bevy::prelude::*;

use crate::AppState;
use crate::driver::PendingEvents;

pub struct TerminalPlugin;

impl Plugin for TerminalPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, print_events.run_if(in_state(AppState::Combat)));
    }
}

fn print_events(mut pending: ResMut<PendingEvents>) {
    for event in pending.drain() {
        println!("[combat] {}", render_event(&event));
    }
}

/// The one home for event-to-text rendering.
fn render_event(event: &Event) -> String {
    match event {
        Event::CombatBegan => "Combat begins.".to_string(),
        Event::TurnBegan { turn } => format!("Turn {turn} begins."),
        Event::CardDrawn { card, .. } => format!("Drew {} #{}.", card.def.name, card.id),
        Event::DeckReshuffled { count, .. } => {
            format!("Deck empty — shuffled {count} discarded card(s) into the deck.")
        }
        Event::CardPlayed { card, .. } => match card.def.on_play {
            OnPlay::Consume => format!("Played {} (consumed).", card.def.name),
            OnPlay::Reusable => format!("Played {}.", card.def.name),
        },
        Event::IntentRejected { intent, reason, .. } => {
            let what = match intent {
                Intent::PlayCard { hand_index } => format!("play card at hand index {hand_index}"),
                Intent::EndTurn => "end turn".to_string(),
            };
            let why = match reason {
                RejectReason::UnknownPlayer => "unknown player",
                RejectReason::AlreadyEndedTurn => "already ended turn",
                RejectReason::InvalidHandIndex => "no card at that hand index",
                RejectReason::NotEnoughEnergy => "not enough energy",
            };
            format!("Rejected: cannot {what} ({why}).")
        }
        Event::TurnEnded { .. } => "Player ends turn.".to_string(),
        Event::HandDiscarded { count, .. } => format!("Discarded {count} card(s)."),
        Event::EnemyTurnPassed { enemy } => format!("{enemy} does nothing."),
    }
}
