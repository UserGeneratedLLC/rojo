#Requires -Version 5.1

$ProjectRbxl = "Project.rbxl"
$ProjectJsonFile = "default.project.json"

# Check for clean flag and filter it out
$DoClean = $false
$FilteredArgs = @()
foreach ($Arg in $args) {
  if ($Arg -eq "clean") {
    $DoClean = $true
  } else {
    $FilteredArgs += $Arg
  }
}

# Load .env file
Get-Content ".env" | ForEach-Object {
  if ($_ -match "^\s*([^#][^=]+)=(.*)$") {
    Set-Variable -Name $Matches[1].Trim() -Value $Matches[2].Trim() -Scope Script
  }
}

$ProjectJson = Get-Content $ProjectJsonFile | ConvertFrom-Json
$PlaceId = $ProjectJson.servePlaceIds[0]
Write-Host "PlaceId: $PlaceId"

# Download rbxl from Roblox
$Url = "https://assetdelivery.roblox.com/v1/asset/?id=$PlaceId"
$Session = New-Object Microsoft.PowerShell.Commands.WebRequestSession
$Cookie = New-Object System.Net.Cookie(".ROBLOSECURITY", $ROBLOSECURITY, "/", ".roblox.com")
$Session.Cookies.Add($Cookie)

Write-Host "Downloading $Url..."
Invoke-WebRequest -Uri $Url -WebSession $Session -OutFile $ProjectRbxl

# Delete src/ completely and recreate directory structure if --clean specified
if ($DoClean) {
  Write-Host "Cleaning src/ directory..."
  if (Test-Path "src") {
    Remove-Item -Path "src" -Recurse -Force
  }
  New-Item -ItemType Directory -Path "src/ReplicatedFirst" -Force | Out-Null
  New-Item -ItemType Directory -Path "src/ReplicatedStorage" -Force | Out-Null
  New-Item -ItemType Directory -Path "src/ServerScriptService" -Force | Out-Null
  New-Item -ItemType Directory -Path "src/ServerStorage" -Force | Out-Null
  New-Item -ItemType Directory -Path "src/StarterPlayer/StarterCharacterScripts" -Force | Out-Null
  New-Item -ItemType Directory -Path "src/StarterPlayer/StarterPlayerScripts" -Force | Out-Null
  New-Item -ItemType Directory -Path "src/Workspace" -Force | Out-Null
}

if ($FilteredArgs.Count -eq 0) {
  rojo syncback --non-interactive --input $ProjectRbxl $ProjectJsonFile
} else {
  rojo syncback --non-interactive @FilteredArgs --input $ProjectRbxl $ProjectJsonFile
}

# Stage all changes
if ($DoClean) {
  Write-Host "Running git add..."
  git add .
}
