//! Server-side networking: accepts TCP clients, feeds their intents into
//! the simulation, and broadcasts authoritative snapshots every tick.
//!
//! Threading model: one accept thread, plus a reader and a writer thread
//! per connection. Threads talk to the ECS exclusively through crossbeam
//! channels ([`NetEvent`] in, pre-encoded frames out), so systems never
//! block on sockets.

use std::collections::HashMap;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bevy::prelude::*;
use crossbeam_channel::{Receiver, Sender};

use crate::components::{
    Asteroid, Hull, MiningRig, NetId, PlayerIntent, PlayerName, PlayerShip, ResourceDeposit,
    ShipStats, SimClock, SimPosition, SimRotation, SimSet, Spin, Velocity,
};
use crate::config::GameConfig;
use crate::logic::cargo::Cargo;
use crate::plugins::persistence::{capture_ship, PlayerRoster};
use crate::plugins::resources::{DepositExhausted, ResourceMined};
use crate::plugins::world::NetIdAllocator;
use crate::protocol::{
    encode_frame, read_frame, write_frame, AsteroidNetInit, ClientToServer, DepositUpdate,
    ExhaustedEvent, MinedEvent, PlanetInit, ServerToClient, ShipState, Snapshot, PROTOCOL_VERSION,
};

pub struct ServerNetPlugin {
    pub bind_addr: String,
}

impl Plugin for ServerNetPlugin {
    fn build(&self, app: &mut App) {
        let events = start_listener(&self.bind_addr);
        app.insert_resource(ServerNet { events })
            .init_resource::<ConnectedClients>()
            .add_systems(
                FixedUpdate,
                (
                    handle_net_events.in_set(SimSet::NetSync),
                    broadcast_snapshot.in_set(SimSet::PostSim),
                ),
            );
    }
}

/// Everything a connection thread can tell the simulation.
enum NetEvent {
    Joined {
        conn_id: u64,
        protocol: u32,
        name: String,
        outbound: Sender<Arc<Vec<u8>>>,
    },
    Intent {
        conn_id: u64,
        intent: PlayerIntent,
    },
    Left {
        conn_id: u64,
    },
}

#[derive(Resource)]
struct ServerNet {
    events: Receiver<NetEvent>,
}

struct ClientHandle {
    name: String,
    ship: Entity,
    outbound: Sender<Arc<Vec<u8>>>,
}

#[derive(Resource, Default)]
struct ConnectedClients(HashMap<u64, ClientHandle>);

// ---------------------------------------------------------------------------
// Socket threads
// ---------------------------------------------------------------------------

fn start_listener(bind_addr: &str) -> Receiver<NetEvent> {
    let listener = TcpListener::bind(bind_addr)
        .unwrap_or_else(|err| panic!("failed to bind server on {bind_addr}: {err}"));
    info!("server listening on {bind_addr}");

    let (event_tx, event_rx) = crossbeam_channel::unbounded();
    std::thread::Builder::new()
        .name("net-accept".into())
        .spawn(move || {
            let next_conn_id = AtomicU64::new(1);
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let conn_id = next_conn_id.fetch_add(1, Ordering::Relaxed);
                let _ = stream.set_nodelay(true);
                spawn_connection_threads(conn_id, stream, event_tx.clone());
            }
        })
        .expect("failed to spawn accept thread");
    event_rx
}

fn spawn_connection_threads(conn_id: u64, stream: TcpStream, events: Sender<NetEvent>) {
    let (outbound_tx, outbound_rx) = crossbeam_channel::unbounded::<Arc<Vec<u8>>>();
    let mut write_half = match stream.try_clone() {
        Ok(clone) => clone,
        Err(err) => {
            warn!("conn {conn_id}: failed to clone stream: {err}");
            return;
        }
    };

    std::thread::Builder::new()
        .name(format!("net-write-{conn_id}"))
        .spawn(move || {
            while let Ok(frame) = outbound_rx.recv() {
                if write_frame(&mut write_half, &frame).is_err() {
                    break;
                }
            }
            // Dropping the stream clone closes our write half; the reader
            // thread notices via read error and reports `Left`.
        })
        .expect("failed to spawn writer thread");

    std::thread::Builder::new()
        .name(format!("net-read-{conn_id}"))
        .spawn(move || {
            let mut read_half = stream;
            // First frame must be Hello.
            match read_frame::<ClientToServer>(&mut read_half) {
                Ok(ClientToServer::Hello { protocol, name }) => {
                    let _ = events.send(NetEvent::Joined {
                        conn_id,
                        protocol,
                        name,
                        outbound: outbound_tx,
                    });
                }
                _ => return, // bad handshake; drop connection silently
            }
            loop {
                match read_frame::<ClientToServer>(&mut read_half) {
                    Ok(ClientToServer::Intent(intent)) => {
                        let _ = events.send(NetEvent::Intent { conn_id, intent });
                    }
                    Ok(_) => {} // duplicate Hello: ignore
                    Err(_) => break,
                }
            }
            let _ = events.send(NetEvent::Left { conn_id });
        })
        .expect("failed to spawn reader thread");
}

