//! Extraction math for the mining beam. Pure functions, unit-tested below.
//!
//! Mining is continuous: the beam extracts `rate` units/second, and whole
//! units transfer to cargo as fractional `progress` rolls past 1.0.

/// Result of advancing the mining beam by one fixed timestep.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MiningTick {
    /// Whole units moved into cargo this tick.
    pub extracted: u32,
    /// Fractional progress toward the next unit (0..1).
    pub progress: f32,
    /// Deposit amount remaining after extraction.
    pub deposit_remaining: f32,
}

/// Advance mining on a deposit.
///
/// * `deposit` — units left in the deposit (fractional).
/// * `progress` — carried-over fractional progress from the previous tick.
/// * `rate` — extraction speed in units/second (ship `mining_power`).
/// * `dt` — fixed timestep in seconds.
/// * `free_space` — cargo slots available; extraction stalls when 0.
pub fn mining_tick(deposit: f32, progress: f32, rate: f32, dt: f32, free_space: u32) -> MiningTick {
    if free_space == 0 || deposit < 1.0 {
        // Beam idles: nothing to take or nowhere to put it. Progress does not
        // bank up while stalled.
        return MiningTick {
            extracted: 0,
            progress: 0.0,
            deposit_remaining: deposit,
        };
    }

    let raw = progress + rate * dt;
    let want = raw as u32;
    let extracted = want.min(free_space).min(deposit as u32);
    // Fractional remainder carries over; excess beyond caps is lost so a full
    // hold can't be used to bank extraction.
    let progress = if extracted == want { raw.fract() } else { 0.0 };

    MiningTick {
        extracted,
        progress,
        deposit_remaining: deposit - extracted as f32,
    }
}

/// Regenerate a deposit toward its maximum (planets only; asteroids have
/// `regen_per_sec == 0`).
pub fn regen_tick(amount: f32, max_amount: f32, regen_per_sec: f32, dt: f32) -> f32 {
    (amount + regen_per_sec * dt).min(max_amount)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_accumulates_into_whole_units() {
        // 2 units/sec at 0.25s ticks: one unit every other tick.
        let t1 = mining_tick(100.0, 0.0, 2.0, 0.25, 50);
        assert_eq!(t1.extracted, 0);
        assert!((t1.progress - 0.5).abs() < 1e-5);

        let t2 = mining_tick(t1.deposit_remaining, t1.progress, 2.0, 0.25, 50);
        assert_eq!(t2.extracted, 1);
        assert!(t2.progress < 1e-5);
        assert!((t2.deposit_remaining - 99.0).abs() < 1e-5);
    }

    #[test]
    fn extraction_capped_by_free_space() {
        let tick = mining_tick(100.0, 0.9, 40.0, 0.5, 3);
        assert_eq!(tick.extracted, 3, "only 3 slots free");
        assert_eq!(tick.progress, 0.0, "capped extraction discards surplus");
        assert!((tick.deposit_remaining - 97.0).abs() < 1e-5);
    }

    #[test]
    fn extraction_capped_by_deposit() {
        let tick = mining_tick(2.4, 0.0, 100.0, 1.0, 50);
        assert_eq!(tick.extracted, 2);
        assert!(tick.deposit_remaining >= 0.0);
        assert!(tick.deposit_remaining < 1.0);
    }

    #[test]
    fn full_cargo_stalls_the_beam() {
        let tick = mining_tick(100.0, 0.7, 10.0, 1.0, 0);
        assert_eq!(tick.extracted, 0);
        assert_eq!(tick.progress, 0.0);
        assert!((tick.deposit_remaining - 100.0).abs() < 1e-5);
    }

    #[test]
    fn near_empty_deposit_is_not_minable() {
        let tick = mining_tick(0.6, 0.5, 10.0, 1.0, 50);
        assert_eq!(tick.extracted, 0);
        assert!((tick.deposit_remaining - 0.6).abs() < 1e-5);
    }

    #[test]
    fn total_extracted_never_exceeds_deposit() {
        let mut deposit = 10.0;
        let mut progress = 0.0;
        let mut total = 0;
        for _ in 0..1000 {
            let tick = mining_tick(deposit, progress, 3.0, 0.1, 50);
            deposit = tick.deposit_remaining;
            progress = tick.progress;
            total += tick.extracted;
        }
        assert_eq!(total, 10);
        assert!(deposit < 1.0);
        assert!(deposit >= 0.0);
    }

    #[test]
    fn regen_caps_at_max() {
        assert!((regen_tick(99.9, 100.0, 1.0, 1.0) - 100.0).abs() < 1e-5);
        assert!((regen_tick(50.0, 100.0, 0.5, 2.0) - 51.0).abs() < 1e-5);
    }
}
