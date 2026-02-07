# Watch and rebuild the Rojo plugin on file changes
# Make sure you've enabled the Studio setting to reload plugins on file change!

$ErrorActionPreference = "Stop"

Write-Host "Watching for plugin changes... (Ctrl+C to stop)" -ForegroundColor Cyan
atlas build plugin.project.json --plugin Atlas.rbxm --watch
