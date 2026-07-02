# Helios Drift

A 2D, top-down MMORPG built in Rust with [Bevy](https://bevy.org), set in a
gravitationally live pair of star systems. Pilots mine asteroid belts under
real Newtonian gravity, haul ore between planets whose markets pay different
prices, buy ship upgrades with the profits, settle disputes with blasters,
and jump between systems through gates — all on one authoritative server
that remembers every pilot by name.

The game is client-server: a headless `space_game_server` runs the
simulation; any number of `space_game` clients connect over TCP. See
[`ARCHITECTURE.md`](ARCHITECTURE.md) for the ECS layout, physics model and
wire protocol.

## Building and running

Prerequisites:

- Rust (stable, 1.95+) via [rustup](https://rustup.rs)
- On Linux, Bevy's system dependencies:

  ```sh
  sudo apt-get install libwayland-dev libasound2-dev libudev-dev pkg-config libxkbcommon-dev
  ```

Start a server, then connect one or more clients:

```sh
# Terminal 1 — the authoritative server (default port 5123)
cargo run --bin space_game_server

# Terminal 2, 3, ... — one client per player
cargo run --bin space_game -- 127.0.0.1:5123 your-pilot-name
```

Client arguments are optional: the server address defaults to
`127.0.0.1:5123` (or `SPACE_GAME_SERVER`), the pilot name to
`SPACE_GAME_NAME` or a generated `pilot-<pid>`. Pilot names are identity:
reconnect with the same name and the server restores your ship, credits and
upgrades. Two clients can't be online with the same name at once.

Tests and lints:

```sh
cargo test     # simulation logic, physics, economy and protocol tests
cargo clippy   # must pass cleanly
```

## Controls

| Key            | Action                                     |
| -------------- | ------------------------------------------ |
| `W` / `Up`     | Thrust forward                             |
| `A` / `Left`   | Rotate counter-clockwise                   |
| `D` / `Right`  | Rotate clockwise                           |
| `S` / `Down`   | Brake (heavy drag)                         |
| `Space` (hold) | Mine the nearest deposit in range          |
| `F` (hold)     | Fire blaster                               |
| `1`–`4`        | While docked: sell a resource stack        |
| `5`–`9`        | While docked: buy the next upgrade tier    |
| `Enter`        | Open chat / send; `Esc` cancels            |

## How to play

**Fly like it's space.** Stars and planets pull on your ship (and on blaster
bolts) with inverse-square gravity; drag is minimal. Line up burns, use
gravity assists, brake with `S`. Collisions are real: hard impacts against
rock, planets, or other pilots cost hull, and star coronas burn. Lose all
hull and you wake up in a fresh hull at the home spawn — cargo gone, credits
and upgrades intact.

**Earn.** Mine asteroids (they deplete and shatter; belts regrow slowly) or
planetary pools (they regenerate). Fly close to any planet to dock and sell:
every planet posts different prices, so hauling ore where it's scarce pays
best — Cryon's frontier worlds pay a premium if you survive the trip.

**Progress.** Credits buy five upgrade tracks (thrusters, cargo bay, mining
laser, hull plating, blaster), each with five tiers of doubling cost. Your
"LVL" is the sum of your tiers.

**Travel.** The cyan gate rings connect Helios to the icy Cryon system.
Fly in; you'll be thrown out of the partner gate 80,000 units away.

**Fight (or don't).** Blasters are hitscan-free: bolts inherit your velocity
and curve in gravity wells. Kills are announced with attribution. Fresh
respawns get a 3-second shield.

## Gameplay data

Everything numeric lives in [`assets/config/game.ron`](assets/config/game.ron):
ship stats, physics constants (gravity, restitution, damage thresholds),
both star systems, per-planet markets, belt density and respawn rates, gate
positions. Edit and restart — no recompile. Server and client must share the
same file. After changing the schema in `src/config.rs`, regenerate with
`cargo test regenerate_shipped_config -- --ignored`.

The server saves the whole universe plus every pilot's ship to `save.ron`
every 30 seconds and on clean exit.

## Project layout

```
src/
  main.rs            Client binary: rendering, input, server connection
  bin/server.rs      Server binary: headless authoritative simulation
  lib.rs             Shared library
  components.rs      Serializable simulation components & resources
  protocol.rs        TCP framing + client/server messages (bincode)
  config.rs          Data-driven config schema + RON loading
  resource_types.rs  The extensible ResourceType enum
  logic/             Pure, engine-independent logic (unit-tested):
                     thrust physics, gravity, collisions, orbits, belts,
                     cargo, mining, markets/upgrades, parallax
  plugins/           One Bevy plugin per major system, split client/server
assets/config/       Gameplay tuning (RON)
```
