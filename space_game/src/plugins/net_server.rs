//! Server-side networking: accepts TCP clients, feeds their intents and
//! actions into the simulation, and broadcasts authoritative snapshots,
//! notices and chat every tick.
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
    Asteroid, BodyRadius, Credits, GateCooldown, Hull, LastDamager, MiningRig, NetId, PlayerIntent,
    PlayerName, PlayerShip, Projectile, ResourceDeposit, ShipStats, SimClock, SimPosition,
    SimRotation, SimSet, SpawnShield, Spin, Velocity, WeaponCooldown,
};
use crate::config::GameConfig;
use crate::logic::cargo::Cargo;
use crate::logic::economy::{effective_stats, Upgrades};
use crate::plugins::economy::{ActionRequest, AsteroidSpawned, EconomyNotice};
use crate::plugins::persistence::{capture_pilot, PlayerRoster};
use crate::plugins::physics::{ProjectileImpact, ShipDestroyed};
use crate::plugins::resources::{DepositExhausted, ResourceMined};
use crate::plugins::world::NetIdAllocator;
use crate::protocol::{
    encode_frame, read_frame, write_frame, AsteroidNetInit, ClientToServer, DepositUpdate,
    ExhaustedEvent, MinedEvent, PlanetInit, ProjectileState, ServerToClient, ShipState, Snapshot,
    PROTOCOL_VERSION,
};

/// Chat lines longer than this are truncated server-side.
const MAX_CHAT_LEN: usize = 180;

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
    Action {
        conn_id: u64,
        action: crate::protocol::PlayerAction,
    },
    Chat {
        conn_id: u64,
        text: String,
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

impl ConnectedClients {
    /// Encode once, send to everyone.
    fn broadcast(&self, message: &ServerToClient) {
        let Ok(frame) = encode_frame(message) else {
            return;
        };
        let frame = Arc::new(frame);
        for client in self.0.values() {
            let _ = client.outbound.send(frame.clone());
        }
    }
}

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
                    Ok(ClientToServer::Action(action)) => {
                        let _ = events.send(NetEvent::Action { conn_id, action });
                    }
                    Ok(ClientToServer::Chat(text)) => {
                        let _ = events.send(NetEvent::Chat { conn_id, text });
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
// Ingress: joins, intents, actions, chat, disconnects
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
            &PlayerName,
            &SimPosition,
            &SimRotation,
            &Velocity,
            &Hull,
            &Cargo,
            &Credits,
            &Upgrades,
        ),
        With<PlayerShip>,
    >,
    planets: Query<(&crate::components::Planet, &NetId, Option<&ResourceDeposit>)>,
    asteroids: Query<(
        &NetId,
        &SimPosition,
        &SimRotation,
        &Asteroid,
        &Spin,
        &ResourceDeposit,
    )>,
    mut actions: MessageWriter<ActionRequest>,
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
                                system_index: planet.system_index as u32,
                                planet_index: planet.config_index as u32,
                                net_id: *net_id,
                                deposit_amount: deposit.map(|deposit| deposit.amount),
                            })
                            .collect(),
                        asteroids: asteroids
                            .iter()
                            .map(
                                |(net_id, pos, rot, asteroid, spin, deposit)| AsteroidNetInit {
                                    net_id: *net_id,
                                    position: pos.current,
                                    rotation: rot.current,
                                    size: asteroid.size,
                                    spin: spin.0,
                                    kind: deposit.kind,
                                    amount: deposit.amount,
                                    max_amount: deposit.max_amount,
                                },
                            )
                            .collect(),
                    },
                );
                clients.broadcast(&ServerToClient::Notice(format!(
                    "{name} entered the sector"
                )));
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
            NetEvent::Action { conn_id, action } => {
                let Some(client) = clients.0.get(&conn_id) else {
                    continue;
                };
                actions.write(ActionRequest {
                    ship: client.ship,
                    action,
                });
            }
            NetEvent::Chat { conn_id, text } => {
                let Some(client) = clients.0.get(&conn_id) else {
                    continue;
                };
                let mut text = text.trim().to_string();
                if text.is_empty() {
                    continue;
                }
                text.truncate(MAX_CHAT_LEN);
                clients.broadcast(&ServerToClient::Chat {
                    from: client.name.clone(),
                    text,
                });
            }
            NetEvent::Left { conn_id } => {
                let Some(client) = clients.0.remove(&conn_id) else {
                    continue;
                };
                info!("player '{}' left (conn {conn_id})", client.name);
                if let Ok((name, pos, rot, vel, hull, cargo, credits, upgrades)) =
                    ships.get(client.ship)
                {
                    roster.0.insert(
                        client.name.clone(),
                        capture_pilot(name, pos, rot, vel, hull, cargo, credits, upgrades),
                    );
                }
                commands.entity(client.ship).despawn();
                clients.broadcast(&ServerToClient::Notice(format!(
                    "{} left the sector",
                    client.name
                )));
            }
        }
    }
}

