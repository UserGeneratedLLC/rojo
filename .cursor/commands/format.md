# /format - Auto-fix Formatting & Static Analysis

Run formatting and static analysis (linting) across the entire project.

## Instructions

### 1. Run the Format Script

**Windows (PowerShell):**
```powershell
.\scripts\format.ps1
```

**macOS/Linux (Bash):**
```bash
bash scripts/format.sh
```

**Monitor the script output as it runs.**

### 2. Fix ALL Failures

If any step fails, you MUST fix the issues and re-run until everything passes. This applies to **every** failure -- Clippy, Selene, Prettier, all of them.

**Clippy is not optional.** Fix every Clippy warning, even if the warning is in code you did not write or that existed before your changes. There is no "pre-existing" or "before my time" exception. If Clippy reports it, fix it. The CI gate runs `cargo clippy` and will reject the PR regardless of who introduced the warning.

The same rule applies to Selene warnings on Lua code. If the linter flags it, fix it.

Keep re-running the format script until the overall result is **PASS** with zero failures.

### 3. Report

Once formatting passes clean, provide a brief summary and confirm the final result.
