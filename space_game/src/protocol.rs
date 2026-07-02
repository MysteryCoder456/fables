//! The client-server wire protocol.
//!
//! Transport is TCP with length-prefixed bincode frames. TCP's ordering and
//! reliability let the protocol be event-based: world layout is sent once at
//! join (`Welcome`), after which per-tick `Snapshot`s carry only what
//! changes — ship states, deposit levels and mining events. Planet positions
//! are never sent at all: both sides derive them from the replicated sim
//! clock, and asteroid tumble is animated client-side from its spin rate.
//!
//! All entity references on the wire use [`NetId`]s; `Entity` ids are
//! process-local and never cross the network.

use std::io::{Read, Write};

use bevy::math::Vec2;
use serde::{Deserialize, Serialize};

use crate::components::{NetId, PlayerIntent, ShipStats};
use crate::logic::cargo::Cargo;
use crate::logic::economy::{UpgradeKind, Upgrades};
use crate::resource_types::ResourceType;

/// Bumped on any incompatible message change; mismatched clients are
/// rejected at `Hello` time.
pub const PROTOCOL_VERSION: u32 = 2;

/// Default TCP port; override with CLI args on both binaries.
pub const DEFAULT_PORT: u16 = 5123;

/// Upper bound on a single frame, as a sanity guard against corrupt length
/// prefixes. The largest legitimate frame is `Welcome` with every asteroid
/// (~64 bytes each), far below this.
pub const MAX_FRAME_BYTES: u32 = 8 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ClientToServer {
    /// First message on the socket.
    Hello { protocol: u32, name: String },
    /// The player's input for the current tick.
    Intent(PlayerIntent),
    /// A discrete, reliable action (server validates everything).
    Action(PlayerAction),
    /// A chat line for everyone.
    Chat(String),
}

/// One-shot actions, valid only while docked at a planet.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum PlayerAction {
    /// Sell the entire stack of one resource at the docked planet's price.
    Sell(ResourceType),
    /// Buy the next tier of an upgrade track.
    BuyUpgrade(UpgradeKind),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ServerToClient {
    /// Connection refused (version mismatch, duplicate name, ...).
    Reject { reason: String },
    /// Join accepted: full world description. Only sent once.
    Welcome {
        /// The `NetId` of the ship the server spawned for this client.
        your_ship: NetId,
        sim_elapsed: f64,
        planets: Vec<PlanetInit>,
        asteroids: Vec<AsteroidNetInit>,
    },
    /// Authoritative per-tick state.
    Snapshot(Snapshot),
    /// A chat line from another pilot.
    Chat { from: String, text: String },
    /// A system announcement (joins, kills, trades...) for the event feed.
    Notice(String),
    /// Somebody's ship was destroyed (the client whose ship it is shows the
    /// death overlay; everyone gets a Notice alongside).
    Died { who: NetId },
}

