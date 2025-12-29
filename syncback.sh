#!/bin/sh
set -e

ProjectRbxl="Project.rbxl"
ProjectJsonFile="default.project.json"

# Load .env file (auto-export all variables)
if [ -f ".env" ]; then
  set -a
  . ./.env
  set +a
fi

# Extract PlaceId from project json (first number after servePlaceIds line)
PlaceId=$(grep -A1 'servePlaceIds' "$ProjectJsonFile" | grep -o '[0-9]\+')
echo "PlaceId: $PlaceId"

# Download rbxl from Roblox
Url="https://assetdelivery.roblox.com/v1/asset/?id=$PlaceId"
echo "Downloading $Url..."
curl -fSL --compressed -o "$ProjectRbxl" -H "Cookie: .ROBLOSECURITY=${ROBLOSECURITY}" "$Url"

if [ $# -eq 0 ]; then
  rojo syncback --non-interactive --input "$ProjectRbxl" "$ProjectJsonFile"
else
  rojo syncback "$@" --input "$ProjectRbxl" "$ProjectJsonFile"
fi
