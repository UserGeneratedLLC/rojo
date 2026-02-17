#!/usr/bin/env bash
# Update Reflection Database
# Regenerates database.msgpack (Rust) and database.json (Lua) from Roblox Studio.
# Requires: Roblox Studio installed locally.
# Usage: bash scripts/update-reflection.sh [--dry-run]

set -e

cd "$(dirname "$0")/../rbx-dom"

if [ "$1" = "--dry-run" ]; then
    echo "Dry run: generating reflection database (no output files)"
    cargo run --bin rbx_reflector -- generate --patches patches
else
    echo "Generating reflection database..."
    cargo run --bin rbx_reflector -- generate --patches patches rbx_reflection_database/database.msgpack rbx_dom_lua/src/database.json

    echo "Generating allValues.json..."
    cargo run --bin rbx_reflector -- values rbx_dom_lua/src/allValues.json

    echo "Copying to plugin/rbx_dom_lua/..."
    cp rbx_dom_lua/src/database.json ../plugin/rbx_dom_lua/database.json
    cp rbx_dom_lua/src/allValues.json ../plugin/rbx_dom_lua/allValues.json
    cp rbx_dom_lua/src/Error.lua ../plugin/rbx_dom_lua/Error.lua
    cp rbx_dom_lua/src/base64.lua ../plugin/rbx_dom_lua/base64.lua
    cp rbx_dom_lua/src/PropertyDescriptor.lua ../plugin/rbx_dom_lua/PropertyDescriptor.lua
    cp rbx_dom_lua/src/customProperties.lua ../plugin/rbx_dom_lua/customProperties.lua
    # NOTE: init.lua and EncodedValue.lua are intentionally different in the plugin
    # (findClassDescriptor, Int64/msgpack handling, float serialization) — do NOT copy them.
fi

echo "Done."
