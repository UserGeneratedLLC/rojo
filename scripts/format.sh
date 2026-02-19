#!/usr/bin/env bash
# Auto-fix Formatting & Static Analysis
# Usage: bash scripts/format.sh [--no-rbx-dom]

set -o pipefail

RBX_DOM=true
for arg in "$@"; do
    case "$arg" in
        --no-rbx-dom) RBX_DOM=false ;;
    esac
done

THREADS=$(nproc 2>/dev/null || sysctl -n hw.logicalcpu 2>/dev/null || echo 4)

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

step 4 "Lua Static Analysis (Selene)"
selene plugin/src
selene_exit=$?
record "Selene" $selene_exit

step 5 "Rust Linting (Clippy) - Auto-fix"
cargo clippy -j $THREADS --fix --allow-dirty --allow-staged >/dev/null 2>&1
echo "Verifying..."
cargo clippy -j $THREADS --all-targets --all-features 2>&1
clippy_exit=$?
record "Clippy" $clippy_exit

# ─── rbx-dom ─────────────────────────────────────────────────────────────────

if [ "$RBX_DOM" = true ]; then
    step 6 "Auto-fix Lua Formatting (Stylua) for rbx_dom_lua"
    (cd rbx-dom && stylua rbx_dom_lua/src)
    stylua_rbxdom=$?
    record "Stylua (rbx_dom_lua)" $stylua_rbxdom

    step 7 "Auto-fix Rust Formatting (rbx-dom)"
    (cd rbx-dom && cargo fmt)
    rustfmt_rbxdom=$?
    record "Rustfmt (rbx-dom)" $rustfmt_rbxdom

    step 8 "Lua Static Analysis (Selene) for rbx_dom_lua"
    (cd rbx-dom && selene rbx_dom_lua/src)
    selene_rbxdom=$?
    record "Selene (rbx_dom_lua)" $selene_rbxdom

    step 9 "Rust Linting (Clippy) for rbx-dom - Auto-fix"
    (cd rbx-dom && cargo clippy -j $THREADS --fix --allow-dirty --allow-staged >/dev/null 2>&1)
    echo "Verifying..."
    (cd rbx-dom && cargo clippy -j $THREADS --all-targets --all-features -- -D warnings 2>&1)
    clippy_rbxdom=$?
    record "Clippy (rbx-dom)" $clippy_rbxdom
fi

# ─── Report ──────────────────────────────────────────────────────────────────

pass_or_fail() { [ "$1" -eq 0 ] && echo "PASS" || echo "FAIL"; }

echo ""
echo "=== FORMAT COMPLETE ==="
echo ""
echo "--- Rojo ---"
echo ""
echo "Formatting:"
echo "  - Stylua: $(pass_or_fail $stylua_rojo)"
echo "  - Rustfmt: $(pass_or_fail $rustfmt_exit)"
echo "  - Prettier (vscode-rojo): $(pass_or_fail $prettier_exit)"
echo ""
echo "Linting:"
echo "  - Selene: $(pass_or_fail $selene_exit)"
echo "  - Clippy: $(pass_or_fail $clippy_exit)"

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
