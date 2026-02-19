# Build and publish the Atlas plugin to the Creator Store via OpenCloud.
# Loads env vars from .env file in the repo root.
# Required .env vars: PLUGIN_UPLOAD_TOKEN, PLUGIN_CI_UNIVERSE_ID, PLUGIN_CI_PLACE_ID

$ErrorActionPreference = "Stop"

$envFile = Join-Path $PSScriptRoot "..\.env"
if (Test-Path $envFile) {
    foreach ($line in Get-Content $envFile) {
        $line = $line.Trim()
        if ($line -and -not $line.StartsWith("#")) {
            $key, $value = $line -split "=", 2
            if ($key -and $value) {
                [Environment]::SetEnvironmentVariable($key.Trim(), $value.Trim(), "Process")
            }
        }
    }
}

if (-not $env:PLUGIN_UPLOAD_TOKEN) {
    Write-Host "Error: PLUGIN_UPLOAD_TOKEN not set. Add it to .env or set as env var." -ForegroundColor Red
    exit 1
}

$env:RBX_API_KEY = $env:PLUGIN_UPLOAD_TOKEN
$env:RBX_UNIVERSE_ID = $env:PLUGIN_CI_UNIVERSE_ID
$env:RBX_PLACE_ID = $env:PLUGIN_CI_PLACE_ID

Write-Host "Building and uploading plugin..." -ForegroundColor Cyan
lune run upload-plugin Atlas.rbxm
