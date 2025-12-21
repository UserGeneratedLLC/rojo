#Requires -Version 5.1

param([string]$Mode = "release")

Set-Location $PSScriptRoot
cargo build "--$Mode"
$Rojo = ".\target\$Mode\rojo.exe"
& "$Rojo" build plugin.project.json --plugin Rojo.rbxm
if ($LASTEXITCODE -ne 0) { throw "Plugin build failed" }
gsudo Stop-Process -Name "rojo" -Force -ErrorAction SilentlyContinue
gsudo Copy-Item "$Rojo" "C:\Program Files\Rojo\"
