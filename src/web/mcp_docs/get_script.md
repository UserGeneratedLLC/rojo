Read a script's source code from Roblox Studio.

Parameters `id` and `fsPath` come from a previous `atlas_sync` response's change entries. At least one is required; `id` takes priority.

If you haven't run `atlas_sync` yet, call `atlas_sync(mode: "dryrun")` first — this establishes the instance mapping without applying any changes, then you can call `get_script` with the `id` or `fsPath` from that response.

Conflict resolution workflow:
1. `atlas_sync(mode: "fastfail")` — get changes with studioHash per script, fail if unresolved conflicts exist.
2. `get_script(id: "...")` — read the Studio version of a conflicting script.
3. Merge the Studio source with your local changes, write the merged result to the filesystem.
4. `atlas_sync(overrides: [{id, direction: "push", studioHash: "...from get_script response..."}])` — retry sync, auto-accepting your merged version. The studioHash proves the script hasn't changed between your read and the retry.

The `studioHash` in the response is the SHA1 of the git blob format (`blob <len>\0<content>`) of the script's Source property. Pass it as the `studioHash` in sync overrides to verify the script hasn't been edited by someone else between your `get_script` call and the retry sync.

Set `fromDraft: true` to read the unsaved editor content (via ScriptEditorService) instead of the saved Source property.