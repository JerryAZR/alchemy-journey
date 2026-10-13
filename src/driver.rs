//! The app-layer driver (COMBAT.md Ruling 4): owns the combat state, the
//! intent queue, and the pending-event buffer. Frontends submit bare
//! `Intent`s; this layer stamps the local player identity and calls `step`.
//! A future multiplayer server is a different driver over the same core.

use std::collections::VecDeque;

use alchemy_combat::{CombatState, Event, Intent, StampedIntent};
use bevy::prelude::*;

use crate::AppState;

/// The combat state, present only during `AppState::Combat`.
#[derive(Resource)]
pub struct Combat(pub CombatState);

/// Player intents awaiting application. One drain point: `drain_intents`.
#[derive(Resource, Default)]
pub struct IntentQueue(VecDeque<Intent>);

impl IntentQueue {
    pub fn submit(&mut self, intent: Intent) {
        self.0.push_back(intent);
    }

    fn clear(&mut self) {
        self.0.clear();
    }
}

/// Events emitted by the core, awaiting frontend consumption.
/// One drain point: each frontend drains what it needs.
#[derive(Resource, Default)]
pub struct PendingEvents(Vec<Event>);

impl PendingEvents {
    pub fn push_all(&mut self, events: Vec<Event>) {
        self.0.extend(events);
    }

    pub fn drain(&mut self) -> std::vec::Drain<'_, Event> {
        self.0.drain(..)
    }

    fn clear(&mut self) {
        self.0.clear();
    }
}

pub struct DriverPlugin;

impl Plugin for DriverPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<IntentQueue>()
            .init_resource::<PendingEvents>()
            .add_systems(OnEnter(AppState::Combat), setup_combat)
            .add_systems(OnExit(AppState::Combat), teardown_combat)
            .add_systems(Update, drain_intents.run_if(in_state(AppState::Combat)));
    }
}

fn seed_from_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1)
}

fn setup_combat(mut commands: Commands, mut pending: ResMut<PendingEvents>) {
    let (state, events) = alchemy_combat::starter_combat(seed_from_time());
    pending.push_all(events);
    commands.insert_resource(Combat(state));
}

fn teardown_combat(
    mut commands: Commands,
    mut queue: ResMut<IntentQueue>,
    mut pending: ResMut<PendingEvents>,
) {
    commands.remove_resource::<Combat>();
    queue.clear();
    pending.clear();
}

/// The single intent drain: stamps the local player identity and steps the
/// core. Events flow into the pending buffer for frontends.
fn drain_intents(
    combat: Option<ResMut<Combat>>,
    mut queue: ResMut<IntentQueue>,
    mut pending: ResMut<PendingEvents>,
) {
    let Some(mut combat) = combat else {
        return;
    };
    let actor = combat.0.player_id();
    while let Some(intent) = queue.0.pop_front() {
        let events = combat.0.step(StampedIntent { actor, intent });
        pending.push_all(events);
    }
}
