# /ci - Complete CI Pipeline

Run the complete CI pipeline: auto-fix all formatting, run all linters, build everything, and run ALL tests.

## Instructions

Execute ALL of the following steps in order. Fix issues automatically where possible.

### 1. Auto-fix Lua Formatting (Stylua)
```powershell
stylua plugin/src
```

### 2. Auto-fix Rust Formatting
```powershell
cargo fmt
```

### 3. Lua Static Analysis (Selene) - MUST PASS WITH ZERO ISSUES
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

### 4. Rust Linting (Clippy) - Auto-fix where possible
```powershell
cargo clippy --fix --allow-dirty --allow-staged
```
Then verify with:
```powershell
cargo clippy --all-targets --all-features
```
If there are remaining warnings/errors, fix them before proceeding.

### 5. Build Everything
```powershell
cargo build --locked --all-targets --all-features
```

### 6. Run ALL Rust Tests
```powershell
cargo test --locked --all-targets --all-features
```

### 7. Run Roblox Plugin Tests (requires Roblox Studio)
```powershell
.\scripts\unit-test-plugin.ps1
```

## Reporting

After ALL steps complete, provide a summary:

```
=== CI COMPLETE ===

Formatting:
  - Stylua: [fixed X files / no changes needed]
  - Rustfmt: [fixed X files / no changes needed]

Linting:
  - Selene: PASS (0 errors, 0 warnings) / FAIL (X errors, Y warnings)
  - Clippy: PASS / FAIL (X warnings)

Build: PASS / FAIL

Tests:
  - Rust: X passed, Y failed
  - Plugin: X passed, Y failed, Z skipped

Overall: PASS / FAIL
```

**IMPORTANT:** 
- Selene PASS requires exit code 0 (0 errors AND 0 warnings). Any warnings = FAIL.
- If any step fails, continue running the remaining steps to get a complete picture, then report all failures at the end.
- Do not report "Overall: PASS" unless ALL linters pass with zero issues.
