//! Server-side physics: gravity, blaster fire, projectile flight, collision
//! resolution, star corona burn, and death/respawn.
//!
//! Runs in `SimSet::Movement` (gravity, before thrust integration) and
//! `SimSet::Physics` (everything else, after all movement has settled).

use bevy::prelude::*;

use crate::components::{
    BodyRadius, CentralStar, Credits, GravitySource, Hull, LastDamager, MiningRig, NetId, Orbit,
    PlayerIntent, PlayerName, PlayerShip, Projectile, ShipStats, SimClock, SimPosition,
    SimRotation, SpawnShield, Velocity, WeaponCooldown,
};
use crate::config::GameConfig;
use crate::logic::cargo::Cargo;
use crate::logic::collision::{collide_ships, collide_with_body, impact_damage};
use crate::logic::economy::{blaster_damage, Upgrades};
use crate::logic::gravity::{gravity_accel, GravityBody};
use crate::plugins::world::NetIdAllocator;

use crate::components::SimSet;

pub struct PhysicsPlugin;

impl Plugin for PhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ProjectileImpact>()
            .add_message::<ShipDestroyed>()
            .add_systems(
                FixedUpdate,
                apply_gravity_to_ships
                    .in_set(SimSet::Movement)
                    .before(crate::plugins::player::ship_movement),
            )
            .add_systems(
                FixedUpdate,
                (
                    tick_combat_timers,
                    fire_weapons,
                    integrate_projectiles,
                    collide_projectiles,
                    collide_ships_with_world,
                    star_burn,
                    handle_deaths,
                )
                    .chain()
                    .in_set(SimSet::Physics),
            );
    }
}

/// A bolt (or ship) hit something hard — clients render a spark burst.
#[derive(Message, Debug, Clone, Copy)]
pub struct ProjectileImpact {
    pub position: Vec2,
}

/// A ship's hull reached zero; it has already been respawned.
#[derive(Message, Debug, Clone)]
pub struct ShipDestroyed {
    pub victim: NetId,
    pub victim_name: String,
    pub killer_name: Option<String>,
    pub cargo_lost: u32,
}

/// How long after taking damage a kill still credits the attacker.
const KILL_ATTRIBUTION_SECS: f64 = 8.0;

// ---------------------------------------------------------------------------
// Gravity
// ---------------------------------------------------------------------------

/// Snapshot of all gravity wells this tick (stars are static, planets move).
fn collect_bodies(
    sources: &Query<(&SimPosition, &BodyRadius, &GravitySource, Option<&Orbit>)>,
) -> Vec<(GravityBody, Vec2)> {
    sources
        .iter()
        .map(|(pos, radius, source, orbit)| {
            (
                GravityBody {
                    center: pos.current,
                    mu: source.0,
                    radius: radius.0,
                },
                orbital_velocity(pos.current, orbit),
            )
        })
        .collect()
}

/// Instantaneous velocity of a body on a circular orbit (zero for stars).
fn orbital_velocity(position: Vec2, orbit: Option<&Orbit>) -> Vec2 {
    let Some(orbit) = orbit else {
        return Vec2::ZERO;
    };
    let radial = position - orbit.center;
    // Counter-clockwise orbit: velocity is the CCW perpendicular.
    Vec2::new(-radial.y, radial.x) * orbit.angular_speed
}

fn apply_gravity_to_ships(
    time: Res<Time>,
    sources: Query<(&SimPosition, &BodyRadius, &GravitySource, Option<&Orbit>)>,
    mut ships: Query<(&SimPosition, &mut Velocity), (With<PlayerShip>, Without<GravitySource>)>,
) {
    let dt = time.delta_secs();
    let bodies: Vec<GravityBody> = collect_bodies(&sources).into_iter().map(|b| b.0).collect();
    for (pos, mut vel) in &mut ships {
        vel.0 += gravity_accel(pos.current, &bodies) * dt;
    }
}

// ---------------------------------------------------------------------------
// Weapons
// ---------------------------------------------------------------------------

fn tick_combat_timers(
    time: Res<Time>,
    mut cooldowns: Query<&mut WeaponCooldown>,
    mut shields: Query<&mut SpawnShield>,
) {
    let dt = time.delta_secs();
    for mut cooldown in &mut cooldowns {
        cooldown.0 = (cooldown.0 - dt).max(0.0);
    }
    for mut shield in &mut shields {
        shield.0 = (shield.0 - dt).max(0.0);
    }
}

