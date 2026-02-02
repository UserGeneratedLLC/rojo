---
name: Comprehensive Plugin Tests
overview: Create comprehensive Lua plugin stress tests (.spec.lua files) covering edge cases, large-scale scenarios, and abuse testing for scripts-only mode, everything mode, and concurrent multi-developer workflows.
todos:
  - id: test-utils
    content: Create test utility modules (MockApiContext, LargeTreeGenerator, PatchGenerator)
    status: completed
  - id: diff-stress
    content: Create diff.stress.spec.lua with 25+ tests for large trees, duplicates, equality edge cases
    status: completed
  - id: apply-stress
    content: Create applyPatch.stress.spec.lua with 20+ tests for partial failures, refs, className changes
    status: completed
  - id: reify-stress
    content: Create reify.stress.spec.lua with 15+ tests for deep/wide trees, property edge cases
    status: completed
  - id: property-read
    content: Create getProperty.spec.lua with 15+ tests for all error paths and property types
    status: completed
  - id: property-write
    content: Create setProperty.spec.lua with 15+ tests for error paths and type coercion
    status: completed
  - id: decode-values
    content: Create decodeValue.spec.lua with 15+ tests for refs, types, and failure scenarios
    status: completed
  - id: encode-stress
    content: Create encodeInstance.stress.spec.lua with 25+ tests for hierarchies and all instance types
    status: completed
  - id: encode-property
    content: Create encodeProperty.spec.lua with 20+ tests for all property types
    status: completed
  - id: patchset-stress
    content: Create PatchSet.stress.spec.lua with 15+ tests for merge operations and scale
    status: completed
  - id: instancemap-stress
    content: Create InstanceMap.stress.spec.lua with 10+ tests for scale and concurrency
    status: completed
  - id: integration-sync
    content: Create syncflow.spec.lua with 15+ tests for E2E sync flow and error recovery
    status: completed
  - id: integration-twoway
    content: Create twoWaySync.spec.lua with 15+ tests for bidirectional sync and conflicts
    status: completed
  - id: scripts-only-mode
    content: Create scriptsOnly.spec.lua with tests for scripts-only mode edge cases
    status: pending
  - id: everything-mode
    content: Create everythingMode.spec.lua with tests for everything mode and all instance types
    status: pending
  - id: mode-transitions
    content: Create modeTransitions.spec.lua with tests for switching between sync modes
    status: pending
  - id: multi-developer
    content: Create multiDeveloper.spec.lua simulating concurrent changes from multiple sources
    status: pending
  - id: race-conditions
    content: Create raceConditions.spec.lua detecting race conditions during sync
    status: pending
  - id: chaos-tests
    content: Create stressPatterns.spec.lua with 20+ chaos engineering tests
    status: completed
  - id: wire-tests
    content: Update runTests.lua to include all new test modules
    status: completed
isProject: false
---

# Comprehensive Lua Plugin Stress Tests

## Vision and Philosophy

**Goal**: Make the plugin/API interface bulletproof for complex games. The plugin must handle abuse from real-world scenarios - huge games, many developers, concurrent Rojo setups, and chaotic Studio environments.

**Testing Philosophy**:

- Tests exist to EXPOSE potential issues, not just pass
- A failing test requires INVESTIGATION - it could mean:
  - The code has a bug (fix the code)
  - The test is invalid or has wrong assumptions (fix the test)
  - The test is testing something that's not actually a requirement (remove/adjust the test)
- Don't automatically assume failing test = broken code, but also don't dismiss failures
- When a test fails, determine the root cause before deciding on the fix
- Tests should be grounded in real requirements and real-world scenarios

**Code Quality Awareness**:

- The plugin code has been iterated by AI agents and may have accumulated "fix on fix" complexity
- If a subsystem needs repeated patches, step back and consider a ground-up redesign
- Example: if diff equality checks keep needing fixes, redesign the comparison logic
- Use good judgment - don't rewrite everything, but don't just pile on fixes either

**Why Abuse Testing Matters**:

- The plugin/API interface is a mega weak layer
- It constantly crashes, nukes files, syncs incorrectly, misses files, or syncs wrong files
- Real games have thousands of instances, concurrent developers, and unpredictable Studio behavior
- Scripts-only mode AND everything-mode must both be bulletproof

---

## Motivation

The Rojo plugin/API interface is a critical weak layer that must handle:

- Scripts-only mode and everything-mode
- Large games with many developers (potentially 10+ developers with their own Rojo setups)
- Concurrent Rojo setups on the same game
- Edge cases causing crashes, file nuking, incorrect syncing
- Rapid changes during active development
- Recovery from partial failures and network issues

## Known Bug Patterns to Catch

These are the types of bugs that stress tests should expose:

