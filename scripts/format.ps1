# Auto-fix Formatting & Static Analysis
# Usage: .\scripts\format.ps1 [-NoRbxDom]

param(
    [switch]$NoRbxDom
)

$RbxDom = -not $NoRbxDom
$Threads = [Environment]::ProcessorCount

$ErrorActionPreference = "Continue"
$failures = @()

function Write-Step($number, $name) {
    Write-Host ""
    Write-Host "=== Step ${number}: $name ===" -ForegroundColor Cyan
}

function Record-Result($step, $exitCode) {
    if ($exitCode -ne 0) {
        $script:failures += $step
        Write-Host "FAIL" -ForegroundColor Red
    } else {
        Write-Host "PASS" -ForegroundColor Green
    }
}

# ─── Rojo ────────────────────────────────────────────────────────────────────

Write-Step 1 "Auto-fix Lua Formatting (Stylua)"
stylua plugin/src
$styluaRojo = $LASTEXITCODE
Record-Result "Stylua (Rojo)" $styluaRojo

Write-Step 2 "Auto-fix Rust Formatting"
cargo fmt
$rustfmt = $LASTEXITCODE
Record-Result "Rustfmt" $rustfmt

Write-Step 3 "Auto-fix TypeScript Formatting (Prettier)"
Push-Location vscode-rojo
npx prettier --write "src/**/*.ts"
$prettier = $LASTEXITCODE
Pop-Location
Record-Result "Prettier" $prettier

Write-Step 4 "Lua Static Analysis (Selene)"
selene plugin/src
$selene = $LASTEXITCODE
Record-Result "Selene" $selene

Write-Step 5 "Rust Linting (Clippy) - Auto-fix"
cargo clippy -j $Threads --fix --allow-dirty --allow-staged 2>&1 | Out-Null
Write-Host "Verifying..." -ForegroundColor Yellow
cargo clippy -j $Threads --all-targets --all-features 2>&1
$clippy = $LASTEXITCODE
Record-Result "Clippy" $clippy

# ─── rbx-dom ─────────────────────────────────────────────────────────────────

if ($RbxDom) {
    Write-Step 6 "Auto-fix Lua Formatting (Stylua) for rbx_dom_lua"
    stylua rbx-dom/rbx_dom_lua/src
    $styluaRbxDom = $LASTEXITCODE
    Record-Result "Stylua (rbx_dom_lua)" $styluaRbxDom

    Write-Step 7 "Auto-fix Rust Formatting (rbx-dom)"
    Push-Location rbx-dom
    cargo fmt
    $rustfmtRbxDom = $LASTEXITCODE
    Pop-Location
    Record-Result "Rustfmt (rbx-dom)" $rustfmtRbxDom

    Write-Step 8 "Lua Static Analysis (Selene) for rbx_dom_lua"
    selene rbx-dom/rbx_dom_lua/src
    $seleneRbxDom = $LASTEXITCODE
    Record-Result "Selene (rbx_dom_lua)" $seleneRbxDom

    Write-Step 9 "Rust Linting (Clippy) for rbx-dom - Auto-fix"
    Push-Location rbx-dom
    cargo clippy -j $Threads --fix --allow-dirty --allow-staged 2>&1 | Out-Null
    Write-Host "Verifying..." -ForegroundColor Yellow
    cargo clippy -j $Threads --all-targets --all-features -- -D warnings 2>&1
    $clippyRbxDom = $LASTEXITCODE
    Pop-Location
    Record-Result "Clippy (rbx-dom)" $clippyRbxDom
}

# ─── Report ──────────────────────────────────────────────────────────────────

Write-Host ""
Write-Host "=== FORMAT COMPLETE ===" -ForegroundColor Magenta
Write-Host ""
Write-Host "--- Rojo ---" -ForegroundColor White
Write-Host ""
Write-Host "Formatting:"
Write-Host "  - Stylua: $(if ($styluaRojo -eq 0) {'PASS'} else {'FAIL'})"
Write-Host "  - Rustfmt: $(if ($rustfmt -eq 0) {'PASS'} else {'FAIL'})"
Write-Host "  - Prettier (vscode-rojo): $(if ($prettier -eq 0) {'PASS'} else {'FAIL'})"
Write-Host ""
Write-Host "Linting:"
Write-Host "  - Selene: $(if ($selene -eq 0) {'PASS'} else {'FAIL'})"
Write-Host "  - Clippy: $(if ($clippy -eq 0) {'PASS'} else {'FAIL'})"

if ($RbxDom) {
    Write-Host ""
    Write-Host "--- rbx-dom ---" -ForegroundColor White
    Write-Host ""
    Write-Host "Formatting:"
    Write-Host "  - Stylua (rbx_dom_lua): $(if ($styluaRbxDom -eq 0) {'PASS'} else {'FAIL'})"
    Write-Host "  - Rustfmt: $(if ($rustfmtRbxDom -eq 0) {'PASS'} else {'FAIL'})"
    Write-Host ""
    Write-Host "Linting:"
    Write-Host "  - Selene (rbx_dom_lua): $(if ($seleneRbxDom -eq 0) {'PASS'} else {'FAIL'})"
    Write-Host "  - Clippy: $(if ($clippyRbxDom -eq 0) {'PASS'} else {'FAIL'})"
}

Write-Host ""
if ($failures.Count -eq 0) {
    Write-Host "--- Overall: PASS ---" -ForegroundColor Green
    exit 0
} else {
    Write-Host "--- Overall: FAIL ---" -ForegroundColor Red
    Write-Host "Failed steps: $($failures -join ', ')" -ForegroundColor Red
    exit 1
}
