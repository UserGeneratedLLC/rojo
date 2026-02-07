#Requires -Version 5.1

param([string]$Mode = "release")

$InstallDir = "C:\Program Files\Atlas"

Set-Location $PSScriptRoot
if ($Mode -eq "release") {
  cargo build --release --config "profile.release.debug=true"
} else {
  cargo build "--$Mode"
}
$Atlas = ".\target\$Mode\atlas.exe"
& "$Atlas" build plugin.project.json --plugin Atlas.rbxm
if ($LASTEXITCODE -ne 0) { throw "Plugin build failed" }

gsudo Stop-Process -Name "atlas" -Force -ErrorAction SilentlyContinue
gsudo New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
gsudo Copy-Item "$Atlas" "$InstallDir\"
gsudo Copy-Item ".\target\$Mode\atlas.pdb" "$InstallDir\"

$MachinePath = [Environment]::GetEnvironmentVariable("Path", "Machine")
if ($MachinePath -notlike "*$InstallDir*") {
  gsudo [Environment]::SetEnvironmentVariable "Path" "$MachinePath;$InstallDir" "Machine"
  Write-Host "Added '$InstallDir' to system PATH"
}
