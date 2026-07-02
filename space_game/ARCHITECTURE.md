# Architecture

This document describes the client-server split, the ECS layout, the wire
protocol, and how the pieces fit together.

## Overview

The game is two binaries sharing one library:

- **`space_game_server`** — headless, authoritative. Runs the entire gameplay
  simulation (ship physics, orbits, mining, depletion, persistence) on Bevy's
  fixed timestep with `MinimalPlugins`: no window, no input, no rendering.
- **`space_game`** (client) — renders replicated state and sends input. Runs
  **no gameplay simulation**: every gameplay-affecting decision (movement,
  mining eligibility, cargo limits, depletion) happens on the server.

The transport is TCP with length-prefixed bincode frames (`src/protocol.rs`).
TCP's reliability and ordering let the protocol be event-based — nothing
needs re-sending, and one-shot events (mined! exhausted!) can't be lost.
A hand-rolled transport was chosen over `bevy_replicon`/`lightyear` for this
pass because the protocol is tiny (two message types up, three down), the
sim/render seams were already cut for it, and it keeps the dependency
surface flat; swapping in a replication crate later remains possible since
all gameplay state is serializable components (see below).

## The tick loop

```
CLIENT (each frame)                      SERVER (64 Hz FixedUpdate)
  keyboard -> LocalIntent                  SimSet::CachePrevious
  PreUpdate: drain socket                    copy current->previous, advance clock
    Welcome  -> build world, Playing       SimSet::NetSync
    Snapshot -> buffer                       apply client intents to ship components
                                             spawn ships for joiners (+ send Welcome)
CLIENT (64 Hz FixedUpdate)                   capture & despawn leavers' ships
  SimSet::CachePrevious                    SimSet::Movement
  SimSet::NetSync                            integrate thrust physics per ship
    apply snapshots: ship states,            planet orbits, asteroid tumble
    deposit levels, mined/exhausted        SimSet::Mining
  SimSet::Movement (decorative)              beam targeting, extraction, cargo,
    planet orbits, asteroid tumble           depletion, regeneration
  SimSet::PostSim                          SimSet::PostSim
    send LocalIntent                         broadcast one snapshot to all clients
```

Key properties:

- **Intent, not input.** The client sends `PlayerIntent { thrust, turn,
  brake, mine }` once per tick. The server clamps and applies it to that
  player's ship component; simulation systems consume it per-ship. Cheating
  by modified clients is limited to what intent can express.
- **Determinism saves bandwidth.** Planet positions are a pure function of
  the replicated sim clock (`logic/orbit.rs`, f64 math), and asteroid tumble
  is a pure function of spin rate — so neither is ever sent. A snapshot with
  one connected ship is ~150 bytes at 64 Hz (~10 KB/s per client); the
  one-time `Welcome` with the full 220-asteroid world is ~9 KB.
- **Snapshots are applied on the fixed tick.** The client buffers incoming
  snapshots and applies them in `SimSet::NetSync`: events (deposit updates,
  mined, exhausted) from *every* buffered snapshot, authoritative ship state
  from the *newest*. `previous` state was already cached that tick, so the
  existing render interpolation (blend by `Time<Fixed>::overstep_fraction()`)
  smooths server snapshots exactly as it smoothed local simulation.
- **No client prediction yet.** The local ship is rendered from server state
  like everyone else's, costing one round-trip of input latency —
  imperceptible on LAN. `logic/physics::step_ship` is a pure function
  precisely so prediction can be added later by re-running it locally.

## Wire protocol (`src/protocol.rs`)

```
client -> server:  Hello { protocol, name }        (once)
                   Intent(PlayerIntent)             (every tick)

server -> client:  Reject { reason }                (bad version / name taken)
                   Welcome { your_ship, sim_elapsed,
                             planets, asteroids }   (once)
                   Snapshot { sim_elapsed, ships,
                              deposit_updates,
                              mined, exhausted }    (every tick)
```

All wire entity references are `NetId`s (server-allocated `u64`s); `Entity`
ids never cross the network. The client keeps a `NetId -> Entity` map, split
into world objects (keyed lifecycle: explicit `exhausted` events) and ships
(presence-keyed: a ship absent from a snapshot has disconnected).

Threading: sockets live on dedicated threads (accept + reader + writer per
connection) and talk to the ECS through crossbeam channels. Systems never
block; broadcast frames are serialized once and shared as `Arc<Vec<u8>>`.

