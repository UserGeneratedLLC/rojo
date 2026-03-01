#!/bin/bash
# sync-sources.sh
# Syncs external documentation from GitHub repos into the rules directory

set -e

# Get script directory (workspace root)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
GRAY='\033[0;90m'
NC='\033[0m' # No Color

echo -e "${CYAN}Starting sync process...${NC}"

# Function to sync a single repo
sync_repo() {
    local owner="$1"
    local repo="$2"
    local branch="$3"
    local subdir="$4"
    local target="$5"
    
    local zip_url="https://github.com/$owner/$repo/archive/refs/heads/$branch.zip"
    local temp_zip=$(mktemp)
    local temp_extract=$(mktemp -d)
    local target_path="$SCRIPT_DIR/$target"
    local extracted_folder="$temp_extract/$repo-$branch"
    local source_subdir="$extracted_folder/$subdir"
    
    echo ""
    echo -e "${YELLOW}Processing: $repo${NC}"
    echo "  Source: $owner/$repo -> $subdir/"
    echo "  Target: $target/"
    
    # Download zip archive
    echo -e "${GRAY}  Downloading archive...${NC}"
    curl -sL "$zip_url" -o "$temp_zip"
    
    # Extract zip
    echo -e "${GRAY}  Extracting...${NC}"
    unzip -q "$temp_zip" -d "$temp_extract"
    
    # Verify source directory exists
    if [ ! -d "$source_subdir" ]; then
        echo -e "${RED}  ERROR: Source subdirectory '$subdir' not found in archive${NC}"
        rm -f "$temp_zip"
        rm -rf "$temp_extract"
        exit 1
    fi
    
    # Remove old target directory contents
    if [ -d "$target_path" ]; then
        echo -e "${GRAY}  Removing old files...${NC}"
        rm -rf "$target_path"
    fi
    
    # Create target directory
    mkdir -p "$target_path"
    
    # Copy new files
    echo -e "${GRAY}  Copying new files...${NC}"
    cp -r "$source_subdir"/* "$target_path/"
    
    # Clean up temp files
    echo -e "${GRAY}  Cleaning up temp files...${NC}"
    rm -f "$temp_zip"
    rm -rf "$temp_extract"
    
    echo -e "${GREEN}  Done!${NC}"
}

# Function to download a single file
download_file() {
    local url="$1"
    local target="$2"
    
    local target_path="$SCRIPT_DIR/$target"
    local file_name=$(basename "$target")
    
    echo ""
    echo -e "${YELLOW}Processing: $file_name${NC}"
    echo "  Source: $url"
    echo "  Target: $target"
    
    echo -e "${GRAY}  Downloading...${NC}"
    mkdir -p "$(dirname "$target_path")"
    curl -sL "$url" -o "$target_path"
    
    echo -e "${GREEN}  Done!${NC}"
}

# Sync each repository (owner, repo, branch, subdir, target)
sync_repo "luau-lang" "rfcs" "master" "docs" "rules/luau-rfcs"
sync_repo "luau-lang" "site" "master" "src/content/docs" "rules/luau"
sync_repo "Roblox" "creator-docs" "main" "content" "rules/roblox"
sync_repo "centau" "vide" "main" "docs" "rules/vide"

# Download single files
#download_file "https://raw.githubusercontent.com/UserGeneratedLLC/rojo/refs/heads/master/.cursor/rules/atlas-project.mdc" "rules/atlas-project.mdc"

# Roblox Full API Dump (dynamic: resolve latest build GUID first)
echo ""
echo -e "${YELLOW}Processing: Full-API-Dump.json${NC}"
latest_meta=$(curl -sL "https://raw.githubusercontent.com/RobloxAPI/build-archive/master/data/production/latest.json")
guid=$(echo "$latest_meta" | grep -o '"GUID":"[^"]*"' | cut -d'"' -f4)
version=$(echo "$latest_meta" | grep -o '"Version":"[^"]*"' | cut -d'"' -f4)
echo "  GUID: $guid  Version: $version"
dump_url="https://raw.githubusercontent.com/RobloxAPI/build-archive/master/data/production/builds/$guid/Full-API-Dump.json"
dump_target="$SCRIPT_DIR/rules/roblox-api/Full-API-Dump.json"
mkdir -p "$(dirname "$dump_target")"
curl -sL "$dump_url" -o "$dump_target"
echo -e "${GREEN}  Done!${NC}"

echo ""
echo -e "${GREEN}Sync completed successfully!${NC}"
