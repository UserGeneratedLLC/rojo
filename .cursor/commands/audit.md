# /audit - Production-Grade Sync Feature Audit

**Mode:** This command runs in **Plan mode** (read-only). It does NOT directly apply code changes. The deliverable is a `.cursor/plans/*.plan.md` file containing every approved fix, ready to be executed in a subsequent Agent-mode session.

**Workflow:** Analyze -> Report -> Quiz user on each fix -> Write plan file.

## Prerequisites

Before beginning the audit, read `.cursor/rules/atlas.mdc` for the project's quality standards, architecture overview, and code conventions. The two standards below are extracted from that file and govern every audit finding:

### The Invariant (from `atlas.mdc` §Quality Standard)

> **Round-trip identity:** Syncback (or two-way sync) writes a directory tree. Building an rbxl from that directory tree and forward-syncing it back must produce a bit-identical instance tree -- same names, same classes, same properties, same hierarchy, same ref targets. Any deviation is a bug.

A "works most of the time" finding is NOT acceptable. If there is ANY code path where an instance name, property, or hierarchy relationship can be lost, mangled, or silently altered through a syncback/two-way-sync -> rebuild cycle, flag it as **critical**.

### CLI Syncback Parity (from `atlas.mdc` §Two-Way Sync Strategy)

> Plugin-based sync must produce **exactly the same filesystem output** that `atlas syncback` would give for the same input. Byte-for-byte identical files, identical directory structures, identical naming.

Any divergence between the two-way sync path and the CLI syncback path is a bug until proven otherwise. The CLI syncback is the ground truth.

### Code Quality (from `atlas.mdc` §Code Quality Standard)

> Watch for duplicated logic -- the same slugify/dedup/meta-update pattern copy-pasted across `change_processor.rs`, `api.rs`, `dir.rs`, `project.rs`, etc. The goal is clean, DRY code that is easy to reason about and hard to get wrong.

- **Small refactor** (helper function, 2-3 call sites): flag it and include it in the fix plan.
- **Major rewrite** (10+ call sites, pipeline redesign): flag it in a "Deferred Refactors" section of the plan.

---

## Instructions

### 0. Read Project Standards

Read `.cursor/rules/atlas.mdc` in full. This file defines architecture, conventions, and quality bars that inform every audit step. Do not proceed without reading it.

### 1. Identify the Feature Scope

**Default behavior:** Audit all changes in the current workspace against `origin/master`. This captures everything: committed branch changes, unpushed commits, staged changes, and unstaged changes. Run:

```bash
git log origin/master..HEAD --oneline
git diff origin/master --stat
git diff origin/master
```

Read the commit log (for intent) and the full diff (for code). If the user provides plan files (via `@` reference), read those for additional context. Do NOT search for or read plan files on your own -- plans may be stale and describe outdated designs or already-fixed issues.

**If the user specifies a feature or branch:** Scope the audit to that feature's changes only.

From the diff, identify:

- Which files were modified (plugin Lua, server Rust, test fixtures)
- What the feature does (new property type support, naming change, sync flow change, etc.)
- What the correct round-trip behavior should be
- Which sync paths are affected (forward sync, reverse sync, two-way sync, all three)

### 2. Trace Round-Trip Identity (MOST CRITICAL)

For each operation the feature introduces, trace the **complete lifecycle** through every path:

#### 2a. Two-Way Sync -> Forward Sync Round-Trip

Pick a concrete example. Trace:

1. **Plugin encodes** -- How does the plugin read the property/change from Studio? What format does it send to the server? What encoding is used (msgpack, JSON, etc.)?
2. **Server receives** -- How does `handle_api_write` deserialize the data? Any type conversions?
3. **Server writes to disk** -- Which function writes to the filesystem? What file format (meta.json5, model.json5, .luau, etc.)? What path computation is used?
4. **Server applies to tree** -- How is the PatchSet built? Does `apply_patch_set` handle this data type correctly? Any special-case skips (like `Ref::none()` skip in `apply_update_child`)?
5. **VFS echo prevention** -- Is `suppress_path()` called before `fs::write()` for updates? Is suppression correctly NOT used for additions?
6. **Forward sync reads** -- When the file is read back by the snapshot system, does it produce the exact same property value? Any lossy conversion?
7. **Plugin receives** -- Does `decodeValue.lua` correctly decode the value back to a Studio-compatible type?

