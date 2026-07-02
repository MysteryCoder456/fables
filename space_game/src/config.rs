//! Data-driven game configuration.
//!
//! All gameplay numbers (ship stats, celestial bodies, asteroid belts,
//! resource richness) live in `assets/config/game.ron`. The structs below are
//! the schema; `GameConfig::default()` is the built-in fallback used when the
//! file is missing or fails to parse, so the binary always starts.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::components::ShipStats;
use crate::resource_types::ResourceType;

pub const CONFIG_PATH: &str = "assets/config/game.ron";

#[derive(Resource, Debug, Clone, Serialize, Deserialize)]
pub struct GameConfig {
    pub ship: ShipConfig,
    pub system: SolarSystemConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShipConfig {
    pub spawn_position: (f32, f32),
    pub stats: ShipStats,
}

/// One solar system. The world is structured so more of these (plus jump
/// gates) can be added later without a rewrite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolarSystemConfig {
    pub name: String,
    pub star: StarConfig,
    pub planets: Vec<PlanetConfig>,
    pub belts: Vec<BeltConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarConfig {
    pub name: String,
    pub radius: f32,
    pub color: (f32, f32, f32),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanetConfig {
    pub name: String,
    pub radius: f32,
    pub color: (f32, f32, f32),
    pub orbit_radius: f32,
    /// Seconds for one full revolution around the star.
    pub orbit_period: f32,
    /// Initial orbital angle in radians at sim time 0.
    pub orbit_phase: f32,
    /// Optional mineable deposit (planets regenerate slowly).
    pub deposit: Option<DepositConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepositConfig {
    pub kind: ResourceType,
    pub max_amount: f32,
    /// Units per second regenerated (0 for asteroids: they deplete for good).
    pub regen_per_sec: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeltConfig {
    pub inner_radius: f32,
    pub outer_radius: f32,
    pub asteroid_count: u32,
    /// RNG seed so the belt layout is reproducible (and can be regenerated
    /// identically on a future server).
    pub seed: u64,
    /// Resource kinds that spawn in this belt, chosen uniformly per asteroid.
    pub resources: Vec<ResourceType>,
    pub min_amount: f32,
    pub max_amount: f32,
    pub min_size: f32,
    pub max_size: f32,
}

impl GameConfig {
    /// Load from `path`, falling back to built-in defaults on any error.
    pub fn load_or_default(path: &str) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => match ron::from_str(&text) {
                Ok(config) => {
                    info!("loaded game config from {path}");
                    config
                }
                Err(err) => {
                    warn!("failed to parse {path}: {err}; using built-in defaults");
                    Self::default()
                }
            },
            Err(err) => {
                warn!("could not read {path}: {err}; using built-in defaults");
                Self::default()
            }
        }
    }
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            ship: ShipConfig {
                spawn_position: (0.0, -1400.0),
                stats: ShipStats {
                    max_hull: 100.0,
                    cargo_capacity: 60,
                    mining_power: 6.0,
                    mining_range: 160.0,
                    thrust_accel: 300.0,
                    turn_speed: 3.2,
                    max_speed: 500.0,
                    drag: 0.4,
                },
            },
            system: SolarSystemConfig {
                name: "Helios".into(),
                star: StarConfig {
                    name: "Helios".into(),
                    radius: 300.0,
                    color: (1.0, 0.85, 0.45),
                },
                planets: vec![
                    PlanetConfig {
                        name: "Cinder".into(),
                        radius: 55.0,
                        color: (0.85, 0.45, 0.3),
                        orbit_radius: 1200.0,
                        orbit_period: 180.0,
                        orbit_phase: 0.4,
                        deposit: Some(DepositConfig {
                            kind: ResourceType::Iron,
                            max_amount: 600.0,
                            regen_per_sec: 0.5,
                        }),
                    },
                    PlanetConfig {
                        name: "Verdis".into(),
                        radius: 80.0,
                        color: (0.35, 0.7, 0.45),
                        orbit_radius: 2100.0,
                        orbit_period: 320.0,
                        orbit_phase: 2.4,
                        deposit: Some(DepositConfig {
                            kind: ResourceType::Gas,
                            max_amount: 500.0,
                            regen_per_sec: 0.4,
                        }),
                    },
                    PlanetConfig {
                        name: "Glacius".into(),
                        radius: 65.0,
                        color: (0.6, 0.8, 0.95),
                        orbit_radius: 3400.0,
                        orbit_period: 540.0,
                        orbit_phase: 4.2,
                        deposit: Some(DepositConfig {
                            kind: ResourceType::Ice,
                            max_amount: 700.0,
                            regen_per_sec: 0.6,
                        }),
                    },
                    PlanetConfig {
                        name: "Umbra".into(),
                        radius: 95.0,
                        color: (0.5, 0.4, 0.65),
                        orbit_radius: 5200.0,
                        orbit_period: 900.0,
                        orbit_phase: 1.1,
                        deposit: Some(DepositConfig {
                            kind: ResourceType::Crystal,
                            max_amount: 400.0,
                            regen_per_sec: 0.25,
                        }),
                    },
                    PlanetConfig {
                        name: "Ferrum".into(),
                        radius: 45.0,
                        color: (0.7, 0.6, 0.5),
                        orbit_radius: 6800.0,
                        orbit_period: 1400.0,
                        orbit_phase: 5.5,
                        deposit: Some(DepositConfig {
                            kind: ResourceType::Iron,
                            max_amount: 900.0,
                            regen_per_sec: 0.8,
                        }),
                    },
                ],
                belts: vec![
                    BeltConfig {
                        inner_radius: 2600.0,
                        outer_radius: 3000.0,
                        asteroid_count: 90,
                        seed: 7,
                        resources: vec![ResourceType::Iron, ResourceType::Ice],
                        min_amount: 15.0,
                        max_amount: 50.0,
                        min_size: 14.0,
                        max_size: 34.0,
                    },
                    BeltConfig {
                        inner_radius: 4200.0,
                        outer_radius: 4800.0,
                        asteroid_count: 130,
                        seed: 99,
                        resources: vec![
                            ResourceType::Iron,
                            ResourceType::Crystal,
                            ResourceType::Gas,
                        ],
                        min_amount: 25.0,
                        max_amount: 80.0,
                        min_size: 16.0,
                        max_size: 40.0,
                    },
                ],
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped RON file must stay in sync with the schema.
    #[test]
    fn shipped_config_parses() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/config/game.ron");
        let text = std::fs::read_to_string(path).expect("config file missing");
        let config: GameConfig = ron::from_str(&text).expect("config file invalid");
        assert!(!config.system.planets.is_empty());
        assert!(config.ship.stats.cargo_capacity > 0);
    }

    #[test]
    fn default_config_is_sane() {
        let config = GameConfig::default();
        let planet_count = config.system.planets.len();
        assert!((4..=6).contains(&planet_count));
        assert!(config.system.belts.len() >= 2);
        for belt in &config.system.belts {
            assert!(belt.inner_radius < belt.outer_radius);
            assert!(belt.min_amount <= belt.max_amount);
            assert!(!belt.resources.is_empty());
        }
    }
}
