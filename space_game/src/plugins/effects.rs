//! Lightweight particle effects: engine exhaust, mining beam and sparks,
//! depletion bursts. Pure decoration — client-only, frame-rate independent.

use bevy::prelude::*;
use rand::Rng;

use crate::components::{MiningRig, PlayerIntent, PlayerShip, RenderSet, SimPosition};
use crate::plugins::resources::{DepositExhausted, ResourceMined};

pub struct EffectsPlugin;

impl Plugin for EffectsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_mining_beam).add_systems(
            Update,
            (
                spawn_thrust_particles,
                spawn_mining_sparks,
                sparkle_on_intake,
                burst_on_exhausted,
                update_mining_beam,
                update_particles,
            )
                .in_set(RenderSet::Decor),
        );
    }
}

const PARTICLE_Z: f32 = 9.0;
const BEAM_Z: f32 = 8.0;

/// A short-lived, fading sprite.
#[derive(Component)]
struct Particle {
    velocity: Vec2,
    lifetime: Timer,
}

#[derive(Component)]
struct MiningBeam;

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

/// Engine exhaust while thrusting.
fn spawn_thrust_particles(
    mut commands: Commands,
    time: Res<Time>,
    intent: Res<PlayerIntent>,
    ships: Query<&Transform, With<PlayerShip>>,
) {
    if intent.thrust <= 0.0 {
        return;
    }
    let Ok(ship) = ships.single() else {
        return;
    };
    let mut rng = rand::thread_rng();
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

/// Sparks drifting from the mined deposit toward the ship.
fn spawn_mining_sparks(
    mut commands: Commands,
    time: Res<Time>,
    ships: Query<(&SimPosition, &MiningRig), With<PlayerShip>>,
    deposits: Query<(&SimPosition, &crate::components::ResourceDeposit)>,
) {
    let Ok((ship_pos, rig)) = ships.single() else {
        return;
    };
    let Some((deposit_pos, deposit)) = rig.target.and_then(|e| deposits.get(e).ok()) else {
        return;
    };
    let mut rng = rand::thread_rng();
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

/// A small ring of sparkles around the ship when units land in the hold.
fn sparkle_on_intake(
    mut commands: Commands,
    mut messages: MessageReader<ResourceMined>,
    ships: Query<&SimPosition, With<PlayerShip>>,
) {
    let Ok(ship) = ships.single() else {
        return;
    };
    let mut rng = rand::thread_rng();
    for message in messages.read() {
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
// Mining beam
// ---------------------------------------------------------------------------

fn spawn_mining_beam(mut commands: Commands) {
    commands.spawn((
        Name::new("Mining Beam"),
        MiningBeam,
        Sprite::from_color(Color::srgba(1.0, 0.9, 0.4, 0.5), Vec2::new(1.0, 2.0)),
        Transform::from_xyz(0.0, 0.0, BEAM_Z),
        Visibility::Hidden,
    ));
}

/// Stretch a thin sprite between ship and target while mining.
fn update_mining_beam(
    time: Res<Time>,
    ships: Query<(&Transform, &MiningRig), (With<PlayerShip>, Without<MiningBeam>)>,
    deposits: Query<&Transform, (Without<PlayerShip>, Without<MiningBeam>)>,
    mut beams: Query<(&mut Transform, &mut Sprite, &mut Visibility), With<MiningBeam>>,
) {
    let Ok((mut beam_transform, mut sprite, mut visibility)) = beams.single_mut() else {
        return;
    };
    let target = ships.single().ok().and_then(|(ship, rig)| {
        rig.target
            .and_then(|e| deposits.get(e).ok().map(|t| (ship, t)))
    });

    let Some((ship, deposit)) = target else {
        *visibility = Visibility::Hidden;
        return;
    };
    *visibility = Visibility::Visible;

    let from = ship.translation.truncate();
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
