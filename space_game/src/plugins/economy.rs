//! Server-side economy: docked trading, ship upgrades, and the slow
//! replenishment of asteroid belts that keeps the universe worth mining.

use bevy::prelude::*;
use rand::Rng;

use crate::components::{
    Asteroid, BodyRadius, Credits, Hull, Planet, PlayerName, PlayerShip, ShipStats, SimPosition,
    SimSet,
};
use crate::config::GameConfig;
use crate::logic::cargo::Cargo;
use crate::logic::economy::{effective_stats, upgrade_cost, Upgrades};
use crate::plugins::world::{spawn_asteroid, AsteroidInit, NetIdAllocator};
use crate::protocol::{AsteroidNetInit, PlayerAction};

pub struct EconomyPlugin;

impl Plugin for EconomyPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ActionRequest>()
            .add_message::<EconomyNotice>()
            .add_message::<AsteroidSpawned>()
            .init_resource::<BeltRespawnTimers>()
            .add_systems(
                FixedUpdate,
                (handle_actions, respawn_belt_asteroids).in_set(SimSet::Mining),
            );
    }
}

/// A validated-later request from a client (written by the network layer).
#[derive(Message, Debug, Clone, Copy)]
pub struct ActionRequest {
    pub ship: Entity,
    pub action: PlayerAction,
}

/// Human-readable outcome the network layer broadcasts as a Notice.
#[derive(Message, Debug, Clone)]
pub struct EconomyNotice(pub String);

/// A belt grew a new rock; the network layer replicates it.
#[derive(Message, Debug, Clone)]
pub struct AsteroidSpawned(pub AsteroidNetInit);

/// Which planet (if any) a ship is docked at: within docking range of the
/// surface. Shared rule — the client uses the same check for its dock UI.
pub fn docked_planet<'a>(
    ship_pos: Vec2,
    docking_range: f32,
    planets: impl Iterator<Item = (&'a Planet, &'a SimPosition, &'a BodyRadius)>,
) -> Option<&'a Planet> {
    planets
        .filter(|(_, pos, radius)| ship_pos.distance(pos.current) <= radius.0 + docking_range)
        .map(|(planet, ..)| planet)
        .next()
}

#[allow(clippy::too_many_arguments)]
fn handle_actions(
    config: Res<GameConfig>,
    mut requests: MessageReader<ActionRequest>,
    mut ships: Query<
        (
            &PlayerName,
            &SimPosition,
            &mut Cargo,
            &mut Credits,
            &mut Upgrades,
            &mut ShipStats,
            &mut Hull,
        ),
        With<PlayerShip>,
    >,
    planets: Query<(&Planet, &SimPosition, &BodyRadius)>,
    mut notices: MessageWriter<EconomyNotice>,
) {
    for request in requests.read() {
        let Ok((name, pos, mut cargo, mut credits, mut upgrades, mut stats, mut hull)) =
            ships.get_mut(request.ship)
        else {
            continue;
        };

        // Every action requires being docked; the server re-checks.
        let Some(planet) = docked_planet(pos.current, config.ship.docking_range, planets.iter())
        else {
            continue;
        };
        let Some(planet_cfg) = config
            .systems
            .get(planet.system_index)
            .and_then(|system| system.planets.get(planet.config_index))
        else {
            continue;
        };

        match request.action {
            PlayerAction::Sell(kind) => {
                let Some(entry) = planet_cfg.market.iter().find(|entry| entry.kind == kind) else {
                    continue;
                };
                let held = cargo.amount(kind);
                if held == 0 {
                    continue;
                }
                let removed = cargo.remove(kind, held);
                let earned = entry.price * removed as u64;
                credits.0 += earned;
                notices.write(EconomyNotice(format!(
                    "{} sold {} {} at {} (+{} cr)",
                    name.0,
                    removed,
                    kind.name(),
                    planet_cfg.name,
                    earned
                )));
            }
            PlayerAction::BuyUpgrade(kind) => {
                let tier = upgrades.tier(kind);
                let Some(cost) = upgrade_cost(kind, tier) else {
                    continue; // maxed out
                };
                if credits.0 < cost {
                    continue;
                }
                credits.0 -= cost;
                *upgrades.tier_mut(kind) += 1;

                // Stats are always derived from base config + tiers.
                let old_max_hull = stats.max_hull;
                *stats = effective_stats(&config.ship.stats, &upgrades);
                // Hull plating adds the new capacity as intact armor.
                hull.0 += (stats.max_hull - old_max_hull).max(0.0);

                notices.write(EconomyNotice(format!(
                    "{} bought {} tier {} at {} (-{} cr)",
                    name.0,
                    kind.name(),
                    tier + 1,
                    planet_cfg.name,
                    cost
                )));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Belt ecology
// ---------------------------------------------------------------------------

/// Per-(system, belt) countdowns to the next respawn attempt.
#[derive(Resource, Default)]
struct BeltRespawnTimers(std::collections::HashMap<(usize, usize), f32>);

/// Mined-out belts slowly grow new rocks back toward their configured
/// population, one at a time.
#[allow(clippy::too_many_arguments)]
fn respawn_belt_asteroids(
    mut commands: Commands,
    time: Res<Time>,
    config: Res<GameConfig>,
    mut timers: ResMut<BeltRespawnTimers>,
    mut net_ids: ResMut<NetIdAllocator>,
    asteroids: Query<&SimPosition, With<Asteroid>>,
    roots: Query<(Entity, &crate::components::SolarSystem)>,
    mut spawned: MessageWriter<AsteroidSpawned>,
) {
    let dt = time.delta_secs();
    let mut rng = rand::thread_rng();

    for (system_index, system_cfg) in config.systems.iter().enumerate() {
        let center = Vec2::new(system_cfg.center.0, system_cfg.center.1);
        for (belt_index, belt) in system_cfg.belts.iter().enumerate() {
            if belt.respawn_secs <= 0.0 {
                continue;
            }
            let timer = timers
                .0
                .entry((system_index, belt_index))
                .or_insert(belt.respawn_secs);
            *timer -= dt;
            if *timer > 0.0 {
                continue;
            }
            *timer = belt.respawn_secs;

            // Population census by position: cheap and stateless.
            let population = asteroids
                .iter()
                .filter(|pos| {
                    let r = pos.current.distance(center);
                    r >= belt.inner_radius && r <= belt.outer_radius
                })
                .count() as u32;
            if population >= belt.asteroid_count {
                continue;
            }

            let angle = rng.gen_range(0.0..std::f32::consts::TAU);
            let radius = rng.gen_range(belt.inner_radius..belt.outer_radius);
            let amount = rng.gen_range(belt.min_amount..belt.max_amount);
            let kind = belt.resources[rng.gen_range(0..belt.resources.len())];
            let init = AsteroidNetInit {
                net_id: net_ids.allocate(),
                position: center + Vec2::from_angle(angle) * radius,
                rotation: 0.0,
                size: rng.gen_range(belt.min_size..belt.max_size),
                spin: rng.gen_range(-0.9..0.9),
                kind,
                amount,
                max_amount: amount,
            };

            let Some((root, _)) = roots.iter().find(|(_, root)| root.name == system_cfg.name)
            else {
                continue;
            };
            spawn_asteroid(
                &mut commands,
                root,
                AsteroidInit {
                    net_id: init.net_id,
                    position: init.position,
                    rotation: init.rotation,
                    size: init.size,
                    spin: init.spin,
                    kind: init.kind,
                    amount: init.amount,
                    max_amount: init.max_amount,
                },
            );
            spawned.write(AsteroidSpawned(init));
        }
    }
}
