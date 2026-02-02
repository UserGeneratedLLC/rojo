---
name: Comprehensive Plugin Tests
overview: Create comprehensive Lua plugin stress tests (.spec.lua files) covering edge cases, large-scale scenarios, and abuse testing for scripts-only mode, everything mode, and concurrent multi-developer workflows.
todos:
  - id: test-utils
    content: Create test utility modules (MockApiContext, LargeTreeGenerator, PatchGenerator)
    status: pending
  - id: diff-stress
    content: Create diff.stress.spec.lua with 25+ tests for large trees, duplicates, equality edge cases
    status: pending
  - id: apply-stress
    content: Create applyPatch.stress.spec.lua with 20+ tests for partial failures, refs, className changes
    status: pending
  - id: reify-stress
    content: Create reify.stress.spec.lua with 15+ tests for deep/wide trees, property edge cases
    status: pending
  - id: property-read
    content: Create getProperty.spec.lua with 15+ tests for all error paths and property types
    status: pending
  - id: property-write
    content: Create setProperty.spec.lua with 15+ tests for error paths and type coercion
    status: pending
  - id: decode-values
    content: Create decodeValue.spec.lua with 15+ tests for refs, types, and failure scenarios
    status: pending
  - id: encode-stress
    content: Create encodeInstance.stress.spec.lua with 25+ tests for hierarchies and all instance types
    status: pending
  - id: encode-property
    content: Create encodeProperty.spec.lua with 20+ tests for all property types
    status: pending
  - id: patchset-stress
    content: Create PatchSet.stress.spec.lua with 15+ tests for merge operations and scale
    status: pending
  - id: instancemap-stress
    content: Create InstanceMap.stress.spec.lua with 10+ tests for scale and concurrency
    status: pending
  - id: integration-sync
    content: Create syncflow.spec.lua with 15+ tests for E2E sync flow and error recovery
    status: pending
  - id: integration-twoway
    content: Create twoWaySync.spec.lua with 15+ tests for bidirectional sync and conflicts
    status: pending
  - id: chaos-tests
    content: Create stressPatterns.spec.lua with 20+ chaos engineering tests
    status: pending
  - id: wire-tests
    content: Update runTests.lua to include all new test modules
    status: pending
isProject: false
---

# Comprehensive Lua Plugin Stress Tests

## Motivation

The Rojo plugin/API interface is a critical weak layer that must handle:

- Scripts-only mode and everything-mode
- Large games with many developers
- Concurrent Rojo setups
- Edge cases causing crashes, file nuking, incorrect syncing

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

### 6. Edge Case and Chaos Tests

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


| Category           | File                           | Test Count (Est.) | Purpose                           |
| ------------------ | ------------------------------ | ----------------- | --------------------------------- |
| Diff Stress        | diff.stress.spec.lua           | 25+               | Large trees, duplicates, equality |
| Apply Stress       | applyPatch.stress.spec.lua     | 20+               | Partial failures, refs, className |
| Reify Stress       | reify.stress.spec.lua          | 15+               | Deep/wide trees, properties       |
| Property Read      | getProperty.spec.lua           | 15+               | All error paths, types            |
| Property Write     | setProperty.spec.lua           | 15+               | All error paths, coercion         |
| Decode Values      | decodeValue.spec.lua           | 15+               | Refs, types, failures             |
| Encode Instance    | encodeInstance.stress.spec.lua | 25+               | Hierarchies, duplicates, types    |
| Encode Property    | encodeProperty.spec.lua        | 20+               | All property types                |
| PatchSet Stress    | PatchSet.stress.spec.lua       | 15+               | Merge, scale, conflict            |
| InstanceMap Stress | InstanceMap.stress.spec.lua    | 10+               | Scale, concurrency                |
| Sync Flow          | syncflow.spec.lua              | 15+               | E2E, modes, recovery              |
| Two-Way Sync       | twoWaySync.spec.lua            | 15+               | Bidirectional, conflicts          |
| Chaos Tests        | stressPatterns.spec.lua        | 20+               | Random ops, flapping              |


**Estimated Total: 225+ new tests**

## Success Criteria

1. All tests pass in CI
2. Tests catch regressions in the critical paths identified
3. Edge cases like duplicate names, ambiguous paths are thoroughly covered
4. Both scripts-only and everything modes have dedicated test coverage
5. Large-scale scenarios (1000+ instances) run without timeouts
6. Memory usage is reasonable during stress tests

