//! The game client: renders the world, gathers input, and talks to a
//! `space_game_server` over TCP. All gameplay simulation is server-side.
//!
//! Usage: `space_game [server_addr] [pilot_name]`
//!   server_addr defaults to `127.0.0.1:5123` (or `SPACE_GAME_SERVER`).
//!   pilot_name defaults to `SPACE_GAME_NAME` or `pilot-<pid>`.

use bevy::prelude::*;

use space_game::config::{self, GameConfig};
use space_game::plugins::net_client::ClientNetPlugin;
use space_game::plugins::player::PlayerClientPlugin;
use space_game::plugins::sim::SimulationPlugin;
use space_game::plugins::world::{WorldClientPlugin, WorldMotionPlugin};
use space_game::plugins::{
    camera::GameCameraPlugin, effects::EffectsPlugin, ui::UiPlugin, visuals::VisualsPlugin,
};
use space_game::protocol::DEFAULT_PORT;

fn main() {
    let mut args = std::env::args().skip(1);
    let server_addr = args
        .next()
        .or_else(|| std::env::var("SPACE_GAME_SERVER").ok())
        .unwrap_or_else(|| format!("127.0.0.1:{DEFAULT_PORT}"));
    let player_name = args
        .next()
        .or_else(|| std::env::var("SPACE_GAME_NAME").ok())
        .unwrap_or_else(|| format!("pilot-{}", std::process::id()));

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: format!("Helios Drift — {player_name}"),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.02, 0.02, 0.05)))
        .insert_resource(GameConfig::load_or_default(config::CONFIG_PATH))
        // Shared sim infrastructure + deterministic decorative motion.
        // No gameplay simulation here: the server is authoritative.
        .add_plugins((SimulationPlugin, WorldMotionPlugin))
        // Client-only presentation and input.
        .add_plugins((
            GameCameraPlugin,
            VisualsPlugin,
            UiPlugin,
            EffectsPlugin,
            PlayerClientPlugin,
            WorldClientPlugin,
        ))
        .add_plugins(ClientNetPlugin {
            server_addr,
            player_name,
        })
        .run();
}
