//! The universe: multiple solar systems with stars, orbiting planets,
//! asteroid belts and jump gates, split along the client/server seam:
//!
//! - [`WorldSimPlugin`] (server): spawns the authoritative universe (sim
//!   components only, no meshes) from config or a save file, and runs jump
//!   gate traversal.
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
    Asteroid, BodyRadius, CentralStar, Gate, GateCooldown, GravitySource, NetId, Orbit, Planet,
    PlayerShip, ResourceDeposit, SimClock, SimPosition, SimRotation, SimSet, SolarSystem, Spin,
};
use crate::config::{GameConfig, GateConfig, PlanetConfig, StarConfig};
use crate::logic::belt::generate_belt;
use crate::logic::orbit::{angular_speed, orbit_position};
use crate::logic::physics::wrap_angle;
use crate::plugins::persistence::PendingLoad;
use crate::resource_types::ResourceType;

// Z layering for world entities.
const ORBIT_RING_Z: f32 = -10.0;
const GATE_Z: f32 = -5.0;
const STAR_Z: f32 = 0.0;
const PLANET_Z: f32 = 1.0;
const ASTEROID_Z: f32 = 2.0;

/// Seconds a ship is gate-locked after jumping (stops instant ping-pong).
const GATE_COOLDOWN_SECS: f32 = 4.0;

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

