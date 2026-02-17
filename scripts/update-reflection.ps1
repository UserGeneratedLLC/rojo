# Update Reflection Database
# Regenerates database.msgpack (Rust) and database.json (Lua) from Roblox Studio.
# Requires: Roblox Studio installed locally.
# Usage: .\scripts\update-reflection.ps1 [--dry-run]

param(
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"

Push-Location rbx-dom

try {
    if ($DryRun) {
        Write-Host "Dry run: generating reflection database (no output files)" -ForegroundColor Yellow
        cargo run --bin rbx_reflector -- generate --patches patches
    } else {
        Write-Host "Generating reflection database..." -ForegroundColor Cyan
        cargo run --bin rbx_reflector -- generate --patches patches rbx_reflection_database/database.msgpack rbx_dom_lua/src/database.json

        Write-Host "Generating allValues.json..." -ForegroundColor Cyan
        cargo run --bin rbx_reflector -- values rbx_dom_lua/src/allValues.json

        if ($LASTEXITCODE -ne 0) {
            Write-Host "Failed with exit code $LASTEXITCODE" -ForegroundColor Red
            exit $LASTEXITCODE
        }

        Write-Host "Copying to plugin/rbx_dom_lua/..." -ForegroundColor Cyan
        Copy-Item rbx_dom_lua/src/database.json ../plugin/rbx_dom_lua/database.json
        Copy-Item rbx_dom_lua/src/allValues.json ../plugin/rbx_dom_lua/allValues.json
        Copy-Item rbx_dom_lua/src/Error.lua ../plugin/rbx_dom_lua/Error.lua
        Copy-Item rbx_dom_lua/src/base64.lua ../plugin/rbx_dom_lua/base64.lua
        Copy-Item rbx_dom_lua/src/PropertyDescriptor.lua ../plugin/rbx_dom_lua/PropertyDescriptor.lua
        Copy-Item rbx_dom_lua/src/customProperties.lua ../plugin/rbx_dom_lua/customProperties.lua
        # NOTE: init.lua and EncodedValue.lua are intentionally different in the plugin
        # (findClassDescriptor, Int64/msgpack handling, float serialization) — do NOT copy them.
    }

    Write-Host "Done." -ForegroundColor Green
} finally {
    Pop-Location
}
