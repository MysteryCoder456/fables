//! Shared ECS components, resources and system sets.
//!
//! Everything in here is *simulation state*: it is deliberately kept
//! serializable (`serde` derives) and independent of rendering so the same
//! components can later live on a headless server. See `ARCHITECTURE.md`.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// App-level state
// ---------------------------------------------------------------------------

/// Top-level game state. Simulation systems only run while `Playing`.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GameState {
    #[default]
    Playing,
    Paused,
}

/// Fixed-timestep simulation phases. Everything gameplay-related runs in
/// `FixedUpdate` inside one of these sets; rendering interpolates between the
/// last two simulation states in `Update`.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SimSet {
    /// Copy `current -> previous` on all interpolated state.
    CachePrevious,
    /// Integrate ship physics and orbital motion.
    Movement,
    /// Mining, cargo transfer, depletion and regeneration.
    Mining,
}

/// Render-side phases in `Update`. These never mutate simulation state.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderSet {
    /// Write interpolated sim state into `Transform`s.
    SyncTransforms,
    /// Camera follow (needs final ship transform).
    Camera,
    /// Starfield, particles, HUD (need final camera transform).
    Decor,
}

// ---------------------------------------------------------------------------
// Simulation clock
// ---------------------------------------------------------------------------

/// Total simulated time in seconds. Advanced only in `FixedUpdate`, saved to
/// disk, and used as the sole input for deterministic orbital positions.
#[derive(Resource, Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct SimClock {
    pub elapsed: f64,
}

// ---------------------------------------------------------------------------
// Interpolated spatial state
// ---------------------------------------------------------------------------

/// Authoritative world-space position, double-buffered for render
/// interpolation. Simulation systems write `current`; the render sync system
/// lerps `previous -> current` by the fixed-timestep overstep fraction.
#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SimPosition {
    pub current: Vec2,
    pub previous: Vec2,
}

impl SimPosition {
    pub fn new(position: Vec2) -> Self {
        Self {
            current: position,
            previous: position,
        }
    }
}

/// Authoritative rotation in radians (0 = facing +X), double-buffered like
/// [`SimPosition`].
#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SimRotation {
    pub current: f32,
    pub previous: f32,
}

impl SimRotation {
    pub fn new(angle: f32) -> Self {
        Self {
            current: angle,
            previous: angle,
        }
    }
}

/// Linear velocity in world units per second.
#[derive(Component, Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct Velocity(pub Vec2);

// ---------------------------------------------------------------------------
// Player ship
// ---------------------------------------------------------------------------

/// Marker for the locally controlled ship. In a networked build every
/// connected player gets one ship entity; only the local one carries input.
#[derive(Component, Debug)]
pub struct PlayerShip;

/// Data-driven ship statistics (loaded from `assets/config/game.ron`).
/// Kept as a component so upgrades can later modify a single ship without
/// touching global config.
#[derive(Component, Debug, Clone, Serialize, Deserialize)]
pub struct ShipStats {
    pub max_hull: f32,
    pub cargo_capacity: u32,
    /// Resource units extracted per second while mining.
    pub mining_power: f32,
    /// Maximum distance at which a deposit can be mined.
    pub mining_range: f32,
    /// Forward acceleration in units/s^2 at full thrust.
    pub thrust_accel: f32,
    /// Turn rate in radians/s.
    pub turn_speed: f32,
    /// Speed clamp in units/s.
    pub max_speed: f32,
    /// Exponential drag coefficient (per second).
    pub drag: f32,
}

/// Current hull integrity.
#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Hull(pub f32);

// ---------------------------------------------------------------------------
// World / celestial bodies
// ---------------------------------------------------------------------------

/// Root entity of one solar system. Stars, planets and asteroids are spawned
/// as children so a second system (reached via a future jump gate) is just
/// another `SolarSystem` entity with its own children.
#[derive(Component, Debug, Clone, Serialize, Deserialize)]
pub struct SolarSystem {
    pub name: String,
}

/// The central star of a system.
#[derive(Component, Debug)]
pub struct CentralStar;

/// A planet; `config_index` keys it back to `GameConfig` (and save files).
#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Planet {
    pub config_index: usize,
}

/// Circular orbit around `center`. Position is a pure function of the
/// [`SimClock`], so orbits never accumulate integration error and need no
/// state saved beyond the clock itself.
#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Orbit {
    pub center: Vec2,
    pub radius: f32,
    /// Radians per second.
    pub angular_speed: f32,
    /// Angle at sim time zero.
    pub phase: f32,
}

/// A mineable rock. `size` doubles as collision/mining radius helper.
#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Asteroid {
    pub size: f32,
}

/// Rotation speed in radians/second (asteroid tumble).
#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Spin(pub f32);

/// A pool of extractable resources attached to an asteroid or planet.
/// Asteroids use `regen_per_sec == 0` and are removed when exhausted;
/// planets regenerate slowly and never despawn.
#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ResourceDeposit {
    pub kind: crate::resource_types::ResourceType,
    pub amount: f32,
    pub max_amount: f32,
    pub regen_per_sec: f32,
}

impl ResourceDeposit {
    pub fn is_exhausted(&self) -> bool {
        self.amount <= f32::EPSILON && self.regen_per_sec <= 0.0
    }
}

/// Physical radius of a celestial body; mining range is measured from the
/// body's surface, not its center.
#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BodyRadius(pub f32);

/// Mining beam state for one ship. `target` is transient (entity ids are not
/// stable across sessions) and intentionally skipped by serde.
#[derive(Component, Debug, Default, Clone, Serialize, Deserialize)]
pub struct MiningRig {
    #[serde(skip)]
    pub target: Option<Entity>,
    /// Fractional progress (0..1) toward the next extracted unit.
    pub progress: f32,
}

/// Player input expressed as *intent*, decoupled from raw key events.
/// This is exactly the message shape a client would send to a server each
/// tick, which is why simulation systems consume this instead of reading
/// `ButtonInput` directly.
#[derive(Resource, Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct PlayerIntent {
    /// 0..=1 forward throttle.
    pub thrust: f32,
    /// -1..=1 turn input, positive is counter-clockwise.
    pub turn: f32,
    /// Active braking (extra drag).
    pub brake: bool,
    /// Mining beam engaged.
    pub mine: bool,
}
