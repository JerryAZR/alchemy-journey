//! Combat domain core. See COMBAT.md at the repository root for the rulings
//! this crate enforces. In short: this crate owns all combat truth, depends
//! on nothing Bevy, accepts only user actions through its `validate` /
//! `process` pair, and reports every consequence as events. Resolution is
//! synchronous and total; between actions the state rests only at input
//! boundaries.
//!
//! Module map:
//! - [`wire`] — intents, rejections, events: the messages that cross
//!   engine boundaries.
//! - [`effect`] — the effect IR and its pure planning half. Nothing here
//!   touches combat state.
//! - [`state`] — what exists, at every data tier (identities, definitions,
//!   run state, live combat state). Data and trivial accessors only.
//! - [`engine`] — what happens: every mutation of [`CombatState`].
//! - [`content`] — the starter combat, assembled from definition types.

mod content;
mod effect;
mod engine;
mod state;
mod wire;

pub use content::*;
pub use effect::*;
pub use state::*;
pub use wire::*;

pub const HAND_SIZE: usize = 5;
pub const ENERGY_PER_TURN: u32 = 3;

/// The local player in a single-player combat is always id 0.
pub const LOCAL_PLAYER: PlayerId = PlayerId(0);

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
