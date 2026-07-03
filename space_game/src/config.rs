//! Data-driven game configuration.
//!
//! All gameplay numbers (ship stats, physics constants, star systems,
//! markets, asteroid belts, jump gates) live in `assets/config/game.ron`.
//! The structs below are the schema; `GameConfig::default()` is the built-in
//! fallback used when the file is missing or fails to parse, so the binaries
//! always start. Server and client must share the same config file: the
//! server is authoritative for gameplay, the client uses it for cosmetics
//! and world layout.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::components::ShipStats;
use crate::resource_types::ResourceType;

pub const CONFIG_PATH: &str = "assets/config/game.ron";

#[derive(Resource, Debug, Clone, Serialize, Deserialize)]
pub struct GameConfig {
    pub ship: ShipConfig,
    pub physics: PhysicsConfig,
    pub systems: Vec<SolarSystemConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShipConfig {
    /// Where new (and destroyed) pilots appear, world space.
    pub spawn_position: (f32, f32),
    /// Distance from a planet's surface within which trading works.
    pub docking_range: f32,
    pub stats: ShipStats,
}

/// Physics and combat constants.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhysicsConfig {
    /// Bounciness of collisions (0 = dead stop, 1 = perfectly elastic).
    pub restitution: f32,
    /// Impacts slower than this (units/s along the normal) are free.
    pub min_impact_for_damage: f32,
    /// Hull damage per unit/s of impact speed above the threshold.
    pub impact_damage_scale: f32,
    /// Collision radius of every ship.
    pub ship_radius: f32,
    /// Damage per second while inside a star's corona.
    pub star_burn_dps: f32,
    /// Corona extends to `star radius * this`.
    pub star_burn_radius_factor: f32,
    /// Blaster muzzle speed (added to the ship's velocity).
    pub projectile_speed: f32,
    /// Bolt lifetime in seconds.
    pub projectile_ttl: f32,
    pub projectile_radius: f32,
    /// Bolt damage before Blaster upgrades.
    pub projectile_base_damage: f32,
    /// Seconds between shots.
    pub fire_cooldown: f32,
    /// Invulnerability window after respawning.
    pub respawn_shield_secs: f32,
}

/// One solar system: a star, its planets, belts and jump gates, all offset
/// by `center` in world space.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolarSystemConfig {
    pub name: String,
    pub center: (f32, f32),
    pub star: StarConfig,
    pub planets: Vec<PlanetConfig>,
    pub belts: Vec<BeltConfig>,
    pub gates: Vec<GateConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarConfig {
    pub name: String,
    pub radius: f32,
    pub color: (f32, f32, f32),
    /// Standard gravitational parameter G·M (units³/s²).
    pub gravity_mu: f32,
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
    /// Standard gravitational parameter G·M (units³/s²).
    pub gravity_mu: f32,
    /// Optional mineable deposit (planets regenerate slowly).
    pub deposit: Option<DepositConfig>,
    /// What this planet pays per unit of each resource (credits).
    pub market: Vec<MarketEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketEntry {
    pub kind: ResourceType,
    pub price: u64,
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
    /// RNG seed so the belt layout is reproducible.
    pub seed: u64,
    /// Resource kinds that spawn in this belt, chosen uniformly per asteroid.
    pub resources: Vec<ResourceType>,
    pub min_amount: f32,
    pub max_amount: f32,
    pub min_size: f32,
    pub max_size: f32,
    /// Seconds between respawn attempts while under population (0 = never).
    pub respawn_secs: f32,
}

/// A jump gate; positions are relative to the system center.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateConfig {
    pub name: String,
    pub position: (f32, f32),
    pub radius: f32,
    pub to_system: usize,
    pub to_gate: usize,
}

