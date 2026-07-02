//! Server-side save/load of world and player state to a local RON file.
//!
//! The save schema (`SaveGame`) is a plain serde struct, decoupled from
//! entity ids: asteroids are saved by value, planets by config index,
//! orbital positions implicitly via the sim clock, and each player's ship
//! by pilot name. Offline players stay in the [`PlayerRoster`] and get their
//! ship back when they reconnect.

use std::collections::HashMap;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::components::{
    Asteroid, Hull, Planet, PlayerName, PlayerShip, ResourceDeposit, ShipStats, SimClock,
    SimPosition, SimRotation, Spin, Velocity,
};
use crate::logic::cargo::Cargo;
use crate::resource_types::ResourceType;

pub const SAVE_PATH: &str = "save.ron";
const SAVE_VERSION: u32 = 2;
const AUTOSAVE_INTERVAL_SECS: f32 = 30.0;

pub struct PersistencePlugin;

impl Plugin for PersistencePlugin {
    fn build(&self, app: &mut App) {
        // Loading happens before Startup systems run, so world spawn code
        // can consume the pending state.
        let save = load_save(SAVE_PATH);
        let roster = PlayerRoster(
            save.as_ref()
                .map(|save| {
                    save.players
                        .iter()
                        .map(|player| (player.name.clone(), player.ship.clone()))
                        .collect()
                })
                .unwrap_or_default(),
        );
        app.insert_resource(PendingLoad(save))
            .insert_resource(roster)
            .insert_resource(AutosaveTimer(Timer::from_seconds(
                AUTOSAVE_INTERVAL_SECS,
                TimerMode::Repeating,
            )))
            .add_systems(PostStartup, clear_pending_load)
            .add_systems(Last, save_when_triggered);
    }
}

