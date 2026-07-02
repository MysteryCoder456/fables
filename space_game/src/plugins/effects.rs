//! Lightweight particle effects: engine exhaust, mining beams and sparks,
//! depletion bursts. Pure decoration — client-only, frame-rate independent.
//!
//! Everything here is multi-ship aware: remote players' ships get exhaust
//! and mining beams from their replicated intent/rig state.

use std::collections::HashMap;

use bevy::prelude::*;
use rand::Rng;

use crate::components::{MiningRig, PlayerIntent, PlayerShip, RenderSet, SimPosition};
use crate::plugins::resources::{DepositExhausted, ResourceMined};

pub struct EffectsPlugin;

impl Plugin for EffectsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ImpactFlash>().add_systems(
            Update,
            (
                spawn_thrust_particles,
                spawn_mining_sparks,
                sparkle_on_intake,
                burst_on_exhausted,
                burst_on_impact,
                update_mining_beams,
                update_particles,
            )
                .in_set(RenderSet::Decor),
        );
    }
}

/// A hard impact somewhere (collision or blaster hit), replicated from the
/// server for spark bursts.
#[derive(Message, Debug, Clone, Copy)]
pub struct ImpactFlash {
    pub position: Vec2,
}

const PARTICLE_Z: f32 = 9.0;
const BEAM_Z: f32 = 8.0;

/// A short-lived, fading sprite.
#[derive(Component)]
struct Particle {
    velocity: Vec2,
    lifetime: Timer,
}

/// The mining beam belonging to one ship.
#[derive(Component)]
struct BeamOf(Entity);

fn spawn_particle(
    commands: &mut Commands,
    position: Vec2,
    velocity: Vec2,
    color: Color,
    size: f32,
    lifetime_secs: f32,
) {
    commands.spawn((
        Particle {
            velocity,
            lifetime: Timer::from_seconds(lifetime_secs, TimerMode::Once),
        },
        Sprite::from_color(color, Vec2::splat(size)),
        Transform::from_translation(position.extend(PARTICLE_Z)),
    ));
}

/// Engine exhaust behind every thrusting ship (local or remote).
fn spawn_thrust_particles(
    mut commands: Commands,
    time: Res<Time>,
    ships: Query<(&Transform, &PlayerIntent), With<PlayerShip>>,
) {
    let mut rng = rand::thread_rng();
    for (ship, intent) in &ships {
        if intent.thrust <= 0.0 {
            continue;
        }
        let forward = (ship.rotation * Vec3::X).truncate();
        let rear = ship.translation.truncate() - forward * 14.0;

        // Emission rate ~120 particles/sec regardless of frame rate.
        let count = (time.delta_secs() * 120.0).ceil() as u32;
        for _ in 0..count.min(8) {
            let jitter = Vec2::new(rng.gen_range(-1.0..1.0), rng.gen_range(-1.0..1.0)) * 30.0;
            let color = Color::srgba(1.0, rng.gen_range(0.5..0.8), 0.2, 0.9);
            spawn_particle(
                &mut commands,
                rear + jitter * 0.1,
                -forward * rng.gen_range(120.0..220.0) + jitter,
                color,
                rng.gen_range(2.0..4.5),
                rng.gen_range(0.25..0.5),
            );
        }
    }
}

/// Sparks drifting from each mined deposit toward the mining ship.
fn spawn_mining_sparks(
    mut commands: Commands,
    time: Res<Time>,
    ships: Query<(&SimPosition, &MiningRig), With<PlayerShip>>,
    deposits: Query<(&SimPosition, &crate::components::ResourceDeposit)>,
) {
    let mut rng = rand::thread_rng();
    for (ship_pos, rig) in &ships {
        let Some((deposit_pos, deposit)) = rig.target.and_then(|e| deposits.get(e).ok()) else {
            continue;
        };
        let count = (time.delta_secs() * 40.0).ceil() as u32;
        let toward_ship = (ship_pos.current - deposit_pos.current).normalize_or_zero();
        for _ in 0..count.min(4) {
            let jitter = Vec2::new(rng.gen_range(-1.0..1.0), rng.gen_range(-1.0..1.0)) * 12.0;
            spawn_particle(
                &mut commands,
                deposit_pos.current + jitter,
                toward_ship * rng.gen_range(60.0..140.0) + jitter * 2.0,
                deposit.kind.color(),
                rng.gen_range(1.5..3.0),
                rng.gen_range(0.4..0.8),
            );
        }
    }
}

