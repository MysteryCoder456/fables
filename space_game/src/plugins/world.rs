//! The solar system: star, orbiting planets and procedural asteroid belts.
//!
//! Spawning reads only `GameConfig`; orbital motion and asteroid tumble are
//! simulation systems in `FixedUpdate`.

use bevy::prelude::*;

use crate::components::{
    Asteroid, BodyRadius, CentralStar, Orbit, Planet, ResourceDeposit, SimClock, SimPosition,
    SimRotation, SimSet, SolarSystem, Spin,
};
use crate::config::GameConfig;
use crate::logic::belt::generate_belt;
use crate::logic::orbit::{angular_speed, orbit_position};
use crate::logic::physics::wrap_angle;
use crate::plugins::persistence::PendingLoad;
use crate::resource_types::ResourceType;

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_solar_system).add_systems(
            FixedUpdate,
            (orbit_planets, spin_asteroids).in_set(SimSet::Movement),
        );
    }
}

// Z layering for world entities.
const ORBIT_RING_Z: f32 = -10.0;
const STAR_Z: f32 = 0.0;
const PLANET_Z: f32 = 1.0;
const ASTEROID_Z: f32 = 2.0;

/// Everything needed to spawn one asteroid entity (fresh or from a save).
struct AsteroidInit {
    position: Vec2,
    rotation: f32,
    size: f32,
    spin: f32,
    kind: ResourceType,
    amount: f32,
    max_amount: f32,
}

fn spawn_solar_system(
    mut commands: Commands,
    config: Res<GameConfig>,
    pending: Option<Res<PendingLoad>>,
    mut clock: ResMut<SimClock>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let system_cfg = &config.system;
    let save = pending.as_ref().and_then(|pending| pending.0.as_ref());

    // Restore the sim clock first: planet spawn positions derive from it.
    if let Some(save) = save {
        clock.elapsed = save.sim_elapsed;
    }

    let system = commands
        .spawn((
            Name::new(format!("Solar System: {}", system_cfg.name)),
            SolarSystem {
                name: system_cfg.name.clone(),
            },
            Transform::default(),
            Visibility::default(),
        ))
        .id();

    // --- Star (with a simple layered glow) ---
    let star = &system_cfg.star;
    let star_color = Color::srgb(star.color.0, star.color.1, star.color.2);
    let star_entity = commands
        .spawn((
            Name::new(format!("Star: {}", star.name)),
            CentralStar,
            SimPosition::new(Vec2::ZERO),
            Mesh2d(meshes.add(Circle::new(star.radius))),
            MeshMaterial2d(materials.add(star_color)),
            Transform::from_xyz(0.0, 0.0, STAR_Z),
            ChildOf(system),
        ))
        .id();
    // Glow halos: translucent circles behind the star disc.
    for (scale, alpha) in [(1.35, 0.20), (1.8, 0.10), (2.4, 0.05)] {
        commands.spawn((
            Name::new("Star Glow"),
            Mesh2d(meshes.add(Circle::new(star.radius * scale))),
            MeshMaterial2d(materials.add(star_color.with_alpha(alpha))),
            Transform::from_xyz(0.0, 0.0, STAR_Z - 0.1 * scale),
            ChildOf(star_entity),
        ));
    }

    // --- Planets ---
    for (index, planet_cfg) in system_cfg.planets.iter().enumerate() {
        let orbit = Orbit {
            center: Vec2::ZERO,
            radius: planet_cfg.orbit_radius,
            angular_speed: angular_speed(planet_cfg.orbit_period),
            phase: planet_cfg.orbit_phase,
        };
        let start = orbit_position(
            orbit.center,
            orbit.radius,
            orbit.angular_speed,
            orbit.phase,
            clock.elapsed,
        );
        let color = Color::srgb(planet_cfg.color.0, planet_cfg.color.1, planet_cfg.color.2);

        let mut planet = commands.spawn((
            Name::new(format!("Planet: {}", planet_cfg.name)),
            Planet {
                config_index: index,
            },
            BodyRadius(planet_cfg.radius),
            orbit,
            SimPosition::new(start),
            Mesh2d(meshes.add(Circle::new(planet_cfg.radius))),
            MeshMaterial2d(materials.add(color)),
            Transform::from_translation(start.extend(PLANET_Z)),
            ChildOf(system),
        ));
        if let Some(deposit) = &planet_cfg.deposit {
            // Restore the saved pool level if this planet appears in the save.
            let amount = save
                .and_then(|save| {
                    save.planet_deposits
                        .iter()
                        .find(|entry| entry.config_index == index)
                })
                .map(|entry| entry.amount.clamp(0.0, deposit.max_amount))
                .unwrap_or(deposit.max_amount);
            planet.insert(ResourceDeposit {
                kind: deposit.kind,
                amount,
                max_amount: deposit.max_amount,
                regen_per_sec: deposit.regen_per_sec,
            });
        }

        // Faint orbit ring so the system layout is readable.
        commands.spawn((
            Name::new(format!("Orbit Ring: {}", planet_cfg.name)),
            Mesh2d(meshes.add(Annulus::new(
                planet_cfg.orbit_radius - 2.0,
                planet_cfg.orbit_radius + 2.0,
            ))),
            MeshMaterial2d(materials.add(Color::srgba(1.0, 1.0, 1.0, 0.05))),
            Transform::from_xyz(0.0, 0.0, ORBIT_RING_Z),
            ChildOf(system),
        ));
    }

    // --- Asteroid belts ---
    // Fresh worlds generate belts from config seeds; saved worlds restore
    // the surviving asteroids by value (mined-out rocks stay gone).
    if let Some(save) = save {
        for saved in &save.asteroids {
            spawn_asteroid(
                &mut commands,
                &mut meshes,
                &mut materials,
                system,
                AsteroidInit {
                    position: saved.position,
                    rotation: saved.rotation,
                    size: saved.size,
                    spin: saved.spin,
                    kind: saved.kind,
                    amount: saved.amount,
                    max_amount: saved.max_amount,
                },
            );
        }
    } else {
        for belt_cfg in &system_cfg.belts {
            for spawn in generate_belt(belt_cfg) {
                spawn_asteroid(
                    &mut commands,
                    &mut meshes,
                    &mut materials,
                    system,
                    AsteroidInit {
                        position: spawn.position,
                        rotation: 0.0,
                        size: spawn.size,
                        spin: spawn.spin,
                        kind: spawn.kind,
                        amount: spawn.amount,
                        max_amount: spawn.amount,
                    },
                );
            }
        }
    }
}

