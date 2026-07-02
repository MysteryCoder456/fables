//! The solar system, split along the client/server seam:
//!
//! - [`WorldSimPlugin`] (server): spawns the authoritative world (sim
//!   components only, no meshes) from config or a save file.
//! - [`WorldMotionPlugin`] (both): orbital motion and asteroid tumble —
//!   deterministic functions of the sim clock, so the client runs them
//!   locally instead of receiving positions over the wire.
//! - [`WorldClientPlugin`] (client): attaches meshes/materials to world
//!   entities however they were spawned (locally or via replication).
//!
//! The `spawn_*` functions are shared: the server calls them at startup,
//! the client network plugin calls them when the server's `Welcome` message
//! describes the world.

use bevy::prelude::*;

use crate::components::{
    Asteroid, BodyRadius, CentralStar, NetId, Orbit, Planet, ResourceDeposit, SimClock,
    SimPosition, SimRotation, SimSet, SolarSystem, Spin,
};
use crate::config::{GameConfig, PlanetConfig, StarConfig};
use crate::logic::belt::generate_belt;
use crate::logic::orbit::{angular_speed, orbit_position};
use crate::logic::physics::wrap_angle;
use crate::plugins::persistence::PendingLoad;
use crate::resource_types::ResourceType;

// Z layering for world entities.
const ORBIT_RING_Z: f32 = -10.0;
const STAR_Z: f32 = 0.0;
const PLANET_Z: f32 = 1.0;
const ASTEROID_Z: f32 = 2.0;

/// Allocates stable [`NetId`]s. Server-side only; clients receive ids over
/// the wire.
#[derive(Resource, Debug, Default)]
pub struct NetIdAllocator(u64);

impl NetIdAllocator {
    pub fn allocate(&mut self) -> NetId {
        self.0 += 1;
        NetId(self.0)
    }
}

/// Everything needed to spawn one asteroid entity (fresh, from a save, or
/// from the server's world description).
pub struct AsteroidInit {
    pub net_id: NetId,
    pub position: Vec2,
    pub rotation: f32,
    pub size: f32,
    pub spin: f32,
    pub kind: ResourceType,
    pub amount: f32,
    pub max_amount: f32,
}

// ---------------------------------------------------------------------------
// Shared spawn functions (sim components only — no meshes)
// ---------------------------------------------------------------------------

pub fn spawn_system_root(commands: &mut Commands, name: &str) -> Entity {
    commands
        .spawn((
            Name::new(format!("Solar System: {name}")),
            SolarSystem { name: name.into() },
            Transform::default(),
            Visibility::default(),
        ))
        .id()
}

pub fn spawn_star(commands: &mut Commands, system: Entity, cfg: &StarConfig) -> Entity {
    commands
        .spawn((
            Name::new(format!("Star: {}", cfg.name)),
            CentralStar,
            BodyRadius(cfg.radius),
            SimPosition::new(Vec2::ZERO),
            ChildOf(system),
        ))
        .id()
}

pub fn spawn_planet(
    commands: &mut Commands,
    system: Entity,
    config_index: usize,
    cfg: &PlanetConfig,
    net_id: NetId,
    deposit_amount: Option<f32>,
    sim_time: f64,
) -> Entity {
    let orbit = Orbit {
        center: Vec2::ZERO,
        radius: cfg.orbit_radius,
        angular_speed: angular_speed(cfg.orbit_period),
        phase: cfg.orbit_phase,
    };
    let start = orbit_position(
        orbit.center,
        orbit.radius,
        orbit.angular_speed,
        orbit.phase,
        sim_time,
    );
    let mut planet = commands.spawn((
        Name::new(format!("Planet: {}", cfg.name)),
        Planet { config_index },
        net_id,
        BodyRadius(cfg.radius),
        orbit,
        SimPosition::new(start),
        ChildOf(system),
    ));
    if let Some(deposit) = &cfg.deposit {
        let amount = deposit_amount
            .map(|amount| amount.clamp(0.0, deposit.max_amount))
            .unwrap_or(deposit.max_amount);
        planet.insert(ResourceDeposit {
            kind: deposit.kind,
            amount,
            max_amount: deposit.max_amount,
            regen_per_sec: deposit.regen_per_sec,
        });
    }
    planet.id()
}

pub fn spawn_asteroid(commands: &mut Commands, system: Entity, init: AsteroidInit) -> Entity {
    commands
        .spawn((
            Name::new("Asteroid"),
            Asteroid { size: init.size },
            init.net_id,
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
            ChildOf(system),
        ))
        .id()
}

// ---------------------------------------------------------------------------
// Server: authoritative world spawn
// ---------------------------------------------------------------------------

pub struct WorldSimPlugin;

impl Plugin for WorldSimPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NetIdAllocator>()
            .add_systems(Startup, spawn_solar_system);
    }
}

