//! Circle collision resolution with restitution. Pure functions,
//! unit-tested below.
//!
//! Two cases cover everything in the game:
//! - a ship (or projectile) against a massive body (star, planet, asteroid):
//!   the body is unaffected, the ship bounces;
//! - ship against ship: equal masses exchange momentum along the contact
//!   normal.
//!
//! Every resolution reports the normal impact speed so callers can convert
//! hard hits into hull damage.

use bevy::math::Vec2;

/// Outcome of resolving one colliding object.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CollisionResult {
    pub position: Vec2,
    pub velocity: Vec2,
    /// Closing speed along the contact normal at impact (>= 0).
    pub impact_speed: f32,
}

/// Collide a moving circle against a massive body (unaffected by the hit).
/// Returns `None` when the circles don't overlap.
///
/// `body_velocity` matters for orbiting planets: damage and bounce are
/// computed in the body's rest frame.
pub fn collide_with_body(
    position: Vec2,
    velocity: Vec2,
    radius: f32,
    body_center: Vec2,
    body_radius: f32,
    body_velocity: Vec2,
    restitution: f32,
) -> Option<CollisionResult> {
    let offset = position - body_center;
    let distance = offset.length();
    let min_distance = radius + body_radius;
    if distance >= min_distance {
        return None;
    }
    // Degenerate overlap (exactly centered): push out along +X.
    let normal = if distance > f32::EPSILON {
        offset / distance
    } else {
        Vec2::X
    };

    let relative = velocity - body_velocity;
    let closing = -relative.dot(normal); // > 0 when moving into the body
    let impact_speed = closing.max(0.0);

    let mut new_relative = relative;
    if closing > 0.0 {
        // Reflect the normal component, keep the tangential component.
        new_relative += normal * (1.0 + restitution) * closing;
    }

    Some(CollisionResult {
        // Push out to the surface so the pair never stays interpenetrated.
        position: body_center + normal * min_distance,
        velocity: new_relative + body_velocity,
        impact_speed,
    })
}

/// Collide two equal-mass ships. Returns `None` when they don't overlap.
pub fn collide_ships(
    pos_a: Vec2,
    vel_a: Vec2,
    pos_b: Vec2,
    vel_b: Vec2,
    radius: f32,
    restitution: f32,
) -> Option<(CollisionResult, CollisionResult)> {
    let offset = pos_a - pos_b;
    let distance = offset.length();
    let min_distance = radius * 2.0;
    if distance >= min_distance {
        return None;
    }
    let normal = if distance > f32::EPSILON {
        offset / distance
    } else {
        Vec2::X
    };

    let relative = vel_a - vel_b;
    let closing = -relative.dot(normal);
    let impact_speed = closing.max(0.0);

    // Equal masses: exchange the normal component, scaled by restitution.
    let impulse = if closing > 0.0 {
        normal * (1.0 + restitution) * closing / 2.0
    } else {
        Vec2::ZERO
    };

    // Separate symmetrically around the midpoint.
    let midpoint = (pos_a + pos_b) / 2.0;
    let half_gap = normal * min_distance / 2.0;

    Some((
        CollisionResult {
            position: midpoint + half_gap,
            velocity: vel_a + impulse,
            impact_speed,
        },
        CollisionResult {
            position: midpoint - half_gap,
            velocity: vel_b - impulse,
            impact_speed,
        },
    ))
}

/// Hull damage for a given normal impact speed: free below the threshold,
/// linear above it.
pub fn impact_damage(impact_speed: f32, min_impact: f32, damage_scale: f32) -> f32 {
    ((impact_speed - min_impact) * damage_scale).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_overlap_no_collision() {
        assert!(collide_with_body(
            Vec2::new(100.0, 0.0),
            Vec2::new(-50.0, 0.0),
            10.0,
            Vec2::ZERO,
            50.0,
            Vec2::ZERO,
            0.5,
        )
        .is_none());
    }

    #[test]
    fn head_on_bounce_reflects_velocity() {
        let hit = collide_with_body(
            Vec2::new(55.0, 0.0),
            Vec2::new(-100.0, 0.0),
            10.0,
            Vec2::ZERO,
            50.0,
            Vec2::ZERO,
            0.5,
        )
        .expect("should collide");
        assert!((hit.impact_speed - 100.0).abs() < 1e-3);
        // Reflected with restitution 0.5: -100 -> +50.
        assert!((hit.velocity.x - 50.0).abs() < 1e-3);
        assert!(hit.velocity.y.abs() < 1e-4);
        // Pushed out to the combined radius.
        assert!((hit.position.length() - 60.0).abs() < 1e-3);
    }

    #[test]
    fn tangential_graze_deals_no_impact() {
        // Moving parallel to the surface while slightly overlapped.
        let hit = collide_with_body(
            Vec2::new(59.0, 0.0),
            Vec2::new(0.0, 200.0),
            10.0,
            Vec2::ZERO,
            50.0,
            Vec2::ZERO,
            0.5,
        )
        .expect("overlapping");
        assert!(hit.impact_speed < 1e-3, "no closing speed, no impact");
        // Tangential velocity untouched.
        assert!((hit.velocity.y - 200.0).abs() < 1e-3);
        // Still separated.
        assert!((hit.position.length() - 60.0).abs() < 1e-3);
    }

    #[test]
    fn moving_body_frame_is_respected() {
        // Ship at rest, planet sweeping into it at 100 u/s.
        let hit = collide_with_body(
            Vec2::new(55.0, 0.0),
            Vec2::ZERO,
            10.0,
            Vec2::ZERO,
            50.0,
            Vec2::new(100.0, 0.0),
            0.5,
        )
        .expect("should collide");
        assert!((hit.impact_speed - 100.0).abs() < 1e-3);
        // Ship gets carried away faster than the planet moves.
        assert!(hit.velocity.x > 100.0);
    }

    #[test]
    fn ship_collision_conserves_momentum() {
        let (a, b) = collide_ships(
            Vec2::new(-10.0, 0.0),
            Vec2::new(100.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(-100.0, 0.0),
            14.0,
            1.0,
        )
        .expect("should collide");
        // Perfectly elastic head-on with equal masses: velocities swap.
        assert!((a.velocity.x + 100.0).abs() < 1e-3);
        assert!((b.velocity.x - 100.0).abs() < 1e-3);
        // Momentum sum unchanged (was zero).
        assert!((a.velocity + b.velocity).length() < 1e-3);
        // Separated to two radii.
        assert!((a.position - b.position).length() >= 28.0 - 1e-3);
        assert!((a.impact_speed - 200.0).abs() < 1e-3);
    }

    #[test]
    fn separating_ships_get_no_impulse() {
        // Overlapping but already flying apart.
        let (a, b) = collide_ships(
            Vec2::new(-5.0, 0.0),
            Vec2::new(-50.0, 0.0),
            Vec2::new(5.0, 0.0),
            Vec2::new(50.0, 0.0),
            14.0,
            0.8,
        )
        .expect("overlapping");
        assert!(a.impact_speed < 1e-3);
        assert!((a.velocity.x + 50.0).abs() < 1e-3, "velocity unchanged");
        assert!((b.velocity.x - 50.0).abs() < 1e-3);
    }

    #[test]
    fn impact_damage_has_free_threshold() {
        assert_eq!(impact_damage(50.0, 90.0, 0.18), 0.0);
        assert!((impact_damage(190.0, 90.0, 0.18) - 18.0).abs() < 1e-4);
    }
}