/// Maps a planet (spawned client-side from shared config) to its server
/// `NetId` and current deposit level.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlanetInit {
    pub system_index: u32,
    pub planet_index: u32,
    pub net_id: NetId,
    pub deposit_amount: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AsteroidNetInit {
    pub net_id: NetId,
    pub position: Vec2,
    pub rotation: f32,
    pub size: f32,
    pub spin: f32,
    pub kind: ResourceType,
    pub amount: f32,
    pub max_amount: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShipState {
    pub net_id: NetId,
    pub name: String,
    pub position: Vec2,
    pub rotation: f32,
    pub velocity: Vec2,
    pub hull: f32,
    pub stats: ShipStats,
    pub cargo: Cargo,
    pub credits: u64,
    pub upgrades: Upgrades,
    /// Replicated intent (drives remote exhaust particles).
    pub intent: PlayerIntent,
    pub mining_target: Option<NetId>,
    pub mining_progress: f32,
}

/// A blaster bolt in flight.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct ProjectileState {
    pub net_id: NetId,
    pub position: Vec2,
    pub velocity: Vec2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DepositUpdate {
    pub net_id: NetId,
    pub amount: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MinedEvent {
    pub ship: NetId,
    pub kind: ResourceType,
    pub amount: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExhaustedEvent {
    pub net_id: NetId,
    pub position: Vec2,
    pub kind: ResourceType,
}

/// One fixed-timestep tick's worth of authoritative state. Ships and bolts
/// are sent in full (they're few and small); everything else is event/delta
/// based. A ship or bolt absent from its list is gone.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Snapshot {
    pub sim_elapsed: f64,
    pub ships: Vec<ShipState>,
    pub projectiles: Vec<ProjectileState>,
    pub deposit_updates: Vec<DepositUpdate>,
    pub mined: Vec<MinedEvent>,
    pub exhausted: Vec<ExhaustedEvent>,
    /// New asteroids (belt respawns) since the last snapshot.
    pub spawned_asteroids: Vec<AsteroidNetInit>,
    /// Hard impacts this tick (collisions, bolt hits) for spark bursts.
    pub impacts: Vec<Vec2>,
}

// ---------------------------------------------------------------------------
// Framing
// ---------------------------------------------------------------------------

/// Serialize a message into a length-prefixed frame (u32 little-endian
/// payload length, then the bincode payload).
pub fn encode_frame<T: Serialize>(message: &T) -> Result<Vec<u8>, bincode::Error> {
    let payload = bincode::serialize(message)?;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// Read one frame from a blocking stream and deserialize it. Returns
/// `Err` on EOF, I/O error, oversized frame or malformed payload.
pub fn read_frame<T: for<'de> Deserialize<'de>>(
    stream: &mut impl Read,
) -> Result<T, Box<dyn std::error::Error + Send + Sync>> {
    let mut len_bytes = [0u8; 4];
    stream.read_exact(&mut len_bytes)?;
    let len = u32::from_le_bytes(len_bytes);
    if len > MAX_FRAME_BYTES {
        return Err(format!("frame of {len} bytes exceeds limit").into());
    }
    let mut payload = vec![0u8; len as usize];
    stream.read_exact(&mut payload)?;
    Ok(bincode::deserialize(&payload)?)
}

/// Write a pre-encoded frame to a blocking stream.
pub fn write_frame(stream: &mut impl Write, frame: &[u8]) -> std::io::Result<()> {
    stream.write_all(frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn sample_snapshot() -> Snapshot {
        let mut cargo = Cargo::new(60);
        cargo.add(ResourceType::Ice, 7);
        Snapshot {
            sim_elapsed: 987.25,
            ships: vec![ShipState {
                net_id: NetId(42),
                name: "ada".into(),
                position: Vec2::new(100.0, -250.5),
                rotation: 1.2,
                velocity: Vec2::new(-3.0, 8.0),
                hull: 88.0,
                stats: crate::config::GameConfig::default().ship.stats,
                cargo,
                credits: 1250,
                upgrades: Upgrades {
                    thrusters: 2,
                    cargo_bay: 1,
                    ..Default::default()
                },
                intent: PlayerIntent {
                    thrust: 1.0,
                    turn: -0.5,
                    brake: false,
                    mine: true,
                    fire: true,
                },
                mining_target: Some(NetId(7)),
                mining_progress: 0.6,
            }],
            projectiles: vec![ProjectileState {
                net_id: NetId(300),
                position: Vec2::new(120.0, -240.0),
                velocity: Vec2::new(600.0, 30.0),
            }],
            deposit_updates: vec![DepositUpdate {
                net_id: NetId(7),
                amount: 12.5,
            }],
            mined: vec![MinedEvent {
                ship: NetId(42),
                kind: ResourceType::Ice,
                amount: 2,
            }],
            exhausted: vec![ExhaustedEvent {
                net_id: NetId(9),
                position: Vec2::new(2800.0, 40.0),
                kind: ResourceType::Iron,
            }],
            spawned_asteroids: vec![AsteroidNetInit {
                net_id: NetId(500),
                position: Vec2::new(2700.0, 100.0),
                rotation: 0.1,
                size: 20.0,
                spin: 0.4,
                kind: ResourceType::Iron,
                amount: 30.0,
                max_amount: 30.0,
            }],
            impacts: vec![Vec2::new(50.0, 60.0)],
        }
    }

    #[test]
    fn actions_and_chat_round_trip() {
        for message in [
            ClientToServer::Action(PlayerAction::Sell(ResourceType::Crystal)),
            ClientToServer::Action(PlayerAction::BuyUpgrade(UpgradeKind::Thrusters)),
            ClientToServer::Chat("o7 pilots".into()),
        ] {
            let frame = encode_frame(&message).unwrap();
            let decoded: ClientToServer = read_frame(&mut Cursor::new(frame)).unwrap();
            assert_eq!(decoded, message);
        }
    }

    #[test]
    fn frame_round_trip() {
        let message = ServerToClient::Snapshot(sample_snapshot());
        let frame = encode_frame(&message).unwrap();
        let mut cursor = Cursor::new(frame);
        let decoded: ServerToClient = read_frame(&mut cursor).unwrap();
        assert_eq!(decoded, message);
    }

    #[test]
    fn multiple_frames_in_sequence() {
        let first = ClientToServer::Hello {
            protocol: PROTOCOL_VERSION,
            name: "grace".into(),
        };
        let second = ClientToServer::Intent(PlayerIntent {
            thrust: 1.0,
            turn: 0.0,
            brake: true,
            mine: false,
            fire: false,
        });
        let mut bytes = encode_frame(&first).unwrap();
        bytes.extend(encode_frame(&second).unwrap());

        let mut cursor = Cursor::new(bytes);
        let a: ClientToServer = read_frame(&mut cursor).unwrap();
        let b: ClientToServer = read_frame(&mut cursor).unwrap();
        assert_eq!(a, first);
        assert_eq!(b, second);
        // Stream exhausted: next read fails cleanly.
        assert!(read_frame::<ClientToServer>(&mut cursor).is_err());
    }

    #[test]
    fn oversized_frame_is_rejected() {
        let mut bytes = (MAX_FRAME_BYTES + 1).to_le_bytes().to_vec();
        bytes.extend([0u8; 16]);
        let mut cursor = Cursor::new(bytes);
        assert!(read_frame::<ServerToClient>(&mut cursor).is_err());
    }

    #[test]
    fn truncated_frame_is_rejected() {
        let message = ServerToClient::Snapshot(sample_snapshot());
        let mut frame = encode_frame(&message).unwrap();
        frame.truncate(frame.len() - 5);
        let mut cursor = Cursor::new(frame);
        assert!(read_frame::<ServerToClient>(&mut cursor).is_err());
    }
}
