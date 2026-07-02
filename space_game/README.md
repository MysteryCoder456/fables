# Space Game

A 2D, top-down, space-themed RPG built in Rust with [Bevy](https://bevy.org).
You pilot a mining ship through a living solar system: planets orbit a central
star in real time, asteroid belts hold mineable resources, and your progress
persists between sessions. The codebase is structured from day one for a
future MMO client/server split — see [`ARCHITECTURE.md`](ARCHITECTURE.md).

## Building and running

Prerequisites:

- Rust (stable, 1.95+) via [rustup](https://rustup.rs)
- On Linux, Bevy's system dependencies:

  ```sh
  sudo apt-get install libwayland-dev libasound2-dev libudev-dev pkg-config libxkbcommon-dev
  ```

Run the game:

```sh
cargo run                  # normal build
cargo run --features dev   # fast iteration build (dynamic linking, dev only)
cargo run --release        # optimized build
```

Tests and lints:

```sh
cargo test     # unit tests for the pure simulation logic
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
| `F5`           | Save game                         |
| `P`            | Pause / resume                    |

Flight is thrust-based: the ship has inertia and drag, so line up your vector
before you burn. Fly close to an asteroid or planet and hold `Space` to mine;
extracted units fill your cargo hold (watch the HUD). Asteroids run dry and
break apart permanently; planets regenerate slowly.

## Gameplay data

All tuning lives in [`assets/config/game.ron`](assets/config/game.ron): ship
stats, planet orbits, asteroid belt density, resource richness. Edit and
restart — no recompile needed. If the file is missing or invalid the game
falls back to identical built-in defaults.

Your session is saved to `save.ron` (next to the working directory) on `F5`,
every 30 seconds, and on exit. Delete the file for a fresh start.

## Project layout

```
src/
  main.rs            App wiring: which plugins a client runs
  components.rs      Serializable simulation components & resources
  config.rs          Data-driven config schema + RON loading
  resource_types.rs  The extensible ResourceType enum
  logic/             Pure, engine-independent logic (unit-tested)
  plugins/           One Bevy plugin per major system
assets/config/       Gameplay tuning (RON)
```
