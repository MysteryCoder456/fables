//! Simulation infrastructure: fixed-timestep scheduling, the sim clock and
//! double-buffering of interpolated state.
//!
//! This plugin is shared by the server (authoritative simulation) and the
//! client (decorative simulation of deterministic state plus snapshot
//! application); the network plugins hang their systems off [`SimSet`].

use bevy::prelude::*;

use crate::components::{GameState, SimClock, SimPosition, SimRotation, SimSet};

pub struct SimulationPlugin;

impl Plugin for SimulationPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<GameState>()
            .init_resource::<SimClock>()
            .configure_sets(
                FixedUpdate,
                (
                    SimSet::CachePrevious,
                    SimSet::NetSync,
                    SimSet::Movement,
                    SimSet::Mining,
                    SimSet::PostSim,
                )
                    .chain()
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(
                FixedUpdate,
                (advance_sim_clock, cache_previous_state).in_set(SimSet::CachePrevious),
            );
    }
}

/// Startup helper for apps that are immediately live (the server): the
/// client instead enters `Playing` when the server welcomes it.
pub fn enter_playing(mut next: ResMut<NextState<GameState>>) {
    next.set(GameState::Playing);
}

fn advance_sim_clock(time: Res<Time>, mut clock: ResMut<SimClock>) {
    clock.elapsed += time.delta_secs_f64();
}

/// Copy `current -> previous` on every interpolated entity before any
/// simulation system moves things this tick.
fn cache_previous_state(
    mut positions: Query<&mut SimPosition>,
    mut rotations: Query<&mut SimRotation>,
) {
    for mut pos in &mut positions {
        pos.previous = pos.current;
    }
    for mut rot in &mut rotations {
        rot.previous = rot.current;
    }
}