impl GameConfig {
    /// Where and how new (and destroyed) pilots appear: position, rotation
    /// and velocity of a stable circular orbit around the home star, facing
    /// prograde. Spawning *in orbit* means an idle new player circles the
    /// star gracefully instead of plummeting into it, and thrusting forward
    /// raises the orbit — away from the sun, never into it.
    pub fn spawn_kinematics(&self) -> (Vec2, f32, Vec2) {
        let position = Vec2::new(self.ship.spawn_position.0, self.ship.spawn_position.1);
        let Some(home) = self.systems.first() else {
            return (position, std::f32::consts::FRAC_PI_2, Vec2::ZERO);
        };
        let center = Vec2::new(home.center.0, home.center.1);
        let radial = position - center;
        let radius = radial.length();
        if radius < f32::EPSILON {
            return (position, std::f32::consts::FRAC_PI_2, Vec2::ZERO);
        }
        let speed = crate::logic::gravity::circular_orbit_speed(home.star.gravity_mu, radius);
        // Counter-clockwise orbit: velocity is the CCW perpendicular of the
        // outward radial, matching the planets' direction of travel.
        let velocity = Vec2::new(-radial.y, radial.x) / radius * speed;
        let rotation = velocity.to_angle();
        (position, rotation, velocity)
    }

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

fn market(iron: u64, ice: u64, crystal: u64, gas: u64) -> Vec<MarketEntry> {
    vec![
        MarketEntry {
            kind: ResourceType::Iron,
            price: iron,
        },
        MarketEntry {
            kind: ResourceType::Ice,
            price: ice,
        },
        MarketEntry {
            kind: ResourceType::Crystal,
            price: crystal,
        },
        MarketEntry {
            kind: ResourceType::Gas,
            price: gas,
        },
    ]
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            ship: ShipConfig {
                // New pilots start on the inner edge of the first asteroid
                // belt, in a stable orbit, with mining targets in view.
                spawn_position: (0.0, -2600.0),
                docking_range: 160.0,
                stats: ShipStats {
                    max_hull: 100.0,
                    cargo_capacity: 60,
                    mining_power: 6.0,
                    mining_range: 160.0,
                    thrust_accel: 320.0,
                    turn_speed: 3.4,
                    max_speed: 700.0,
                    // Nearly no drag: spawn orbits stay stable for minutes
                    // and gravity assists feel real. Brake for control.
                    drag: 0.02,
                },
            },
            physics: PhysicsConfig {
                restitution: 0.45,
                min_impact_for_damage: 90.0,
                impact_damage_scale: 0.18,
                ship_radius: 14.0,
                star_burn_dps: 25.0,
                star_burn_radius_factor: 1.2,
                projectile_speed: 650.0,
                projectile_ttl: 2.2,
                projectile_radius: 3.0,
                projectile_base_damage: 10.0,
                fire_cooldown: 0.22,
                respawn_shield_secs: 3.0,
            },
            systems: vec![helios(), cryon()],
        }
    }
}

