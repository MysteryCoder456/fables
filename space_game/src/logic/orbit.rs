//! Circular orbit math. Pure functions, unit-tested below.
//!
//! Planet positions are a deterministic function of the simulation clock,
//! which makes them trivially reproducible on a server and free to persist
//! (only the clock needs saving).

use bevy::math::Vec2;

/// Angular speed (radians/second) for a full revolution every `period` seconds.
pub fn angular_speed(period: f32) -> f32 {
    std::f32::consts::TAU / period
}

/// Position on a circular orbit around `center` at simulation time `t`.
pub fn orbit_position(center: Vec2, radius: f32, angular_speed: f32, phase: f32, t: f64) -> Vec2 {
    // Keep the angle computation in f64: t grows without bound and
    // `angular_speed * t` would lose precision in f32 over long sessions.
    let angle = (angular_speed as f64 * t + phase as f64) % std::f64::consts::TAU;
    center + Vec2::from_angle(angle as f32) * radius
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_phase_angle() {
        let pos = orbit_position(Vec2::ZERO, 100.0, 1.0, 0.0, 0.0);
        assert!((pos - Vec2::new(100.0, 0.0)).length() < 1e-4);

        let pos = orbit_position(Vec2::ZERO, 100.0, 1.0, std::f32::consts::FRAC_PI_2, 0.0);
        assert!((pos - Vec2::new(0.0, 100.0)).length() < 1e-3);
    }

    #[test]
    fn radius_is_preserved() {
        for t in [0.0, 10.0, 1234.5, 1e6] {
            let pos = orbit_position(Vec2::new(50.0, -20.0), 300.0, 0.02, 1.0, t);
            let r = (pos - Vec2::new(50.0, -20.0)).length();
            assert!((r - 300.0).abs() < 0.1, "radius drifted to {r} at t={t}");
        }
    }

    #[test]
    fn full_period_returns_to_start() {
        let period = 240.0;
        let speed = angular_speed(period);
        let start = orbit_position(Vec2::ZERO, 500.0, speed, 0.7, 0.0);
        let after = orbit_position(Vec2::ZERO, 500.0, speed, 0.7, period as f64);
        assert!((start - after).length() < 0.5);
    }

    #[test]
    fn orbits_counter_clockwise() {
        let a = orbit_position(Vec2::ZERO, 100.0, 0.1, 0.0, 0.0);
        let b = orbit_position(Vec2::ZERO, 100.0, 0.1, 0.0, 1.0);
        // Cross product z of (a x b) positive => counter-clockwise motion.
        assert!(a.perp_dot(b) > 0.0);
    }

    #[test]
    fn precision_holds_after_long_sim_times() {
        // A week of continuous sim time.
        let t = 7.0 * 24.0 * 3600.0;
        let pos = orbit_position(Vec2::ZERO, 1000.0, angular_speed(180.0), 0.0, t);
        assert!((pos.length() - 1000.0).abs() < 0.5);
    }
}
