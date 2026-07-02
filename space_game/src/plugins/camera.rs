//! Smooth-follow camera. Client-only.

use bevy::prelude::*;

use crate::components::{PlayerShip, RenderSet};

pub struct GameCameraPlugin;

impl Plugin for GameCameraPlugin {
    fn build(&self, app: &mut App) {
        app.configure_sets(
            Update,
            (
                RenderSet::SyncTransforms,
                RenderSet::Camera,
                RenderSet::Decor,
            )
                .chain(),
        )
        .add_systems(Startup, spawn_camera)
        .add_systems(Update, follow_player.in_set(RenderSet::Camera));
    }
}

#[derive(Component)]
pub struct MainCamera;

/// Exponential smoothing rate: higher = tighter follow.
const FOLLOW_RATE: f32 = 4.0;

fn spawn_camera(mut commands: Commands) {
    commands.spawn((Name::new("Main Camera"), MainCamera, Camera2d));
}

fn follow_player(
    time: Res<Time>,
    ships: Query<&Transform, (With<PlayerShip>, Without<MainCamera>)>,
    mut cameras: Query<&mut Transform, With<MainCamera>>,
) {
    let Ok(ship) = ships.single() else {
        return;
    };
    let Ok(mut camera) = cameras.single_mut() else {
        return;
    };
    // Frame-rate independent easing toward the ship.
    let alpha = 1.0 - (-FOLLOW_RATE * time.delta_secs()).exp();
    let target = ship.translation.truncate();
    let current = camera.translation.truncate();
    let eased = current.lerp(target, alpha);
    camera.translation.x = eased.x;
    camera.translation.y = eased.y;
}