fn fire_weapons(
    mut commands: Commands,
    config: Res<GameConfig>,
    mut net_ids: ResMut<NetIdAllocator>,
    mut ships: Query<
        (
            &NetId,
            &PlayerIntent,
            &SimPosition,
            &SimRotation,
            &Velocity,
            &mut WeaponCooldown,
            Option<&Upgrades>,
        ),
        With<PlayerShip>,
    >,
) {
    let physics = &config.physics;
    for (net_id, intent, pos, rot, vel, mut cooldown, upgrades) in &mut ships {
        if !intent.fire || cooldown.0 > 0.0 {
            continue;
        }
        cooldown.0 = physics.fire_cooldown;

        let forward = Vec2::from_angle(rot.current);
        let tier = upgrades.map(|up| up.blaster).unwrap_or(0);
        let muzzle = pos.current + forward * (physics.ship_radius + 8.0);
        commands.spawn((
            Name::new("Bolt"),
            Projectile {
                owner: *net_id,
                damage: blaster_damage(physics.projectile_base_damage, tier),
                ttl: physics.projectile_ttl,
            },
            net_ids.allocate(),
            SimPosition::new(muzzle),
            Velocity(vel.0 + forward * physics.projectile_speed),
        ));
    }
}

/// Bolts fly under gravity too — orbital trick shots are legal.
fn integrate_projectiles(
    mut commands: Commands,
    time: Res<Time>,
    sources: Query<(&SimPosition, &BodyRadius, &GravitySource, Option<&Orbit>)>,
    mut projectiles: Query<
        (Entity, &mut Projectile, &mut SimPosition, &mut Velocity),
        Without<GravitySource>,
    >,
) {
    let dt = time.delta_secs();
    let bodies: Vec<GravityBody> = collect_bodies(&sources).into_iter().map(|b| b.0).collect();
    for (entity, mut projectile, mut pos, mut vel) in &mut projectiles {
        projectile.ttl -= dt;
        if projectile.ttl <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        vel.0 += gravity_accel(pos.current, &bodies) * dt;
        let step = vel.0 * dt;
        pos.current += step;
    }
}