fn spawn_asteroid(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    system: Entity,
    init: AsteroidInit,
) {
    let tint = init.kind.color().to_srgba();
    // Darken toward gray so belts read as rock with a resource hue.
    let rock_color = Color::srgb(
        tint.red * 0.45 + 0.15,
        tint.green * 0.45 + 0.15,
        tint.blue * 0.45 + 0.15,
    );
    let sides = 5 + (init.size as u32 % 4);
    commands.spawn((
        Name::new("Asteroid"),
        Asteroid { size: init.size },
        BodyRadius(init.size),
        ResourceDeposit {
            kind: init.kind,
            amount: init.amount,
            max_amount: init.max_amount,
            regen_per_sec: 0.0,
        },
        Spin(init.spin),
        SimPosition::new(init.position),
        SimRotation::new(init.rotation),
        Mesh2d(meshes.add(RegularPolygon::new(init.size, sides))),
        MeshMaterial2d(materials.add(rock_color)),
        Transform::from_translation(init.position.extend(ASTEROID_Z)),
        ChildOf(system),
    ));
}

/// Planets follow their orbit as a pure function of the sim clock.
fn orbit_planets(clock: Res<SimClock>, mut planets: Query<(&Orbit, &mut SimPosition)>) {
    for (orbit, mut pos) in &mut planets {
        pos.current = orbit_position(
            orbit.center,
            orbit.radius,
            orbit.angular_speed,
            orbit.phase,
            clock.elapsed,
        );
    }
}

/// Asteroids tumble at their individual spin rate.
fn spin_asteroids(time: Res<Time>, mut asteroids: Query<(&Spin, &mut SimRotation)>) {
    let dt = time.delta_secs();
    for (spin, mut rot) in &mut asteroids {
        rot.current = wrap_angle(rot.current + spin.0 * dt);
    }
}
