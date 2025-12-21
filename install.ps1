#Requires -Version 5.1

param([string]$Mode = "release")
$DeployFolder = "C:\Program Files\Rojo"

Set-Location $PSScriptRoot
cargo build "--$Mode"
& ".\target\$Mode\rojo.exe" build plugin.project.json --plugin Rojo.rbxm
if ($LASTEXITCODE -ne 0) { throw "Plugin build failed" }
gsudo {
  Stop-Process -Name "rojo" -Force -ErrorAction SilentlyContinue
  #Remove-Item "$DeployFolder\*" -Recurse -Force
  Copy-Item ".\target\$Mode\rojo.exe" "$DeployFolder\"
}
