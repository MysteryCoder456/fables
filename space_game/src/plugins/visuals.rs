//! Client-only visuals: render interpolation and the parallax starfield.

use bevy::prelude::*;
use rand::{Rng, SeedableRng};

use crate::components::{RenderSet, SimPosition, SimRotation};
use crate::logic::parallax::star_position;
use crate::logic::physics::lerp_angle;
use crate::plugins::camera::MainCamera;

pub struct VisualsPlugin;

impl Plugin for VisualsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_starfield).add_systems(
            Update,
            (
                sync_sim_transforms.in_set(RenderSet::SyncTransforms),
                update_starfield.in_set(RenderSet::Decor),
            ),
        );
    }
}

/// Write interpolated simulation state into render `Transform`s.
///
/// Simulation runs at a fixed timestep; rendering runs every frame. We blend
/// between the previous and current sim states by the fixed-clock overstep
/// fraction so motion looks smooth at any frame rate.
fn sync_sim_transforms(
    fixed_time: Res<Time<Fixed>>,
    mut query: Query<(&SimPosition, Option<&SimRotation>, &mut Transform)>,
) {
    let alpha = fixed_time.overstep_fraction();
    for (pos, rot, mut transform) in &mut query {
        let blended = pos.previous.lerp(pos.current, alpha);
        transform.translation.x = blended.x;
        transform.translation.y = blended.y;
        if let Some(rot) = rot {
            let angle = lerp_angle(rot.previous, rot.current, alpha);
            transform.rotation = Quat::from_rotation_z(angle);
        }
    }
}

// ---------------------------------------------------------------------------
// Starfield
// ---------------------------------------------------------------------------

/// A background star. `base` is its home position inside the wrapping tile;
/// `factor` is the parallax depth (closer to 1 = farther away).
#[derive(Component)]
struct Star {
    base: Vec2,
    factor: f32,
}

/// Size of the wrapping tile the stars live in. Must comfortably exceed the
/// largest visible viewport extent.
const STAR_TILE: f32 = 4096.0;

const STAR_LAYERS: [StarLayer; 3] = [
    StarLayer {
        factor: 0.92,
        count: 140,
        size: 1.5,
        brightness: 0.45,
        z: -100.0,
    },
    StarLayer {
        factor: 0.82,
        count: 90,
        size: 2.5,
        brightness: 0.7,
        z: -99.0,
    },
    StarLayer {
        factor: 0.65,
        count: 50,
        size: 3.5,
        brightness: 1.0,
        z: -98.0,
    },
];

struct StarLayer {
    factor: f32,
    count: u32,
    size: f32,
    brightness: f32,
    z: f32,
}

fn spawn_starfield(mut commands: Commands) {
    // Fixed seed: the starfield is decoration, but determinism keeps
    // screenshots and tests stable.
    let mut rng = rand::rngs::StdRng::seed_from_u64(42);
    for layer in &STAR_LAYERS {
        for _ in 0..layer.count {
            let base = Vec2::new(
                rng.gen_range(-STAR_TILE / 2.0..STAR_TILE / 2.0),
                rng.gen_range(-STAR_TILE / 2.0..STAR_TILE / 2.0),
            );
            let tint = rng.gen_range(0.8..1.0);
            commands.spawn((
                Star {
                    base,
                    factor: layer.factor,
                },
                Sprite::from_color(
                    Color::srgba(tint, tint, 1.0, layer.brightness),
                    Vec2::splat(layer.size),
                ),
                Transform::from_translation(base.extend(layer.z)),
            ));
        }
    }
}

fn update_starfield(
    cameras: Query<&Transform, (With<MainCamera>, Without<Star>)>,
    mut stars: Query<(&Star, &mut Transform)>,
) {
    let Ok(camera) = cameras.single() else {
        return;
    };
    let cam = camera.translation.truncate();
    for (star, mut transform) in &mut stars {
        let pos = star_position(star.base, cam, star.factor, STAR_TILE);
        transform.translation.x = pos.x;
        transform.translation.y = pos.y;
    }
}
