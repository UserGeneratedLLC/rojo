#!/usr/bin/env bash
# Complete CI Pipeline
# Usage: bash scripts/ci.sh [--no-rbx-dom]

set -o pipefail

RBX_DOM=true
for arg in "$@"; do
    case "$arg" in
        --no-rbx-dom) RBX_DOM=false ;;
    esac
done

BUILD_THREADS=$(nproc 2>/dev/null || sysctl -n hw.logicalcpu 2>/dev/null || echo 4)
# Cap test threads: two_way_sync tests spawn child processes (atlas serve)
# with file watchers, HTTP servers, and temp dirs. Running too many in
# parallel (e.g. 32) exhausts OS resources and causes spurious failures.
_cpu_count=$(nproc 2>/dev/null || sysctl -n hw.logicalcpu 2>/dev/null || echo 4)
TEST_THREADS=$(( _cpu_count < 8 ? _cpu_count : 8 ))

failures=()

step() {
    echo ""
    echo "=== Step $1: $2 ==="
}

record() {
    if [ "$2" -ne 0 ]; then
        failures+=("$1")
        echo "FAIL"
    else
        echo "PASS"
    fi
}

# ─── Rojo ────────────────────────────────────────────────────────────────────

step 1 "Auto-fix Lua Formatting (Stylua)"
stylua plugin/src
stylua_rojo=$?
record "Stylua (Rojo)" $stylua_rojo

step 2 "Auto-fix Rust Formatting"
cargo fmt
rustfmt_exit=$?
record "Rustfmt" $rustfmt_exit

step 3 "Auto-fix TypeScript Formatting (Prettier)"
(cd vscode-rojo && npx prettier --write "src/**/*.ts")
prettier_exit=$?
record "Prettier" $prettier_exit

step 4 "Compile TypeScript Extension (vscode-rojo)"
(cd vscode-rojo && npm run compile)
ts_build=$?
record "TS Compile (vscode-rojo)" $ts_build

step 5 "Lua Static Analysis (Selene)"
selene plugin/src
selene_exit=$?
record "Selene" $selene_exit

step 6 "Rust Linting (Clippy) - Auto-fix"
cargo clippy -j $BUILD_THREADS --fix --allow-dirty --allow-staged >/dev/null 2>&1
echo "Verifying..."
cargo clippy -j $BUILD_THREADS --all-targets --all-features 2>&1
clippy_exit=$?
record "Clippy" $clippy_exit

step 7 "Build Everything"
cargo build --locked --all-targets --all-features
build_exit=$?
record "Build" $build_exit

step 8 "Run ALL Rust Tests"
cargo test --locked --all-features -- --test-threads=$TEST_THREADS 2>&1
rust_tests=$?
record "Rust Tests" $rust_tests

step 9 "Run Roblox Plugin Tests"
./target/debug/atlas build plugin/test-place.project.json -o TestPlace.rbxl
if [ $? -eq 0 ]; then
    run-in-roblox --script plugin/run-tests.server.lua --place TestPlace.rbxl
    plugin_tests=$?
else
    echo "Skipped: build failed"
    plugin_tests=1
fi
record "Plugin Tests" $plugin_tests

# ─── rbx-dom ─────────────────────────────────────────────────────────────────

if [ "$RBX_DOM" = true ]; then
    step 10 "Auto-fix Lua Formatting (Stylua) for rbx_dom_lua"
    (cd rbx-dom && stylua rbx_dom_lua/src)
    stylua_rbxdom=$?
    record "Stylua (rbx_dom_lua)" $stylua_rbxdom

    step 11 "Auto-fix Rust Formatting (rbx-dom)"
    (cd rbx-dom && cargo fmt)
    rustfmt_rbxdom=$?
    record "Rustfmt (rbx-dom)" $rustfmt_rbxdom

    step 12 "Lua Static Analysis (Selene) for rbx_dom_lua"
    (cd rbx-dom && selene rbx_dom_lua/src)
    selene_rbxdom=$?
    record "Selene (rbx_dom_lua)" $selene_rbxdom

    step 13 "Rust Linting (Clippy) for rbx-dom - Auto-fix"
    (cd rbx-dom && cargo clippy -j $BUILD_THREADS --fix --allow-dirty --allow-staged >/dev/null 2>&1)
    echo "Verifying..."
    (cd rbx-dom && cargo clippy -j $BUILD_THREADS --all-targets --all-features -- -D warnings 2>&1)
    clippy_rbxdom=$?
    record "Clippy (rbx-dom)" $clippy_rbxdom

    step 14 "Build rbx-dom"
    (cd rbx-dom && cargo build --verbose)
    build_rbxdom=$?
    (cd rbx-dom && cargo build --all-features --verbose)
    build_rbxdom_all=$?
    record "Build (rbx-dom)" $build_rbxdom
    record "Build (rbx-dom all-features)" $build_rbxdom_all

    step 15 "Run ALL rbx-dom Rust Tests"
    (cd rbx-dom && cargo test --verbose -- --test-threads=$TEST_THREADS)
    tests_rbxdom=$?
    (cd rbx-dom && cargo test --all-features --verbose -- --test-threads=$TEST_THREADS)
    tests_rbxdom_all=$?
    record "Tests (rbx-dom)" $tests_rbxdom
    record "Tests (rbx-dom all-features)" $tests_rbxdom_all
fi

# ─── Report ──────────────────────────────────────────────────────────────────

pass_or_fail() { [ "$1" -eq 0 ] && echo "PASS" || echo "FAIL"; }

echo ""
echo "=== CI COMPLETE ==="
echo ""
echo "--- Rojo ---"
echo ""
echo "Formatting:"
echo "  - Stylua: $(pass_or_fail $stylua_rojo)"
echo "  - Rustfmt: $(pass_or_fail $rustfmt_exit)"
echo "  - Prettier (vscode-rojo): $(pass_or_fail $prettier_exit)"
echo ""
echo "TS Compile (vscode-rojo): $(pass_or_fail $ts_build)"
echo ""
echo "Linting:"
echo "  - Selene: $(pass_or_fail $selene_exit)"
echo "  - Clippy: $(pass_or_fail $clippy_exit)"
echo ""
echo "Build: $(pass_or_fail $build_exit)"
echo ""
echo "Tests:"
echo "  - Rust: $(pass_or_fail $rust_tests)"
echo "  - Plugin: $(pass_or_fail $plugin_tests)"

if [ "$RBX_DOM" = true ]; then
    echo ""
    echo "--- rbx-dom ---"
    echo ""
    echo "Formatting:"
    echo "  - Stylua (rbx_dom_lua): $(pass_or_fail $stylua_rbxdom)"
    echo "  - Rustfmt: $(pass_or_fail $rustfmt_rbxdom)"
    echo ""
    echo "Linting:"
    echo "  - Selene (rbx_dom_lua): $(pass_or_fail $selene_rbxdom)"
    echo "  - Clippy: $(pass_or_fail $clippy_rbxdom)"
    echo ""
    echo "Build: $(pass_or_fail $build_rbxdom)"
    echo "Build (all features): $(pass_or_fail $build_rbxdom_all)"
    echo ""
    echo "Tests:"
    echo "  - Rust: $(pass_or_fail $tests_rbxdom)"
    echo "  - Rust (all features): $(pass_or_fail $tests_rbxdom_all)"
fi

echo ""
if [ ${#failures[@]} -eq 0 ]; then
    echo "--- Overall: PASS ---"
    exit 0
else
    echo "--- Overall: FAIL ---"
    echo "Failed steps: $(IFS=', '; echo "${failures[*]}")"
    exit 1
fi
