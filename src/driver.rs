//! The app-layer driver (COMBAT.md Ruling 4): owns the combat state, the
//! intent queue, and the pending-event buffer. Frontends submit bare
//! `Intent`s; this layer stamps the local player identity, validates, and
//! processes approved actions.
//! A future multiplayer server is a different driver over the same core.

use std::collections::VecDeque;

use alchemy_combat::{CombatState, Event, Intent, LOCAL_PLAYER, RejectReason, StampedIntent};
use bevy::prelude::*;

use crate::AppState;

/// The combat state, present only during `AppState::Combat`.
#[derive(Resource)]
pub struct Combat(pub CombatState);

/// Player intents awaiting validation — the *intent channel* (COMBAT.md
/// Ruling 5). Today: an in-memory queue fed by the local UI. In multiplayer
/// the host's queue is fed by every connection, and each guest's UI submits
/// over the network instead; submission code is unchanged.
///
/// MULTIPLAYER SEAM: entries are bare `Intent`s with no source. Couch co-op
/// (one connection, several local players) will need the submitting seat
/// attached at submission time, so the host can stamp the right actor.
/// One drain point: `drain_intents`.
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
///
/// Under lockstep (Ruling 5) events never cross the network: every engine
/// derives its own identical stream from the approved-action channel, and
/// each frontend consumes its local engine's stream.
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

/// A rejected intent — a notice to the submitter, never part of the
/// canonical event stream (COMBAT.md Ruling 3).
pub struct Rejection {
    pub intent: Intent,
    pub reason: RejectReason,
}

/// Rejection notices awaiting frontend consumption.
#[derive(Resource, Default)]
pub struct PendingNotices(Vec<Rejection>);

impl PendingNotices {
    pub fn drain(&mut self) -> std::vec::Drain<'_, Rejection> {
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
            .init_resource::<PendingNotices>()
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
    // MULTIPLAYER HOOK — combat start handshake: the host decides the
    // participants' run states, the enemy defs, and the seed, and broadcasts
    // them; every engine then constructs CombatState::start identically.
    // Today the local app is the host and decides alone.
    let (state, events) = alchemy_combat::starter_combat(seed_from_time());
    pending.push_all(events);
    commands.insert_resource(Combat(state));
}

fn teardown_combat(
    mut commands: Commands,
    mut queue: ResMut<IntentQueue>,
    mut pending: ResMut<PendingEvents>,
    mut notices: ResMut<PendingNotices>,
) {
    commands.remove_resource::<Combat>();
    queue.clear();
    pending.clear();
    notices.clear();
}

/// The host's intent pump (COMBAT.md Ruling 5): drains the intent channel,
/// stamps identity, validates, and loopbacks approved actions into the
/// engine (`process` + event fan-out).
///
/// MULTIPLAYER HOOKS, in order of the pipeline below:
/// 1. COLLECTION: the queue is fed by every connection, not just the local
///    UI. Arrival order here *is* the host-decided resolution order.
/// 2. STAMPING: `actor` comes from the connection's controlled player set
///    (lobby assignment), not the `LOCAL_PLAYER` constant.
/// 3. BROADCAST: approved actions are additionally sent to all engines.
/// 4. REJECTIONS route back to the submitter's connection only.
///
/// Guests never run this function. Their side is a separate receive pump —
/// receive approved action → `process` → fan out to frontends — which the
/// core's `validate`/`process` split already provides for.
fn drain_intents(
    combat: Option<ResMut<Combat>>,
    mut queue: ResMut<IntentQueue>,
    mut pending: ResMut<PendingEvents>,
    mut notices: ResMut<PendingNotices>,
) {
    let Some(mut combat) = combat else {
        return;
    };
    while let Some(intent) = queue.0.pop_front() {
        let action = StampedIntent {
            actor: LOCAL_PLAYER,
            intent,
        };
        match combat.0.validate(action) {
            // 3. LOOPBACK: every engine processes the approved action.
            Ok(()) => pending.push_all(combat.0.process(action)),
            // 4. Rejection: notice to the submitter only, never an event.
            Err(reason) => notices.0.push(Rejection { intent, reason }),
        }
    }
}
