//! Pure, engine-independent game logic.
//!
//! Nothing in this module touches rendering, input, scheduling or any other
//! Bevy machinery beyond plain math types (`Vec2`). These functions are the
//! unit-tested core of the simulation and would run unchanged on a headless
//! server.

pub mod parallax;
pub mod physics;
