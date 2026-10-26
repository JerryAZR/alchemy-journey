//! The terminal frontend: the smallest possible projection of the combat
//! event stream. Owns the single event-to-text rendering function
//! (COMBAT.md Ruling 4).

use alchemy_combat::{CombatOutcome, Event, Intent, OnPlay, RejectReason, UnitRef};
use bevy::prelude::*;

use crate::AppState;
use crate::driver::{Combat, PendingEvents, PendingNotices, Rejection};

pub struct TerminalPlugin;

impl Plugin for TerminalPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, print_events.run_if(in_state(AppState::Combat)));
    }
}

fn print_events(
    mut pending: ResMut<PendingEvents>,
    mut notices: ResMut<PendingNotices>,
    combat: Option<Res<Combat>>,
) {
    for event in pending.drain() {
        println!("[combat] {}", render_event(&event, combat.as_deref()));
    }
    for notice in notices.drain() {
        println!("[combat] {}", render_rejection(&notice));
    }
}

/// Resolves a unit identity to its display name. Events carry identities,
/// not names — name resolution is frontend policy.
fn unit_name(combat: Option<&Combat>, target: UnitRef) -> String {
    combat
        .and_then(|c| c.0.unit(target))
        .map(|u| u.name.clone())
        .unwrap_or_else(|| format!("{target:?}"))
}

/// Renders a rejection notice (host-side, submitter-only — never an event).
fn render_rejection(notice: &Rejection) -> String {
    let what = match notice.intent {
        Intent::PlayCard { hand_index, .. } => format!("play card at hand index {hand_index}"),
        Intent::EndTurn => "end turn".to_string(),
    };
    let why = match notice.reason {
        RejectReason::UnknownPlayer => "unknown player",
        RejectReason::AlreadyEndedTurn => "already ended turn",
        RejectReason::InvalidHandIndex => "no card at that hand index",
        RejectReason::NotEnoughEnergy => "not enough energy",
        RejectReason::CombatOver => "combat is over",
        RejectReason::TargetRequired => "that card needs a target",
        RejectReason::TargetNotUsed => "that card doesn't take a target",
        RejectReason::InvalidTarget => "no living unit by that identity",
    };
    format!("Rejected: cannot {what} ({why}).")
}

/// The one home for event-to-text rendering.
fn render_event(event: &Event, combat: Option<&Combat>) -> String {
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
        Event::TurnEnded { .. } => "Player ends turn.".to_string(),
        Event::HandDiscarded { count, .. } => format!("Discarded {count} card(s)."),
        Event::EnemyTurnPassed { enemy } => format!("{enemy} does nothing."),
        Event::DamageDealt {
            target,
            amount,
            blocked,
        } => {
            let name = unit_name(combat, *target);
            if *blocked > 0 {
                format!("{name} takes {amount} damage ({blocked} blocked).")
            } else {
                format!("{name} takes {amount} damage.")
            }
        }
        Event::BlockGained { unit, amount } => {
            format!("{} gains {amount} block.", unit_name(combat, *unit))
        }
        Event::UnitDied { unit } => format!("{} dies.", unit_name(combat, *unit)),
        Event::CombatOver { outcome } => match outcome {
            CombatOutcome::Victory => "Victory!".to_string(),
        },
    }
}