/// System 0: the industrial home system.
fn helios() -> SolarSystemConfig {
    SolarSystemConfig {
        name: "Helios".into(),
        center: (0.0, 0.0),
        star: StarConfig {
            name: "Helios".into(),
            radius: 300.0,
            color: (1.0, 0.85, 0.45),
            // Strong enough to bend trajectories and hold orbits, weak
            // enough that full thrust out-pulls it 20:1 at the spawn belt.
            gravity_mu: 3.5e7,
        },
        planets: vec![
            PlanetConfig {
                name: "Cinder".into(),
                radius: 55.0,
                color: (0.85, 0.45, 0.3),
                orbit_radius: 1200.0,
                orbit_period: 180.0,
                orbit_phase: 0.4,
                gravity_mu: 7.5e4,
                deposit: Some(DepositConfig {
                    kind: ResourceType::Iron,
                    max_amount: 600.0,
                    regen_per_sec: 0.5,
                }),
                market: market(2, 5, 9, 6),
            },
            PlanetConfig {
                name: "Verdis".into(),
                radius: 80.0,
                color: (0.35, 0.7, 0.45),
                orbit_radius: 2100.0,
                orbit_period: 320.0,
                orbit_phase: 2.4,
                gravity_mu: 1.6e5,
                deposit: Some(DepositConfig {
                    kind: ResourceType::Gas,
                    max_amount: 500.0,
                    regen_per_sec: 0.4,
                }),
                market: market(4, 5, 8, 3),
            },
            PlanetConfig {
                name: "Glacius".into(),
                radius: 65.0,
                color: (0.6, 0.8, 0.95),
                orbit_radius: 3400.0,
                orbit_period: 540.0,
                orbit_phase: 4.2,
                gravity_mu: 1.05e5,
                deposit: Some(DepositConfig {
                    kind: ResourceType::Ice,
                    max_amount: 700.0,
                    regen_per_sec: 0.6,
                }),
                market: market(4, 2, 9, 7),
            },
            PlanetConfig {
                name: "Umbra".into(),
                radius: 95.0,
                color: (0.5, 0.4, 0.65),
                orbit_radius: 5200.0,
                orbit_period: 900.0,
                orbit_phase: 1.1,
                gravity_mu: 2.25e5,
                deposit: Some(DepositConfig {
                    kind: ResourceType::Crystal,
                    max_amount: 400.0,
                    regen_per_sec: 0.25,
                }),
                market: market(5, 6, 5, 7),
            },
            PlanetConfig {
                name: "Ferrum".into(),
                radius: 45.0,
                color: (0.7, 0.6, 0.5),
                orbit_radius: 6800.0,
                orbit_period: 1400.0,
                orbit_phase: 5.5,
                gravity_mu: 5.0e4,
                deposit: Some(DepositConfig {
                    kind: ResourceType::Iron,
                    max_amount: 900.0,
                    regen_per_sec: 0.8,
                }),
                market: market(3, 4, 10, 8),
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
                respawn_secs: 45.0,
            },
            BeltConfig {
                inner_radius: 4200.0,
                outer_radius: 4800.0,
                asteroid_count: 130,
                seed: 99,
                resources: vec![ResourceType::Iron, ResourceType::Crystal, ResourceType::Gas],
                min_amount: 25.0,
                max_amount: 80.0,
                min_size: 16.0,
                max_size: 40.0,
                respawn_secs: 60.0,
            },
        ],
        gates: vec![GateConfig {
            name: "Cryon Gate".into(),
            position: (0.0, 5800.0),
            radius: 90.0,
            to_system: 1,
            to_gate: 0,
        }],
    }
}