// ---------------------------------------------------------------------------
// Save schema
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveGame {
    pub version: u32,
    pub sim_elapsed: f64,
    pub players: Vec<PlayerSave>,
    pub asteroids: Vec<AsteroidSave>,
    pub planet_deposits: Vec<PlanetDepositSave>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerSave {
    pub name: String,
    pub ship: ShipSave,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShipSave {
    pub position: Vec2,
    pub rotation: f32,
    pub velocity: Vec2,
    pub hull: f32,
    pub stats: ShipStats,
    pub cargo: Cargo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsteroidSave {
    pub position: Vec2,
    pub rotation: f32,
    pub size: f32,
    pub spin: f32,
    pub kind: ResourceType,
    pub amount: f32,
    pub max_amount: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanetDepositSave {
    pub config_index: usize,
    pub amount: f32,
}

/// Present between app construction and `PostStartup`; spawn systems take
/// their initial state from here instead of config when a save exists.
#[derive(Resource, Debug)]
pub struct PendingLoad(pub Option<SaveGame>);

/// Ship state for every player the server has ever seen, keyed by pilot
/// name. Connected players' entries are refreshed on save and disconnect.
#[derive(Resource, Debug, Default)]
pub struct PlayerRoster(pub HashMap<String, ShipSave>);

#[derive(Resource)]
struct AutosaveTimer(Timer);

/// Capture a live ship's persistent state (used on save and on disconnect).
pub fn capture_ship(
    pos: &SimPosition,
    rot: &SimRotation,
    vel: &Velocity,
    hull: &Hull,
    stats: &ShipStats,
    cargo: &Cargo,
) -> ShipSave {
    ShipSave {
        position: pos.current,
        rotation: rot.current,
        velocity: vel.0,
        hull: hull.0,
        stats: stats.clone(),
        cargo: cargo.clone(),
    }
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

fn load_save(path: &str) -> Option<SaveGame> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
        Err(err) => {
            warn!("could not read save file {path}: {err}");
            return None;
        }
    };
    match ron::from_str::<SaveGame>(&text) {
        Ok(save) if save.version == SAVE_VERSION => {
            info!("loaded save from {path}");
            Some(save)
        }
        Ok(save) => {
            warn!(
                "save file {path} has version {} (expected {SAVE_VERSION}); starting fresh",
                save.version
            );
            None
        }
        Err(err) => {
            warn!("save file {path} is corrupt: {err}; starting fresh");
            None
        }
    }
}

fn clear_pending_load(mut commands: Commands) {
    commands.remove_resource::<PendingLoad>();
}

// ---------------------------------------------------------------------------
// Saving
// ---------------------------------------------------------------------------

/// Save every autosave interval and on app exit.
#[allow(clippy::too_many_arguments)]
fn save_when_triggered(
    time: Res<Time>,
    mut timer: ResMut<AutosaveTimer>,
    mut exit_messages: MessageReader<AppExit>,
    clock: Res<SimClock>,
    mut roster: ResMut<PlayerRoster>,
    ships: Query<
        (
            &PlayerName,
            &SimPosition,
            &SimRotation,
            &Velocity,
            &Hull,
            &ShipStats,
            &Cargo,
        ),
        With<PlayerShip>,
    >,
    asteroids: Query<(&SimPosition, &SimRotation, &Asteroid, &Spin, &ResourceDeposit)>,
    planets: Query<(&Planet, &ResourceDeposit)>,
) {
    let auto = timer.0.tick(time.delta()).just_finished();
    let exiting = exit_messages.read().next().is_some();
    if !(auto || exiting) {
        return;
    }

    // Refresh the roster from live ships; offline entries persist as-is.
    for (name, pos, rot, vel, hull, stats, cargo) in &ships {
        roster
            .0
            .insert(name.0.clone(), capture_ship(pos, rot, vel, hull, stats, cargo));
    }

    let save = SaveGame {
        version: SAVE_VERSION,
        sim_elapsed: clock.elapsed,
        players: roster
            .0
            .iter()
            .map(|(name, ship)| PlayerSave {
                name: name.clone(),
                ship: ship.clone(),
            })
            .collect(),
        asteroids: asteroids
            .iter()
            .map(|(pos, rot, asteroid, spin, deposit)| AsteroidSave {
                position: pos.current,
                rotation: rot.current,
                size: asteroid.size,
                spin: spin.0,
                kind: deposit.kind,
                amount: deposit.amount,
                max_amount: deposit.max_amount,
            })
            .collect(),
        planet_deposits: planets
            .iter()
            .map(|(planet, deposit)| PlanetDepositSave {
                config_index: planet.config_index,
                amount: deposit.amount,
            })
            .collect(),
    };

    match write_save(&save, SAVE_PATH) {
        Ok(()) => {
            let reason = if exiting { "exit" } else { "auto" };
            info!("game saved to {SAVE_PATH} ({reason})");
        }
        Err(err) => warn!("failed to save game: {err}"),
    }
}

fn write_save(save: &SaveGame, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let text = ron::ser::to_string_pretty(save, ron::ser::PrettyConfig::default())?;
    std::fs::write(path, text)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_save() -> SaveGame {
        let mut cargo = Cargo::new(60);
        cargo.add(ResourceType::Iron, 12);
        cargo.add(ResourceType::Crystal, 3);
        SaveGame {
            version: SAVE_VERSION,
            sim_elapsed: 1234.567,
            players: vec![PlayerSave {
                name: "ada".into(),
                ship: ShipSave {
                    position: Vec2::new(-321.5, 908.25),
                    rotation: 1.25,
                    velocity: Vec2::new(10.0, -4.5),
                    hull: 87.5,
                    stats: crate::config::GameConfig::default().ship.stats,
                    cargo,
                },
            }],
            asteroids: vec![AsteroidSave {
                position: Vec2::new(2800.0, -150.0),
                rotation: 0.4,
                size: 22.0,
                spin: -0.3,
                kind: ResourceType::Ice,
                amount: 17.25,
                max_amount: 40.0,
            }],
            planet_deposits: vec![PlanetDepositSave {
                config_index: 2,
                amount: 655.0,
            }],
        }
    }

    /// The full save schema must survive a RON round-trip unchanged.
    #[test]
    fn save_round_trips_through_ron() {
        let save = sample_save();
        let text = ron::ser::to_string_pretty(&save, ron::ser::PrettyConfig::default()).unwrap();
        let loaded: SaveGame = ron::from_str(&text).unwrap();

        assert_eq!(loaded.version, save.version);
        assert_eq!(loaded.sim_elapsed, save.sim_elapsed);
        assert_eq!(loaded.players.len(), 1);
        assert_eq!(loaded.players[0].name, "ada");
        assert_eq!(loaded.players[0].ship.position, save.players[0].ship.position);
        assert_eq!(loaded.players[0].ship.cargo, save.players[0].ship.cargo);
        assert_eq!(loaded.asteroids.len(), 1);
        assert_eq!(loaded.asteroids[0].kind, ResourceType::Ice);
        assert_eq!(loaded.planet_deposits[0].config_index, 2);
    }

    #[test]
    fn corrupt_or_missing_files_are_rejected_gracefully() {
        assert!(load_save("/nonexistent/save.ron").is_none());

        let dir = std::env::temp_dir().join("space_game_test_saves");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("corrupt.ron");
        std::fs::write(&path, "not ron at all {{{").unwrap();
        assert!(load_save(path.to_str().unwrap()).is_none());

        // Wrong version (e.g. a v1 single-player save) is also rejected.
        let mut old = sample_save();
        old.version = SAVE_VERSION - 1;
        let path = dir.join("wrong_version.ron");
        std::fs::write(
            &path,
            ron::ser::to_string_pretty(&old, ron::ser::PrettyConfig::default()).unwrap(),
        )
        .unwrap();
        assert!(load_save(path.to_str().unwrap()).is_none());
    }
}
