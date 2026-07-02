//! Client-side networking: connects to the server, builds the world from
//! its `Welcome`, applies per-tick snapshots, and sends local intent.
//!
//! The client runs no gameplay simulation. It renders replicated state,
//! animates the deterministic parts locally (orbits from the replicated sim
//! clock, asteroid tumble from spin rates), and interpolates between
//! snapshots with the same double-buffer machinery used since day one.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::net::TcpStream;
use std::time::Duration;

use bevy::prelude::*;
use crossbeam_channel::{Receiver, Sender};

use crate::components::{
    Hull, LocalIntent, LocalShip, MiningRig, NetId, PlayerIntent, PlayerName, PlayerShip,
    ResourceDeposit, ShipStats, SimClock, SimPosition, SimRotation, SimSet, Velocity,
};
use crate::config::GameConfig;
use crate::logic::cargo::Cargo;
use crate::plugins::resources::{DepositExhausted, ResourceMined};
use crate::plugins::world::{
    spawn_asteroid, spawn_planet, spawn_star, spawn_system_root, AsteroidInit,
};
use crate::protocol::{
    encode_frame, read_frame, write_frame, ClientToServer, ServerToClient, ShipState, Snapshot,
    PROTOCOL_VERSION,
};

const CONNECT_ATTEMPTS: u32 = 10;
const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(500);

pub struct ClientNetPlugin {
    pub server_addr: String,
    pub player_name: String,
}

impl Plugin for ClientNetPlugin {
    fn build(&self, app: &mut App) {
        let net = connect(&self.server_addr, &self.player_name);
        app.insert_resource(net)
            .init_resource::<NetMap>()
            .init_resource::<SnapshotBuffer>()
            .insert_resource(MyShip(None))
            // The client re-emits these from snapshot events for the
            // effects systems (no ResourcesPlugin on this side).
            .add_message::<ResourceMined>()
            .add_message::<DepositExhausted>()
            .add_systems(PreUpdate, drain_incoming)
            .add_systems(
                FixedUpdate,
                (
                    apply_snapshots.in_set(SimSet::NetSync),
                    send_intent.in_set(SimSet::PostSim),
                ),
            );
    }
}

enum ClientEvent {
    Message(ServerToClient),
    Disconnected(String),
}

#[derive(Resource)]
struct ClientNet {
    incoming: Receiver<ClientEvent>,
    outgoing: Sender<ClientToServer>,
}

/// NetId -> local entity, split by kind so ship lifecycle (which is driven
/// by snapshot presence) can't collide with world objects.
#[derive(Resource, Default)]
struct NetMap {
    objects: HashMap<NetId, Entity>,
    ships: HashMap<NetId, Entity>,
}

/// The `NetId` of the ship the server assigned to this client.
#[derive(Resource)]
struct MyShip(Option<NetId>);

/// Snapshots received since the last fixed tick. TCP keeps them ordered;
/// all are applied each tick (events from every one, ship state from the
/// newest) so backlog self-corrects.
#[derive(Resource, Default)]
struct SnapshotBuffer(VecDeque<Snapshot>);

// ---------------------------------------------------------------------------
// Socket threads
// ---------------------------------------------------------------------------

fn connect(addr: &str, name: &str) -> ClientNet {
    let mut stream = None;
    for attempt in 1..=CONNECT_ATTEMPTS {
        match TcpStream::connect(addr) {
            Ok(connected) => {
                stream = Some(connected);
                break;
            }
            Err(err) => {
                warn!("connect to {addr} failed (attempt {attempt}/{CONNECT_ATTEMPTS}): {err}");
                std::thread::sleep(CONNECT_RETRY_DELAY);
            }
        }
    }
    let stream = stream.unwrap_or_else(|| {
        panic!("could not reach server at {addr} — is space_game_server running?")
    });
    let _ = stream.set_nodelay(true);
    info!("connected to server at {addr} as '{name}'");

    let (incoming_tx, incoming_rx) = crossbeam_channel::unbounded();
    let (outgoing_tx, outgoing_rx) = crossbeam_channel::unbounded::<ClientToServer>();

    let mut write_half = stream.try_clone().expect("failed to clone stream");
    let hello = ClientToServer::Hello {
        protocol: PROTOCOL_VERSION,
        name: name.into(),
    };
    let frame = encode_frame(&hello).expect("failed to encode hello");
    write_frame(&mut write_half, &frame).expect("failed to send hello");

    std::thread::Builder::new()
        .name("net-write".into())
        .spawn(move || {
            while let Ok(message) = outgoing_rx.recv() {
                let Ok(frame) = encode_frame(&message) else {
                    continue;
                };
                if write_frame(&mut write_half, &frame).is_err() {
                    break;
                }
            }
        })
        .expect("failed to spawn writer thread");

    std::thread::Builder::new()
        .name("net-read".into())
        .spawn(move || {
            let mut read_half = stream;
            loop {
                match read_frame::<ServerToClient>(&mut read_half) {
                    Ok(message) => {
                        if incoming_tx.send(ClientEvent::Message(message)).is_err() {
                            break;
                        }
                    }
                    Err(err) => {
                        let _ = incoming_tx.send(ClientEvent::Disconnected(err.to_string()));
                        break;
                    }
                }
            }
        })
        .expect("failed to spawn reader thread");

    ClientNet {
        incoming: incoming_rx,
        outgoing: outgoing_tx,
    }
}

