#!/bin/sh
set -e

ProjectRbxl="Project.rbxl"
ProjectJsonFile="default.project.json"

# Check for clean flag and filter it out
DoClean=false
FilteredArgs=""
for Arg in "$@"; do
  if [ "$Arg" = "clean" ]; then
    DoClean=true
  else
    FilteredArgs="$FilteredArgs $Arg"
  fi
done

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

# Delete src/ completely and recreate directory structure if --clean specified
if [ "$DoClean" = true ]; then
  echo "Cleaning src/ directory..."
  rm -rf src
  mkdir -p src/ReplicatedFirst
  mkdir -p src/ReplicatedStorage
  mkdir -p src/ServerScriptService
  mkdir -p src/ServerStorage
  mkdir -p src/StarterPlayer/StarterCharacterScripts
  mkdir -p src/StarterPlayer/StarterPlayerScripts
  mkdir -p src/Workspace
fi

if [ -z "$FilteredArgs" ]; then
  rojo syncback --non-interactive --input "$ProjectRbxl" "$ProjectJsonFile"
else
  # shellcheck disable=SC2086
  rojo syncback --non-interactive $FilteredArgs --input "$ProjectRbxl" "$ProjectJsonFile"
fi

# Stage all changes
if [ "$DoClean" = true ]; then
  echo "Running git add..."
  git add .
  git add --renormalize .
fi
