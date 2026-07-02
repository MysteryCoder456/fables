//! Standalone (single-process) build of the game, used while the network
//! client is under construction: runs the authoritative simulation and the
//! renderer in one app, with local input feeding the ship's intent directly.
//!
//! This binary becomes the network client in the multiplayer pass.

use bevy::prelude::*;

use space_game::components::{LocalIntent, LocalShip, MiningRig, PlayerIntent, PlayerName, SimSet};
use space_game::components::{Hull, PlayerShip, SimPosition, SimRotation, Velocity};
use space_game::config::{self, GameConfig};
use space_game::logic::cargo::Cargo;
use space_game::plugins::persistence::{PendingLoad, PersistencePlugin};
use space_game::plugins::player::{PlayerClientPlugin, PlayerSimPlugin};
use space_game::plugins::resources::ResourcesPlugin;
use space_game::plugins::sim::{enter_playing, SimulationPlugin};
use space_game::plugins::world::{NetIdAllocator, WorldClientPlugin, WorldMotionPlugin, WorldSimPlugin};
use space_game::plugins::{camera::GameCameraPlugin, effects::EffectsPlugin, ui::UiPlugin, visuals::VisualsPlugin};

const LOCAL_PLAYER_NAME: &str = "local";

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
        // Simulation plugins (the server side of the future split).
        .add_plugins((
            SimulationPlugin,
            PlayerSimPlugin,
            WorldSimPlugin,
            WorldMotionPlugin,
            ResourcesPlugin,
            PersistencePlugin,
        ))
        // Client-only plugins.
        .add_plugins((
            GameCameraPlugin,
            VisualsPlugin,
            UiPlugin,
            EffectsPlugin,
            PlayerClientPlugin,
            WorldClientPlugin,
        ))
        .add_systems(Startup, (enter_playing, spawn_local_ship))
        // Stand-in for the network round-trip: local intent -> ship intent.
        .add_systems(FixedUpdate, apply_local_intent.in_set(SimSet::NetSync))
        .run();
}

fn spawn_local_ship(
    mut commands: Commands,
    config: Res<GameConfig>,
    pending: Option<Res<PendingLoad>>,
    mut net_ids: ResMut<NetIdAllocator>,
) {
    let saved = pending.as_ref().and_then(|pending| {
        pending.0.as_ref().and_then(|save| {
            save.players
                .iter()
                .find(|player| player.name == LOCAL_PLAYER_NAME)
                .map(|player| player.ship.clone())
        })
    });

    let (position, rotation, velocity, hull, stats, cargo) = match saved {
        Some(ship) => (
            ship.position,
            ship.rotation,
            ship.velocity,
            ship.hull,
            ship.stats,
            ship.cargo,
        ),
        None => {
            let stats = config.ship.stats.clone();
            (
                Vec2::new(config.ship.spawn_position.0, config.ship.spawn_position.1),
                std::f32::consts::FRAC_PI_2,
                Vec2::ZERO,
                stats.max_hull,
                stats.clone(),
                Cargo::new(stats.cargo_capacity),
            )
        }
    };

    commands.spawn((
        Name::new("Player Ship"),
        PlayerShip,
        LocalShip,
        PlayerName(LOCAL_PLAYER_NAME.into()),
        net_ids.allocate(),
        PlayerIntent::default(),
        SimPosition::new(position),
        SimRotation::new(rotation),
        Velocity(velocity),
        Hull(hull),
        cargo,
        MiningRig::default(),
        stats,
    ));
}

fn apply_local_intent(
    local: Res<LocalIntent>,
    mut ships: Query<&mut PlayerIntent, With<LocalShip>>,
) {
    for mut intent in &mut ships {
        *intent = local.0;
    }
}
