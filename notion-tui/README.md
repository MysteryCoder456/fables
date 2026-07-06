# notion-tui

A keyboard-first terminal client for Notion. Offline-first: everything renders
from a local SQLite cache; edits queue locally and sync in the background.

## Install

Prebuilt static binaries: grab `notion-tui-<arch>-unknown-linux-musl` from a
release, `chmod +x`, and put it on your `PATH`.

From source:

    cargo install --path crates/notion-tui

Static release binaries for both supported targets:

    ./scripts/build-release.sh   # emits dist/notion-tui-{aarch64,x86_64}-unknown-linux-musl

## Setup

Run `notion-tui`. On first run it prompts for a Notion internal-integration
token (create one at notion.so/profile/integrations and share pages with it),
validates it, and stores it in your system keyring (or, with a warning, in
`~/.config/notion-tui/config.toml` with 0600 permissions).

## Config (`~/.config/notion-tui/config.toml`)

    poll_interval_secs = 30       # background sync interval
    theme = "default"             # default | dark | light
    mouse = true
    editor = "nvim"               # overrides $EDITOR for the `e` flow
    db_path = "/custom/notion.db"

    [keys]                        # rebind any action (see `?` in-app for the list)
    quit = "x"
    search = "f"

## Keys

Press `?` in the app for the full, live (rebind-aware) list.