/// Spawn the authoritative ship for a player: restored from the roster if
/// they've played before, otherwise fresh from config. Stats are always
/// derived from base config + upgrade tiers.
fn spawn_player_ship(
    commands: &mut Commands,
    config: &GameConfig,
    roster: &PlayerRoster,
    name: &str,
    net_id: NetId,
) -> Entity {
    let record = roster.0.get(name);
    let upgrades = record.map(|save| save.upgrades).unwrap_or_default();
    let credits = record.map(|save| save.credits).unwrap_or(0);
    let stats = effective_stats(&config.ship.stats, &upgrades);

    let (position, rotation, velocity, hull, cargo) = match record {
        Some(saved) => (
            saved.ship.position,
            saved.ship.rotation,
            saved.ship.velocity,
            saved.ship.hull.min(stats.max_hull),
            saved.ship.cargo.clone(),
        ),
        None => (
            Vec2::new(config.ship.spawn_position.0, config.ship.spawn_position.1),
            std::f32::consts::FRAC_PI_2,
            Vec2::ZERO,
            stats.max_hull,
            Cargo::new(stats.cargo_capacity),
        ),
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
            (
                BodyRadius(config.physics.ship_radius),
                Credits(credits),
                upgrades,
                WeaponCooldown::default(),
                SpawnShield(config.physics.respawn_shield_secs),
                LastDamager::default(),
                GateCooldown::default(),
            ),
        ))
        .id()
}

// ---------------------------------------------------------------------------
// Egress: per-tick snapshot + event broadcast
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
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
            &Credits,
            &Upgrades,
            &PlayerIntent,
            &MiningRig,
        ),
        With<PlayerShip>,
    >,
    projectiles: Query<(&NetId, &SimPosition, &Velocity), With<Projectile>>,
    changed_deposits: Query<(&NetId, &ResourceDeposit), Changed<ResourceDeposit>>,
    net_ids_of: Query<&NetId>,
    mut mined: MessageReader<ResourceMined>,
    mut exhausted: MessageReader<DepositExhausted>,
    mut impacts: MessageReader<ProjectileImpact>,
    mut destroyed: MessageReader<ShipDestroyed>,
    mut spawned_asteroids: MessageReader<AsteroidSpawned>,
    mut economy_notices: MessageReader<EconomyNotice>,
) {
    if clients.0.is_empty() {
        // Still drain messages so they don't pile up between connections.
        mined.clear();
        exhausted.clear();
        impacts.clear();
        destroyed.clear();
        spawned_asteroids.clear();
        economy_notices.clear();
        return;
    }

    // Deaths become both a Died (for the victim's overlay) and a Notice.
    for death in destroyed.read() {
        clients.broadcast(&ServerToClient::Died { who: death.victim });
        let text = match (&death.killer_name, death.cargo_lost) {
            (Some(killer), 0) => format!("{} was destroyed by {}", death.victim_name, killer),
            (Some(killer), lost) => format!(
                "{} was destroyed by {} ({} cargo lost)",
                death.victim_name, killer, lost
            ),
            (None, 0) => format!("{} was destroyed", death.victim_name),
            (None, lost) => format!("{} was destroyed ({} cargo lost)", death.victim_name, lost),
        };
        clients.broadcast(&ServerToClient::Notice(text));
    }
    for notice in economy_notices.read() {
        clients.broadcast(&ServerToClient::Notice(notice.0.clone()));
    }

    let snapshot = Snapshot {
        sim_elapsed: clock.elapsed,
        ships: ships
            .iter()
            .map(
                |(
                    net_id,
                    name,
                    pos,
                    rot,
                    vel,
                    hull,
                    stats,
                    cargo,
                    credits,
                    upgrades,
                    intent,
                    rig,
                )| ShipState {
                    net_id: *net_id,
                    name: name.0.clone(),
                    position: pos.current,
                    rotation: rot.current,
                    velocity: vel.0,
                    hull: hull.0,
                    stats: stats.clone(),
                    cargo: cargo.clone(),
                    credits: credits.0,
                    upgrades: *upgrades,
                    intent: *intent,
                    mining_target: rig
                        .target
                        .and_then(|entity| net_ids_of.get(entity).ok().copied()),
                    mining_progress: rig.progress,
                },
            )
            .collect(),
        projectiles: projectiles
            .iter()
            .map(|(net_id, pos, vel)| ProjectileState {
                net_id: *net_id,
                position: pos.current,
                velocity: vel.0,
            })
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
        spawned_asteroids: spawned_asteroids
            .read()
            .map(|message| message.0.clone())
            .collect(),
        impacts: impacts.read().map(|impact| impact.position).collect(),
    };

    clients.broadcast(&ServerToClient::Snapshot(snapshot));
}