## Plugin layout (`src/plugins/`)

| Plugin               | Runs on | Responsibilities                                         |
| -------------------- | ------- | -------------------------------------------------------- |
| `SimulationPlugin`   | both    | fixed-timestep sets, `SimClock`, state double-buffering  |
| `PlayerSimPlugin`    | server  | thrust physics per ship from per-ship `PlayerIntent`     |
| `WorldSimPlugin`     | server  | authoritative world spawn (config or save), `NetId`s     |
| `WorldMotionPlugin`  | both    | orbits + tumble (deterministic, so clients run it too)   |
| `ResourcesPlugin`    | server  | mining beam, cargo transfer, depletion, regeneration     |
| `PersistencePlugin`  | server  | RON save/load: world + per-pilot ship roster             |
| `ServerNetPlugin`    | server  | TCP listener, joins/leaves, intent ingress, snapshots    |
| `ClientNetPlugin`    | client  | connect, world build from `Welcome`, snapshot apply      |
| `PlayerClientPlugin` | client  | keyboard -> `LocalIntent`; ship mesh attachment           |
| `WorldClientPlugin`  | client  | star/planet/asteroid mesh attachment                     |
| `GameCameraPlugin`   | client  | smooth-follow camera on the `LocalShip`                  |
| `VisualsPlugin`      | client  | render interpolation, parallax starfield                 |
| `EffectsPlugin`      | client  | per-ship exhaust/beams/sparks (remote ships included)    |
| `UiPlugin`           | client  | HUD, mining bar, minimap (dots track ships dynamically)  |

The sim/render seam is structural: entities carry only serializable sim
components (`SimPosition`, `SimRotation`, `Velocity`, `ShipStats`, `Hull`,
`Cargo`, `ResourceDeposit`, `Orbit`, `NetId`, ...); client-side `attach_*`
systems notice bare entities and add meshes. The same code path decorates
locally spawned and replicated entities.

Pure math (orbit positions, thrust integration, cargo accounting, extraction
rates, belt generation, parallax wrapping) lives in `src/logic/` with no
engine dependencies beyond `Vec2`, unit-tested alongside the protocol
framing and save schema.

## World model

```
SolarSystem "Helios"            (root entity)
├── CentralStar                 (static; glow attached client-side)
├── Planet ×5                   (Orbit, SimPosition, ResourceDeposit, NetId)
└── Asteroid ×220               (SimPosition, SimRotation, Spin,
                                 ResourceDeposit, BodyRadius, NetId)
Ship (per connected player)     (PlayerShip, PlayerName, NetId, PlayerIntent,
                                 SimPosition/Rotation, Velocity, Hull,
                                 ShipStats, Cargo, MiningRig)
```

- The **server** spawns the world from `assets/config/game.ron` (or restores
  it from `save.ron`): belts are generated from explicit RNG seeds, so
  layout is reproducible from config alone.
- The **client** rebuilds the same world from `Welcome`: planets/star from
  its own config (cosmetics) plus server deposit levels; asteroids entirely
  by value from the server (some may already be mined out).
- Adding more solar systems later means more `SolarSystem` roots and
  jump-gate entities; nothing assumes a single system except the shared
  `SimClock`.

## Persistence (server-side)

`PersistencePlugin` saves every 30 s and on exit: sim clock (implies planet
positions), surviving asteroids by value, planet pool levels by config
index, and a **pilot roster** — every player's ship state keyed by pilot
name. Ships are captured into the roster on disconnect and restored on
reconnect, so cargo and position survive sessions. A version field guards
against schema drift (v1 single-player saves are rejected gracefully).

## Known simplifications (deliberate for this pass)

- TCP only: a laggy client sees delayed state, never wrong state. UDP-style
  unreliable channels (and client prediction) are future work.
- Snapshots go to every client at full rate — no interest management. Fine
  for a handful of players; sharding by `SolarSystem` is the natural next
  cut.
- All ships' cargo is included in the broadcast snapshot (no per-client
  filtering); nothing secret lives there today.
- No collision/damage: `Hull` exists, is replicated and persisted, but
  nothing damages ships yet.
- Pilot "auth" is just the name string. Real accounts/sessions are future
  work.
- Ships spawn at the config spawn point, so two new pilots briefly overlap.
