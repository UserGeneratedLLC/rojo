---
name: Fix Live Syncback Service Properties
overview: Fix live syncback to preserve service-level properties and respect syncCurrentCamera by encoding service properties/attributes/tags/refs alongside the existing rbxm children data.
todos:
  - id: plugin-encode-service
    content: Create encodeService helper in plugin that returns className, data, properties, attributes, tags, refs
    status: completed
  - id: plugin-use-helper
    content: Update performSyncback to use encodeService, remove children-only guard
    status: completed
  - id: server-service-chunk
    content: Extend ServiceChunk struct with properties, attributes, tags, refs fields
    status: completed
  - id: server-apply-props
    content: Update build_dom_from_chunks to apply service properties/attributes/tags and resolve refs
    status: completed
  - id: test-helper
    content: Update make_service_chunk helpers and make_rbxl_from_chunks
    status: completed
  - id: fixture-update
    content: Update live_syncback fixture project with service $path entries
    status: completed
  - id: roundtrip-update
    content: Update roundtrip test to include service properties
    status: completed
  - id: new-tests
    content: Add parity tests for service properties, childless services, and Camera removal
    status: completed
isProject: false
---

# Fix Live Syncback Service Properties

## Root Cause

1. **Service properties lost** -- Plugin serializes `service:GetChildren()` only. Service-level properties never sent.
2. **Childless services skipped** -- `#service:GetChildren() > 0` guard skips services like VoiceChatService.
3. **Camera not removed** -- `syncback_loop` reads `Workspace.CurrentCamera` to find/remove Camera, but Workspace has no properties in the DOM.

## Constraint

`SerializationService:SerializeInstancesAsync` throws if services are in the top-level list. Children-only rbxm stays; service metadata must be encoded separately.

## Changes

### 1. Plugin -- New `encodeService` helper

Create `[plugin/src/ChangeBatcher/encodeService.lua](plugin/src/ChangeBatcher/encodeService.lua)`. Returns a full `ServiceChunk` table:

```lua
-- Returns:
-- {
--     className = "Lighting",
--     data = buffer,           -- rbxm of children (empty buffer if no children)
--     properties = {},         -- non-Ref encoded properties (via RbxDom)
--     attributes = {},         -- encoded attributes map
--     tags = {},               -- tags array
--     refs = {},               -- Ref properties: { PropName = { name, className } }
-- }
```

Implementation reuses existing `encodeProperty`, `RbxDom.findClassDescriptor`, `RbxDom.findCanonicalPropertyDescriptor`, and the `UNENCODABLE_DATA_TYPES` filter from `[plugin/src/ChangeBatcher/propertyFilter.lua](plugin/src/ChangeBatcher/propertyFilter.lua)`.

**Property encoding** -- same loop as `[encodeInstance.lua](plugin/src/ChangeBatcher/encodeInstance.lua)` lines 200-249 but without script-specific skips:

```lua
for propertyName, propertyMeta in classDescriptor.properties do
    -- skip Parent, Name, Archivable, Attributes, Tags
    if isReadable and doesSerialize then
        if descriptor.dataType == "Ref" then
            -- Read ref target; if it's a direct child, record name+className
            local ok, target = descriptor:read(service)
            if ok and target and target.Parent == service then
                refs[propertyName] = { name = target.Name, className = target.ClassName }
            end
        elseif not UNENCODABLE_DATA_TYPES[descriptor.dataType] then
            local ok, encoded = encodeProperty(service, propertyName, descriptor)
            if ok and encoded ~= nil then
                properties[propertyName] = encoded
            end
        end
    end
end
```

**Attributes** -- encoded via the Attributes property descriptor (same as `encodeAttributes` in `encodeInstance.lua` lines 121-137), but returned as a separate field.

**Tags** -- read via `service:GetTags()`, returned as a plain array.

**Refs** -- Ref properties where the target is a direct child of the service. Encoded as `{ name = target.Name, className = target.ClassName }` so the server can resolve them by finding the matching child after cloning from rbxm.

**Children rbxm** -- `SerializationService:SerializeInstancesAsync(service:GetChildren())` if children exist, otherwise empty buffer.

### 2. Plugin -- Update `performSyncback` in `[plugin/src/App/init.lua](plugin/src/App/init.lua)` (lines 641-651)

```lua
local encodeService = require(Plugin.ChangeBatcher.encodeService)

local services = {}
for _, className in SYNCBACK_SERVICES do
    local ok, service = pcall(game.FindService, game, className)
    if ok and service then
        table.insert(services, encodeService(service))
    end
end
```

No children guard. Every found service is encoded.

