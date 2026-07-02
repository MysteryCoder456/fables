//! Space Game: a 2D, multiplayer, space-themed RPG.
//!
//! This library holds everything shared between the two binaries:
//! - `space_game` (client): renders the world and sends player intent.
//! - `space_game_server` (server): runs the authoritative headless
//!   simulation and replicates state to clients.
//!
//! See `README.md` for controls and `ARCHITECTURE.md` for the ECS layout and
//! the client/server protocol.

#![allow(clippy::type_complexity)]

pub mod components;
pub mod config;
pub mod logic;
pub mod plugins;
pub mod protocol;
pub mod resource_types;
