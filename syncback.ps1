#Requires -Version 5.1

$ProjectRbxl = "Project.rbxl"
$ProjectJsonFile = "default.project.json"

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

if ($args.Count -eq 0) {
  rojo syncback --non-interactive --input $ProjectRbxl $ProjectJsonFile
} else {
  rojo syncback @args --input $ProjectRbxl $ProjectJsonFile
}
