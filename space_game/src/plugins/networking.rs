//! Networking scaffold. No sockets yet — this plugin only pins down the
//! seams the future client-server split plugs into. See `ARCHITECTURE.md`
//! ("Multiplayer plan") for the full design.
//!
//! The invariants the rest of the codebase already honors:
//!
//! 1. All gameplay state lives in serializable components/resources
//!    (`SimPosition`, `Velocity`, `Cargo`, `ResourceDeposit`, `SimClock`...)
//!    — exactly what a replication crate like `bevy_replicon` or `lightyear`
//!    needs to snapshot.
//! 2. Simulation runs exclusively in `FixedUpdate` off a `PlayerIntent`
//!    struct; the client's job shrinks to "send intent, render snapshots".
//! 3. Rendering interpolates double-buffered sim state, which is the same
//!    machinery needed to interpolate server snapshots.

use bevy::prelude::*;

pub struct NetworkingPlugin;

impl Plugin for NetworkingPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(NetRole::Standalone);
    }
}

/// Which role this process plays. Today always `Standalone`; a dedicated
/// server build would run the simulation plugins headless as `Server`, and
/// clients would forward `PlayerIntent` to it each tick.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetRole {
    Standalone,
    // Client,
    // Server,
}
