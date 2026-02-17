---
name: Reconciler robustness overhaul
overview: Fix the Atlas plugin Reconciler to handle MeshPart creation via AssetService, MeshId swapping, and CSG reconstruction -- modeled after the proven rbxsync Sync.luau / CSGHandler.luau implementation.
todos:
  - id: tags-attrs-verify
    content: Verify Tags/Attributes work through existing RbxDom customProperties.lua path (read-only audit, no code changes)
    status: pending
  - id: meshpart-create
    content: Add MeshPart creation via AssetService:CreateMeshPartAsync in reify.lua, skip MeshId in property loop
    status: pending
  - id: meshid-swap
    content: "Add MeshId change detection in applyPatch.lua: recreate MeshPart, copy children/properties, swap in instanceMap"
    status: pending
  - id: csg-handler
    content: Port CSGHandler from rbxsync to Atlas plugin (Plugin:Separate for decomposition, UnionAsync/SubtractAsync for reconstruction)
    status: pending
  - id: csg-twoway-decompose
    content: "Add CSG decomposition to ChangeBatcher/encodeInstance: when encoding a CSG part, call Plugin:Separate and send component parts to server"
    status: pending
  - id: csg-server-store
    content: "Server-side: store decomposed CSG component parts on disk and send them back to plugin during forward sync"
    status: pending
  - id: csg-reify-reconstruct
    content: "Plugin-side: detect CSG component data in patches, reconstruct via CSGHandler in reify.lua"
    status: pending
  - id: csg-warning
    content: Add warning for .rbxm-sourced CSG parts that have never been decomposed (appear as empty shells until first two-way sync)
    status: pending
isProject: false
---

# Reconciler Robustness Overhaul

## Tags/Attributes: Already Working (Verification Only)

Tags and Attributes ARE correctly handled through the RbxDom `customProperties.lua` system ([plugin/rbx_dom_lua/customProperties.lua](plugin/rbx_dom_lua/customProperties.lua) lines 51-120):

- **Tags** (lines 97-120): Uses `CollectionService:GetTags()` for read, `CollectionService:AddTag()/RemoveTag()` for write. Properly diffs existing tags against new tags.
- **Attributes** (lines 51-96): Uses `instance:GetAttributes()` for read, `instance:SetAttribute()` for write. Clears removed attributes. Skips reserved `RBX` prefixed names.

They arrive in the `Properties` map from the server (e.g. `Properties.Tags`, `Properties.Attributes`) and flow through `setProperty` -> RbxDom `PropertyDescriptor:write` -> custom handler. The property application order within a single instance doesn't matter per user confirmation.

No code changes needed -- just verify during implementation that this path is exercised.

---

## Problem 1: MeshPart Creation Fails Silently

**File:** [plugin/src/Reconciler/reify.lua](plugin/src/Reconciler/reify.lua) line 63

```lua
local createSuccess, instance = pcall(Instance.new, virtualInstance.ClassName)
```

`Instance.new("MeshPart")` creates an instance with empty MeshId. **MeshId is read-only** after creation in plugins. The server sends MeshId in Properties, but `setProperty` fails silently (property added to `unappliedProperties`). Result: every MeshPart synced from server has no mesh geometry.

**rbxsync reference:** [Sync.luau](d:\UserGenerated\rbxsync\plugin\src\Sync.luau) lines 624-681 use `AssetService:CreateMeshPartAsync(meshId)`, then apply remaining properties with MeshId excluded.

**Fix in reify.lua:** Before `Instance.new`, check for MeshPart and use `AssetService:CreateMeshPartAsync`:

```lua
local instance
local meshIdAlreadySet = false

if virtualInstance.ClassName == "MeshPart" then
    local meshIdValue = virtualInstance.Properties.MeshId
    if meshIdValue then
        local decodeSuccess, meshId = decodeValue(meshIdValue, instanceMap)
        if decodeSuccess and type(meshId) == "string" and meshId ~= "" then
            local AssetService = game:GetService("AssetService")
            local ok, meshPart = pcall(function()
                return AssetService:CreateMeshPartAsync(meshId)
            end)
            if ok and meshPart then
                instance = meshPart
                meshIdAlreadySet = true
            end
        end
    end
end

if instance == nil then
    local createSuccess
    createSuccess, instance = pcall(Instance.new, virtualInstance.ClassName)
    if not createSuccess then
        addAllToPatch(unappliedPatch, virtualInstances, id)
        return
    end
end
```

Then in the property loop, skip MeshId if already set:

```lua
if meshIdAlreadySet and propertyName == "MeshId" then
    continue
end
```

