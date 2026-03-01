Sync filesystem changes to Roblox Studio.

Modes:
- `standard` (default): auto-accepts if all changes are git-resolved, otherwise shows Studio UI for manual review.
- `manual`: always shows Studio UI, never auto-accepts.
- `fastfail`: fails immediately with `fastfail_unresolved` status if any changes are unresolved after defaults and overrides. Use this to probe for conflicts before committing to a sync.
- `dryrun`: returns what would change without applying anything. Useful for exploring the current diff or establishing the instance id mapping for `get_script`.

Overrides let you auto-accept specific changes by instance id with optional verification:
- `id` (required): the 32-char hex server Ref from a previous sync response.
- `direction` (required): `"push"` (Atlas to Studio) or `"pull"` (Studio to Atlas).
- `studioHash` (optional, scripts only): SHA1 of the git blob format of the script's current Studio Source. Override is rejected if the hash doesn't match (concurrent edit detected).
- `expectedProperties` (optional): map of property name to expected current Studio value in RbxDom encoded format. Override is rejected if any value doesn't match.

Recommended workflow:
1. `atlas_sync()` — basic sync. Auto-accepts if everything is git-resolved, else shows Studio UI.
2. `atlas_sync(mode: "dryrun")` — explore what would change, get instance ids and fsPath mappings.
3. `atlas_sync(mode: "fastfail")` — get enriched change data (studioHash, properties, fsPath). Fails immediately if unresolved.
4. On conflict: call `get_script(id)` to read the Studio version, merge with your local changes, write to filesystem, then `atlas_sync(overrides: [{id, direction: "push", studioHash: "...from get_script..."}])`.

Response includes a human-readable summary followed by a `<json>` block with structured data. Each change entry contains: path, id, direction, patchType (Add/Edit/Remove), className, studioHash (scripts only), defaultSelection, fsPath (Atlas filesystem path), and properties (current vs incoming values in RbxDom encoded format, Source omitted for scripts).