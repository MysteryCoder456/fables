//! Bevy plugins, one per major system, split along the client/server seam.
//!
//! Server (headless, authoritative): `sim`, `player::PlayerSimPlugin`,
//! `world::{WorldSimPlugin, WorldMotionPlugin}`, `resources`, `persistence`,
//! `net_server`.
//!
//! Client (render + input): `sim`, `world::{WorldMotionPlugin,
//! WorldClientPlugin}`, `player::PlayerClientPlugin`, `camera`, `visuals`,
//! `ui`, `effects`, `net_client`.

pub mod camera;
pub mod effects;
pub mod persistence;
pub mod player;
pub mod resources;
pub mod sim;
pub mod ui;
pub mod visuals;
pub mod world;