---

## Problem 2: MeshId Swap on Existing MeshPart

**File:** [plugin/src/Reconciler/applyPatch.lua](plugin/src/Reconciler/applyPatch.lua) lines 250-277

When an update changes MeshId on an existing MeshPart, `setProperty` fails (read-only). The property ends up in `unappliedUpdate` -- the MeshPart keeps the old mesh.

**rbxsync reference:** [Sync.luau](d:\UserGenerated\rbxsync\plugin\src\Sync.luau) lines 852-941 detect MeshId change, recreate via `CreateMeshPartAsync`, copy properties/children, destroy old.

**Fix in applyPatch.lua:** Add a MeshPart recreation block before the normal update logic:

```lua
-- Before the normal property update loop:
if instance:IsA("MeshPart") and update.changedProperties
    and update.changedProperties.MeshId then

    local decodeSuccess, newMeshId = decodeValue(update.changedProperties.MeshId, instanceMap)

    if decodeSuccess and type(newMeshId) == "string" and newMeshId ~= "" then
        local AssetService = game:GetService("AssetService")
        local ok, newMeshPart = pcall(function()
            return AssetService:CreateMeshPartAsync(newMeshId)
        end)

        if ok and newMeshPart then
            -- Apply name
            newMeshPart.Name = update.changedName or instance.Name

            -- Apply changed properties (excluding MeshId)
            for propName, propValue in pairs(update.changedProperties) do
                if propName ~= "MeshId" then
                    local ds, dv = decodeValue(propValue, instanceMap)
                    if ds then setProperty(newMeshPart, propName, dv) end
                end
            end

            -- Move children
            for _, child in ipairs(instance:GetChildren()) do
                child.Parent = newMeshPart
            end

            -- Swap in instanceMap
            local parent = instance.Parent
            newMeshPart.Parent = parent
            instanceMap:insert(update.id, newMeshPart)
            instance.Parent = nil  -- Not Destroy(), allows undo

            continue  -- Skip normal update path
        end
    end
end
```

---

## Problem 3: CSG Parts Created as Empty Shells

### Root Cause

The server filters out CSG geometry data before sending to the plugin:

- `**MeshData**` (BinaryString) -- can't be serialized to JSON/MessagePack ([src/web/api.rs](src/web/api.rs) line 3606-3608)
- `**PhysicalConfigData**` (SharedString) -- explicitly filtered ([src/web/interface.rs](src/web/interface.rs) lines 147-153)

Even if these bytes reached the plugin, they're **not scriptable** -- no Roblox API exists to set `MeshData` or `PhysicalConfigData` on instances from Lua. No plugin API exists to load raw .rbxm bytes into instances either.

**The only way to create CSG geometry in Studio is `BasePart:UnionAsync()` / `BasePart:SubtractAsync()` from component parts.** This is exactly what rbxsync does.

### CSG Data Flow

```
┌─────────────────────────────────────────────────────────────────────┐
│ TWO-WAY SYNC (Studio → Server → Studio)                            │
│                                                                     │
│ 1. User edits CSG in Studio                                        │
│ 2. Plugin detects change via ChangeBatcher                         │
│ 3. Plugin calls Plugin:Separate() to decompose → component parts   │
│ 4. Plugin encodes component parts + CSG metadata                   │
│ 5. Server stores component data alongside instance                 │
│ 6. On next forward sync: server sends component data               │
│ 7. Plugin reconstructs via UnionAsync/SubtractAsync                │
│                                                                     │
│ LIMITATION: .rbxm-sourced CSG that was NEVER synced from Studio    │
│ will appear as empty shells until first two-way sync.              │
│ A warning is logged for these cases.                                │
└─────────────────────────────────────────────────────────────────────┘
```

### Implementation: 4 parts

#### Part A: Port CSGHandler module

Create [plugin/src/CSGHandler.lua](plugin/src/CSGHandler.lua), ported from [rbxsync CSGHandler.luau](d:\UserGenerated\rbxsync\plugin\src\CSGHandler.luau).

Key functions:

- `CSGHandler.isCSGOperation(instance)` -- check if UnionOperation/IntersectOperation/NegateOperation
- `CSGHandler.separate(pluginInstance, union)` -- decompose via `Plugin:Separate()`, returns `{addParts, subtractParts}`
- `CSGHandler.reconstruct(basePart, additionalParts, subtractParts)` -- rebuild via `UnionAsync/SubtractAsync`

#### Part B: CSG decomposition in two-way sync (Plugin → Server)