**Verify:** After the full round-trip, is the value identical? If you killed the server and ran `atlas build`, would the property have the correct value?

#### 2b. Nil/Removal Round-Trip

Trace what happens when the property is set to nil or removed:

- Plugin encoding of nil values
- Server handling (property removal vs. nil sentinel)
- Filesystem representation (attribute removed? file deleted? empty value?)
- Forward sync behavior (absent property = default value?)

#### 2c. Change/Update Round-Trip

Trace what happens when the value changes from A to B:

- Does the old value get properly overwritten?
- Any merge conflicts with existing file content?
- Ordering issues (removal before addition? addition before removal?)

#### 2d. Rename/Move Interactions

Trace what happens when an instance involved in the feature is renamed or moved:

- Do stored paths/references become stale?
- Does the ChangeProcessor update dependent data?
- Is there a window of inconsistency during the rename?

### 3. Audit Plugin-Side Encoding

Every code path in the plugin that encodes the feature's data must be checked:

- **Property filter** (`propertyFilter.lua`): Is the data type correctly included/excluded?
- **Patch update encoding** (`encodePatchUpdate.lua`): Does the encoding handle all cases (nil, valid, edge)?
- **Instance encoding** (`encodeInstance.lua`): For newly added instances, is the data type handled correctly? Any fragile error paths (pcall catching errors instead of explicit checks)?
- **Patch creation** (`createPatchSet.lua`): Are all required parameters passed through?
- **All callers**: Search for ALL callers of modified functions. Any call site that was NOT updated when a function signature changed? (Use grep to find every caller.)

**CRITICAL -- Lua silent argument mismatch:** In Lua/Luau, calling `foo(a, b, c)` when the function signature is `function(a, b, c, d)` does NOT produce any error -- `d` is simply `nil`. This means adding a new parameter to a function signature produces **zero errors** at callers that weren't updated. The missing argument silently becomes `nil`, and the function may take a fallback path that drops data without warning. **Grep for every caller of any modified function and verify the argument count matches.** This is the #1 source of silent data loss bugs in the plugin.

**Multiple plugin entry points:** Property changes flow through the plugin via TWO separate paths:
1. **ChangeBatcher flow** (live two-way sync): `Instance.Changed` -> `InstanceMap.__connectSignals` -> `ChangeBatcher:add()` -> `createPatchSet()` -> `encodePatchUpdate()` -> `ApiContext:write()`
2. **Confirmation dialog flow** (initial sync pull): `ServeSession.__confirmAndApplyInitialPatch` -> `encodePatchUpdate()` -> `ApiContext:write()`

Both paths must be audited. A function modified in path 1 may not have been updated in path 2.

**Change detection signals:** `Instance.Changed` fires for all scriptable properties on regular instances, but ValueBase instances (ObjectValue, IntValue, etc.) override Changed to only fire for `Value`. The InstanceMap connects `GetPropertyChangedSignal` for `Name`, `Value`, and `Parent` on ValueBase instances. Verify the feature's data type is detected by the correct signal.

### 4. Audit Server-Side Processing

Every code path in `api.rs` and `change_processor.rs`:

- **Data extraction**: Is the feature's data extracted correctly from the write request?
- **Path/value computation**: Are computed values correct? **Verify inverse consistency:** if you compute a value from data (e.g., `full_path_of(ref)` -> path), then resolve it back (e.g., `get_instance_by_path(path)` -> ref), do you get the same original data? Any case where the round-trip fails (e.g., instance names containing the path separator `/`)?
- **Meta/model file writing**: Correct file format? Correct section (properties vs. attributes)? Merge with existing content? Creation from scratch?
- **PatchSet construction**: Data included correctly? Any type conversions that lose information?
- **Filter functions**: Any filter that should now pass the data type but was not updated?
- **Dual write path consistency**: `handle_api_write` does TWO things: (1) writes files to disk via `syncback_updated_properties`, and (2) sends a PatchSet to the ChangeProcessor to update the in-memory tree. These MUST stay consistent. If the file write succeeds but the PatchSet doesn't include the same change (or vice versa), the tree and filesystem diverge silently.
- **Processing order**: `handle_api_write` processes removals, then additions, then updates. Property writes (`syncback_updated_properties`) happen BEFORE the PatchSet is sent to the ChangeProcessor. This ordering matters: if a property change references an instance that was renamed in the same batch, the path is computed from the PRE-rename tree state.
- **Sentinel values in patch system**: Check `apply_update_child` and `apply_add_child` for any data type that is silently skipped or treated as a no-op. Example: `Variant::Ref(Ref::none())` is skipped with `continue` at line 281-282 of `patch_apply.rs`. If the feature's nil/empty representation matches a sentinel, the patch system may silently ignore it instead of applying it.