/// System 1: the icy frontier — richer deposits, better prices, longer haul.
fn cryon() -> SolarSystemConfig {
    SolarSystemConfig {
        name: "Cryon".into(),
        center: (80_000.0, 0.0),
        star: StarConfig {
            name: "Cryon".into(),
            radius: 220.0,
            color: (0.65, 0.78, 1.0),
            gravity_mu: 2.2e7,
        },
        planets: vec![
            PlanetConfig {
                name: "Boreas".into(),
                radius: 70.0,
                color: (0.7, 0.85, 1.0),
                orbit_radius: 1500.0,
                orbit_period: 260.0,
                orbit_phase: 0.9,
                gravity_mu: 1.2e5,
                deposit: Some(DepositConfig {
                    kind: ResourceType::Ice,
                    max_amount: 800.0,
                    regen_per_sec: 0.7,
                }),
                market: market(7, 2, 11, 9),
            },
            PlanetConfig {
                name: "Halcyon".into(),
                radius: 85.0,
                color: (0.9, 0.75, 0.5),
                orbit_radius: 2800.0,
                orbit_period: 430.0,
                orbit_phase: 3.6,
                gravity_mu: 1.8e5,
                deposit: Some(DepositConfig {
                    kind: ResourceType::Gas,
                    max_amount: 650.0,
                    regen_per_sec: 0.5,
                }),
                market: market(6, 6, 12, 4),
            },
            PlanetConfig {
                name: "Vesper".into(),
                radius: 50.0,
                color: (0.8, 0.6, 0.95),
                orbit_radius: 4200.0,
                orbit_period: 700.0,
                orbit_phase: 5.1,
                gravity_mu: 6.0e4,
                deposit: Some(DepositConfig {
                    kind: ResourceType::Crystal,
                    max_amount: 500.0,
                    regen_per_sec: 0.3,
                }),
                market: market(8, 7, 6, 8),
            },
        ],
        belts: vec![
            BeltConfig {
                inner_radius: 1900.0,
                outer_radius: 2200.0,
                asteroid_count: 70,
                seed: 88,
                resources: vec![ResourceType::Ice],
                min_amount: 25.0,
                max_amount: 70.0,
                min_size: 14.0,
                max_size: 30.0,
                respawn_secs: 50.0,
            },
            BeltConfig {
                inner_radius: 3300.0,
                outer_radius: 3800.0,
                asteroid_count: 120,
                seed: 431,
                resources: vec![ResourceType::Crystal, ResourceType::Ice, ResourceType::Iron],
                min_amount: 30.0,
                max_amount: 90.0,
                min_size: 15.0,
                max_size: 38.0,
                respawn_secs: 40.0,
            },
        ],
        gates: vec![GateConfig {
            name: "Helios Gate".into(),
            position: (0.0, -4600.0),
            radius: 90.0,
            to_system: 0,
            to_gate: 0,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dev utility, not a test: regenerate the shipped config from the
    /// built-in defaults after a schema change. Run with
    /// `cargo test regenerate_shipped_config -- --ignored`.
    #[test]
    #[ignore]
    fn regenerate_shipped_config() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/config/game.ron");
        let header = "\
// Data-driven game configuration (schema: src/config.rs).
// Edit numbers freely — no recompile needed; both server and client read
// this file and must share it. Regenerate from code defaults with:
//   cargo test regenerate_shipped_config -- --ignored
";
        let body =
            ron::ser::to_string_pretty(&GameConfig::default(), ron::ser::PrettyConfig::default())
                .expect("defaults serialize");
        std::fs::write(path, format!("{header}{body}")).expect("write config");
    }

    /// The shipped RON file must stay in sync with the schema.
    #[test]
    fn shipped_config_parses() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/config/game.ron");
        let text = std::fs::read_to_string(path).expect("config file missing");
        let config: GameConfig = ron::from_str(&text).expect("config file invalid");
        assert_eq!(config.systems.len(), 2);
        assert!(config.ship.stats.cargo_capacity > 0);
    }

    #[test]
    fn default_config_is_sane() {
        let config = GameConfig::default();
        assert_eq!(config.systems.len(), 2);
        for system in &config.systems {
            assert!(!system.planets.is_empty());
            assert!(system.star.gravity_mu > 0.0);
            for belt in &system.belts {
                assert!(belt.inner_radius < belt.outer_radius);
                assert!(belt.min_amount <= belt.max_amount);
                assert!(!belt.resources.is_empty());
            }
            for planet in &system.planets {
                assert!(planet.gravity_mu > 0.0);
                assert_eq!(planet.market.len(), 4, "every planet trades everything");
            }
        }
    }

    /// New pilots must start on a genuinely stable circular orbit: the
    /// centripetal acceleration of the spawn velocity has to match the
    /// star's gravity at the spawn radius, and the ship must face prograde.
    #[test]
    fn spawn_orbit_is_stable_and_prograde() {
        let config = GameConfig::default();
        let (position, rotation, velocity) = config.spawn_kinematics();

        let home = &config.systems[0];
        let center = Vec2::new(home.center.0, home.center.1);
        let radial = position - center;
        let radius = radial.length();

        // v²/r == mu/r² (circular orbit condition).
        let centripetal = velocity.length_squared() / radius;
        let gravity = home.star.gravity_mu / (radius * radius);
        assert!(
            (centripetal - gravity).abs() / gravity < 1e-4,
            "spawn velocity is not a circular orbit"
        );

        // Velocity is tangential (no radial component) and CCW.
        assert!(velocity.dot(radial).abs() / (velocity.length() * radius) < 1e-4);
        assert!(radial.perp_dot(velocity) > 0.0, "orbit should be CCW");

        // The nose points along the velocity (prograde), so thrusting
        // forward raises the orbit instead of diving sunward.
        let facing = Vec2::from_angle(rotation);
        assert!(facing.dot(velocity.normalize()) > 0.999);

        // And full thrust dominates local gravity by a wide margin.
        assert!(config.ship.stats.thrust_accel > gravity * 10.0);
    }

    #[test]
    fn gates_are_bidirectional_and_valid() {
        let config = GameConfig::default();
        for (system_index, system) in config.systems.iter().enumerate() {
            for gate in &system.gates {
                let target_system = config
                    .systems
                    .get(gate.to_system)
                    .expect("gate points at a real system");
                let partner = target_system
                    .gates
                    .get(gate.to_gate)
                    .expect("gate points at a real partner gate");
                assert_eq!(
                    partner.to_system, system_index,
                    "partner gate must point back"
                );
            }
        }
    }
}