fn send_message(outbound: &Sender<Arc<Vec<u8>>>, message: &ServerToClient) {
    if let Ok(frame) = encode_frame(message) {
        let _ = outbound.send(Arc::new(frame));
    }
}

// ---------------------------------------------------------------------------
// Ingress: joins, intents, disconnects
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn handle_net_events(
    mut commands: Commands,
    net: Res<ServerNet>,
    mut clients: ResMut<ConnectedClients>,
    mut roster: ResMut<PlayerRoster>,
    config: Res<GameConfig>,
    clock: Res<SimClock>,
    mut net_ids: ResMut<NetIdAllocator>,
    mut intents: Query<&mut PlayerIntent>,
    ships: Query<
        (
            &SimPosition,
            &SimRotation,
            &Velocity,
            &Hull,
            &ShipStats,
            &Cargo,
        ),
        With<PlayerShip>,
    >,
    planets: Query<(&crate::components::Planet, &NetId, Option<&ResourceDeposit>)>,
    asteroids: Query<(&NetId, &SimPosition, &SimRotation, &Asteroid, &Spin, &ResourceDeposit)>,
) {
    while let Ok(event) = net.events.try_recv() {
        match event {
            NetEvent::Joined {
                conn_id,
                protocol,
                name,
                outbound,
            } => {
                if protocol != PROTOCOL_VERSION {
                    send_message(
                        &outbound,
                        &ServerToClient::Reject {
                            reason: format!(
                                "protocol mismatch: server {PROTOCOL_VERSION}, client {protocol}"
                            ),
                        },
                    );
                    continue;
                }
                if clients.0.values().any(|client| client.name == name) {
                    send_message(
                        &outbound,
                        &ServerToClient::Reject {
                            reason: format!("pilot name '{name}' is already online"),
                        },
                    );
                    continue;
                }

                let net_id = net_ids.allocate();
                let ship = spawn_player_ship(&mut commands, &config, &roster, &name, net_id);
                info!("player '{name}' joined (conn {conn_id}, ship {net_id:?})");

                send_message(
                    &outbound,
                    &ServerToClient::Welcome {
                        your_ship: net_id,
                        sim_elapsed: clock.elapsed,
                        planets: planets
                            .iter()
                            .map(|(planet, net_id, deposit)| PlanetInit {
                                config_index: planet.config_index as u32,
                                net_id: *net_id,
                                deposit_amount: deposit.map(|deposit| deposit.amount),
                            })
                            .collect(),
                        asteroids: asteroids
                            .iter()
                            .map(|(net_id, pos, rot, asteroid, spin, deposit)| AsteroidNetInit {
                                net_id: *net_id,
                                position: pos.current,
                                rotation: rot.current,
                                size: asteroid.size,
                                spin: spin.0,
                                kind: deposit.kind,
                                amount: deposit.amount,
                                max_amount: deposit.max_amount,
                            })
                            .collect(),
                    },
                );
                clients.0.insert(
                    conn_id,
                    ClientHandle {
                        name,
                        ship,
                        outbound,
                    },
                );
            }
            NetEvent::Intent { conn_id, intent } => {
                let Some(client) = clients.0.get(&conn_id) else {
                    continue;
                };
                // Ship spawned this tick isn't queryable yet; drop one tick.
                if let Ok(mut ship_intent) = intents.get_mut(client.ship) {
                    *ship_intent = PlayerIntent {
                        thrust: intent.thrust.clamp(0.0, 1.0),
                        turn: intent.turn.clamp(-1.0, 1.0),
                        ..intent
                    };
                }
            }
            NetEvent::Left { conn_id } => {
                let Some(client) = clients.0.remove(&conn_id) else {
                    continue;
                };
                info!("player '{}' left (conn {conn_id})", client.name);
                if let Ok((pos, rot, vel, hull, stats, cargo)) = ships.get(client.ship) {
                    roster
                        .0
                        .insert(client.name, capture_ship(pos, rot, vel, hull, stats, cargo));
                }
                commands.entity(client.ship).despawn();
            }
        }
    }
}