### 5. Audit Forward-Sync Resolution

The filesystem -> server -> plugin direction:

- **Snapshot middleware**: Does the middleware that reads the file format correctly parse the feature's data?
- **Patch computation**: Does `compute_patch_set` handle the data type? Any special-case resolution (like `compute_ref_properties` for Rojo_Ref_* attributes)?
- **Patch application**: Does `apply_patch_set` correctly apply the data? Any sentinel values that are skipped?
- **Attribute cleanup**: Are internal attributes (Rojo_Ref_*, Rojo_Target_*, etc.) cleaned from the Attributes property before being sent to Studio?
- **Plugin decoding**: Does `decodeValue.lua` handle the type?

### 6. Audit Interactions with Existing Systems

- **Legacy systems**: Does the new feature coexist with legacy systems? Priority conflicts? Duplicate data?
- **Other callers**: Are there callers of modified functions from OUTSIDE the feature's scope? Were they updated?
- **Shared filters/helpers**: Were shared data structures (like `UNENCODABLE_DATA_TYPES`) modified? Does this affect other code paths that import them?

### 7. Audit Meta File Lifecycle

- **Creation**: New file with just the feature's data works?
- **Merge**: Adding to existing file with other content preserves everything?
- **Update**: Changing the value overwrites correctly?
- **Deletion**: Removing the value cleans up correctly? Empty sections removed?
- **Orphaned data**: Instance deleted -> file deleted -> no orphaned data?

### 8. Audit VFS Echo Prevention

The suppression system uses per-path counters. Verify:

- **Updates to existing files**: MUST call `suppress_path()` before `fs::write()`
- **New instance additions**: Must NOT suppress (VFS watcher needs to pick up new files)
- **All write branches**: Every branch that writes a file must be checked

### 9. Audit CLI Syncback Parity

Compare two-way sync output with CLI syncback output:

- Same data format on disk?
- Same file placement (attributes section vs. properties)?
- Same nil handling?
- Shared helpers used consistently?
- Any divergence is a bug per `atlas.mdc`

### 10. Audit Wire Format

- **Msgpack serialization**: Does the data type survive the plugin -> server msgpack round-trip?
- **Type parsing**: Does the server's deserializer correctly parse the wire format?
- **Edge values**: Does nil/zero/empty round-trip correctly?

### 11. Check Sync-Specific Patterns

These are patterns specific to Atlas's sync system that every audit must verify:

#### 11a. The 4 File Format Branches

`syncback_updated_properties` has 4 branches for writing property changes. EVERY audit must verify the feature works in ALL 4:

1. **Directory format**: `init.meta.json5` inside the instance's directory
2. **Standalone script**: adjacent `ScriptName.meta.json5` (strips `.server`/`.client`/`.plugin`/`.local`/`.legacy` suffix from stem)
3. **Model file**: inline in `.model.json5` (writes directly into the file, NOT adjacent meta)
4. **Other file types** (`.txt`, `.csv`, `.toml`): adjacent `FileName.meta.json5`

Common bug: treating model files like scripts (writing adjacent meta instead of inline).

#### 11b. Compound File Extensions

`.server.luau`, `.client.luau`, `.model.json5`, `.meta.json5`, `.project.json5` are multi-part extensions that need special handling:

- **Rename**: Must preserve the compound extension. The suffix list in `change_processor.rs` must include all compound parts. Missing `.model` from the suffix list caused a critical bug where `.model.json5` became `.json5` on rename.
- **Meta pairing**: `Foo.server.luau` pairs with `Foo.meta.json5` (NOT `Foo.server.meta.json5`). Script suffixes are stripped from the stem before computing the meta path.
- **Model files store `name` inline**: `.model.json5` has `"name"` in the root JSON object. `.meta.json5` has `"name"` as a top-level field. These are different locations.

