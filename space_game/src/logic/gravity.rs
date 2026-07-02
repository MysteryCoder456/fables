//! Newtonian point-mass gravity. Pure functions, unit-tested below.
//!
//! Stars and planets exert inverse-square gravity on ships and projectiles.
//! Planets themselves stay on their configured orbits (treat them as the
//! long-term solution of the star's field); asteroids are too small to care.

use bevy::math::Vec2;

/// A gravity source: `mu` is the standard gravitational parameter (G·M) in
/// world units³/s².
#[derive(Debug, Clone, Copy)]
pub struct GravityBody {
    pub center: Vec2,
    pub mu: f32,
    /// Physical radius; the field is clamped at the surface so objects
    /// inside a body (during collision resolution) don't see a singularity.
    pub radius: f32,
}

/// Total gravitational acceleration at `position`.
pub fn gravity_accel(position: Vec2, bodies: &[GravityBody]) -> Vec2 {
    let mut accel = Vec2::ZERO;
    for body in bodies {
        let offset = body.center - position;
        let distance = offset.length().max(body.radius);
        if distance <= f32::EPSILON {
            continue;
        }
        accel += offset / distance * (body.mu / (distance * distance));
    }
    accel
}

/// Speed of a circular orbit at `radius` around a body with parameter `mu`.
/// (Used by tests and handy for tuning config values.)
pub fn circular_orbit_speed(mu: f32, radius: f32) -> f32 {
    (mu / radius).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAR: GravityBody = GravityBody {
        center: Vec2::ZERO,
        mu: 6.0e7,
        radius: 300.0,
    };

    #[test]
    fn pulls_toward_the_body() {
        let accel = gravity_accel(Vec2::new(1000.0, 0.0), &[STAR]);
        assert!(accel.x < 0.0, "should pull toward the origin");
        assert!(accel.y.abs() < 1e-4);
    }

    #[test]
    fn inverse_square_falloff() {
        let near = gravity_accel(Vec2::new(1000.0, 0.0), &[STAR]).length();
        let far = gravity_accel(Vec2::new(2000.0, 0.0), &[STAR]).length();
        assert!(
            (near / far - 4.0).abs() < 1e-3,
            "double distance = quarter pull"
        );
    }

    #[test]
    fn interior_field_is_finite_and_linear() {
        // Inside the body the formula degrades to the uniform-sphere interior
        // solution: field grows linearly with r (Newton's shell theorem),
        // never a singularity.
        let at_surface = gravity_accel(Vec2::new(STAR.radius, 0.0), &[STAR]).length();
        let half_way = gravity_accel(Vec2::new(STAR.radius / 2.0, 0.0), &[STAR]).length();
        let near_center = gravity_accel(Vec2::new(1.0, 0.0), &[STAR]).length();
        assert!(half_way <= at_surface, "no singularity inside the body");
        assert!(
            (half_way - at_surface / 2.0).abs() < 1e-3,
            "linear interior falloff"
        );
        assert!(near_center < at_surface * 0.01);
    }

    #[test]
    fn fields_superpose() {
        let other = GravityBody {
            center: Vec2::new(2000.0, 0.0),
            mu: 1.0e5,
            radius: 50.0,
        };
        let combined = gravity_accel(Vec2::new(1000.0, 0.0), &[STAR, other]);
        let separate = gravity_accel(Vec2::new(1000.0, 0.0), &[STAR])
            + gravity_accel(Vec2::new(1000.0, 0.0), &[other]);
        assert!((combined - separate).length() < 1e-4);
    }

    #[test]
    fn circular_orbit_is_self_consistent() {
        // Centripetal acceleration v²/r must equal gravity at that radius.
        let r = 1400.0;
        let v = circular_orbit_speed(STAR.mu, r);
        let g = gravity_accel(Vec2::new(r, 0.0), &[STAR]).length();
        assert!((v * v / r - g).abs() / g < 1e-4);
    }
}
