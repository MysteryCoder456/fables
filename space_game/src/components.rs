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
///
/// The client starts in `Connecting` and enters `Playing` when the server's
/// `Welcome` arrives; the server (and its world) enters `Playing` at startup.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GameState {
    #[default]
    Connecting,
    Playing,
}

/// Fixed-timestep simulation phases. Everything gameplay-related runs in
/// `FixedUpdate` inside one of these sets; rendering interpolates between the
/// last two simulation states in `Update`.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SimSet {
    /// Copy `current -> previous` on all interpolated state.
    CachePrevious,
    /// Network ingress: server applies client intents, client applies
    /// server snapshots. Runs before any local simulation this tick.
    NetSync,
    /// Integrate ship physics and orbital motion (gravity + thrust).
    Movement,
    /// Weapons fire, collision resolution, damage, death and respawn.
    Physics,
    /// Mining, cargo transfer, depletion and regeneration.
    Mining,
    /// Network egress: server broadcasts the post-simulation snapshot,
    /// client sends its intent for the next tick.
    PostSim,
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

/// Marker for any player's ship (local or remote). Every connected player
/// owns exactly one.
#[derive(Component, Debug)]
pub struct PlayerShip;

/// Marker for the ship controlled by *this* process (client-side only).
/// HUD and camera follow this one; there is at most one per app.
#[derive(Component, Debug)]
pub struct LocalShip;

/// The player's chosen pilot name; persistence and reconnection key on it.
#[derive(Component, Debug, Clone, Serialize, Deserialize)]
pub struct PlayerName(pub String);

/// Stable network identity shared between server and clients. Entity ids
/// are process-local, so all replication references use `NetId` instead.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NetId(pub u64);

/// Data-driven ship statistics (loaded from `assets/config/game.ron`).
/// Kept as a component so upgrades can later modify a single ship without
/// touching global config.
#[derive(Component, Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// The central star of a system; `system_index` keys it into `GameConfig`.
#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CentralStar {
    pub system_index: usize,
}

/// A planet; `(system_index, config_index)` keys it back to `GameConfig`
/// (and save files).
#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Planet {
    pub system_index: usize,
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
    /// True when this deposit can never yield another whole unit: less than
    /// one unit left and no regeneration.
    pub fn is_exhausted(&self) -> bool {
        self.amount < 1.0 && self.regen_per_sec <= 0.0
    }
}

/// Physical radius of a celestial body or ship: used for mining range
/// (measured from the surface) and collision circles.
#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BodyRadius(pub f32);

/// This body pulls on ships and projectiles: G·M in units³/s².
#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct GravitySource(pub f32);

/// A blaster bolt in flight. Server-simulated (with gravity!), replicated
/// to clients by presence in snapshots.
#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Projectile {
    /// Who fired it (bolts never hit their owner).
    pub owner: NetId,
    pub damage: f32,
    /// Seconds of flight left.
    pub ttl: f32,
}

/// Blaster cooldown bookkeeping (server-side).
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct WeaponCooldown(pub f32);

/// Post-respawn invulnerability window, seconds remaining (server-side).
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct SpawnShield(pub f32);

/// Who hurt this ship last (for kill attribution), server-side.
#[derive(Component, Debug, Default, Clone)]
pub struct LastDamager {
    pub name: Option<String>,
    /// Sim time of the hit; attribution expires after a few seconds.
    pub at: f64,
}

/// A pilot's bank balance.
#[derive(Component, Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct Credits(pub u64);

/// A jump gate: fly into it to be thrown to its partner in another system.
#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Gate {
    pub system_index: usize,
    pub gate_index: usize,
    pub to_system: usize,
    pub to_gate: usize,
}

/// Seconds until this ship may use a gate again (stops instant ping-pong).
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct GateCooldown(pub f32);

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
/// This is exactly the message a client sends to the server each tick.
/// It lives as a component on each ship: the server writes it from the
/// owning client's messages, and simulation systems consume it per-ship.
#[derive(Component, Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlayerIntent {
    /// 0..=1 forward throttle.
    pub thrust: f32,
    /// -1..=1 turn input, positive is counter-clockwise.
    pub turn: f32,
    /// Active braking (extra drag).
    pub brake: bool,
    /// Mining beam engaged.
    pub mine: bool,
    /// Blaster trigger held.
    pub fire: bool,
}

/// The intent gathered from this process's keyboard, before it goes to the
/// server. Client-side only.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct LocalIntent(pub PlayerIntent);

// ---------------------------------------------------------------------------
// Client-side bus (UI <-> network plumbing, no gameplay state)
// ---------------------------------------------------------------------------

/// True while the chat input line is capturing the keyboard; flight and
/// dock keybinds are suppressed.
#[derive(Resource, Debug, Default)]
pub struct ChatTyping(pub bool);

/// UI asks the network layer to send a chat line.
#[derive(Message, Debug, Clone)]
pub struct SendChat(pub String);

/// UI asks the network layer to send a dock action.
#[derive(Message, Debug, Clone, Copy)]
pub struct SendAction(pub crate::protocol::PlayerAction);

/// Chat + system notices shown in the feed panel (newest last).
#[derive(Resource, Debug, Default)]
pub struct Feed(pub std::collections::VecDeque<String>);

impl Feed {
    pub fn push(&mut self, line: String) {
        self.0.push_back(line);
        while self.0.len() > 8 {
            self.0.pop_front();
        }
    }
}

/// Set when our own ship is destroyed; drives the death overlay.
#[derive(Resource, Debug, Default)]
pub struct DeathFlash(pub f32);