### 3. Server -- Extend `ServiceChunk` in `[src/web/interface.rs](src/web/interface.rs)` (lines 30-37)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceChunk {
    pub class_name: String,
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
    #[serde(default)]
    pub properties: HashMap<String, Variant>,
    #[serde(default)]
    pub attributes: HashMap<String, Variant>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub refs: HashMap<String, ServiceRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceRef {
    pub name: String,
    pub class_name: String,
}
```

All new fields use `#[serde(default)]` for backwards compatibility.

### 4. Server -- Update `build_dom_from_chunks` in `[src/cli/serve.rs](src/cli/serve.rs)` (lines 140-179)

After creating the service instance, apply properties/attributes/tags:

```rust
for chunk in &payload.services {
    let mut builder = InstanceBuilder::new(&chunk.class_name);

    // Apply encoded properties
    for (key, value) in &chunk.properties {
        builder = builder.with_property(key.as_str(), value.clone());
    }

    // Apply attributes
    if !chunk.attributes.is_empty() {
        let attrs = rbx_dom_weak::types::Attributes::new();
        // ... convert HashMap<String, Variant> to Attributes ...
        builder = builder.with_property("Attributes", Variant::Attributes(attrs));
    }

    // Apply tags
    if !chunk.tags.is_empty() {
        builder = builder.with_property("Tags", Variant::Tags(chunk.tags.clone().into()));
    }

    let service_ref = dom.insert(root_ref, builder);
    created_services.insert(chunk.class_name.clone());

    // Clone children from rbxm
    if !chunk.data.is_empty() {
        let chunk_dom = rbx_binary::from_reader(Cursor::new(&chunk.data))?;
        for &child_ref in chunk_dom.root().children() {
            deep_clone_into(&chunk_dom, &mut dom, child_ref, service_ref, &mut global_ref_map);
        }
    }

    // Resolve Ref properties by finding matching children
    if !chunk.refs.is_empty() {
        let children: Vec<Ref> = dom.get_by_ref(service_ref).unwrap().children().to_vec();
        for (prop_name, target) in &chunk.refs {
            let found = children.iter().find(|&&child_ref| {
                let child = dom.get_by_ref(child_ref).unwrap();
                child.name.as_str() == target.name && child.class.as_str() == target.class_name
            });
            if let Some(&child_ref) = found {
                let service = dom.get_by_ref_mut(service_ref).unwrap();
                service.properties.insert(prop_name.as_str().into(), Variant::Ref(child_ref));
            }
        }
    }
}
```

This handles `Workspace.CurrentCamera` naturally -- the Camera is cloned from the rbxm, then the Ref is resolved by matching `name="Camera", className="Camera"` among children. `syncback_loop` can then find it and destroy it when `syncCurrentCamera: false`.

### 5. Test helpers -- `[tests/rojo_test/serve_util.rs](tests/rojo_test/serve_util.rs)`

Update `make_service_chunk` to accept optional properties/attributes/tags/refs. Add companion:

```rust
pub fn make_service_chunk_full(
    class_name: &str,
    properties: Vec<(&str, Variant)>,
    attributes: Vec<(&str, Variant)>,
    tags: Vec<String>,
    refs: Vec<(&str, &str, &str)>,  // (prop_name, target_name, target_class)
    children: Vec<InstanceBuilder>,
) -> ServiceChunk
```

Update `make_rbxl_from_chunks` to apply properties/attributes/tags/refs to the service in the DOM (mirrors server logic).

Update `roundtrip_build_syncback_rebuild` to extract service properties from the built rbxl and populate `ServiceChunk` fields.

### 6. Fixture -- `[rojo-test/serve-tests/live_syncback/default.project.json5](rojo-test/serve-tests/live_syncback/default.project.json5)`

Add service entries with `$path` for Lighting, SoundService, StarterPlayer, TextChatService, VoiceChatService. Create empty `src/` subdirectories.

### 7. New tests -- `[tests/tests/live_syncback.rs](tests/tests/live_syncback.rs)`

**Service property parity tests** (use `assert_live_matches_cli`):

- `**parity_lighting_properties`** -- Lighting with `Ambient`, `ClockTime`, `Brightness` + child
- `**parity_soundservice_properties`** -- SoundService with `AmbientReverb`, `DistanceFactor`
- `**parity_starterplayer_properties`** -- StarterPlayer with `CameraMaxZoomDistance`
- `**parity_textchatservice_properties**` -- TextChatService with `ChatVersion`
- `**parity_childless_service_with_properties**` -- Service with properties but zero children
- `**parity_service_properties_with_children**` -- Service with both properties AND children
- `**parity_multiple_services_with_properties**` -- Multiple services in one request

**Camera removal test:**

- `**parity_camera_not_synced`** -- Workspace with Camera child + `CurrentCamera` ref. Verify Camera does NOT appear on disk (default `syncCurrentCamera: false`).

**Round-trip test:**

- `**roundtrip_service_properties`** -- Build -> live syncback -> rebuild. Compare service properties survive the cycle.

