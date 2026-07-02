# Space Game

A 2D, top-down, multiplayer space RPG built in Rust with [Bevy](https://bevy.org).
You pilot a mining ship through a shared, living solar system: planets orbit a
central star in real time, asteroid belts hold mineable resources, other
players fly and mine alongside you, and your progress persists on the server
between sessions.

The game is client-server: a headless `space_game_server` runs the
authoritative simulation; any number of `space_game` clients connect to it
over TCP. See [`ARCHITECTURE.md`](ARCHITECTURE.md) for the ECS layout and the
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
`SPACE_GAME_NAME` or a generated `pilot-<pid>`. Pilot names identify you to
the server: reconnect with the same name and you get your ship back, cargo
and all. Two clients can't be online with the same name at once.

Playing solo is the same thing with one client. For fast iteration builds,
add `--features dev` (dev-only dynamic linking). Tests and lints:

```sh
cargo test     # unit tests for simulation logic and the wire protocol
cargo clippy   # must pass cleanly
```

## Controls

| Key            | Action                            |
| -------------- | --------------------------------- |
| `W` / `Up`     | Thrust forward                    |
| `A` / `Left`   | Rotate counter-clockwise          |
| `D` / `Right`  | Rotate clockwise                  |
| `S` / `Down`   | Brake (heavy drag)                |
| `Space` (hold) | Mine the nearest deposit in range |

Flight is thrust-based: the ship has inertia and drag, so line up your vector
before you burn. Fly close to an asteroid or planet and hold `Space` to mine;
extracted units fill your cargo hold (watch the HUD). Asteroids run dry and
break apart permanently — for everyone; planets regenerate slowly. Your ship
is the white one; other pilots are amber.

## Gameplay data

All tuning lives in [`assets/config/game.ron`](assets/config/game.ron): ship
stats, planet orbits, asteroid belt density, resource richness. Edit and
restart — no recompile needed. The server is authoritative for world layout;
clients use the config only for cosmetics (colors, radii), so keep the same
file on both sides.

The server saves the world and every pilot's ship to `save.ron` (in its
working directory) every 30 seconds and on clean exit. Delete the file for a
fresh universe.

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
  logic/             Pure, engine-independent logic (unit-tested)
  plugins/           One Bevy plugin per major system, split client/server
assets/config/       Gameplay tuning (RON)
```
