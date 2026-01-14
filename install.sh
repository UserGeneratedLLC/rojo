#!/bin/bash
set -e

MODE="${1:-release}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

cd "$SCRIPT_DIR"
cargo build "--$MODE"
ROJO="./target/$MODE/rojo"
"$ROJO" build plugin.project.json --plugin Rojo.rbxm
pkill -f rojo || true
sudo cp "$ROJO" /usr/local/bin/