**Duplicate/Ambiguous Path Handling**:

- Instances with duplicate-named siblings cause ambiguous paths
- Nested duplicates at any level of the hierarchy
- Path uniqueness checks must walk entire ancestor chain
- Tests should verify duplicate detection at every level

**Property Equality Edge Cases**:

- Floating-point comparison (epsilon = 0.0001)
- Color3 RGB integer comparison vs float comparison
- CFrame/Vector3/Vector2 precision
- NaN handling (NaN != NaN)
- EnumItem vs number comparisons

**Patch Application Failures**:

- Partial application (some properties succeed, others fail)
- Ref properties to non-existent instances
- ClassName changes with child migration failures
- Instance creation failures (invalid ClassName)
- Permission errors on protected properties

**Concurrency Issues**:

- Changes during confirmation dialogue
- Rapid bidirectional changes
- Instance destruction during patch application
- WebSocket messages during patch processing

**Memory and Scale**:

- Large instance trees (1000+ instances)
- Deep hierarchies (50+ levels)
- Wide hierarchies (100+ siblings)
- Large patches (500+ changes)

---

## Test Files to Create

### 1. Reconciler Stress Tests

`**plugin/src/Reconciler/diff.stress.spec.lua**` - Diff algorithm edge cases and stress testing

```lua
-- Key test scenarios:
-- Large tree diffing (1000+ instances)
-- Duplicate name detection at multiple levels  
-- Ambiguous path handling (nested duplicates)
-- Floating-point property precision (epsilon edge cases)
-- Color3/CFrame/Vector equality edge cases
-- ignoreUnknownInstances with mixed trees
-- Scripts-only mode deletion rules
-- Property decode failures during diff
-- Concurrent structure changes during diff
-- NaN handling in property comparisons
```

`**plugin/src/Reconciler/applyPatch.stress.spec.lua**` - Patch application edge cases

```lua
-- Key test scenarios:
-- Large batch removals (500+ instances)
-- Large batch additions (500+ instances) 
-- Mixed add/remove/update in single patch
-- Partial application failures (some succeed, some fail)
-- Complex className changes (multi-level deep)
-- Ref property chains (A->B->C->D)
-- Circular ref detection
-- Concurrent patch application
-- Race conditions with instance destruction
-- ClassName change with failing child migration
```

`**plugin/src/Reconciler/reify.stress.spec.lua**` - Instance creation stress tests

```lua
-- Key test scenarios:
-- Deep hierarchy creation (50+ levels)
-- Wide hierarchy creation (100+ siblings)
-- Mixed success/failure in sibling creation
-- Property type coercion edge cases
-- Large property sets (50+ properties per instance)
-- Nested ref chains during creation
-- Instance creation during active ChangeHistoryService
-- Memory pressure scenarios
```

### 2. Property Handling Tests

`**plugin/src/Reconciler/getProperty.spec.lua**` - Property reading edge cases

```lua
-- Key test scenarios:
-- All error paths (UnknownProperty, UnreadableProperty, LackingPropertyPermissions)
-- Canonical property name resolution
-- Serialization-only property aliases
-- Permission error detection for protected properties
-- Various property types (all Roblox types)
-- Computed vs stored properties
-- Properties that changed scriptability between engine versions
```

`**plugin/src/Reconciler/setProperty.spec.lua**` - Property writing edge cases

```lua
-- Key test scenarios:
-- All error paths (UnwritableProperty, LackingPropertyPermissions)
-- Type coercion scenarios (number -> int, etc.)
-- Unknown property handling (should not error)
-- Write-only vs ReadWrite properties
-- Properties with validation (e.g., Size on Terrain)
-- Rapid sequential writes to same property
```

`**plugin/src/Reconciler/decodeValue.spec.lua**` - Value decoding stress tests

```lua
-- Key test scenarios:
-- All ref scenarios (null, valid, invalid, circular)
-- All RbxDom encoded value types
-- Decode failure scenarios
-- Malformed encoded values
-- Edge case values (huge numbers, empty strings, etc.)
-- Nested encoded structures
```

### 3. Instance Encoding Stress Tests

`**plugin/src/ChangeBatcher/encodeInstance.stress.spec.lua**` - Encoding for syncback

```lua
-- Key test scenarios:
-- Deep hierarchies (100 levels)
-- Wide hierarchies (1000 siblings)
-- Duplicate name handling at every level
-- All supported instance types encoding
-- Complex attribute encoding (all types)
-- Tag encoding edge cases
-- Instances with 100+ properties
-- Script encoding with special characters in Source
-- Concurrent encoding of changing instances
-- Path uniqueness checking at scale
-- Memory usage with huge instances
```

