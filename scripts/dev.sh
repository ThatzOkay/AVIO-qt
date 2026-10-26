#!/usr/bin/env bash
# Runs the app and relaunches it whenever a .qml file changes. The QML is loaded from src/qml at
# startup (see main.rs), so a relaunch picks the edit up without compiling anything. Rust changes
# are not watched; rebuild with cargo yourself and the next relaunch uses the new binary.
set -euo pipefail
cd "$(dirname "$0")/.."

if ! command -v watchexec >/dev/null; then
    echo "watchexec not found: pkexec pacman -S watchexec" >&2
    exit 1
fi

# One build up front, so the first launch isn't also waiting on cargo
cargo build -j"$(( $(nproc) - 4 > 0 ? $(nproc) - 4 : 1 ))"

exec watchexec --restart --watch src/qml --exts qml -- target/debug/avio_qt
