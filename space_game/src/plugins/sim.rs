//! Simulation infrastructure: fixed-timestep scheduling, the sim clock and
//! double-buffering of interpolated state.
//!
//! This plugin (together with the `FixedUpdate` systems registered by the
//! other plugins) is the part of the game a headless server would run.

use bevy::prelude::*;

use crate::components::{GameState, SimClock, SimPosition, SimRotation, SimSet};

pub struct SimulationPlugin;

impl Plugin for SimulationPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<GameState>()
            .init_resource::<SimClock>()
            .configure_sets(
                FixedUpdate,
                (SimSet::CachePrevious, SimSet::Movement, SimSet::Mining)
                    .chain()
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(
                FixedUpdate,
                (advance_sim_clock, cache_previous_state).in_set(SimSet::CachePrevious),
            )
            .add_systems(Update, toggle_pause);
    }
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

fn toggle_pause(
    keys: Res<ButtonInput<KeyCode>>,
    state: Res<State<GameState>>,
    mut next: ResMut<NextState<GameState>>,
) {
    if keys.just_pressed(KeyCode::KeyP) {
        next.set(match state.get() {
            GameState::Playing => GameState::Paused,
            GameState::Paused => GameState::Playing,
        });
    }
}
