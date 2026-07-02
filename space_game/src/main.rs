//! Space Game: a 2D, multiplayer-ready space RPG.
//!
//! See `README.md` for controls and `ARCHITECTURE.md` for the ECS layout and
//! the client/server split plan.

#![allow(clippy::type_complexity)]

mod components;
mod config;
mod logic;
mod plugins;
mod resource_types;

use bevy::prelude::*;

use config::GameConfig;
use plugins::{
    camera::GameCameraPlugin, player::PlayerPlugin, sim::SimulationPlugin, visuals::VisualsPlugin,
};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Space Game".into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.02, 0.02, 0.05)))
        .insert_resource(GameConfig::load_or_default(config::CONFIG_PATH))
        // Simulation plugins (a headless server would run these).
        .add_plugins((SimulationPlugin, PlayerPlugin))
        // Client-only plugins.
        .add_plugins((GameCameraPlugin, VisualsPlugin))
        .run();
}
