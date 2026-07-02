//! Pure, engine-independent game logic.
//!
//! Nothing in this module touches rendering, input, scheduling or any other
//! Bevy machinery beyond plain math types (`Vec2`). These functions are the
//! unit-tested core of the simulation and would run unchanged on a headless
//! server.

pub mod belt;
pub mod cargo;
pub mod collision;
pub mod economy;
pub mod gravity;
pub mod mining;
pub mod orbit;
pub mod parallax;
pub mod physics;
