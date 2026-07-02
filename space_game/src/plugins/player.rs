//! Player ship: intent gathering (client) and thrust physics (simulation).

use bevy::prelude::*;

use crate::components::{
    Hull, PlayerIntent, PlayerShip, ShipStats, SimPosition, SimRotation, SimSet, Velocity,
};
use crate::config::GameConfig;
use crate::logic::physics::{step_ship, ShipKinematics, ThrustParams};

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlayerIntent>()
            .add_systems(Startup, spawn_player)
            // Client-side: raw device input -> intent. On a networked client
            // this same intent struct would be sent to the server each tick.
            .add_systems(Update, gather_input)
            // Simulation: consumes intent, never reads input devices.
            .add_systems(FixedUpdate, ship_movement.in_set(SimSet::Movement));
    }
}

/// Z layer for the ship sprite (above planets and asteroids).
pub const SHIP_Z: f32 = 10.0;

fn spawn_player(
    mut commands: Commands,
    config: Res<GameConfig>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let spawn = Vec2::new(config.ship.spawn_position.0, config.ship.spawn_position.1);
    let stats = config.ship.stats.clone();

    // Nose points +X at rotation 0, matching the physics convention.
    let hull_mesh = meshes.add(Triangle2d::new(
        Vec2::new(18.0, 0.0),
        Vec2::new(-12.0, 10.0),
        Vec2::new(-12.0, -10.0),
    ));

    commands.spawn((
        Name::new("Player Ship"),
        PlayerShip,
        SimPosition::new(spawn),
        SimRotation::new(std::f32::consts::FRAC_PI_2),
        Velocity::default(),
        Hull(stats.max_hull),
        stats,
        Mesh2d(hull_mesh),
        MeshMaterial2d(materials.add(Color::srgb(0.85, 0.95, 1.0))),
        Transform::from_translation(spawn.extend(SHIP_Z)),
    ));
}

fn gather_input(keys: Res<ButtonInput<KeyCode>>, mut intent: ResMut<PlayerIntent>) {
    let pressed = |a: KeyCode, b: KeyCode| keys.pressed(a) || keys.pressed(b);

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
}

fn ship_movement(
    time: Res<Time>,
    intent: Res<PlayerIntent>,
    mut ships: Query<
        (&mut SimPosition, &mut SimRotation, &mut Velocity, &ShipStats),
        With<PlayerShip>,
    >,
) {
    let dt = time.delta_secs();
    for (mut pos, mut rot, mut vel, stats) in &mut ships {
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
