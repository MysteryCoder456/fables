#!/usr/bin/env bash
# Builds static release binaries for both v1 targets (spec §8).
# Native target builds directly; the foreign target uses `cross` if installed.
set -euo pipefail
cd "$(dirname "$0")/.."

TARGETS=(aarch64-unknown-linux-musl x86_64-unknown-linux-musl)
HOST_ARCH=$(uname -m)
mkdir -p dist

for target in "${TARGETS[@]}"; do
    if [[ "$target" == "$HOST_ARCH"* ]]; then
        rustup target add "$target"
        cargo build --release --target "$target" -p notion-tui
    elif command -v cross >/dev/null; then
        cross build --release --target "$target" -p notion-tui
    else
        echo "skipping $target (install 'cross' for foreign-arch builds: cargo install cross)"
        continue
    fi
    cp "target/$target/release/notion-tui" "dist/notion-tui-$target"
    echo "built dist/notion-tui-$target"
done
