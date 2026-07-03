//! Client-side networking: connects to the server, builds the universe from
//! its `Welcome`, applies per-tick snapshots, and sends local intent,
//! actions and chat.
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
    Credits, DeathFlash, Feed, Hull, LocalIntent, LocalShip, MiningRig, NetId, PlayerIntent,
    PlayerName, PlayerShip, ResourceDeposit, SendAction, SendChat, ShipStats, SimClock,
    SimPosition, SimRotation, SimSet, Velocity,
};
use crate::config::GameConfig;
use crate::logic::cargo::Cargo;
use crate::logic::economy::Upgrades;
use crate::plugins::effects::ImpactFlash;
use crate::plugins::resources::{DepositExhausted, ResourceMined};
use crate::plugins::world::{
    spawn_asteroid, spawn_gate, spawn_planet, spawn_star, spawn_system_root, AsteroidInit,
};
use crate::protocol::{
    encode_frame, read_frame, write_frame, AsteroidNetInit, ClientToServer, ServerToClient,
    ShipState, Snapshot, PROTOCOL_VERSION,
};

const CONNECT_ATTEMPTS: u32 = 10;
const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(500);
/// Position jumps beyond this snap the interpolation buffer (teleports:
/// respawns and gate jumps must not smear across the map).
const TELEPORT_SNAP_DISTANCE: f32 = 1500.0;
/// Seconds the death overlay stays up.
const DEATH_FLASH_SECS: f32 = 3.0;

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
            .init_resource::<Feed>()
            .init_resource::<DeathFlash>()
            .insert_resource(MyShip(None))
            // Re-emitted from snapshot events for the effects systems
            // (no ResourcesPlugin/PhysicsPlugin on this side).
            .add_message::<ResourceMined>()
            .add_message::<DepositExhausted>()
            .add_message::<SendChat>()
            .add_message::<SendAction>()
            .add_systems(PreUpdate, drain_incoming)
            .add_systems(Update, relay_outgoing)
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

/// NetId -> local entity, split by kind: ships and bolts are presence-keyed
/// (absent from a snapshot = gone), world objects have explicit lifecycle
/// events.
#[derive(Resource, Default)]
struct NetMap {
    objects: HashMap<NetId, Entity>,
    ships: HashMap<NetId, Entity>,
    bolts: HashMap<NetId, Entity>,
}

/// The `NetId` of the ship the server assigned to this client.
#[derive(Resource)]
struct MyShip(Option<NetId>);

