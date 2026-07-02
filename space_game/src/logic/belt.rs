//! Deterministic procedural generation of asteroid belts.

use bevy::math::Vec2;
use rand::{Rng, SeedableRng};

use crate::config::BeltConfig;
use crate::resource_types::ResourceType;

/// Everything needed to spawn one asteroid, engine-agnostic.
#[derive(Debug, Clone, PartialEq)]
pub struct AsteroidSpawn {
    pub position: Vec2,
    pub size: f32,
    /// Visual + sim rotation speed, radians/second (can be negative).
    pub spin: f32,
    pub kind: ResourceType,
    pub amount: f32,
}

/// Generate the asteroids for one belt. Deterministic for a given config:
/// the same seed always yields the same field, so a server and client can
/// both derive it from config alone.
pub fn generate_belt(config: &BeltConfig) -> Vec<AsteroidSpawn> {
    let mut rng = rand::rngs::StdRng::seed_from_u64(config.seed);
    (0..config.asteroid_count)
        .map(|_| {
            let angle = rng.gen_range(0.0..std::f32::consts::TAU);
            let radius = rng.gen_range(config.inner_radius..config.outer_radius);
            let kind = config.resources[rng.gen_range(0..config.resources.len())];
            AsteroidSpawn {
                position: Vec2::from_angle(angle) * radius,
                size: rng.gen_range(config.min_size..config.max_size),
                spin: rng.gen_range(-0.9..0.9_f32),
                kind,
                amount: rng.gen_range(config.min_amount..config.max_amount),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> BeltConfig {
        BeltConfig {
            inner_radius: 1000.0,
            outer_radius: 1200.0,
            asteroid_count: 50,
            seed: 1234,
            resources: vec![ResourceType::Iron, ResourceType::Ice],
            min_amount: 10.0,
            max_amount: 30.0,
            min_size: 8.0,
            max_size: 20.0,
            respawn_secs: 0.0,
        }
    }

    #[test]
    fn generates_requested_count() {
        assert_eq!(generate_belt(&test_config()).len(), 50);
    }

    #[test]
    fn asteroids_stay_inside_the_ring() {
        for asteroid in generate_belt(&test_config()) {
            let r = asteroid.position.length();
            assert!(
                (1000.0..1200.0).contains(&r),
                "asteroid at radius {r} outside belt"
            );
        }
    }

    #[test]
    fn values_respect_config_bounds() {
        let config = test_config();
        for asteroid in generate_belt(&config) {
            assert!((config.min_amount..config.max_amount).contains(&asteroid.amount));
            assert!((config.min_size..config.max_size).contains(&asteroid.size));
            assert!(config.resources.contains(&asteroid.kind));
        }
    }

    #[test]
    fn same_seed_is_deterministic() {
        assert_eq!(generate_belt(&test_config()), generate_belt(&test_config()));
    }

    #[test]
    fn different_seeds_differ() {
        let mut other = test_config();
        other.seed = 9999;
        assert_ne!(generate_belt(&test_config()), generate_belt(&other));
    }
}
