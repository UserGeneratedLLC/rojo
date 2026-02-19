#!/usr/bin/env bash
# Build and publish the Atlas plugin to the Creator Store via OpenCloud.
# Loads env vars from .env file in the repo root.
# Required .env vars: PLUGIN_UPLOAD_TOKEN, PLUGIN_CI_UNIVERSE_ID, PLUGIN_CI_PLACE_ID

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENV_FILE="$SCRIPT_DIR/../.env"

if [ -f "$ENV_FILE" ]; then
    set -a
    source "$ENV_FILE"
    set +a
fi

if [ -z "${PLUGIN_UPLOAD_TOKEN:-}" ]; then
    echo "Error: PLUGIN_UPLOAD_TOKEN not set. Add it to .env or set as env var." >&2
    exit 1
fi

export RBX_API_KEY="$PLUGIN_UPLOAD_TOKEN"
export RBX_UNIVERSE_ID="${PLUGIN_CI_UNIVERSE_ID:-}"
export RBX_PLACE_ID="${PLUGIN_CI_PLACE_ID:-}"

echo "Building and uploading plugin..."
lune run upload-plugin Atlas.rbxm