/// Snapshots received since the last fixed tick.
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
    mut feed: ResMut<Feed>,
    mut death_flash: ResMut<DeathFlash>,
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
                feed.push("* tip: you start in orbit: thrust (W) to climb away".into());
                feed.push("* tip: hold Space near rocks to mine, dock at planets to sell".into());

                // Build the universe locally: layout from shared config,
                // dynamic state (deposits, surviving asteroids) from the
                // server.
                let mut roots = Vec::new();
                for (system_index, system_cfg) in config.systems.iter().enumerate() {
                    let center = Vec2::new(system_cfg.center.0, system_cfg.center.1);
                    let root = spawn_system_root(&mut commands, &system_cfg.name);
                    spawn_star(&mut commands, root, system_index, &system_cfg.star, center);
                    for (gate_index, gate_cfg) in system_cfg.gates.iter().enumerate() {
                        spawn_gate(
                            &mut commands,
                            root,
                            system_index,
                            gate_index,
                            gate_cfg,
                            center,
                        );
                    }
                    roots.push((root, center));
                }
                for init in &planets {
                    let system_index = init.system_index as usize;
                    let planet_index = init.planet_index as usize;
                    let Some(cfg) = config
                        .systems
                        .get(system_index)
                        .and_then(|system| system.planets.get(planet_index))
                    else {
                        warn!("server planet {system_index}/{planet_index} not in local config");
                        continue;
                    };
                    let Some((root, center)) = roots.get(system_index).copied() else {
                        continue;
                    };
                    let entity = spawn_planet(
                        &mut commands,
                        root,
                        system_index,
                        planet_index,
                        cfg,
                        center,
                        init.net_id,
                        init.deposit_amount,
                        sim_elapsed,
                    );
                    map.objects.insert(init.net_id, entity);
                }
                if let Some((root, _)) = roots.first().copied() {
                    for init in &asteroids {
                        let entity = spawn_asteroid_from_net(&mut commands, root, init);
                        map.objects.insert(init.net_id, entity);
                    }
                }
                next_state.set(crate::components::GameState::Playing);
            }
            ClientEvent::Message(ServerToClient::Snapshot(snapshot)) => {
                buffer.0.push_back(snapshot);
            }
            ClientEvent::Message(ServerToClient::Chat { from, text }) => {
                feed.push(format!("[{from}] {text}"));
            }
            ClientEvent::Message(ServerToClient::Notice(text)) => {
                feed.push(format!("* {text}"));
            }
            ClientEvent::Message(ServerToClient::Died { who }) => {
                if my_ship.0 == Some(who) {
                    death_flash.0 = DEATH_FLASH_SECS;
                }
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

fn spawn_asteroid_from_net(
    commands: &mut Commands,
    root: Entity,
    init: &AsteroidNetInit,
) -> Entity {
    spawn_asteroid(
        commands,
        root,
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
    )
}

/// Apply buffered snapshots at the fixed tick: events from every snapshot,
/// authoritative ship/bolt state from the newest.
#[allow(clippy::too_many_arguments)]
fn apply_snapshots(
    mut commands: Commands,
    mut buffer: ResMut<SnapshotBuffer>,
    mut map: ResMut<NetMap>,
    my_ship: Res<MyShip>,
    mut clock: ResMut<SimClock>,
    roots: Query<Entity, With<crate::components::SolarSystem>>,
    mut ships: Query<
        (
            &mut SimPosition,
            &mut SimRotation,
            &mut Velocity,
            &mut Hull,
            &mut ShipStats,
            &mut Cargo,
            &mut Credits,
            &mut Upgrades,
            &mut PlayerIntent,
            &mut MiningRig,
        ),
        With<PlayerShip>,
    >,
    mut bolts: Query<(&mut SimPosition, &mut Velocity), (Without<PlayerShip>, With<Sprite>)>,
    mut mined_messages: MessageWriter<ResourceMined>,
    mut exhausted_messages: MessageWriter<DepositExhausted>,
    mut impact_messages: MessageWriter<ImpactFlash>,
    mut deposits: Query<&mut ResourceDeposit>,
) {
    if buffer.0.is_empty() {
        return;
    }

    // Events from every snapshot (they're one-shot and must not be lost).
    let mut snapshots: Vec<Snapshot> = buffer.0.drain(..).collect();
    let first_root = roots.iter().next();
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
        for init in &snapshot.spawned_asteroids {
            if map.objects.contains_key(&init.net_id) {
                continue;
            }
            if let Some(root) = first_root {
                let entity = spawn_asteroid_from_net(&mut commands, root, init);
                map.objects.insert(init.net_id, entity);
            }
        }
        for impact in &snapshot.impacts {
            impact_messages.write(ImpactFlash { position: *impact });
        }
    }

    // Ship + bolt state from the newest snapshot only.
    let newest = snapshots.pop().expect("buffer was non-empty");
    clock.elapsed = newest.sim_elapsed;

    let mut seen_ships: Vec<NetId> = Vec::with_capacity(newest.ships.len());
    for state in &newest.ships {
        seen_ships.push(state.net_id);
        match map.ships.get(&state.net_id) {
            Some(&entity) => {
                if let Ok((
                    mut pos,
                    mut rot,
                    mut vel,
                    mut hull,
                    mut stats,
                    mut cargo,
                    mut credits,
                    mut upgrades,
                    mut intent,
                    mut rig,
                )) = ships.get_mut(entity)
                {
                    // Teleports (respawn, gate jump) snap the interpolation
                    // buffer instead of smearing across the map.
                    if pos.current.distance(state.position) > TELEPORT_SNAP_DISTANCE {
                        *pos = SimPosition::new(state.position);
                        rot.previous = state.rotation;
                    } else {
                        pos.current = state.position;
                    }
                    rot.current = state.rotation;
                    vel.0 = state.velocity;
                    hull.0 = state.hull;
                    if *stats != state.stats {
                        *stats = state.stats.clone();
                    }
                    if *cargo != state.cargo {
                        *cargo = state.cargo.clone();
                    }
                    credits.0 = state.credits;
                    *upgrades = state.upgrades;
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
    let mut removed: Vec<NetId> = Vec::new();
    for (&net_id, &entity) in &map.ships {
        if !seen_ships.contains(&net_id) {
            commands.entity(entity).despawn();
            removed.push(net_id);
        }
    }
    for net_id in removed {
        map.ships.remove(&net_id);
    }

    // Bolts: same presence-keyed lifecycle, tiny bright sprites.
    let mut seen_bolts: Vec<NetId> = Vec::with_capacity(newest.projectiles.len());
    for state in &newest.projectiles {
        seen_bolts.push(state.net_id);
        match map.bolts.get(&state.net_id) {
            Some(&entity) => {
                if let Ok((mut pos, mut vel)) = bolts.get_mut(entity) {
                    pos.current = state.position;
                    vel.0 = state.velocity;
                }
            }
            None => {
                let entity = commands
                    .spawn((
                        Name::new("Bolt"),
                        state.net_id,
                        SimPosition::new(state.position),
                        Velocity(state.velocity),
                        Sprite::from_color(Color::srgb(1.0, 0.95, 0.5), Vec2::new(9.0, 3.0)),
                        Transform::from_translation(state.position.extend(9.5))
                            .with_rotation(Quat::from_rotation_z(state.velocity.to_angle())),
                    ))
                    .id();
                map.bolts.insert(state.net_id, entity);
            }
        }
    }
    let mut removed_bolts: Vec<NetId> = Vec::new();
    for (&net_id, &entity) in &map.bolts {
        if !seen_bolts.contains(&net_id) {
            commands.entity(entity).despawn();
            removed_bolts.push(net_id);
        }
    }
    for net_id in removed_bolts {
        map.bolts.remove(&net_id);
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
            Credits(state.credits),
            state.upgrades,
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
    let _ = net.outgoing.send(ClientToServer::Intent(local.0));
}

/// Relay chat lines and dock actions the UI queued up.
fn relay_outgoing(
    net: Res<ClientNet>,
    mut chats: MessageReader<SendChat>,
    mut actions: MessageReader<SendAction>,
) {
    for chat in chats.read() {
        let _ = net.outgoing.send(ClientToServer::Chat(chat.0.clone()));
    }
    for action in actions.read() {
        let _ = net.outgoing.send(ClientToServer::Action(action.0));
    }
}