pub fn spawn_star(
    commands: &mut Commands,
    system: Entity,
    system_index: usize,
    cfg: &StarConfig,
    center: Vec2,
) -> Entity {
    commands
        .spawn((
            Name::new(format!("Star: {}", cfg.name)),
            CentralStar { system_index },
            BodyRadius(cfg.radius),
            GravitySource(cfg.gravity_mu),
            SimPosition::new(center),
            ChildOf(system),
        ))
        .id()
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_planet(
    commands: &mut Commands,
    system: Entity,
    system_index: usize,
    config_index: usize,
    cfg: &PlanetConfig,
    center: Vec2,
    net_id: NetId,
    deposit_amount: Option<f32>,
    sim_time: f64,
) -> Entity {
    let orbit = Orbit {
        center,
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
        Planet {
            system_index,
            config_index,
        },
        net_id,
        BodyRadius(cfg.radius),
        GravitySource(cfg.gravity_mu),
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

pub fn spawn_gate(
    commands: &mut Commands,
    system: Entity,
    system_index: usize,
    gate_index: usize,
    cfg: &GateConfig,
    center: Vec2,
) -> Entity {
    commands
        .spawn((
            Name::new(format!("Gate: {}", cfg.name)),
            Gate {
                system_index,
                gate_index,
                to_system: cfg.to_system,
                to_gate: cfg.to_gate,
            },
            SimPosition::new(center + Vec2::new(cfg.position.0, cfg.position.1)),
            ChildOf(system),
        ))
        .id()
}

// ---------------------------------------------------------------------------
// Server: authoritative universe spawn + gate traversal
// ---------------------------------------------------------------------------

pub struct WorldSimPlugin;

impl Plugin for WorldSimPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NetIdAllocator>()
            .add_systems(Startup, spawn_universe)
            .add_systems(FixedUpdate, traverse_gates.in_set(SimSet::Physics));
    }
}

fn spawn_universe(
    mut commands: Commands,
    config: Res<GameConfig>,
    pending: Option<Res<PendingLoad>>,
    mut clock: ResMut<SimClock>,
    mut net_ids: ResMut<NetIdAllocator>,
) {
    let save = pending.as_ref().and_then(|pending| pending.0.as_ref());

    // Restore the sim clock first: planet spawn positions derive from it.
    if let Some(save) = save {
        clock.elapsed = save.sim_elapsed;
    }

    for (system_index, system_cfg) in config.systems.iter().enumerate() {
        let center = Vec2::new(system_cfg.center.0, system_cfg.center.1);
        let system = spawn_system_root(&mut commands, &system_cfg.name);
        spawn_star(
            &mut commands,
            system,
            system_index,
            &system_cfg.star,
            center,
        );

        for (index, planet_cfg) in system_cfg.planets.iter().enumerate() {
            // Restore the saved pool level if this planet appears in the save.
            let amount = save.and_then(|save| {
                save.planet_deposits
                    .iter()
                    .find(|entry| entry.system_index == system_index && entry.config_index == index)
                    .map(|entry| entry.amount)
            });
            spawn_planet(
                &mut commands,
                system,
                system_index,
                index,
                planet_cfg,
                center,
                net_ids.allocate(),
                amount,
                clock.elapsed,
            );
        }

        for (gate_index, gate_cfg) in system_cfg.gates.iter().enumerate() {
            spawn_gate(
                &mut commands,
                system,
                system_index,
                gate_index,
                gate_cfg,
                center,
            );
        }

        // Fresh worlds generate belts from config seeds; saved worlds
        // restore surviving asteroids below (they carry absolute positions).
        if save.is_none() {
            for belt_cfg in &system_cfg.belts {
                for spawn in generate_belt(belt_cfg) {
                    spawn_asteroid(
                        &mut commands,
                        system,
                        AsteroidInit {
                            net_id: net_ids.allocate(),
                            position: center + spawn.position,
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

    if let Some(save) = save {
        // Saved asteroids attach to the first system root purely for
        // hierarchy bookkeeping; their positions are absolute anyway.
        let system = spawn_system_root(&mut commands, "Restored Belt Objects");
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
    }
}

/// Fly into a gate, come out of its partner in another system.
fn traverse_gates(
    time: Res<Time>,
    config: Res<GameConfig>,
    gates: Query<(&Gate, &SimPosition), Without<PlayerShip>>,
    mut ships: Query<(&mut SimPosition, &mut GateCooldown), With<PlayerShip>>,
) {
    let dt = time.delta_secs();
    for (mut pos, mut cooldown) in &mut ships {
        cooldown.0 = (cooldown.0 - dt).max(0.0);
        if cooldown.0 > 0.0 {
            continue;
        }
        for (gate, gate_pos) in &gates {
            let Some(gate_cfg) = config
                .systems
                .get(gate.system_index)
                .and_then(|system| system.gates.get(gate.gate_index))
            else {
                continue;
            };
            if pos.current.distance(gate_pos.current) > gate_cfg.radius {
                continue;
            }
            let Some(target_system) = config.systems.get(gate.to_system) else {
                continue;
            };
            let Some(target_gate) = target_system.gates.get(gate.to_gate) else {
                continue;
            };
            let target_center = Vec2::new(target_system.center.0, target_system.center.1);
            let exit = target_center
                + Vec2::new(target_gate.position.0, target_gate.position.1)
                + Vec2::new(0.0, -(target_gate.radius + 120.0));
            // Snap both halves of the interpolation buffer: a jump is a
            // teleport, not something to smear across the galaxy.
            *pos = SimPosition::new(exit);
            cooldown.0 = GATE_COOLDOWN_SECS;
            break;
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
                attach_gate_visuals,
                pulse_gates,
            ),
        );
    }
}

fn attach_star_visuals(
    mut commands: Commands,
    config: Res<GameConfig>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    stars: Query<(Entity, &CentralStar, &BodyRadius, &SimPosition), Without<Mesh2d>>,
) {
    for (entity, star, radius, pos) in &stars {
        let color = config
            .systems
            .get(star.system_index)
            .map(|system| system.star.color)
            .unwrap_or((1.0, 0.9, 0.6));
        let star_color = Color::srgb(color.0, color.1, color.2);
        commands.entity(entity).insert((
            Mesh2d(meshes.add(Circle::new(radius.0))),
            MeshMaterial2d(materials.add(star_color)),
            Transform::from_translation(pos.current.extend(STAR_Z)),
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
        let Some(cfg) = config
            .systems
            .get(planet.system_index)
            .and_then(|system| system.planets.get(planet.config_index))
        else {
            continue;
        };
        let color = Color::srgb(cfg.color.0, cfg.color.1, cfg.color.2);
        commands.entity(entity).insert((
            Mesh2d(meshes.add(Circle::new(cfg.radius))),
            MeshMaterial2d(materials.add(color)),
            Transform::from_translation(pos.current.extend(PLANET_Z)),
        ));
        // Faint orbit ring so the system layout is readable.
        commands.spawn((
            Name::new(format!("Orbit Ring: {}", cfg.name)),
            Mesh2d(meshes.add(Annulus::new(orbit.radius - 2.0, orbit.radius + 2.0))),
            MeshMaterial2d(materials.add(Color::srgba(1.0, 1.0, 1.0, 0.05))),
            Transform::from_translation(orbit.center.extend(ORBIT_RING_Z)),
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

/// Marker for the gate ring so it can pulse.
#[derive(Component)]
struct GateRing;

fn attach_gate_visuals(
    mut commands: Commands,
    config: Res<GameConfig>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    gates: Query<(Entity, &Gate, &SimPosition), Without<Mesh2d>>,
) {
    for (entity, gate, pos) in &gates {
        let Some(cfg) = config
            .systems
            .get(gate.system_index)
            .and_then(|system| system.gates.get(gate.gate_index))
        else {
            continue;
        };
        commands.entity(entity).insert((
            GateRing,
            Mesh2d(meshes.add(Annulus::new(cfg.radius - 8.0, cfg.radius))),
            MeshMaterial2d(materials.add(Color::srgba(0.4, 0.9, 1.0, 0.7))),
            Transform::from_translation(pos.current.extend(GATE_Z)),
        ));
        // Inner shimmer disc.
        commands.spawn((
            Name::new("Gate Shimmer"),
            Mesh2d(meshes.add(Circle::new(cfg.radius - 10.0))),
            MeshMaterial2d(materials.add(Color::srgba(0.4, 0.8, 1.0, 0.12))),
            Transform::from_xyz(0.0, 0.0, -0.1),
            ChildOf(entity),
        ));
    }
}

/// Gates slowly rotate so they read as active machinery.
fn pulse_gates(time: Res<Time>, mut gates: Query<&mut Transform, With<GateRing>>) {
    for mut transform in &mut gates {
        transform.rotation = Quat::from_rotation_z(time.elapsed_secs() * 0.4);
    }
}
