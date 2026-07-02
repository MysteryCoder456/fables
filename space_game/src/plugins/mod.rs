//! Bevy plugins, one per major system.
//!
//! Simulation plugins (`sim`, plus the `FixedUpdate` halves of `player`,
//! `world`, `resources`) are what a headless server would run; the rest
//! (`camera`, `visuals`, `ui`, the input half of `player`) are client-only.

pub mod camera;
pub mod player;
pub mod sim;
pub mod visuals;
pub mod world;
