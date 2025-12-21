#Requires -Version 5.1

param([string]$Mode = "release")

Set-Location $PSScriptRoot
cargo build "--$Mode"
$ExePath = ".\target\$Mode\rojo.exe"
& "$ExePath" build plugin.project.json --plugin Rojo.rbxm
if ($LASTEXITCODE -ne 0) { throw "Plugin build failed" }
gsudo Stop-Process -Name "rojo" -Force -ErrorAction SilentlyContinue
gsudo Copy-Item "$ExePath" "C:\Program Files\Rojo\"
