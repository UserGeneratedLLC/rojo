# Complete CI Pipeline
# Usage: .\scripts\ci.ps1 [-NoRbxDom]

param(
    [switch]$NoRbxDom
)

$RbxDom = -not $NoRbxDom

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

Write-Step 4 "Compile TypeScript Extension (vscode-rojo)"
Push-Location vscode-rojo
npm run compile
$tsBuild = $LASTEXITCODE
Pop-Location
Record-Result "TS Compile (vscode-rojo)" $tsBuild

Write-Step 5 "Lua Static Analysis (Selene)"
selene plugin/src
$selene = $LASTEXITCODE
Record-Result "Selene" $selene

Write-Step 6 "Rust Linting (Clippy) - Auto-fix"
cargo clippy -j 16 --fix --allow-dirty --allow-staged 2>&1 | Out-Null
Write-Host "Verifying..." -ForegroundColor Yellow
cargo clippy -j 16 --all-targets --all-features 2>&1
$clippy = $LASTEXITCODE
Record-Result "Clippy" $clippy

Write-Step 7 "Build Everything"
cargo build -j 16 --locked --all-targets --all-features
$build = $LASTEXITCODE
Record-Result "Build" $build

Write-Step 8 "Run ALL Rust Tests"
$testOutput = cargo test -j 16 --locked --all-features -- --test-threads=16 2>&1
$rustTests = $LASTEXITCODE
$testOutput | Write-Host
Record-Result "Rust Tests" $rustTests

Write-Step 9 "Run Roblox Plugin Tests"
.\target\debug\atlas.exe build plugin/test-place.project.json -o TestPlace.rbxl
if ($LASTEXITCODE -eq 0) {
    run-in-roblox --script plugin/run-tests.server.lua --place TestPlace.rbxl
    $pluginTests = $LASTEXITCODE
} else {
    Write-Host "Skipped: build failed" -ForegroundColor Yellow
    $pluginTests = 1
}
Record-Result "Plugin Tests" $pluginTests

# ─── rbx-dom ─────────────────────────────────────────────────────────────────

if ($RbxDom) {
    Write-Step 10 "Auto-fix Lua Formatting (Stylua) for rbx_dom_lua"
    stylua rbx-dom/rbx_dom_lua/src
    $styluaRbxDom = $LASTEXITCODE
    Record-Result "Stylua (rbx_dom_lua)" $styluaRbxDom

    Write-Step 11 "Auto-fix Rust Formatting (rbx-dom)"
    Push-Location rbx-dom
    cargo fmt
    $rustfmtRbxDom = $LASTEXITCODE
    Pop-Location
    Record-Result "Rustfmt (rbx-dom)" $rustfmtRbxDom

    Write-Step 12 "Lua Static Analysis (Selene) for rbx_dom_lua"
    selene rbx-dom/rbx_dom_lua/src
    $seleneRbxDom = $LASTEXITCODE
    Record-Result "Selene (rbx_dom_lua)" $seleneRbxDom

    Write-Step 13 "Rust Linting (Clippy) for rbx-dom - Auto-fix"
    Push-Location rbx-dom
    cargo clippy -j 16 --fix --allow-dirty --allow-staged 2>&1 | Out-Null
    Write-Host "Verifying..." -ForegroundColor Yellow
    cargo clippy -j 16 --all-targets --all-features -- -D warnings 2>&1
    $clippyRbxDom = $LASTEXITCODE
    Pop-Location
    Record-Result "Clippy (rbx-dom)" $clippyRbxDom

    Write-Step 14 "Build rbx-dom"
    Push-Location rbx-dom
    cargo build -j 16 --verbose
    $buildRbxDom = $LASTEXITCODE
    cargo build -j 16 --all-features --verbose
    $buildRbxDomAll = $LASTEXITCODE
    Pop-Location
    Record-Result "Build (rbx-dom)" $buildRbxDom
    Record-Result "Build (rbx-dom all-features)" $buildRbxDomAll

    Write-Step 15 "Run ALL rbx-dom Rust Tests"
    Push-Location rbx-dom
    cargo test -j 16 --verbose -- --test-threads=16
    $testsRbxDom = $LASTEXITCODE
    cargo test -j 16 --all-features --verbose -- --test-threads=16
    $testsRbxDomAll = $LASTEXITCODE
    Pop-Location
    Record-Result "Tests (rbx-dom)" $testsRbxDom
    Record-Result "Tests (rbx-dom all-features)" $testsRbxDomAll
}

# ─── Report ──────────────────────────────────────────────────────────────────

Write-Host ""
Write-Host "=== CI COMPLETE ===" -ForegroundColor Magenta
Write-Host ""
Write-Host "--- Rojo ---" -ForegroundColor White
Write-Host ""
Write-Host "Formatting:"
Write-Host "  - Stylua: $(if ($styluaRojo -eq 0) {'PASS'} else {'FAIL'})"
Write-Host "  - Rustfmt: $(if ($rustfmt -eq 0) {'PASS'} else {'FAIL'})"
Write-Host "  - Prettier (vscode-rojo): $(if ($prettier -eq 0) {'PASS'} else {'FAIL'})"
Write-Host ""
Write-Host "TS Compile (vscode-rojo): $(if ($tsBuild -eq 0) {'PASS'} else {'FAIL'})"
Write-Host ""
Write-Host "Linting:"
Write-Host "  - Selene: $(if ($selene -eq 0) {'PASS'} else {'FAIL'})"
Write-Host "  - Clippy: $(if ($clippy -eq 0) {'PASS'} else {'FAIL'})"
Write-Host ""
Write-Host "Build: $(if ($build -eq 0) {'PASS'} else {'FAIL'})"
Write-Host ""
Write-Host "Tests:"
Write-Host "  - Rust: $(if ($rustTests -eq 0) {'PASS'} else {'FAIL'})"
Write-Host "  - Plugin: $(if ($pluginTests -eq 0) {'PASS'} else {'FAIL'})"

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
    Write-Host ""
    Write-Host "Build: $(if ($buildRbxDom -eq 0) {'PASS'} else {'FAIL'})"
    Write-Host "Build (all features): $(if ($buildRbxDomAll -eq 0) {'PASS'} else {'FAIL'})"
    Write-Host ""
    Write-Host "Tests:"
    Write-Host "  - Rust: $(if ($testsRbxDom -eq 0) {'PASS'} else {'FAIL'})"
    Write-Host "  - Rust (all features): $(if ($testsRbxDomAll -eq 0) {'PASS'} else {'FAIL'})"
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