#### 11c. InstigatingSource

Every instance in the RojoTree tracks where it came from:
- `InstigatingSource::Path(PathBuf)` -- created from a file, can be written back
- `InstigatingSource::ProjectNode { path, name, node, parent_class }` -- defined in project file, CANNOT be modified via two-way sync

**Verify:** Does the feature correctly guard against writing to ProjectNode instances? Is `InstigatingSource` set correctly for new instances created by the feature?

#### 11d. Pre-seeding Dedup Sets

When adding instances, `sibling_slugs` (or `taken_names`) must be seeded from existing tree children via their `instigating_source` paths (not instance names). This prevents slug collisions with existing files on disk.

**Verify:** Does the feature seed the dedup set correctly? Does it use `strip_middleware_extension` to derive the dedup key from filesystem paths?

#### 11e. Init File Detection

`init.luau`, `init.server.luau`, etc. inside directories make the directory represent the script instance. When auditing:

- **Renames**: Renaming a directory instance must rename the directory AND its init file path. The meta file is `init.meta.json5` inside the directory.
- **Deletions**: Deleting a directory instance deletes the entire directory. Also clean up adjacent `DirName.meta.json5` at the grandparent level.
- **Property writes**: Properties for directory instances go into `init.meta.json5` inside the directory, NOT an adjacent file.

#### 11f. Atomicity and File Watcher Races

When an operation involves multiple filesystem changes (delete old file + create new file, or rename + meta update), there is a window where the VFS watcher could fire between operations and produce incorrect intermediate state.

**Verify:** Are multi-step operations ordered to minimize race windows? Does `suppress_path` cover all intermediate states? Could the ChangeProcessor snapshot an inconsistent state?

#### 11g. Determinism

Given the same instance tree, does the feature ALWAYS produce the same filesystem output?

- Is child ordering stable (sorted, not random)?
- Are generated identifiers (dedup suffixes, IDs) deterministic?
- Non-determinism = different builds from the same source = git churn = invariant violation.

#### 11h. Syncback Idempotency

Running the operation twice should produce zero changes on the second run. If re-running writes the same files again (even with identical content), something is unstable.

#### 11i. Edge Cases

