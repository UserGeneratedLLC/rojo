# /ci - Complete CI Pipeline

Run the complete CI pipeline: auto-fix all formatting, run all linters, build everything, and run ALL tests.

## Instructions

- If the user's message contains **"rbx-dom"**, run **both** the Rojo and rbx-dom sections below.
- Otherwise, **skip the entire rbx-dom section** (steps 9–14) to save time.

Execute ALL applicable steps in order. Fix issues automatically where possible.

---

## Rojo

### 1. Auto-fix Lua Formatting (Stylua)
```powershell
stylua plugin/src
```

### 2. Auto-fix Rust Formatting
```powershell
cargo fmt
```

### 3. Auto-fix TypeScript Formatting (Prettier) for vscode-rojo
```powershell
cd vscode-rojo
npx prettier --write "src/**/*.ts"
cd ..
```

### 4. Lua Static Analysis (Selene) - MUST PASS WITH ZERO ISSUES
```powershell
selene plugin/src
```

**CRITICAL: Selene must exit with code 0 (zero errors AND zero warnings).** Warnings count as failures.

If Selene reports ANY errors or warnings:
1. **FIX THEM IMMEDIATELY** before proceeding - do not just report them
2. Common fixes:
   - `unused_variable`: Replace with `_` (e.g., `for _ = 1, 10 do`) or remove the variable entirely
   - `unused_variable` for imports: Remove the unused require/import line
3. Re-run Selene after fixes to confirm exit code 0
4. Only proceed once Selene shows: `Results: 0 errors, 0 warnings, 0 parse errors`

### 5. Rust Linting (Clippy) - Auto-fix where possible
```powershell
cargo clippy --fix --allow-dirty --allow-staged
```
Then verify with:
```powershell
cargo clippy --all-targets --all-features
```
If there are remaining warnings/errors, fix them before proceeding.

### 6. Build Everything
```powershell
cargo build --locked --all-targets --all-features
```

### 7. Run ALL Rust Tests
```powershell
cargo test --locked --all-targets --all-features
```

### 8. Run Roblox Plugin Tests

**Do NOT use the scripts (`unit-test-plugin.ps1` / `unit-test-plugin.sh`).** They reference a system-installed binary which may not exist. Always use the freshly built binary from step 6.

```powershell
.\target\debug\atlas.exe build plugin/test-place.project.json -o TestPlace.rbxl
run-in-roblox --script plugin/run-tests.server.lua --place TestPlace.rbxl
```

---

## rbx-dom (submodule)

All rbx-dom steps run inside the `rbx-dom/` directory.

### 9. Auto-fix Lua Formatting (Stylua) for rbx_dom_lua
```powershell
cd rbx-dom
stylua rbx_dom_lua/src
cd ..
```

### 10. Auto-fix Rust Formatting
```powershell
cd rbx-dom
cargo fmt
cd ..
```

### 11. Lua Static Analysis (Selene) for rbx_dom_lua - MUST PASS WITH ZERO ISSUES
```powershell
cd rbx-dom
selene rbx_dom_lua/src
cd ..
```

Same rules as step 4: exit code 0 required, fix all errors and warnings before proceeding.

### 12. Rust Linting (Clippy) - Auto-fix where possible
```powershell
cd rbx-dom
cargo clippy --fix --allow-dirty --allow-staged
```
Then verify with:
```powershell
cd rbx-dom
cargo clippy --all-targets --all-features -- -D warnings
cd ..
```
If there are remaining warnings/errors, fix them before proceeding.

### 13. Build rbx-dom
```powershell
cd rbx-dom
cargo build --verbose
cargo build --all-features --verbose
cd ..
```

### 14. Run ALL rbx-dom Rust Tests
```powershell
cd rbx-dom
cargo test --verbose
cargo test --all-features --verbose
cd ..
```

---

## Reporting

After ALL steps complete, provide a summary:

```
=== CI COMPLETE ===

--- Rojo ---

Formatting:
  - Stylua: [fixed X files / no changes needed]
  - Rustfmt: [fixed X files / no changes needed]
  - Prettier (vscode-rojo): [fixed X files / no changes needed]

Linting:
  - Selene: PASS (0 errors, 0 warnings) / FAIL (X errors, Y warnings)
  - Clippy: PASS / FAIL (X warnings)

Build: PASS / FAIL

Tests:
  - Rust: X passed, Y failed
  - Plugin: X passed, Y failed

--- rbx-dom (only if included) ---

Formatting:
  - Stylua (rbx_dom_lua): [fixed X files / no changes needed]
  - Rustfmt: [fixed X files / no changes needed]

Linting:
  - Selene (rbx_dom_lua): PASS (0 errors, 0 warnings) / FAIL (X errors, Y warnings)
  - Clippy: PASS / FAIL (X warnings)

Build: PASS / FAIL
Build (all features): PASS / FAIL

Tests:
  - Rust: X passed, Y failed
  - Rust (all features): X passed, Y failed

--- Overall: PASS / FAIL ---
```

**IMPORTANT:** 
- Selene PASS requires exit code 0 (0 errors AND 0 warnings). Any warnings = FAIL.
- If any step fails, continue running the remaining steps to get a complete picture, then report all failures at the end.
- Do not report "Overall: PASS" unless ALL linters pass with zero issues across all sections that were run.
- If rbx-dom was skipped, omit its section from the report entirely.