`**plugin/src/ChangeBatcher/encodeProperty.spec.lua**` - Property encoding tests

```lua
-- Key test scenarios:
-- All property types (String, Bool, Int32, Int64, Float32, Float64, etc.)
-- Vector2, Vector3, Color3 encoding
-- UDim, UDim2 encoding
-- Enum encoding
-- Attributes with all value types
-- BinaryString encoding
-- CFrame encoding precision
-- Large string properties (100KB+)
-- Unicode handling
```

### 4. PatchSet and InstanceMap Stress Tests

`**plugin/src/PatchSet.stress.spec.lua**` - PatchSet operations at scale

```lua
-- Key test scenarios:
-- Merging large patches (1000+ changes each)
-- Conflict resolution in merge (same ID, different changes)
-- Rapid merge operations
-- Deep equality checking on complex patches
-- countChanges/countInstances at scale
-- Serialization/deserialization of large patches
```

`**plugin/src/InstanceMap.stress.spec.lua**` - InstanceMap at scale

```lua
-- Key test scenarios:
-- Large maps (10000+ instances)
-- Rapid insert/remove cycles
-- Concurrent change callbacks
-- Memory cleanup verification
-- Pause/unpause with many instances
-- ID collision handling
```

### 5. Integration and End-to-End Tests

`**plugin/src/integration/syncflow.spec.lua**` - Full sync flow testing

```lua
-- Key test scenarios:
-- diff -> applyPatch -> verify cycle
-- Multiple consecutive patches
-- Interleaved local and remote changes
-- Error recovery after failed patch
-- State consistency after partial failure
-- scripts-only mode full flow
-- everything mode full flow
```

`**plugin/src/integration/twoWaySync.spec.lua**` - Two-way sync scenarios

```lua
-- Key test scenarios:
-- Rapid bidirectional changes
-- Conflict resolution (both sides change same instance)
-- Pull flow for all instance types
-- Push flow for all property types
-- Mixed push/pull in single confirm
-- Concurrent Studio changes during confirm
-- ChangeBatcher pause/resume during confirm
```

### 6. Scripts-Only vs Everything Mode Tests

`**plugin/src/modes/scriptsOnly.spec.lua**` - Scripts-only mode specific tests

```lua
-- Key test scenarios:
-- Only scripts are synced (ModuleScript, Script, LocalScript)
-- Non-script instances are preserved in Studio (not deleted)
-- Script children of non-script parents sync correctly
-- RunContext property handling for Script class
-- Source property encoding/decoding
-- ignoreUnknownInstances behavior with non-scripts
-- Nested scripts under non-script containers
-- Script deletion rules (scripts can always be deleted)
-- Format transitions (standalone <-> directory) for scripts only
```

`**plugin/src/modes/everythingMode.spec.lua**` - Everything mode specific tests

```lua
-- Key test scenarios:
-- All instance types sync correctly
-- Folder creation and deletion
-- Part, Model, and other non-script instances
-- Instance properties beyond Source (Anchored, Size, Color, etc.)
-- Attributes on all instance types
-- Tags on all instance types
-- .model.json generation for complex instances
-- Directory structure for instances with children
-- Meta file generation (init.meta.json5)
-- Mixed hierarchies (scripts and non-scripts)
```

`**plugin/src/modes/modeTransitions.spec.lua**` - Switching between modes

```lua
-- Key test scenarios:
-- Switch from scripts-only to everything mode
-- Switch from everything mode to scripts-only
-- Files created in everything mode preserved when switching to scripts-only
-- Orphan non-script files in scripts-only mode
-- State consistency after mode switch
```

### 7. Multi-Developer and Concurrent Scenarios

`**plugin/src/concurrent/multiDeveloper.spec.lua**` - Simulating multiple developers

```lua
-- Key test scenarios:
-- Rapid changes from "multiple sources" (simulated)
-- Conflicting changes to same instance
-- Interleaved add/remove operations
-- Property changes while confirmation dialogue is open
-- Patches arriving during patch application
-- WebSocket message queue overflow scenarios
-- State recovery after connection drop
-- Merge conflicts in PatchSet
```

`**plugin/src/concurrent/raceConditions.spec.lua**` - Race condition detection

```lua
-- Key test scenarios:
-- Instance destroyed during property read
-- Instance reparented during diff
-- InstanceMap modified during iteration
-- ChangeBatcher cycle during pause/resume
-- Confirmation callback during state transition
-- Multiple simultaneous patch applications
-- ID collision during rapid add/remove
```

### 8. Edge Case and Chaos Tests

`**plugin/src/chaos/stressPatterns.spec.lua**` - Chaos engineering patterns

