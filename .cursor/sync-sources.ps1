# sync-sources.ps1
# Syncs external documentation from GitHub repos into the rules directory

$ErrorActionPreference = "Stop"
$ProgressPreference = 'SilentlyContinue'

# Configuration: Owner, Repo, Branch, Subdir, Target
$repos = @(
    @{
        Owner = "luau-lang"
        Repo = "rfcs"
        Branch = "master"
        Subdir = "docs"
        Target = "rules/luau-rfcs"
    },
    @{
        Owner = "luau-lang"
        Repo = "site"
        Branch = "master"
        Subdir = "src/content/docs"
        Target = "rules/luau"
    },
    @{
        Owner = "Roblox"
        Repo = "creator-docs"
        Branch = "main"
        Subdir = "content"
        Target = "rules/roblox"
    },
    @{
        Owner = "centau"
        Repo = "vide"
        Branch = "main"
        Subdir = "docs"
        Target = "rules/vide"
    }
)

# Single file downloads: Url, Target
$files = @(
    #@{
    #    Url = "https://raw.githubusercontent.com/UserGeneratedLLC/rojo/refs/heads/master/.cursor/rules/atlas-project.mdc"
    #    Target = "rules/atlas-project.mdc"
    #}
)

# Get script directory (workspace root)
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path

Write-Host "Starting sync process..." -ForegroundColor Cyan

foreach ($repo in $repos) {
    $zipUrl = "https://github.com/$($repo.Owner)/$($repo.Repo)/archive/refs/heads/$($repo.Branch).zip"
    $tempZip = Join-Path $env:TEMP "sync-docs-$($repo.Repo)-$(Get-Random).zip"
    $tempExtract = Join-Path $env:TEMP "sync-docs-$($repo.Repo)-$(Get-Random)"
    $targetPath = Join-Path $scriptDir $repo.Target
    
    # After extraction, the folder will be named like "repo-branch"
    $extractedFolder = Join-Path $tempExtract "$($repo.Repo)-$($repo.Branch)"
    $sourceSubdir = Join-Path $extractedFolder $repo.Subdir

    Write-Host ""
    Write-Host "Processing: $($repo.Repo)" -ForegroundColor Yellow
    Write-Host "  Source: $($repo.Owner)/$($repo.Repo) -> $($repo.Subdir)/"
    Write-Host "  Target: $($repo.Target)/"

    try {
        # Download zip archive
        Write-Host "  Downloading archive..." -ForegroundColor Gray
        (New-Object System.Net.WebClient).DownloadFile($zipUrl, $tempZip)

        # Extract zip (using tar for speed, built into Windows 10+)
        Write-Host "  Extracting..." -ForegroundColor Gray
        New-Item -ItemType Directory -Path $tempExtract -Force | Out-Null
        tar -xf $tempZip -C $tempExtract

        # Verify source directory exists
        if (-not (Test-Path $sourceSubdir)) {
            throw "Source subdirectory '$($repo.Subdir)' not found in archive"
        }

        # Remove old target directory contents
        if (Test-Path $targetPath) {
            Write-Host "  Removing old files..." -ForegroundColor Gray
            Remove-Item -Path $targetPath -Recurse -Force
        }

        # Create target directory
        New-Item -ItemType Directory -Path $targetPath -Force | Out-Null

        # Move new files (faster than copy since we delete temp anyway)
        Write-Host "  Moving files..." -ForegroundColor Gray
        robocopy $sourceSubdir $targetPath /E /MOVE /NFL /NDL /NJH /NJS /NC /NS /NP > $null
        # Robocopy exit codes < 8 are success (1=files copied, 2=extras, etc.)
        if ($LASTEXITCODE -ge 8) {
            throw "robocopy failed with exit code $LASTEXITCODE"
        }

        Write-Host "  Done!" -ForegroundColor Green

    } catch {
        Write-Host "  ERROR: $_" -ForegroundColor Red
        exit 1
    } finally {
        # Clean up temp files
        Write-Host "  Cleaning up temp files..." -ForegroundColor Gray
        if (Test-Path $tempZip) { Remove-Item -Path $tempZip -Force -ErrorAction SilentlyContinue }
        if (Test-Path $tempExtract) { Remove-Item -Path $tempExtract -Recurse -Force -ErrorAction SilentlyContinue }
    }
}

# Download single files
foreach ($file in $files) {
    $targetPath = Join-Path $scriptDir $file.Target
    $fileName = Split-Path -Leaf $file.Target

    Write-Host ""
    Write-Host "Processing: $fileName" -ForegroundColor Yellow
    Write-Host "  Source: $($file.Url)"
    Write-Host "  Target: $($file.Target)"

    try {
        Write-Host "  Downloading..." -ForegroundColor Gray
        $targetDir = Split-Path -Parent $targetPath
        if (-not (Test-Path $targetDir)) {
            New-Item -ItemType Directory -Path $targetDir -Force | Out-Null
        }
        Invoke-WebRequest -Uri $file.Url -OutFile $targetPath -UseBasicParsing
        Write-Host "  Done!" -ForegroundColor Green
    } catch {
        Write-Host "  ERROR: $_" -ForegroundColor Red
        exit 1
    }
}

# Roblox Full API Dump (dynamic: resolve latest build GUID first)
Write-Host ""
Write-Host "Processing: Full-API-Dump.json" -ForegroundColor Yellow
try {
    $latestMeta = Invoke-WebRequest -Uri "https://raw.githubusercontent.com/RobloxAPI/build-archive/master/data/production/latest.json" -UseBasicParsing | ConvertFrom-Json
    $guid = $latestMeta.GUID
    Write-Host "  GUID: $guid  Version: $($latestMeta.Version)"
    $dumpUrl = "https://raw.githubusercontent.com/RobloxAPI/build-archive/master/data/production/builds/$guid/Full-API-Dump.json"
    $dumpTarget = Join-Path $scriptDir "rules/roblox-api/Full-API-Dump.json"
    New-Item -ItemType Directory -Path (Split-Path -Parent $dumpTarget) -Force | Out-Null
    (New-Object System.Net.WebClient).DownloadFile($dumpUrl, $dumpTarget)
    Write-Host "  Done!" -ForegroundColor Green
} catch {
    Write-Host "  ERROR: $_" -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "Sync completed successfully!" -ForegroundColor Green
exit 0
