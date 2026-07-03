//! Thrust-based ship kinematics. Pure functions, unit-tested below.

use bevy::math::Vec2;

/// Extra drag multiplier applied while braking. Base drag is nearly zero
/// (so orbits persist), so braking brings most of the stopping power.
const BRAKE_DRAG_FACTOR: f32 = 150.0;

/// Mutable kinematic state of a ship for one integration step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShipKinematics {
    pub position: Vec2,
    /// Radians; 0 = facing +X, counter-clockwise positive.
    pub rotation: f32,
    pub velocity: Vec2,
}

/// Tuning parameters for the integrator (mirrors `ShipStats`).
#[derive(Debug, Clone, Copy)]
pub struct ThrustParams {
    pub thrust_accel: f32,
    pub turn_speed: f32,
    pub max_speed: f32,
    pub drag: f32,
}

/// Advance ship kinematics by `dt` seconds given normalized intent.
///
/// - `thrust` is clamped to 0..=1, `turn` to -1..=1.
/// - Drag is exponential (`v *= e^(-drag*dt)`) so behavior is stable for any
///   timestep; braking multiplies the drag coefficient.
/// - Speed is clamped to `max_speed`.
pub fn step_ship(
    kin: &mut ShipKinematics,
    thrust: f32,
    turn: f32,
    brake: bool,
    params: &ThrustParams,
    dt: f32,
) {
    let thrust = thrust.clamp(0.0, 1.0);
    let turn = turn.clamp(-1.0, 1.0);

    kin.rotation = wrap_angle(kin.rotation + turn * params.turn_speed * dt);

    let forward = Vec2::from_angle(kin.rotation);
    kin.velocity += forward * (params.thrust_accel * thrust * dt);

    let drag = if brake {
        params.drag * BRAKE_DRAG_FACTOR
    } else {
        params.drag
    };
    kin.velocity *= (-drag * dt).exp();

    let speed = kin.velocity.length();
    if speed > params.max_speed {
        kin.velocity *= params.max_speed / speed;
    }

    kin.position += kin.velocity * dt;
}

/// Wrap an angle to the half-open interval [-PI, PI).
pub fn wrap_angle(angle: f32) -> f32 {
    let two_pi = std::f32::consts::TAU;
    let wrapped = angle - two_pi * (angle / two_pi).round();
    if wrapped >= std::f32::consts::PI {
        wrapped - two_pi
    } else {
        wrapped
    }
}

/// Shortest-path interpolation between two angles (for render smoothing).
pub fn lerp_angle(from: f32, to: f32, alpha: f32) -> f32 {
    from + wrap_angle(to - from) * alpha
}

#[cfg(test)]
mod tests {
    use super::*;

    const PARAMS: ThrustParams = ThrustParams {
        thrust_accel: 100.0,
        turn_speed: 2.0,
        max_speed: 50.0,
        drag: 0.5,
    };

    fn at_rest() -> ShipKinematics {
        ShipKinematics {
            position: Vec2::ZERO,
            rotation: 0.0,
            velocity: Vec2::ZERO,
        }
    }

    #[test]
    fn thrust_accelerates_along_facing() {
        let mut kin = at_rest();
        step_ship(&mut kin, 1.0, 0.0, false, &PARAMS, 0.1);
        assert!(kin.velocity.x > 0.0, "should accelerate along +X");
        assert!(kin.velocity.y.abs() < 1e-5);
        assert!(kin.position.x > 0.0);
    }

    #[test]
    fn no_thrust_means_drag_only_decay() {
        let mut kin = at_rest();
        kin.velocity = Vec2::new(40.0, 0.0);
        let initial = kin.velocity.length();
        step_ship(&mut kin, 0.0, 0.0, false, &PARAMS, 0.1);
        let after = kin.velocity.length();
        assert!(after < initial);
        assert!(after > 0.0, "drag never fully stops the ship in one step");
    }

    #[test]
    fn braking_decays_faster_than_coasting() {
        let mut coasting = at_rest();
        coasting.velocity = Vec2::new(40.0, 0.0);
        let mut braking = coasting;
        step_ship(&mut coasting, 0.0, 0.0, false, &PARAMS, 0.1);
        step_ship(&mut braking, 0.0, 0.0, true, &PARAMS, 0.1);
        assert!(braking.velocity.length() < coasting.velocity.length());
    }

    #[test]
    fn speed_is_clamped_to_max() {
        let mut kin = at_rest();
        for _ in 0..1000 {
            step_ship(&mut kin, 1.0, 0.0, false, &PARAMS, 0.05);
        }
        assert!(kin.velocity.length() <= PARAMS.max_speed + 1e-3);
    }

    #[test]
    fn turning_changes_rotation_and_wraps() {
        let mut kin = at_rest();
        // Turn counter-clockwise for a long time; rotation must stay wrapped.
        for _ in 0..1000 {
            step_ship(&mut kin, 0.0, 1.0, false, &PARAMS, 0.1);
        }
        assert!(kin.rotation >= -std::f32::consts::PI);
        assert!(kin.rotation < std::f32::consts::PI);
    }

    #[test]
    fn intent_is_clamped() {
        let mut cheated = at_rest();
        let mut honest = at_rest();
        step_ship(&mut cheated, 100.0, 0.0, false, &PARAMS, 0.1);
        step_ship(&mut honest, 1.0, 0.0, false, &PARAMS, 0.1);
        assert!((cheated.velocity - honest.velocity).length() < 1e-6);
    }

    #[test]
    fn wrap_angle_bounds() {
        for a in [-100.0f32, -3.2, 0.0, 3.2, 100.0] {
            let w = wrap_angle(a);
            assert!((-std::f32::consts::PI..std::f32::consts::PI).contains(&w));
        }
        assert!((wrap_angle(std::f32::consts::TAU + 0.5) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn lerp_angle_takes_shortest_path() {
        // From just below PI to just above -PI is a tiny step across the seam.
        let from = std::f32::consts::PI - 0.1;
        let to = -std::f32::consts::PI + 0.1;
        let mid = lerp_angle(from, to, 0.5);
        // Midpoint should sit on the seam (|PI|), not at 0.
        assert!(mid.abs() > 3.0);
    }
}