- **ProjectNode instances**: Can the feature interact with instances defined in project files? Is there a guard?
- **Nested projects**: Does data span project boundaries correctly?
- **Concurrent changes**: Multiple changes in one batch handled correctly?
- **Non-existent targets**: Graceful handling when referenced data doesn't exist?
- **Name conflicts**: Instance names with special characters (`/`, `:`, `?`, `*`, `<`, `>`, `|`, `"`, `\`) that affect the feature?
- **Script type transitions**: ClassName changes (ModuleScript -> Script) cause file extension changes. Does the feature handle this?
- **One-shot mode**: Does `Settings:get("oneShotSync")` correctly block the feature's outgoing writes?

### 12. Run Static Analysis

Run `cargo clippy` on the modified code to catch Rust issues the audit might miss:

```bash
cargo clippy --all-targets 2>&1
```

Focus on warnings in files modified by the feature. Clippy catches: unused variables, dead code, redundant clones, suspicious patterns, missing error handling, and type conversion issues that could silently lose data.

Also run `selene plugin/src` for Lua linting if plugin files were modified.

### 13. Produce the Report

Structured report with:

- **Critical issues** -- data loss, incorrect values, silent corruption on round-trip
- **Correctness concerns** -- edge cases that might not manifest immediately
- **Missing test coverage** -- specific test cases needed, prioritized by risk
- **Known limitations** -- for each limitation: what it is, the concrete scenario, severity, and what fixing it would require
- **Challenged limitations** -- for each limitation: what it is, user's decision (accept/fix), and if accepted, confirmation that warnings are logged and no silent corruption occurs beyond the accepted scope
- **Code quality items** -- DRY violations, dead code, fragile error paths
- **Deferred refactors** -- major structural improvements too large to do inline
- For each issue: **file path, line numbers, description, and suggested fix**

### 14. Challenge Known Limitations

Do NOT blindly accept "known limitations" from implementation plans. For every limitation that violates round-trip identity:

1. **Present each limitation to the user** using the AskQuestion tool. State:
   - What the limitation is
   - The concrete scenario where data is lost or corrupted
   - The severity (how likely is this to hit users?)
   - What fixing it would require
2. **Ask the user to explicitly confirm** it's acceptable for this release, or whether it should be fixed now.
3. **Never silently skip a round-trip violation** just because a plan document says "known limitation." Plans are written before implementation -- the user may have changed their mind, or the severity may be worse than anticipated.

### 15. Quiz User on Each Planned Fix

**No code changes may be applied until the user explicitly approves each one.** After producing the report and challenging limitations, present EVERY planned fix to the user for approval before writing any code.

For each fix identified in the report (bug fixes, refactors, test additions, etc.), use the **AskQuestion tool** to present:

1. **What the fix changes** -- which file(s), what code, what behavior changes
2. **Why it's needed** -- which audit finding it addresses (reference the report section)
3. **Risk assessment** -- could this fix break anything else? What's the blast radius?
4. **Options:**
   - **Apply** -- proceed with this fix
   - **Skip** -- do not apply this fix (document the reason)
   - **Modify** -- the user wants a different approach (wait for their input before proceeding)

**Rules:**
- Present fixes **one at a time** so the user can evaluate each independently
- Group closely related fixes (e.g., a bug fix + its regression test) into a single question, but never bundle unrelated fixes
- If the user selects "Modify," wait for their revised instructions before continuing to the next fix
- After all fixes are quizzed, summarize which were approved, skipped, and modified
- Only proceed to step 16 with the approved and modified fixes

### 16. Create the Fix Plan

After all fixes have been quizzed and the user has made their decisions, write a plan file to `.cursor/plans/`. This plan is the deliverable of the audit -- do NOT directly apply any code changes.

**Filename:** `.cursor/plans/<descriptive_slug>.plan.md` (e.g., `fix_ref_round_trip_issues.plan.md`)

**Plan structure:**

```markdown
# <Title describing the audit scope>

> This plan was generated by `/audit` from a Plan-mode session.
> Refer to `.cursor/rules/atlas.mdc` for full project standards.

## Standards

These standards govern every fix in this plan. An implementer MUST read `.cursor/rules/atlas.mdc` before starting.

### Round-Trip Identity (from `atlas.mdc` §Quality Standard)

Syncback (or two-way sync) writes a directory tree. Building an rbxl from that directory tree and forward-syncing it back must produce a **bit-identical instance tree** -- same names, same classes, same properties, same hierarchy, same ref targets. Any deviation is a bug.

### CLI Syncback Parity (from `atlas.mdc` §Two-Way Sync Strategy)

Plugin-based sync must produce **exactly the same filesystem output** that `atlas syncback` would give for the same input. Byte-for-byte identical files, identical directory structures, identical naming. Any divergence is a bug until proven otherwise.

### Code Quality (from `atlas.mdc` §Code Quality Standard)

DRY code that is easy to reason about and hard to get wrong. The same slugify/dedup/meta-update pattern should not be copy-pasted across files. Small refactors (2-3 call sites) are included as fixes. Major rewrites (10+ call sites) are deferred.

## Context

<Brief summary of what was audited, which branch/feature, and the audit findings.>

## Fixes

Each fix below was approved by the user during the audit quiz (step 15). Implement them in order. Each fix includes its test requirements -- a fix is not complete until its tests pass.

### Fix N: <Short title>

- **Status:** Approved | Modified
- **Finding:** <Which audit section identified this (e.g., "Step 2a: Round-trip identity")>
- **Files:** <List of files to modify>
- **Problem:** <What's wrong, with file paths and line numbers>
- **Solution:** <Exact description of what to change -- specific enough to implement without re-reading the audit. For modified fixes, this reflects the user's revised approach.>
- **Risk:** <Blast radius, what else could break>
- **Verify round-trip:** <How to confirm this fix preserves round-trip identity -- specific test scenario>
- **Verify syncback parity:** <How to confirm two-way sync output matches CLI syncback -- or "N/A" if this fix doesn't touch sync paths>
- **Tests required:**
  - <Specific test description, which test layer (unit/integration/spec/snapshot), expected behavior>

## Skipped Fixes

Documented so they aren't lost -- they may be revisited later.

### Skipped: <Short title>

- **Finding:** <Audit section>
- **Problem:** <What's wrong>
- **Reason skipped:** <User's stated reason>

## Accepted Limitations

For each limitation the user accepted in step 14:

- **Limitation:** <What it is>
- **Scenario:** <When data is lost/corrupted>
- **User decision:** Accepted for this release
- **Mitigation:** <Any warnings/logging that should be in place>

## Deferred Refactors

Major structural improvements flagged during the audit that are too large to include. These should be evaluated after the fixes above are complete.

- <Refactor description, affected files, estimated scope>

## Test Plan

Tests are not optional. Every approved fix must have test coverage. A fix is not complete until its tests pass (or intentionally fail, for known limitations).

Summary of ALL tests to be written across every fix, organized by layer. Each entry references back to the fix that requires it.

### Rust Unit Tests (`#[cfg(test)]` blocks)
For isolated logic: helpers, path computation, patch compute/apply.
- <test description> (Fix N)

### Rust Integration Tests (`tests/tests/`)
For end-to-end server behavior: serve tests, two-way sync via `/api/write`, connected mode, syncback.
- <test description> (Fix N)

### Lua Spec Tests (`.spec.lua` files)
For plugin-side encoding, decoding, batching, reconciliation.
- <test description> (Fix N)

### Snapshot Tests (`insta` crate)
For forward-sync patch output verification.
- <test description> (Fix N)

### Failing Tests for Known Limitations
- <test description -- these assert correct behavior and are expected to fail> (Accepted Limitation)

## Test Rules

The implementer MUST follow these rules when writing tests for this plan:

### For bugs found and fixed

Every bug fix MUST have a test that:
1. Would have **FAILED** before the fix
2. **PASSES** after the fix
3. Prevents the bug from regressing

### For missing coverage identified

For each "missing test coverage" gap:
1. Write the test
2. Verify it passes against the current implementation
3. If it fails, investigate -- it may have found another bug

### For known limitations (Accepted Limitations section)

For each known limitation that violates round-trip identity:
1. Write a test that asserts the **correct** behavior
2. The test SHOULD FAIL against the current implementation
3. **Leave the test failing** -- do NOT mark it `#[ignore]` or fix the implementation
4. If a test unexpectedly PASSES, analyze why. If correct, keep it as free coverage
5. These tests serve as acceptance criteria for future fixes

### Scope: test every affected layer

For each finding, determine which test layers are affected and write tests in ALL of them:
- Does the bug manifest at the plugin level? -> Lua spec test
- Does it manifest at the server level? -> Rust unit test or integration test
- Does it affect the round-trip? -> Integration test with `/api/write` + filesystem verification
- Does it affect forward-sync? -> Serve test with `recv_socket_packet`
- Could it regress from a different code path? -> Test that code path too

## Final Step: Run CI

After ALL fixes and tests are implemented, run the `/ci` command (see `.cursor/commands/ci.md`) to execute the full CI pipeline. Every fix must pass CI before the plan is considered complete. Do not skip this step.
```

### 17. Run CI After Plan Execution

After the plan has been fully executed in Agent mode (all approved fixes applied, all tests written), run the `/ci` command (`.cursor/commands/ci.md`) as the final step. The plan is not complete until CI passes clean.

**Rules for the plan file:**

- The plan must be **self-contained**: an implementer should be able to execute it in Agent mode by reading only the plan file and `atlas.mdc`, without re-reading the audit chat
- The Standards section is mandatory -- it anchors every fix to the project's quality bars
- Every fix must include `Verify round-trip` and `Verify syncback parity` fields so the implementer knows how to validate correctness
- Include exact file paths and line numbers (as of the current commit)
- For each fix, list the tests that must accompany it -- a fix without a test entry is incomplete
- Tests for known limitations must assert the **correct** behavior (they are expected to fail against the current implementation and should be left failing)
- Skipped fixes are documented with their problem description so future audits don't rediscover them
- The plan file is the single source of truth for post-audit work; do not leave information only in the chat