**File:** [plugin/src/ChangeBatcher/encodeInstance.lua](plugin/src/ChangeBatcher/encodeInstance.lua)

When encoding a CSG instance for syncback, decompose it:

1. Call `CSGHandler.separate(plugin, instance)` to get component parts
2. Encode each component part as a child instance under a special `_csg` key
3. Include CSG metadata (operation types: add vs subtract)
4. Send to server as part of the normal write patch

The server write handler ([src/web/api.rs](src/web/api.rs)) needs to recognize the `_csg` data and store it.

#### Part C: Server-side CSG storage and retrieval

**File:** [src/web/api.rs](src/web/api.rs) and [src/change_processor.rs](src/change_processor.rs)

When the server receives CSG component data via `/api/write`:

1. Store component part snapshots alongside the CSG instance (e.g., as metadata or in a `_csg/` subdirectory in the snapshot tree)
2. During forward sync, when sending a CSG instance to the plugin, include the component data in the instance's properties/metadata

This needs a new mechanism -- CSG component data is not a normal property. Options:

- Add a `csgComponents` field to the `Instance` struct in [src/web/interface.rs](src/web/interface.rs)
- Or encode as a special metadata entry in `InstanceMetadata`

#### Part D: CSG reconstruction in reify.lua

**File:** [plugin/src/Reconciler/reify.lua](plugin/src/Reconciler/reify.lua) and [plugin/src/Reconciler/applyPatch.lua](plugin/src/Reconciler/applyPatch.lua)

When reifying a CSG instance that has component data:

1. Instead of `Instance.new("UnionOperation")`, queue it for deferred CSG reconstruction
2. Create the component parts first (as normal instances, unparented)
3. After all instances are created, reconstruct CSG via `CSGHandler.reconstruct()`
4. Apply properties (skip CFrame -- geometry defines position, per rbxsync line 1499-1509)
5. Parent the reconstructed union

For CSG instances WITHOUT component data (from .rbxm, never two-way synced):

- Create the empty shell via `Instance.new()` as today
- Log a warning: "CSG instance 'X' has no component data -- appears as empty shell"

---

## Implementation Order

The changes are independent enough to implement incrementally:

1. **MeshPart creation** (reify.lua) -- immediate high-value fix, no server changes
2. **MeshId swap** (applyPatch.lua) -- immediate high-value fix, no server changes
3. **CSGHandler module** (new file) -- pure plugin code, self-contained
4. **CSG warning** (reify.lua) -- trivial addition
5. **CSG decomposition** (encodeInstance.lua + server) -- requires server changes
6. **CSG server storage** (api.rs + interface.rs) -- requires protocol thinking
7. **CSG reconstruction** (reify.lua) -- requires steps 5+6

Steps 1-4 can ship as one PR. Steps 5-7 require more design and could be a separate PR.

---

## Files Modified


| File                                                                                       | Changes                                                     |
| ------------------------------------------------------------------------------------------ | ----------------------------------------------------------- |
| [plugin/src/Reconciler/reify.lua](plugin/src/Reconciler/reify.lua)                         | MeshPart creation, CSG warning, CSG deferred reconstruction |
| [plugin/src/Reconciler/applyPatch.lua](plugin/src/Reconciler/applyPatch.lua)               | MeshId swap detection + recreation                          |
| [plugin/src/CSGHandler.lua](plugin/src/CSGHandler.lua)                                     | **NEW** -- ported from rbxsync                              |
| [plugin/src/ChangeBatcher/encodeInstance.lua](plugin/src/ChangeBatcher/encodeInstance.lua) | CSG decomposition during two-way sync                       |
| [src/web/interface.rs](src/web/interface.rs)                                               | CSG component data in Instance struct                       |
| [src/web/api.rs](src/web/api.rs)                                                           | CSG storage on write, CSG data on read                      |


## Files Verified (no changes)


| File                                                                               | Status                                                             |
| ---------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| [plugin/rbx_dom_lua/customProperties.lua](plugin/rbx_dom_lua/customProperties.lua) | Tags (lines 97-120) and Attributes (lines 51-96) correctly handled |
| [plugin/src/Reconciler/setProperty.lua](plugin/src/Reconciler/setProperty.lua)     | Routes to RbxDom descriptors, including custom handlers            |
| [plugin/src/Reconciler/decodeValue.lua](plugin/src/Reconciler/decodeValue.lua)     | Decodes all property types via RbxDom                              |
| [plugin/src/Reconciler/diff.lua](plugin/src/Reconciler/diff.lua)                   | Compares Tags/Attributes via Properties map                        |