// ---------------------------------------------------------------------------
// Ingress
// ---------------------------------------------------------------------------

/// Drain the socket channel every frame: control messages act immediately,
/// snapshots queue for the fixed-timestep application.
#[allow(clippy::too_many_arguments)]
fn drain_incoming(
    mut commands: Commands,
    net: Res<ClientNet>,
    config: Res<GameConfig>,
    mut clock: ResMut<SimClock>,
    mut map: ResMut<NetMap>,
    mut my_ship: ResMut<MyShip>,
    mut buffer: ResMut<SnapshotBuffer>,
    mut next_state: ResMut<NextState<crate::components::GameState>>,
    mut exit: MessageWriter<AppExit>,
) {
    while let Ok(event) = net.incoming.try_recv() {
        match event {
            ClientEvent::Message(ServerToClient::Welcome {
                your_ship,
                sim_elapsed,
                planets,
                asteroids,
            }) => {
                info!(
                    "welcome: ship {your_ship:?}, {} planets, {} asteroids",
                    planets.len(),
                    asteroids.len()
                );
                clock.elapsed = sim_elapsed;
                my_ship.0 = Some(your_ship);

                // Build the world locally: layout comes from shared config,
                // dynamic state (deposit levels, surviving asteroids) from
                // the server.
                let system = spawn_system_root(&mut commands, &config.system.name);
                spawn_star(&mut commands, system, &config.system.star);
                for init in &planets {
                    let index = init.config_index as usize;
                    let Some(cfg) = config.system.planets.get(index) else {
                        warn!("server sent planet {index} not in local config; skipping");
                        continue;
                    };
                    let entity = spawn_planet(
                        &mut commands,
                        system,
                        index,
                        cfg,
                        init.net_id,
                        init.deposit_amount,
                        sim_elapsed,
                    );
                    map.objects.insert(init.net_id, entity);
                }
                for init in &asteroids {
                    let entity = spawn_asteroid(
                        &mut commands,
                        system,
                        AsteroidInit {
                            net_id: init.net_id,
                            position: init.position,
                            rotation: init.rotation,
                            size: init.size,
                            spin: init.spin,
                            kind: init.kind,
                            amount: init.amount,
                            max_amount: init.max_amount,
                        },
                    );
                    map.objects.insert(init.net_id, entity);
                }
                next_state.set(crate::components::GameState::Playing);
            }
            ClientEvent::Message(ServerToClient::Snapshot(snapshot)) => {
                buffer.0.push_back(snapshot);
            }
            ClientEvent::Message(ServerToClient::Reject { reason }) => {
                error!("server rejected connection: {reason}");
                exit.write(AppExit::error());
            }
            ClientEvent::Disconnected(reason) => {
                error!("lost connection to server: {reason}");
                exit.write(AppExit::error());
            }
        }
    }
}

