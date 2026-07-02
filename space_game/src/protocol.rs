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
use crate::resource_types::ResourceType;

/// Bumped on any incompatible message change; mismatched clients are
/// rejected at `Hello` time.
pub const PROTOCOL_VERSION: u32 = 1;

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
}

/// Maps a planet (spawned client-side from shared config) to its server
/// `NetId` and current deposit level.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlanetInit {
    pub config_index: u32,
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
    /// Replicated intent (drives remote exhaust particles).
    pub intent: PlayerIntent,
    pub mining_target: Option<NetId>,
    pub mining_progress: f32,
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

/// One fixed-timestep tick's worth of authoritative state. Ships are sent
/// in full (they're few and small); everything else is event/delta based.
/// A ship absent from `ships` has disconnected.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Snapshot {
    pub sim_elapsed: f64,
    pub ships: Vec<ShipState>,
    pub deposit_updates: Vec<DepositUpdate>,
    pub mined: Vec<MinedEvent>,
    pub exhausted: Vec<ExhaustedEvent>,
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
                intent: PlayerIntent {
                    thrust: 1.0,
                    turn: -0.5,
                    brake: false,
                    mine: true,
                },
                mining_target: Some(NetId(7)),
                mining_progress: 0.6,
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
