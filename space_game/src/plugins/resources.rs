//! Resource collection: mining beam targeting, extraction, asteroid
//! depletion and planetary regeneration. All simulation-side.

use bevy::prelude::*;

use crate::components::{
    BodyRadius, MiningRig, PlayerIntent, PlayerShip, ResourceDeposit, ShipStats, SimPosition,
    SimSet,
};
use crate::logic::cargo::Cargo;
use crate::logic::mining::{mining_tick, regen_tick};
use crate::resource_types::ResourceType;

pub struct ResourcesPlugin;

impl Plugin for ResourcesPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ResourceMined>()
            .add_message::<DepositExhausted>()
            .add_systems(
                FixedUpdate,
                (mine_deposits, deplete_asteroids, regenerate_deposits)
                    .chain()
                    .in_set(SimSet::Mining),
            );
    }
}

/// Sent whenever units land in a cargo hold (drives HUD/particle feedback).
#[derive(Message, Debug, Clone, Copy)]
pub struct ResourceMined {
    pub kind: ResourceType,
    pub amount: u32,
}

/// Sent when an asteroid runs dry and despawns.
#[derive(Message, Debug, Clone, Copy)]
pub struct DepositExhausted {
    pub position: Vec2,
    pub kind: ResourceType,
}

/// Acquire a target and extract resources while the mine intent is held.
fn mine_deposits(
    time: Res<Time>,
    intent: Res<PlayerIntent>,
    mut ships: Query<(&SimPosition, &ShipStats, &mut MiningRig, &mut Cargo), With<PlayerShip>>,
    mut deposits: Query<(Entity, &SimPosition, &BodyRadius, &mut ResourceDeposit)>,
    mut mined_messages: MessageWriter<ResourceMined>,
) {
    let dt = time.delta_secs();

    for (ship_pos, stats, mut rig, mut cargo) in &mut ships {
        if !intent.mine {
            rig.target = None;
            rig.progress = 0.0;
            continue;
        }

        // Re-validate the current target, otherwise pick the nearest deposit
        // whose surface is within mining range.
        let target = find_target(ship_pos.current, stats.mining_range, &rig, &deposits);
        if target != rig.target {
            rig.progress = 0.0;
            rig.target = target;
        }

        let Some(target_entity) = rig.target else {
            continue;
        };
        let Ok((_, _, _, mut deposit)) = deposits.get_mut(target_entity) else {
            rig.target = None;
            continue;
        };

        let tick = mining_tick(
            deposit.amount,
            rig.progress,
            stats.mining_power,
            dt,
            cargo.free_space(),
        );
        rig.progress = tick.progress;
        if tick.extracted > 0 {
            let stored = cargo.add(deposit.kind, tick.extracted);
            debug_assert_eq!(
                stored, tick.extracted,
                "mining_tick already caps at free space"
            );
            deposit.amount = tick.deposit_remaining;
            mined_messages.write(ResourceMined {
                kind: deposit.kind,
                amount: stored,
            });
        }
    }
}

fn find_target(
    ship: Vec2,
    range: f32,
    rig: &MiningRig,
    deposits: &Query<(Entity, &SimPosition, &BodyRadius, &mut ResourceDeposit)>,
) -> Option<Entity> {
    let in_range = |entity: Entity| -> Option<(Entity, f32)> {
        let (_, pos, radius, deposit) = deposits.get(entity).ok()?;
        if deposit.is_exhausted() {
            return None;
        }
        let surface_distance = ship.distance(pos.current) - radius.0;
        (surface_distance <= range).then_some((entity, surface_distance))
    };

    // Keep the existing target while it stays valid to avoid beam flicker
    // between two equally close rocks.
    if let Some(current) = rig.target {
        if in_range(current).is_some() {
            return Some(current);
        }
    }

    deposits
        .iter()
        .filter_map(|(entity, ..)| in_range(entity))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(entity, _)| entity)
}

/// Remove asteroids whose deposit ran dry (planets regenerate instead).
fn deplete_asteroids(
    mut commands: Commands,
    deposits: Query<(Entity, &SimPosition, &ResourceDeposit)>,
    mut exhausted_messages: MessageWriter<DepositExhausted>,
) {
    for (entity, pos, deposit) in &deposits {
        if deposit.is_exhausted() {
            exhausted_messages.write(DepositExhausted {
                position: pos.current,
                kind: deposit.kind,
            });
            commands.entity(entity).despawn();
        }
    }
}

/// Planetary pools slowly refill toward their maximum.
fn regenerate_deposits(time: Res<Time>, mut deposits: Query<&mut ResourceDeposit>) {
    let dt = time.delta_secs();
    for mut deposit in &mut deposits {
        if deposit.regen_per_sec > 0.0 && deposit.amount < deposit.max_amount {
            deposit.amount = regen_tick(
                deposit.amount,
                deposit.max_amount,
                deposit.regen_per_sec,
                dt,
            );
        }
    }
}
