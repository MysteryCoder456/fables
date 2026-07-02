//! Player ships, split along the client/server seam:
//!
//! - [`PlayerSimPlugin`] (server): integrates thrust physics for every ship
//!   from its per-ship [`PlayerIntent`] component. Ship *spawning* happens in
//!   the server network plugin when a client joins.
//! - [`PlayerClientPlugin`] (client): gathers keyboard input into
//!   [`LocalIntent`] (sent to the server by the network plugin) and attaches
//!   render meshes to replicated ship entities.

use bevy::prelude::*;

use crate::components::{
    LocalIntent, LocalShip, PlayerIntent, PlayerShip, ShipStats, SimPosition, SimRotation, SimSet,
    Velocity,
};
use crate::logic::physics::{step_ship, ShipKinematics, ThrustParams};

/// Z layer for ship meshes (above planets and asteroids).
pub const SHIP_Z: f32 = 10.0;

// ---------------------------------------------------------------------------
// Simulation half (server)
// ---------------------------------------------------------------------------

pub struct PlayerSimPlugin;

impl Plugin for PlayerSimPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, ship_movement.in_set(SimSet::Movement));
    }
}

pub fn ship_movement(
    time: Res<Time>,
    mut ships: Query<
        (
            &mut SimPosition,
            &mut SimRotation,
            &mut Velocity,
            &ShipStats,
            &PlayerIntent,
        ),
        With<PlayerShip>,
    >,
) {
    let dt = time.delta_secs();
    for (mut pos, mut rot, mut vel, stats, intent) in &mut ships {
        let mut kin = ShipKinematics {
            position: pos.current,
            rotation: rot.current,
            velocity: vel.0,
        };
        let params = ThrustParams {
            thrust_accel: stats.thrust_accel,
            turn_speed: stats.turn_speed,
            max_speed: stats.max_speed,
            drag: stats.drag,
        };
        step_ship(
            &mut kin,
            intent.thrust,
            intent.turn,
            intent.brake,
            &params,
            dt,
        );
        pos.current = kin.position;
        rot.current = kin.rotation;
        vel.0 = kin.velocity;
    }
}

// ---------------------------------------------------------------------------
// Client half
// ---------------------------------------------------------------------------

pub struct PlayerClientPlugin;

impl Plugin for PlayerClientPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LocalIntent>()
            .init_resource::<crate::components::ChatTyping>()
            .add_systems(Update, (gather_input, attach_ship_visuals));
    }
}

fn gather_input(
    keys: Res<ButtonInput<KeyCode>>,
    typing: Res<crate::components::ChatTyping>,
    mut local: ResMut<LocalIntent>,
) {
    // While the chat line is open the keyboard belongs to it.
    if typing.0 {
        local.0 = crate::components::PlayerIntent::default();
        return;
    }
    let pressed = |a: KeyCode, b: KeyCode| keys.pressed(a) || keys.pressed(b);
    let intent = &mut local.0;

    intent.thrust = if pressed(KeyCode::KeyW, KeyCode::ArrowUp) {
        1.0
    } else {
        0.0
    };

    let mut turn = 0.0;
    if pressed(KeyCode::KeyA, KeyCode::ArrowLeft) {
        turn += 1.0;
    }
    if pressed(KeyCode::KeyD, KeyCode::ArrowRight) {
        turn -= 1.0;
    }
    intent.turn = turn;

    intent.brake = pressed(KeyCode::KeyS, KeyCode::ArrowDown);
    intent.mine = keys.pressed(KeyCode::Space);
    intent.fire = keys.pressed(KeyCode::KeyF);
}

/// Give freshly spawned/replicated ship entities a render mesh. The local
/// ship is ice-white; other players' ships are amber so they read instantly.
fn attach_ship_visuals(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    ships: Query<(Entity, &SimPosition, Has<LocalShip>), (With<PlayerShip>, Without<Mesh2d>)>,
) {
    for (entity, pos, is_local) in &ships {
        // Nose points +X at rotation 0, matching the physics convention.
        let hull_mesh = meshes.add(Triangle2d::new(
            Vec2::new(18.0, 0.0),
            Vec2::new(-12.0, 10.0),
            Vec2::new(-12.0, -10.0),
        ));
        let color = if is_local {
            Color::srgb(0.85, 0.95, 1.0)
        } else {
            Color::srgb(1.0, 0.75, 0.4)
        };
        commands.entity(entity).insert((
            Mesh2d(hull_mesh),
            MeshMaterial2d(materials.add(color)),
            Transform::from_translation(pos.current.extend(SHIP_Z)),
        ));
    }
}
