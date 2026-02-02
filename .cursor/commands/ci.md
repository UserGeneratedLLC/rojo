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

### 3. Lua Static Analysis (Selene)
```powershell
selene plugin/src
```
If there are errors, report them and offer to help fix.

### 4. Rust Linting (Clippy) - Auto-fix where possible
```powershell
cargo clippy --fix --allow-dirty --allow-staged
```
Then verify with:
```powershell
cargo clippy --all-targets --all-features
```
If there are remaining warnings/errors, report them and offer to help fix.

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
  - Selene: PASS / FAIL (X issues)
  - Clippy: PASS / FAIL (X warnings)

Build: PASS / FAIL

Tests:
  - Rust: X passed, Y failed
  - Plugin: X passed, Y failed, Z skipped

Overall: PASS / FAIL
```

If any step fails, continue running the remaining steps to get a complete picture, then report all failures at the end.