/// Apply buffered snapshots at the fixed tick: events from every snapshot,
/// authoritative ship state from the newest.
#[allow(clippy::too_many_arguments)]
fn apply_snapshots(
    mut commands: Commands,
    mut buffer: ResMut<SnapshotBuffer>,
    mut map: ResMut<NetMap>,
    my_ship: Res<MyShip>,
    mut clock: ResMut<SimClock>,
    mut ships: Query<(
        &mut SimPosition,
        &mut SimRotation,
        &mut Velocity,
        &mut Hull,
        &mut ShipStats,
        &mut Cargo,
        &mut PlayerIntent,
        &mut MiningRig,
    )>,
    mut mined_messages: MessageWriter<ResourceMined>,
    mut exhausted_messages: MessageWriter<DepositExhausted>,
    mut deposits: Query<&mut ResourceDeposit>,
) {
    if buffer.0.is_empty() {
        return;
    }

    // Events from every snapshot (they're one-shot and must not be lost).
    let mut snapshots: Vec<Snapshot> = buffer.0.drain(..).collect();
    for snapshot in &snapshots {
        for update in &snapshot.deposit_updates {
            if let Some(&entity) = map.objects.get(&update.net_id) {
                if let Ok(mut deposit) = deposits.get_mut(entity) {
                    deposit.amount = update.amount;
                }
            }
        }
        for event in &snapshot.mined {
            if let Some(&ship) = map.ships.get(&event.ship) {
                mined_messages.write(ResourceMined {
                    ship,
                    kind: event.kind,
                    amount: event.amount,
                });
            }
        }
        for event in &snapshot.exhausted {
            if let Some(entity) = map.objects.remove(&event.net_id) {
                commands.entity(entity).despawn();
            }
            exhausted_messages.write(DepositExhausted {
                net_id: event.net_id,
                position: event.position,
                kind: event.kind,
            });
        }
    }

    // Ship state from the newest snapshot only.
    let newest = snapshots.pop().expect("buffer was non-empty");
    clock.elapsed = newest.sim_elapsed;

    let mut seen: Vec<NetId> = Vec::with_capacity(newest.ships.len());
    for state in &newest.ships {
        seen.push(state.net_id);
        match map.ships.get(&state.net_id) {
            Some(&entity) => {
                if let Ok((
                    mut pos,
                    mut rot,
                    mut vel,
                    mut hull,
                    mut stats,
                    mut cargo,
                    mut intent,
                    mut rig,
                )) = ships.get_mut(entity)
                {
                    // `previous` was already cached this tick; writing only
                    // `current` keeps render interpolation seamless.
                    pos.current = state.position;
                    rot.current = state.rotation;
                    vel.0 = state.velocity;
                    hull.0 = state.hull;
                    if *stats != state.stats {
                        *stats = state.stats.clone();
                    }
                    if *cargo != state.cargo {
                        *cargo = state.cargo.clone();
                    }
                    *intent = state.intent;
                    rig.target = state
                        .mining_target
                        .and_then(|net_id| map.objects.get(&net_id).copied());
                    rig.progress = state.mining_progress;
                }
            }
            None => {
                let entity = spawn_replicated_ship(&mut commands, state, &map);
                if my_ship.0 == Some(state.net_id) {
                    commands.entity(entity).insert(LocalShip);
                }
                map.ships.insert(state.net_id, entity);
            }
        }
    }

    // Ships absent from the snapshot have disconnected.
    let mut removed: Vec<NetId> = Vec::new();
    for (&net_id, &entity) in &map.ships {
        if !seen.contains(&net_id) {
            commands.entity(entity).despawn();
            removed.push(net_id);
        }
    }
    for net_id in removed {
        map.ships.remove(&net_id);
    }
}

fn spawn_replicated_ship(commands: &mut Commands, state: &ShipState, map: &NetMap) -> Entity {
    commands
        .spawn((
            Name::new(format!("Ship: {}", state.name)),
            PlayerShip,
            PlayerName(state.name.clone()),
            state.net_id,
            state.intent,
            SimPosition::new(state.position),
            SimRotation::new(state.rotation),
            Velocity(state.velocity),
            Hull(state.hull),
            state.stats.clone(),
            state.cargo.clone(),
            MiningRig {
                target: state
                    .mining_target
                    .and_then(|net_id| map.objects.get(&net_id).copied()),
                progress: state.mining_progress,
            },
        ))
        .id()
}

// ---------------------------------------------------------------------------
// Egress
// ---------------------------------------------------------------------------

/// Send the local player's intent once per fixed tick.
fn send_intent(net: Res<ClientNet>, local: Res<LocalIntent>) {
    // Errors mean the writer thread is gone; `drain_incoming` handles the
    // disconnect on its next run.
    let _ = net.outgoing.send(ClientToServer::Intent(local.0));
}
