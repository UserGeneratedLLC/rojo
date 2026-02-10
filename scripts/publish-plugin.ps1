# Build and publish the Rojo plugin to the Creator Store
# Loads API key from .env file, ROBLOX_API_KEY env var, or -ApiKey argument.
# Get an API key at https://create.roblox.com/credentials (assets read+write).

param(
    [string]$ApiKey
)

$ErrorActionPreference = "Stop"

# Load .env file if it exists
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

# Resolve API key: argument > env var > .env (already loaded above)
if (-not $ApiKey) {
    $ApiKey = $env:ROBLOX_API_KEY
}

if (-not $ApiKey -or $ApiKey -eq "your-key-here") {
    Write-Host "Error: No API key provided." -ForegroundColor Red
    Write-Host "Set ROBLOX_API_KEY in .env, env var, or pass -ApiKey <key>" -ForegroundColor Yellow
    exit 1
}

Write-Host "Building and uploading plugin..." -ForegroundColor Cyan
cargo run -- upload plugin.project.json --asset_id 111151139098227