#[allow(clippy::too_many_arguments)]
fn collide_projectiles(
    mut commands: Commands,
    config: Res<GameConfig>,
    clock: Res<SimClock>,
    projectiles: Query<(Entity, &Projectile, &SimPosition), Without<PlayerShip>>,
    mut ships: Query<
        (
            &NetId,
            &SimPosition,
            &mut Hull,
            &mut LastDamager,
            Option<&SpawnShield>,
        ),
        With<PlayerShip>,
    >,
    bodies: Query<(&SimPosition, &BodyRadius), (Without<PlayerShip>, Without<Projectile>)>,
    shooters: Query<(&NetId, &PlayerName), With<PlayerShip>>,
    mut impacts: MessageWriter<ProjectileImpact>,
) {
    let physics = &config.physics;
    'bolts: for (bolt_entity, bolt, bolt_pos) in &projectiles {
        // Ships first (the interesting target).
        for (ship_id, ship_pos, mut hull, mut damager, shield) in &mut ships {
            if *ship_id == bolt.owner {
                continue;
            }
            let hit_range = physics.projectile_radius + physics.ship_radius;
            if bolt_pos.current.distance(ship_pos.current) > hit_range {
                continue;
            }
            let shielded = shield.map(|s| s.0 > 0.0).unwrap_or(false);
            if !shielded {
                hull.0 -= bolt.damage;
                damager.name = shooters
                    .iter()
                    .find(|(id, _)| **id == bolt.owner)
                    .map(|(_, name)| name.0.clone());
                damager.at = clock.elapsed;
            }
            impacts.write(ProjectileImpact {
                position: bolt_pos.current,
            });
            commands.entity(bolt_entity).despawn();
            continue 'bolts;
        }
        // Then terrain: planets, stars, asteroids all stop bolts.
        for (body_pos, body_radius) in &bodies {
            if bolt_pos.current.distance(body_pos.current)
                <= body_radius.0 + physics.projectile_radius
            {
                impacts.write(ProjectileImpact {
                    position: bolt_pos.current,
                });
                commands.entity(bolt_entity).despawn();
                continue 'bolts;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Ship collisions
// ---------------------------------------------------------------------------

fn collide_ships_with_world(
    config: Res<GameConfig>,
    clock: Res<SimClock>,
    mut ships: Query<
        (
            &mut SimPosition,
            &mut Velocity,
            &mut Hull,
            &mut LastDamager,
            &PlayerName,
            Option<&SpawnShield>,
        ),
        With<PlayerShip>,
    >,
    bodies: Query<
        (&SimPosition, &BodyRadius, Option<&Orbit>),
        (Without<PlayerShip>, Without<Projectile>),
    >,
    mut impacts: MessageWriter<ProjectileImpact>,
) {
    let physics = &config.physics;

    // Ships against celestial bodies.
    for (mut pos, mut vel, mut hull, _damager, _name, shield) in &mut ships {
        for (body_pos, body_radius, orbit) in &bodies {
            let Some(hit) = collide_with_body(
                pos.current,
                vel.0,
                physics.ship_radius,
                body_pos.current,
                body_radius.0,
                orbital_velocity(body_pos.current, orbit),
                physics.restitution,
            ) else {
                continue;
            };
            pos.current = hit.position;
            vel.0 = hit.velocity;
            let shielded = shield.map(|s| s.0 > 0.0).unwrap_or(false);
            let damage = impact_damage(
                hit.impact_speed,
                physics.min_impact_for_damage,
                physics.impact_damage_scale,
            );
            if damage > 0.0 {
                if !shielded {
                    hull.0 -= damage;
                }
                impacts.write(ProjectileImpact {
                    position: pos.current,
                });
            }
        }
    }

    // Ships against each other (equal masses).
    let mut pairs = ships.iter_combinations_mut();
    while let Some(
        [(mut pos_a, mut vel_a, mut hull_a, mut damager_a, name_a, shield_a), (mut pos_b, mut vel_b, mut hull_b, mut damager_b, name_b, shield_b)],
    ) = pairs.fetch_next()
    {
        let Some((hit_a, hit_b)) = collide_ships(
            pos_a.current,
            vel_a.0,
            pos_b.current,
            vel_b.0,
            physics.ship_radius,
            physics.restitution,
        ) else {
            continue;
        };
        pos_a.current = hit_a.position;
        vel_a.0 = hit_a.velocity;
        pos_b.current = hit_b.position;
        vel_b.0 = hit_b.velocity;

        let damage = impact_damage(
            hit_a.impact_speed,
            physics.min_impact_for_damage,
            physics.impact_damage_scale,
        );
        if damage > 0.0 {
            if !shield_a.map(|s| s.0 > 0.0).unwrap_or(false) {
                hull_a.0 -= damage;
                damager_a.name = Some(name_b.0.clone());
                damager_a.at = clock.elapsed;
            }
            if !shield_b.map(|s| s.0 > 0.0).unwrap_or(false) {
                hull_b.0 -= damage;
                damager_b.name = Some(name_a.0.clone());
                damager_b.at = clock.elapsed;
            }
            impacts.write(ProjectileImpact {
                position: (pos_a.current + pos_b.current) / 2.0,
            });
        }
    }
}

/// Flying inside a star's corona cooks the hull.
fn star_burn(
    time: Res<Time>,
    config: Res<GameConfig>,
    clock: Res<SimClock>,
    stars: Query<(&CentralStar, &SimPosition, &BodyRadius)>,
    mut ships: Query<
        (
            &SimPosition,
            &mut Hull,
            &mut LastDamager,
            Option<&SpawnShield>,
        ),
        With<PlayerShip>,
    >,
) {
    let physics = &config.physics;
    let dt = time.delta_secs();
    for (star, star_pos, star_radius) in &stars {
        let corona = star_radius.0 * physics.star_burn_radius_factor;
        for (pos, mut hull, mut damager, shield) in &mut ships {
            if pos.current.distance(star_pos.current) > corona {
                continue;
            }
            if shield.map(|s| s.0 > 0.0).unwrap_or(false) {
                continue;
            }
            hull.0 -= physics.star_burn_dps * dt;
            damager.name = Some(format!(
                "the {} corona",
                config
                    .systems
                    .get(star.system_index)
                    .map(|s| s.star.name.as_str())
                    .unwrap_or("star")
            ));
            damager.at = clock.elapsed;
        }
    }
}

// ---------------------------------------------------------------------------
// Death & respawn
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn handle_deaths(
    config: Res<GameConfig>,
    clock: Res<SimClock>,
    mut ships: Query<
        (
            &NetId,
            &PlayerName,
            &mut SimPosition,
            &mut SimRotation,
            &mut Velocity,
            &mut Hull,
            &ShipStats,
            &mut Cargo,
            &mut MiningRig,
            &mut LastDamager,
            &mut SpawnShield,
            &Credits,
        ),
        With<PlayerShip>,
    >,
    mut destroyed: MessageWriter<ShipDestroyed>,
) {
    let spawn = Vec2::new(config.ship.spawn_position.0, config.ship.spawn_position.1);
    for (
        net_id,
        name,
        mut pos,
        mut rot,
        mut vel,
        mut hull,
        stats,
        mut cargo,
        mut rig,
        mut damager,
        mut shield,
        _credits,
    ) in &mut ships
    {
        if hull.0 > 0.0 {
            continue;
        }

        let cargo_lost = cargo.total();
        let killer_name = damager
            .name
            .take()
            .filter(|_| clock.elapsed - damager.at < KILL_ATTRIBUTION_SECS);

        // Ship is destroyed: cargo scattered to the void, pilot wakes up in
        // a fresh hull back home. Credits and upgrades survive.
        *cargo = Cargo::new(cargo.capacity());
        hull.0 = stats.max_hull;
        *pos = SimPosition::new(spawn);
        *rot = SimRotation::new(std::f32::consts::FRAC_PI_2);
        vel.0 = Vec2::ZERO;
        rig.target = None;
        rig.progress = 0.0;
        shield.0 = config.physics.respawn_shield_secs;

        destroyed.write(ShipDestroyed {
            victim: *net_id,
            victim_name: name.0.clone(),
            killer_name,
            cargo_lost,
        });
    }
}
