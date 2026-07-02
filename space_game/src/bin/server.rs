//! The headless authoritative game server.
//!
//! Runs the exact simulation plugins the standalone game used, minus
//! rendering, input and windowing — proof of the sim/render split.
//!
//! Usage: `space_game_server [bind_addr]` (default `0.0.0.0:5123`).
//! Run it from the `space_game/` directory so it finds
//! `assets/config/game.ron` and reads/writes `save.ron`.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;

use space_game::config::{self, GameConfig};
use space_game::plugins::net_server::ServerNetPlugin;
use space_game::plugins::persistence::PersistencePlugin;
use space_game::plugins::player::PlayerSimPlugin;
use space_game::plugins::resources::ResourcesPlugin;
use space_game::plugins::sim::{enter_playing, SimulationPlugin};
use space_game::plugins::world::{WorldMotionPlugin, WorldSimPlugin};
use space_game::protocol::DEFAULT_PORT;

fn main() {
    let bind_addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| format!("0.0.0.0:{DEFAULT_PORT}"));

    App::new()
        // Headless: drive the schedule in a loop instead of a window's
        // event loop. 240 Hz outer loop keeps the 64 Hz fixed timestep fed
        // with low latency at negligible idle cost.
        .add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
                1.0 / 240.0,
            ))),
            bevy::log::LogPlugin::default(),
            StatesPlugin,
        ))
        .insert_resource(GameConfig::load_or_default(config::CONFIG_PATH))
        // The same simulation plugins the game has always run — headless.
        .add_plugins((
            SimulationPlugin,
            PlayerSimPlugin,
            WorldSimPlugin,
            WorldMotionPlugin,
            ResourcesPlugin,
            PersistencePlugin,
        ))
        .add_plugins(ServerNetPlugin { bind_addr })
        .add_systems(Startup, enter_playing)
        .run();
}
