#Requires -Version 5.1

param([string]$Mode = "release")

$InstallDir = "C:\Program Files\Rojo"

Set-Location $PSScriptRoot
cargo build "--$Mode"
$Rojo = ".\target\$Mode\rojo.exe"
& "$Rojo" build plugin.project.json --plugin Rojo.rbxm
if ($LASTEXITCODE -ne 0) { throw "Plugin build failed" }

gsudo Stop-Process -Name "rojo" -Force -ErrorAction SilentlyContinue
gsudo New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
gsudo Copy-Item "$Rojo" "$InstallDir\"

$MachinePath = [Environment]::GetEnvironmentVariable("Path", "Machine")
if ($MachinePath -notlike "*$InstallDir*") {
  gsudo [Environment]::SetEnvironmentVariable "Path" "$MachinePath;$InstallDir" "Machine"
  Write-Host "Added '$InstallDir' to system PATH"
}
