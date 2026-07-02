# Architecture

This document describes the ECS layout, the simulation/rendering split, and
the plan for turning this single-player build into an MMO client/server pair
without a rewrite.

## Guiding constraint: MMO-readiness

The long-term goal is an MMORPG. Full networking is deliberately **not**
implemented yet, but every design decision below exists so that a headless
server can later run the simulation and thin clients can render it:

1. **Simulation and rendering are strictly separated.** All gameplay logic
   runs in `FixedUpdate` (Bevy's fixed timestep); everything in `Update` is
   presentation (interpolation, camera, HUD, particles) or input gathering.
   A server build simply doesn't register the `Update` half.
2. **All gameplay state is serializable.** Components and resources that make
   up the simulation (`SimPosition`, `SimRotation`, `Velocity`, `ShipStats`,
   `Hull`, `Cargo`, `ResourceDeposit`, `Orbit`, `SimClock`, ...) derive
   `Serialize`/`Deserialize`. Persistence uses this today; replication uses
   it tomorrow.
3. **Input is an intent struct, not key events.** `PlayerIntent { thrust,
   turn, brake, mine }` is written by a client-side system and consumed by
   simulation systems. It is exactly the message a networked client would
   send to the server each tick.
4. **Determinism where it's cheap.** Orbits are a pure function of the
   simulation clock; asteroid belts are generated from explicit RNG seeds in
   config. Server and client can agree on world layout from config + clock
   alone.

## Schedules and system sets

```
FixedUpdate (simulation — would run headless on a server)
  SimSet::CachePrevious   advance SimClock; copy current → previous state
  SimSet::Movement        ship thrust physics, planet orbits, asteroid spin
  SimSet::Mining          beam targeting, extraction, depletion, regeneration

Update (client-only presentation)
  RenderSet::SyncTransforms  write interpolated sim state into Transforms
  RenderSet::Camera          smooth-follow camera easing
  RenderSet::Decor           starfield parallax, particles, beam, HUD, minimap
```

The whole `FixedUpdate` chain is gated on `GameState::Playing` (pausing stops
the simulation, not the renderer).

### Render interpolation

Simulation state is double-buffered: `SimPosition`/`SimRotation` hold both
`current` and `previous` values. Each frame, `RenderSet::SyncTransforms`
blends them by `Time<Fixed>::overstep_fraction()` and writes the result into
`Transform`. Rendering therefore always lags the simulation by less than one
tick but is perfectly smooth at any frame rate — and the identical mechanism
later interpolates *server snapshots* instead of local ticks.

## Plugin layout (`src/plugins/`)

| Plugin              | Side       | Responsibilities                                            |
| ------------------- | ---------- | ----------------------------------------------------------- |
| `SimulationPlugin`  | sim        | fixed-timestep sets, `SimClock`, state double-buffering, pause |
| `PlayerPlugin`      | sim+client | ship spawn, thrust physics (sim); key → `PlayerIntent` (client) |
| `WorldPlugin`       | sim        | solar system spawn, orbital motion, asteroid tumble          |
| `ResourcesPlugin`   | sim        | mining beam, cargo transfer, depletion, regeneration         |
| `PersistencePlugin` | sim        | RON save/load of ship + world state                          |
| `NetworkingPlugin`  | stub       | pins down the future client/server seam (`NetRole`)          |
| `GameCameraPlugin`  | client     | smooth-follow camera                                         |
| `VisualsPlugin`     | client     | render interpolation, parallax starfield                     |
| `EffectsPlugin`     | client     | thrust/mining particles, mining beam, depletion bursts       |
| `UiPlugin`          | client     | HUD (hull, speed, cargo), mining progress, minimap, pause    |

`PlayerPlugin` registers both halves today; splitting it for a server build
is a matter of moving two `add_systems` calls behind a cfg/feature flag.

Pure math (orbit positions, thrust integration, cargo accounting, extraction
rates, belt generation, parallax wrapping) lives in `src/logic/` with no
engine dependencies beyond `Vec2`, and is covered by unit tests.

## World model

```
SolarSystem "Helios"            (root entity, children below)
├── CentralStar                 (static, glow halos as children)
├── Planet ×5                   (Orbit, SimPosition, ResourceDeposit, BodyRadius)
├── Orbit rings                 (decorative annulus meshes)
└── Asteroid ×220               (SimPosition, SimRotation, Spin,
                                 ResourceDeposit, BodyRadius)
```

- **One solar system per root entity.** Adding more systems later means
  spawning more `SolarSystem` roots (each with its own star/planets/belts)
  and jump-gate entities that teleport ships between them. Nothing in the
  simulation assumes a single system; the only global is the shared
  `SimClock`.
- **Planets orbit deterministically**: position is computed from
  `SimClock.elapsed` every tick (`logic/orbit.rs`, f64 math so precision
  holds over week-long uptimes). No integration drift, nothing to persist
  except the clock.
- **Asteroid belts are seeded**: `logic/belt.rs` generates each belt from an
  explicit seed in config, so world layout is reproducible from config alone.
- **Deposits**: asteroids carry finite `ResourceDeposit`s (`regen == 0`) and
  despawn when exhausted; planets have large, slowly regenerating pools.

## Data-driven configuration

`assets/config/game.ron` (schema in `src/config.rs`) defines ship stats, the
star, every planet (radius, color, orbit, deposit) and every belt (radii,
count, seed, resource mix). `GameConfig::default()` mirrors the file so the
game runs even without assets. New resources are added in one place
(`resource_types.rs`); cargo, HUD, mining and saves handle them generically.

## Persistence

`PersistencePlugin` serializes a `SaveGame` struct to `save.ron` (manual F5,
autosave every 30 s, and on exit) and loads it before startup systems run.
The schema is deliberately decoupled from entity ids:

- ship: position, rotation, velocity, hull, stats, cargo
- world: `SimClock.elapsed` (which implies all planet positions), per-planet
  deposit levels keyed by config index, and the full list of surviving
  asteroids by value (depleted ones are simply absent)

A version field guards against schema drift. On an MMO server the same
schema becomes the per-player / per-system database record.

## Multiplayer plan (not yet built)

The intended split, using [`bevy_replicon`](https://github.com/projectharmonia/bevy_replicon)
or [`lightyear`](https://github.com/cBournhonesque/lightyear):

1. **Server binary**: Bevy `MinimalPlugins` + `SimulationPlugin`,
   `WorldPlugin`, `ResourcesPlugin`, `PersistencePlugin`, and the sim half of
   `PlayerPlugin` — the exact `FixedUpdate` systems that exist today, running
   headless. One ship entity per connected player instead of a singleton.
2. **Client**: sends `PlayerIntent` each tick (it is already a standalone
   serializable struct); receives replicated components (`SimPosition`,
   `SimRotation`, `Velocity`, `Cargo`, `ResourceDeposit`, ...) — the same
   serde derives used by saves. The existing interpolation layer smooths
   snapshots; the existing HUD reads replicated components unchanged.
3. **Authority**: the server owns all mutation (mining checks range/cargo
   server-side already, since those systems only read `PlayerIntent`).
   Client-side prediction can be added later by re-running
   `logic/physics::step_ship` locally — it is a pure function by design.
4. **Sharding**: one `SolarSystem` root per shard/zone; the seeded,
   config-driven world generation means shards need only sync deposits and
   ships, not layout.

## Known simplifications (deliberate for this pass)

- Single player ship as a singleton entity (`PlayerShip` marker +
  `PlayerIntent` as a resource). The MMO version keys intent by player id.
- No collision/damage yet: `Hull` exists and is saved, but nothing damages
  the ship.
- Mining targets the nearest deposit only; no manual target selection.
- Sprites are generated shapes (meshes/quads), no textures or audio yet.
- Spawn systems attach render components (meshes/materials) inline with the
  sim components. A true server build would split spawning into "insert sim
  components" (shared) and "attach visuals" (client-only, e.g. an observer
  reacting to replicated entities). The seam is already narrow: only
  `spawn_player`, `spawn_solar_system`/`spawn_asteroid` touch render assets.
