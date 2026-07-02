//! Math for the wrapping parallax starfield.

use bevy::math::Vec2;

/// Wrap `value` into the half-open interval [-size/2, size/2).
pub fn wrap_centered(value: f32, size: f32) -> f32 {
    let wrapped = value - size * (value / size).round();
    if wrapped >= size / 2.0 {
        wrapped - size
    } else {
        wrapped
    }
}

/// Compute the on-screen world position of a background star.
///
/// `factor` in (0, 1): 0 = pinned to world space (max parallax), values close
/// to 1 = far away (moves almost with the camera). The star is wrapped into a
/// `tile`-sized square centered on the camera so a finite set of stars covers
/// an infinite world.
pub fn star_position(base: Vec2, camera: Vec2, factor: f32, tile: f32) -> Vec2 {
    let apparent = base + camera * factor;
    Vec2::new(
        camera.x + wrap_centered(apparent.x - camera.x, tile),
        camera.y + wrap_centered(apparent.y - camera.y, tile),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_centered_stays_in_range() {
        for v in [-1e6, -1234.5, -50.0, 0.0, 49.9, 51.0, 99999.0] {
            let w = wrap_centered(v, 100.0);
            assert!((-50.0..50.0).contains(&w), "wrap({v}) = {w} out of range");
        }
    }

    #[test]
    fn wrap_centered_is_identity_inside_range() {
        assert!((wrap_centered(30.0, 100.0) - 30.0).abs() < 1e-5);
        assert!((wrap_centered(-49.0, 100.0) + 49.0).abs() < 1e-5);
    }

    #[test]
    fn star_stays_near_camera() {
        let tile = 4000.0;
        for cam_x in [-100_000.0f32, 0.0, 12_345.0] {
            let cam = Vec2::new(cam_x, cam_x * 0.5);
            let pos = star_position(Vec2::new(1000.0, -700.0), cam, 0.8, tile);
            assert!((pos.x - cam.x).abs() <= tile / 2.0);
            assert!((pos.y - cam.y).abs() <= tile / 2.0);
        }
    }

    #[test]
    fn far_layer_moves_less_relative_to_world() {
        // A star with factor close to 1 should barely move in screen space
        // (position relative to camera) when the camera moves by a small step.
        let base = Vec2::new(100.0, 100.0);
        let cam_a = Vec2::ZERO;
        let cam_b = Vec2::new(100.0, 0.0);
        let far_a = star_position(base, cam_a, 0.95, 10_000.0) - cam_a;
        let far_b = star_position(base, cam_b, 0.95, 10_000.0) - cam_b;
        let near_a = star_position(base, cam_a, 0.4, 10_000.0) - cam_a;
        let near_b = star_position(base, cam_b, 0.4, 10_000.0) - cam_b;
        let far_shift = (far_a - far_b).length();
        let near_shift = (near_a - near_b).length();
        assert!(far_shift < near_shift);
    }
}
