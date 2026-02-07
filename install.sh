#!/bin/bash
set -e

MODE="${1:-release}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

cd "$SCRIPT_DIR"
cargo build "--$MODE"
ATLAS="./target/$MODE/atlas"
"$ATLAS" build plugin.project.json --plugin Atlas.rbxm
pkill -f atlas || true
sudo cp "$ATLAS" /usr/local/bin/
