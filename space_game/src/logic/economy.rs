//! Upgrades and market math. Pure functions, unit-tested below.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::components::ShipStats;

pub const MAX_UPGRADE_TIER: u8 = 5;

/// The five upgrade tracks a pilot can invest credits into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UpgradeKind {
    Thrusters,
    CargoBay,
    MiningLaser,
    HullPlating,
    Blaster,
}

impl UpgradeKind {
    pub const ALL: [UpgradeKind; 5] = [
        UpgradeKind::Thrusters,
        UpgradeKind::CargoBay,
        UpgradeKind::MiningLaser,
        UpgradeKind::HullPlating,
        UpgradeKind::Blaster,
    ];

    pub fn name(self) -> &'static str {
        match self {
            UpgradeKind::Thrusters => "Thrusters",
            UpgradeKind::CargoBay => "Cargo Bay",
            UpgradeKind::MiningLaser => "Mining Laser",
            UpgradeKind::HullPlating => "Hull Plating",
            UpgradeKind::Blaster => "Blaster",
        }
    }

    /// Base price of tier 1; each further tier doubles.
    pub fn base_cost(self) -> u64 {
        match self {
            UpgradeKind::Thrusters => 120,
            UpgradeKind::CargoBay => 100,
            UpgradeKind::MiningLaser => 140,
            UpgradeKind::HullPlating => 130,
            UpgradeKind::Blaster => 150,
        }
    }
}

/// A pilot's purchased upgrade tiers. Persisted; effective ship stats are
/// always *derived* from base config + these tiers.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Upgrades {
    pub thrusters: u8,
    pub cargo_bay: u8,
    pub mining_laser: u8,
    pub hull_plating: u8,
    pub blaster: u8,
}

impl Upgrades {
    pub fn tier(&self, kind: UpgradeKind) -> u8 {
        match kind {
            UpgradeKind::Thrusters => self.thrusters,
            UpgradeKind::CargoBay => self.cargo_bay,
            UpgradeKind::MiningLaser => self.mining_laser,
            UpgradeKind::HullPlating => self.hull_plating,
            UpgradeKind::Blaster => self.blaster,
        }
    }

    pub fn tier_mut(&mut self, kind: UpgradeKind) -> &mut u8 {
        match kind {
            UpgradeKind::Thrusters => &mut self.thrusters,
            UpgradeKind::CargoBay => &mut self.cargo_bay,
            UpgradeKind::MiningLaser => &mut self.mining_laser,
            UpgradeKind::HullPlating => &mut self.hull_plating,
            UpgradeKind::Blaster => &mut self.blaster,
        }
    }

    /// Total tiers bought — shown as the pilot's "level".
    pub fn total_tiers(&self) -> u32 {
        UpgradeKind::ALL
            .iter()
            .map(|kind| self.tier(*kind) as u32)
            .sum()
    }
}

/// Cost of buying the *next* tier, or `None` at the cap.
pub fn upgrade_cost(kind: UpgradeKind, current_tier: u8) -> Option<u64> {
    if current_tier >= MAX_UPGRADE_TIER {
        return None;
    }
    Some(kind.base_cost() << current_tier) // base * 2^tier
}

/// Derive effective ship stats from base config stats and upgrade tiers.
pub fn effective_stats(base: &ShipStats, upgrades: &Upgrades) -> ShipStats {
    let mut stats = base.clone();
    stats.thrust_accel = base.thrust_accel * (1.0 + 0.20 * upgrades.thrusters as f32);
    stats.max_speed = base.max_speed * (1.0 + 0.08 * upgrades.thrusters as f32);
    stats.cargo_capacity = base.cargo_capacity + 20 * upgrades.cargo_bay as u32;
    stats.mining_power = base.mining_power + 2.0 * upgrades.mining_laser as f32;
    stats.max_hull = base.max_hull + 25.0 * upgrades.hull_plating as f32;
    stats
}

/// Damage of one blaster bolt at the given tier.
pub fn blaster_damage(base_damage: f32, tier: u8) -> f32 {
    base_damage + 4.0 * tier as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GameConfig;

    #[test]
    fn upgrade_costs_double_per_tier() {
        let kind = UpgradeKind::CargoBay;
        assert_eq!(upgrade_cost(kind, 0), Some(100));
        assert_eq!(upgrade_cost(kind, 1), Some(200));
        assert_eq!(upgrade_cost(kind, 4), Some(1600));
        assert_eq!(upgrade_cost(kind, MAX_UPGRADE_TIER), None, "capped");
    }

    #[test]
    fn effective_stats_scale_with_tiers() {
        let base = GameConfig::default().ship.stats;
        let mut upgrades = Upgrades::default();

        let stock = effective_stats(&base, &upgrades);
        assert_eq!(stock.cargo_capacity, base.cargo_capacity);
        assert!((stock.thrust_accel - base.thrust_accel).abs() < 1e-4);

        upgrades.thrusters = 2;
        upgrades.cargo_bay = 3;
        upgrades.hull_plating = 1;
        let tuned = effective_stats(&base, &upgrades);
        assert!((tuned.thrust_accel - base.thrust_accel * 1.4).abs() < 1e-3);
        assert_eq!(tuned.cargo_capacity, base.cargo_capacity + 60);
        assert!((tuned.max_hull - (base.max_hull + 25.0)).abs() < 1e-3);
    }

    #[test]
    fn blaster_damage_scales() {
        assert!((blaster_damage(10.0, 0) - 10.0).abs() < 1e-5);
        assert!((blaster_damage(10.0, 3) - 22.0).abs() < 1e-5);
    }

    #[test]
    fn total_tiers_sums_all_tracks() {
        let upgrades = Upgrades {
            thrusters: 1,
            cargo_bay: 2,
            mining_laser: 0,
            hull_plating: 5,
            blaster: 1,
        };
        assert_eq!(upgrades.total_tiers(), 9);
    }
}