/// Spawn the authoritative ship for a player: restored from the roster if
/// they've played before, otherwise fresh from config.
fn spawn_player_ship(
    commands: &mut Commands,
    config: &GameConfig,
    roster: &PlayerRoster,
    name: &str,
    net_id: NetId,
) -> Entity {
    let (position, rotation, velocity, hull, stats, cargo) = match roster.0.get(name) {
        Some(saved) => (
            saved.position,
            saved.rotation,
            saved.velocity,
            saved.hull,
            saved.stats.clone(),
            saved.cargo.clone(),
        ),
        None => {
            let stats = config.ship.stats.clone();
            (
                Vec2::new(config.ship.spawn_position.0, config.ship.spawn_position.1),
                std::f32::consts::FRAC_PI_2,
                Vec2::ZERO,
                stats.max_hull,
                stats.clone(),
                Cargo::new(stats.cargo_capacity),
            )
        }
    };

    commands
        .spawn((
            Name::new(format!("Ship: {name}")),
            PlayerShip,
            PlayerName(name.into()),
            net_id,
            PlayerIntent::default(),
            SimPosition::new(position),
            SimRotation::new(rotation),
            Velocity(velocity),
            Hull(hull),
            cargo,
            MiningRig::default(),
            stats,
        ))
        .id()
}

// ---------------------------------------------------------------------------
// Egress: per-tick snapshot broadcast
// ---------------------------------------------------------------------------

fn broadcast_snapshot(
    clients: Res<ConnectedClients>,
    clock: Res<SimClock>,
    ships: Query<
        (
            &NetId,
            &PlayerName,
            &SimPosition,
            &SimRotation,
            &Velocity,
            &Hull,
            &ShipStats,
            &Cargo,
            &PlayerIntent,
            &MiningRig,
        ),
        With<PlayerShip>,
    >,
    changed_deposits: Query<(&NetId, &ResourceDeposit), Changed<ResourceDeposit>>,
    net_ids_of: Query<&NetId>,
    mut mined: MessageReader<ResourceMined>,
    mut exhausted: MessageReader<DepositExhausted>,
) {
    if clients.0.is_empty() {
        // Still drain messages so they don't pile up between connections.
        mined.clear();
        exhausted.clear();
        return;
    }

    let snapshot = Snapshot {
        sim_elapsed: clock.elapsed,
        ships: ships
            .iter()
            .map(
                |(net_id, name, pos, rot, vel, hull, stats, cargo, intent, rig)| ShipState {
                    net_id: *net_id,
                    name: name.0.clone(),
                    position: pos.current,
                    rotation: rot.current,
                    velocity: vel.0,
                    hull: hull.0,
                    stats: stats.clone(),
                    cargo: cargo.clone(),
                    intent: *intent,
                    mining_target: rig
                        .target
                        .and_then(|entity| net_ids_of.get(entity).ok().copied()),
                    mining_progress: rig.progress,
                },
            )
            .collect(),
        deposit_updates: changed_deposits
            .iter()
            .map(|(net_id, deposit)| DepositUpdate {
                net_id: *net_id,
                amount: deposit.amount,
            })
            .collect(),
        mined: mined
            .read()
            .filter_map(|message| {
                Some(MinedEvent {
                    ship: net_ids_of.get(message.ship).ok().copied()?,
                    kind: message.kind,
                    amount: message.amount,
                })
            })
            .collect(),
        exhausted: exhausted
            .read()
            .map(|message| ExhaustedEvent {
                net_id: message.net_id,
                position: message.position,
                kind: message.kind,
            })
            .collect(),
    };

    let Ok(frame) = encode_frame(&ServerToClient::Snapshot(snapshot)) else {
        return;
    };
    let frame = Arc::new(frame);
    for client in clients.0.values() {
        // Send failures mean the writer thread is gone; the reader thread
        // will surface a `Left` event shortly.
        let _ = client.outbound.send(frame.clone());
    }
}
