---
name: Live syncback fixes
overview: "Fix five issues: lost Workspace attributes, spurious ClockTime, Rojo_Ref_CurrentCamera leak, and float precision noise via f32 truncation."
todos:
  - id: fix-attributes-encoding
    content: Add explicit Attributes/Tags encoding to encodeService.lua (mirroring encodeInstance.lua)
    status: completed
  - id: fix-clocktime
    content: Filter non-serializable properties server-side in build_dom_from_chunks using the Rust reflection DB
    status: completed
  - id: fix-currentcamera-ref
    content: "Stop Rojo_Ref_CurrentCamera from leaking: filter deferred_referents after camera destruction in syncback_loop"
    status: completed
  - id: fix-float-rounding
    content: Truncate all float values to f32 in UnresolvedValue::from_variant for rbxl parity
    status: completed
  - id: fix-index-refs
    content: Change service-level refs to index-based with ObjectValue carriers (childCount + refTargetCount, 1-based index, 0=nil)
    status: completed
isProject: false
---

# Live Syncback Fixes

## Issues 1-4: DONE

## Issue 5: Index-Based Service Refs via ObjectValue Carriers

**Problem:** Service Ref targets aren't always direct children. Can't include arbitrary instances directly in the rbxm without duplicating or breaking serialization.

**Solution:** Create temporary ObjectValue instances as Ref carriers. The ObjectValue's `Value` property IS a Ref that gets preserved in the rbxm because both the ObjectValue and the target are in the same blob.

**Layout:**

```
allChildren = [
  ...svc1_children, ...svc1_objectValues,
  ...svc2_children, ...svc2_objectValues,
  ...
]
```

### Plugin -- `[plugin/src/ChangeBatcher/encodeService.lua](plugin/src/ChangeBatcher/encodeService.lua)`

Return metadata + list of ObjectValue carrier instances to append:

```lua
local children = service:GetChildren()
local refTargets = {}  -- ObjectValue carriers

for propertyName ... do
    if descriptor.dataType == "Ref" then
        local readOk, target = descriptor:read(service)
        if readOk and target then
            -- Check if target is a direct child
            for i, child in children do
                if child == target then
                    refs[propertyName] = i  -- 1-based into children range
                    goto found
                end
            end
            -- Not a child: create ObjectValue carrier
            local carrier = Instance.new("ObjectValue")
            carrier.Name = propertyName
            carrier.Value = target
            table.insert(refTargets, carrier)
            refs[propertyName] = #children + #refTargets
            ::found::
        end
        continue
    end
end

chunk.childCount = #children
chunk.refTargetCount = #refTargets
return chunk, refTargets
```

### Plugin -- `[plugin/src/App/init.lua](plugin/src/App/init.lua)` `performSyncback`

Append children + ObjectValue carriers, clean up after serialization:

```lua
local allRefTargets = {}
for _, className in SYNCBACK_SERVICES do
    local ok, service = pcall(game.FindService, game, className)
    if ok and service then
        local chunk, refTargets = encodeService(service)
        for _, child in service:GetChildren() do
            table.insert(allChildren, child)
        end
        for _, carrier in refTargets do
            table.insert(allChildren, carrier)
            table.insert(allRefTargets, carrier)
        end
        table.insert(services, chunk)
    end
end

local data = buffer.create(0)
if #allChildren > 0 then
    data = SerializationService:SerializeInstancesAsync(allChildren)
end

-- Clean up temporary ObjectValue carriers
for _, carrier in allRefTargets do
    carrier:Destroy()
end
```

### Server -- `[src/web/interface.rs](src/web/interface.rs)`

```rust
pub struct ServiceChunk {
    pub class_name: String,
    #[serde(default)]
    pub child_count: u32,
    #[serde(default)]
    pub ref_target_count: u32,
    #[serde(default)]
    pub properties: HashMap<String, Variant>,
    #[serde(default)]
    pub refs: HashMap<String, u32>,  // prop_name -> 1-based index, 0=nil
}
```

Remove `ServiceRef` struct.

### Server -- `[src/cli/serve.rs](src/cli/serve.rs)` `build_dom_from_chunks`

```rust
let child_count = chunk.child_count as usize;
let ref_count = chunk.ref_target_count as usize;
let total = child_count + ref_count;
let end = (cursor + total).min(cloned_children.len());
let service_range = &cloned_children[cursor..end];

// Parent only the real children
for &child_ref in &service_range[..child_count.min(service_range.len())] {
    dom.transfer_within(child_ref, service_ref);
}

// Resolve refs using the full range
let mut carriers_to_destroy = Vec::new();
if !chunk.refs.is_empty() {
    for (prop_name, &idx) in &chunk.refs {
        if idx == 0 || (idx as usize - 1) >= service_range.len() {
            continue;
        }
        let carrier_ref = service_range[idx as usize - 1];
        // If in the ref target range, it's an ObjectValue carrier
        if (idx as usize) > child_count {
            let carrier = dom.get_by_ref(carrier_ref).unwrap();
            if let Some(Variant::Ref(actual_target)) = carrier.properties.get("Value") {
                let service = dom.get_by_ref_mut(service_ref).unwrap();
                service.properties.insert(
                    prop_name.as_str().into(),
                    Variant::Ref(*actual_target),
                );
            }
            carriers_to_destroy.push(carrier_ref);
        } else {
            // Direct child ref
            let service = dom.get_by_ref_mut(service_ref).unwrap();
            service.properties.insert(
                prop_name.as_str().into(),
                Variant::Ref(carrier_ref),
            );
        }
    }
}

// Destroy ObjectValue carriers
for carrier_ref in carriers_to_destroy {
    dom.destroy(carrier_ref);
}

cursor = end;
```

### Test helpers -- `[tests/rojo_test/serve_util.rs](tests/rojo_test/serve_util.rs)`

- `ServiceEntry` gets `ref_targets: Vec<InstanceBuilder>` for ObjectValue carriers
- `make_service_chunk_full` refs param: `Vec<(&str, u32)>` (prop_name, 1-based index)
- `build_syncback_request` appends ref_targets after children per service
- `make_rbxl_from_chunks` resolves refs: direct children by index, carriers by reading `Value` property

### Tests

- Update `parity_camera_not_synced` to use index-based ref with ObjectValue carrier
- Existing tests that don't use refs continue to work (empty refs map, refTargetCount=0)

## Deferred: Matching Parity

Revisit after these fixes land.