/// A small ring of sparkles around a ship when units land in its hold.
fn sparkle_on_intake(
    mut commands: Commands,
    mut messages: MessageReader<ResourceMined>,
    ships: Query<&SimPosition, With<PlayerShip>>,
) {
    let mut rng = rand::thread_rng();
    for message in messages.read() {
        let Ok(ship) = ships.get(message.ship) else {
            continue;
        };
        for _ in 0..(message.amount * 3).min(9) {
            let dir = Vec2::from_angle(rng.gen_range(0.0..std::f32::consts::TAU));
            spawn_particle(
                &mut commands,
                ship.current + dir * 16.0,
                dir * rng.gen_range(20.0..50.0),
                message.kind.color(),
                2.0,
                0.35,
            );
        }
    }
}

/// White-hot spark burst wherever something slammed into something else.
fn burst_on_impact(mut commands: Commands, mut messages: MessageReader<ImpactFlash>) {
    let mut rng = rand::thread_rng();
    for message in messages.read() {
        for _ in 0..10 {
            let dir = Vec2::from_angle(rng.gen_range(0.0..std::f32::consts::TAU));
            spawn_particle(
                &mut commands,
                message.position + dir * 2.0,
                dir * rng.gen_range(60.0..240.0),
                Color::srgba(1.0, 0.9, 0.6, 0.95),
                rng.gen_range(1.5..3.5),
                rng.gen_range(0.2..0.45),
            );
        }
    }
}

/// Radial debris burst when an asteroid runs dry.
fn burst_on_exhausted(mut commands: Commands, mut messages: MessageReader<DepositExhausted>) {
    let mut rng = rand::thread_rng();
    for message in messages.read() {
        for _ in 0..24 {
            let dir = Vec2::from_angle(rng.gen_range(0.0..std::f32::consts::TAU));
            spawn_particle(
                &mut commands,
                message.position + dir * 4.0,
                dir * rng.gen_range(40.0..180.0),
                message.kind.color(),
                rng.gen_range(2.0..5.0),
                rng.gen_range(0.5..1.1),
            );
        }
    }
}

fn update_particles(
    mut commands: Commands,
    time: Res<Time>,
    mut particles: Query<(Entity, &mut Particle, &mut Transform, &mut Sprite)>,
) {
    let dt = time.delta_secs();
    for (entity, mut particle, mut transform, mut sprite) in &mut particles {
        if particle.lifetime.tick(time.delta()).is_finished() {
            commands.entity(entity).despawn();
            continue;
        }
        transform.translation.x += particle.velocity.x * dt;
        transform.translation.y += particle.velocity.y * dt;
        let remaining = particle.lifetime.fraction_remaining();
        let alpha = sprite.color.alpha().min(remaining);
        sprite.color.set_alpha(alpha);
    }
}

// ---------------------------------------------------------------------------
// Mining beams (one per actively mining ship)
// ---------------------------------------------------------------------------

/// Keep exactly one beam sprite per ship, stretched to its mining target.
fn update_mining_beams(
    mut commands: Commands,
    time: Res<Time>,
    ships: Query<(Entity, &Transform, &MiningRig), With<PlayerShip>>,
    targets: Query<&Transform, Without<BeamOf>>,
    mut beams: Query<
        (
            Entity,
            &BeamOf,
            &mut Transform,
            &mut Sprite,
            &mut Visibility,
        ),
        Without<PlayerShip>,
    >,
) {
    let mut beams_by_ship: HashMap<Entity, _> = HashMap::new();
    for (beam_entity, owner, transform, sprite, visibility) in &mut beams {
        beams_by_ship.insert(owner.0, (beam_entity, transform, sprite, visibility));
    }

    for (ship_entity, ship_transform, rig) in &ships {
        let Some((_, mut beam_transform, mut sprite, mut visibility)) =
            beams_by_ship.remove(&ship_entity)
        else {
            commands.spawn((
                Name::new("Mining Beam"),
                BeamOf(ship_entity),
                Sprite::from_color(Color::srgba(1.0, 0.9, 0.4, 0.5), Vec2::new(1.0, 2.0)),
                Transform::from_xyz(0.0, 0.0, BEAM_Z),
                Visibility::Hidden,
            ));
            continue;
        };

        let target = rig.target.and_then(|e| targets.get(e).ok());
        let Some(deposit) = target else {
            *visibility = Visibility::Hidden;
            continue;
        };
        *visibility = Visibility::Visible;

        let from = ship_transform.translation.truncate();
        let to = deposit.translation.truncate();
        let delta = to - from;
        let midpoint = from + delta / 2.0;

        beam_transform.translation.x = midpoint.x;
        beam_transform.translation.y = midpoint.y;
        beam_transform.rotation = Quat::from_rotation_z(delta.to_angle());
        sprite.custom_size = Some(Vec2::new(delta.length(), 2.0));
        // Subtle pulse so the beam reads as active.
        let pulse = 0.4 + 0.2 * (time.elapsed_secs() * 10.0).sin();
        sprite.color.set_alpha(pulse);
    }

    // Beams whose ship despawned (player left).
    for (beam_entity, ..) in beams_by_ship.into_values() {
        commands.entity(beam_entity).despawn();
    }
}