fn spawn_solar_system(
    mut commands: Commands,
    config: Res<GameConfig>,
    pending: Option<Res<PendingLoad>>,
    mut clock: ResMut<SimClock>,
    mut net_ids: ResMut<NetIdAllocator>,
) {
    let system_cfg = &config.system;
    let save = pending.as_ref().and_then(|pending| pending.0.as_ref());

    // Restore the sim clock first: planet spawn positions derive from it.
    if let Some(save) = save {
        clock.elapsed = save.sim_elapsed;
    }

    let system = spawn_system_root(&mut commands, &system_cfg.name);
    spawn_star(&mut commands, system, &system_cfg.star);

    for (index, planet_cfg) in system_cfg.planets.iter().enumerate() {
        // Restore the saved pool level if this planet appears in the save.
        let amount = save.and_then(|save| {
            save.planet_deposits
                .iter()
                .find(|entry| entry.config_index == index)
                .map(|entry| entry.amount)
        });
        spawn_planet(
            &mut commands,
            system,
            index,
            planet_cfg,
            net_ids.allocate(),
            amount,
            clock.elapsed,
        );
    }

    // Fresh worlds generate belts from config seeds; saved worlds restore
    // the surviving asteroids by value (mined-out rocks stay gone).
    if let Some(save) = save {
        for saved in &save.asteroids {
            spawn_asteroid(
                &mut commands,
                system,
                AsteroidInit {
                    net_id: net_ids.allocate(),
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
                    system,
                    AsteroidInit {
                        net_id: net_ids.allocate(),
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

// ---------------------------------------------------------------------------
// Shared: deterministic world motion
// ---------------------------------------------------------------------------

pub struct WorldMotionPlugin;

impl Plugin for WorldMotionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            (orbit_planets, spin_asteroids).in_set(SimSet::Movement),
        );
    }
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

// ---------------------------------------------------------------------------
// Client: attach visuals to world entities
// ---------------------------------------------------------------------------

pub struct WorldClientPlugin;

impl Plugin for WorldClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                attach_star_visuals,
                attach_planet_visuals,
                attach_asteroid_visuals,
            ),
        );
    }
}

fn attach_star_visuals(
    mut commands: Commands,
    config: Res<GameConfig>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    stars: Query<(Entity, &BodyRadius), (With<CentralStar>, Without<Mesh2d>)>,
) {
    let cfg = &config.system.star;
    let star_color = Color::srgb(cfg.color.0, cfg.color.1, cfg.color.2);
    for (entity, radius) in &stars {
        commands.entity(entity).insert((
            Mesh2d(meshes.add(Circle::new(radius.0))),
            MeshMaterial2d(materials.add(star_color)),
            Transform::from_xyz(0.0, 0.0, STAR_Z),
        ));
        // Glow halos: translucent circles behind the star disc.
        for (scale, alpha) in [(1.35, 0.20), (1.8, 0.10), (2.4, 0.05)] {
            commands.spawn((
                Name::new("Star Glow"),
                Mesh2d(meshes.add(Circle::new(radius.0 * scale))),
                MeshMaterial2d(materials.add(star_color.with_alpha(alpha))),
                Transform::from_xyz(0.0, 0.0, STAR_Z - 0.1 * scale),
                ChildOf(entity),
            ));
        }
    }
}

fn attach_planet_visuals(
    mut commands: Commands,
    config: Res<GameConfig>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    planets: Query<(Entity, &Planet, &Orbit, &SimPosition), Without<Mesh2d>>,
) {
    for (entity, planet, orbit, pos) in &planets {
        let Some(cfg) = config.system.planets.get(planet.config_index) else {
            continue;
        };
        let color = Color::srgb(cfg.color.0, cfg.color.1, cfg.color.2);
        commands.entity(entity).insert((
            Mesh2d(meshes.add(Circle::new(cfg.radius))),
            MeshMaterial2d(materials.add(color)),
            Transform::from_translation(pos.current.extend(PLANET_Z)),
        ));
        // Faint orbit ring so the system layout is readable. The system root
        // sits at the origin, so a top-level ring lines up with the orbit.
        commands.spawn((
            Name::new(format!("Orbit Ring: {}", cfg.name)),
            Mesh2d(meshes.add(Annulus::new(orbit.radius - 2.0, orbit.radius + 2.0))),
            MeshMaterial2d(materials.add(Color::srgba(1.0, 1.0, 1.0, 0.05))),
            Transform::from_xyz(0.0, 0.0, ORBIT_RING_Z),
        ));
    }
}

fn attach_asteroid_visuals(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    asteroids: Query<(Entity, &Asteroid, &ResourceDeposit, &SimPosition), Without<Mesh2d>>,
) {
    for (entity, asteroid, deposit, pos) in &asteroids {
        let tint = deposit.kind.color().to_srgba();
        // Darken toward gray so belts read as rock with a resource hue.
        let rock_color = Color::srgb(
            tint.red * 0.45 + 0.15,
            tint.green * 0.45 + 0.15,
            tint.blue * 0.45 + 0.15,
        );
        let sides = 5 + (asteroid.size as u32 % 4);
        commands.entity(entity).insert((
            Mesh2d(meshes.add(RegularPolygon::new(asteroid.size, sides))),
            MeshMaterial2d(materials.add(rock_color)),
            Transform::from_translation(pos.current.extend(ASTEROID_Z)),
        ));
    }
}
