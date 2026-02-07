# Run the Rojo plugin unit tests
# Requires: run-in-roblox (via rokit) and Roblox Studio installed

$ErrorActionPreference = "Stop"

Write-Host "Building test place..." -ForegroundColor Cyan
atlas build plugin/test-place.project.json -o TestPlace.rbxl

Write-Host "Running tests in Roblox..." -ForegroundColor Cyan
run-in-roblox --script plugin/run-tests.server.lua --place TestPlace.rbxl