```lua
-- Key test scenarios:
-- Random operations at high frequency
-- Alternating add/remove of same instances
-- Rapid className changes
-- Property flapping (rapid value changes)
-- Instance reparenting stress
-- Deep tree mutation during reconciliation
-- Memory pressure scenarios
-- Simulated network delays (via timing)
```

## Test Utilities to Create

`**plugin/src/testUtils/MockApiContext.lua**`

```lua
-- Mock API context for testing ServeSession without real server
-- Supports:
-- - Configurable responses
-- - Error injection
-- - Delay simulation
-- - Request logging
```

`**plugin/src/testUtils/LargeTreeGenerator.lua**`

```lua
-- Generates large instance hierarchies for stress testing
-- Supports:
-- - Configurable depth and width
-- - Mixed instance types
-- - Property population
-- - Duplicate name generation (for edge case testing)
```

`**plugin/src/testUtils/PatchGenerator.lua**`

```lua
-- Generates patches for stress testing
-- Supports:
-- - Random valid patches
-- - Edge case patches (all adds, all removes, etc.)
-- - Large batch patches
-- - Invalid/malformed patches for error testing
```

## Key Files to Modify

- `[plugin/src/runTests.lua](plugin/src/runTests.lua)` - Add new test modules
- `[plugin/src/Reconciler/init.spec.lua](plugin/src/init.spec.lua)` - Import stress tests

## Test Categories Summary


| Category           | File                           | Test Count (Est.) | Purpose                                 |
| ------------------ | ------------------------------ | ----------------- | --------------------------------------- |
| Diff Stress        | diff.stress.spec.lua           | 25+               | Large trees, duplicates, equality       |
| Apply Stress       | applyPatch.stress.spec.lua     | 20+               | Partial failures, refs, className       |
| Reify Stress       | reify.stress.spec.lua          | 15+               | Deep/wide trees, properties             |
| Property Read      | getProperty.spec.lua           | 15+               | All error paths, types                  |
| Property Write     | setProperty.spec.lua           | 15+               | All error paths, coercion               |
| Decode Values      | decodeValue.spec.lua           | 15+               | Refs, types, failures                   |
| Encode Instance    | encodeInstance.stress.spec.lua | 25+               | Hierarchies, duplicates, types          |
| Encode Property    | encodeProperty.spec.lua        | 20+               | All property types                      |
| PatchSet Stress    | PatchSet.stress.spec.lua       | 15+               | Merge, scale, conflict                  |
| InstanceMap Stress | InstanceMap.stress.spec.lua    | 10+               | Scale, concurrency                      |
| Sync Flow          | syncflow.spec.lua              | 15+               | E2E, modes, recovery                    |
| Two-Way Sync       | twoWaySync.spec.lua            | 15+               | Bidirectional, conflicts                |
| Scripts-Only Mode  | scriptsOnly.spec.lua           | 15+               | Script-specific sync, ignoreUnknown     |
| Everything Mode    | everythingMode.spec.lua        | 20+               | All instance types, properties          |
| Mode Transitions   | modeTransitions.spec.lua       | 10+               | Switching modes, orphan handling        |
| Multi-Developer    | multiDeveloper.spec.lua        | 15+               | Concurrent changes, conflicts           |
| Race Conditions    | raceConditions.spec.lua        | 15+               | Instance destruction during ops         |
| Chaos Tests        | stressPatterns.spec.lua        | 20+               | Random ops, flapping, chaos engineering |


**Estimated Total: 300+ new tests**

## Success Criteria

**Primary Goal: Build Confidence Through Stress Testing**

1. Failing tests require investigation - could be code bug OR invalid test
2. Each failing test should be analyzed: is this a real bug, or is the test wrong?
3. Don't skip tests without understanding why they fail
4. Tests that pass immediately are fine - they confirm expected behavior

**Quality Gates**:

1. All valid tests pass in CI
2. Tests catch regressions in the critical paths identified
3. Edge cases like duplicate names, ambiguous paths are thoroughly covered
4. Both scripts-only and everything modes have dedicated test coverage
5. Large-scale scenarios (1000+ instances) run without timeouts
6. Memory usage is reasonable during stress tests
7. No "fix on fix" pattern - if code keeps needing patches, consider redesign

**Failure Triage**:

When a stress test fails, investigate:

- The symptom (what failed)
- Is this a real bug, or is the test invalid?
- If bug: does it need a patch or a redesign?
- If invalid test: fix the test or remove it
- Document the decision and reasoning

## Anti-Patterns to Avoid

- Skipping tests without understanding why they fail
- Adding workarounds instead of fixing root causes
- Reducing test scope arbitrarily to make tests pass
- Ignoring intermittent failures without investigation
- Assuming all failing tests mean broken code (tests can be wrong too)
- Writing tests that don't reflect real requirements or real-world usage

