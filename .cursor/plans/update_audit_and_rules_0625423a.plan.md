---
name: Update audit and rules
overview: Update `.cursor/commands/audit.md` to cover the live syncback pipeline, new files (encodeHelpers, encodeService, new test suites), and refactored encoding architecture. Fix discrepancies in rule MDC files where they don't reflect recent code changes.
todos:
  - id: audit-live-syncback
    content: Add Live Syncback Pipeline section to audit.md with full file tree covering plugin encoding, transport, server handling, and serve loop restart
    status: completed
  - id: audit-encode-helpers
    content: Add encodeHelpers.lua and encodeService.lua entries to the Plugin->Server ChangeBatcher section in audit.md
    status: completed
  - id: audit-new-tests
    content: Add live_syncback.rs, git_sync_defaults.rs, matching_fixtures.rs to the Shared Infrastructure test section in audit.md
    status: completed
  - id: audit-11n-live-syncback
    content: Add section 11n (Live Syncback Parity) audit checklist to audit.md
    status: completed
  - id: fix-atlas-api-mdc
    content: Add encodeHelpers.lua to atlas-api.mdc ChangeBatcher table and update encodeInstance.lua description
    status: completed
  - id: fix-atlas-plugin-mdc
    content: Add encodeHelpers.lua to atlas-plugin.mdc ChangeBatcher listing and fix SyncbackConfirm placement
    status: completed
  - id: fix-atlas-dedup-mdc
    content: Add encodeHelpers.lua to atlas-dedup.mdc Key Files Map table
    status: completed
isProject: false
---

# Update Audit Command and Rule Files

## Summary of Discrepancies Found

After comparing all 8 rule MDC files and the audit command against the actual source code and recent git history (40+ commits including live syncback, matching session caching, encodeHelpers refactor, float precision, git sync defaults), here are the issues:

### audit.md - Major Gaps

1. **No live syncback pipeline** -- The audit command describes only two pipelines (CLI syncback, two-way sync). The live syncback pipeline (`POST /api/syncback`) is a third distinct flow involving `encodeService.lua`, `SyncbackConfirm` UI, `handle_api_syncback`, `build_dom_from_chunks`, and `run_live_syncback`. Not covered at all.
2. **Missing `encodeHelpers.lua`** -- New shared helper module (`plugin/src/ChangeBatcher/encodeHelpers.lua`) that consolidates `encodeAttributes`, `encodeTags`, and `forEachEncodableProperty`. Both `encodeInstance.lua` and `encodeService.lua` import from it. Not mentioned anywhere in audit.md.
3. **Missing `encodeService.lua`** -- New file (`plugin/src/ChangeBatcher/encodeService.lua`) for service-level property encoding during live syncback. Not mentioned.
4. **Missing 3 new test suites** -- `tests/tests/live_syncback.rs`, `tests/tests/git_sync_defaults.rs`, `tests/tests/matching_fixtures.rs` are not referenced in the audit's test infrastructure section.
5. **Missing `SyncbackConfirm` UI** -- The floating confirmation dialog for live syncback (rendered in `App/init.lua`) is not mentioned in the plugin UI or data flow sections.
6. **Missing live syncback audit checklist** -- No equivalent of sections 11a-11m for live syncback-specific concerns (service property encoding, ObjectValue carrier lifecycle, rbxm blob reconstruction, clean-mode-only semantics, server restart behavior).

### atlas-api.mdc - Issues

1. **Missing `encodeHelpers.lua`** in the ChangeBatcher file table (section "ChangeBatcher"). The table lists `init.lua`, `createPatchSet.lua`, `encodePatchUpdate.lua`, `encodeInstance.lua`, `encodeService.lua`, `encodeProperty.lua`, `propertyFilter.lua` -- but `encodeHelpers.lua` is missing.
2. `**encodeInstance.lua` description** says "includes duplicate detection helpers" but doesn't note that property iteration was refactored to use `encodeHelpers.forEachEncodableProperty()`.

### atlas-plugin.mdc - Issues

1. **Missing `encodeHelpers.lua`** from the file listing under "4. ChangeBatcher" section. The `encodeHelpers.lua` file exports `encodeAttributes`, `encodeTags`, `forEachEncodableProperty`.
2. `**SyncbackConfirm**` is listed under StatusPages (`SyncbackConfirm` - "Floating dialog for live syncback confirmation (rendered by `App/init.lua`)") which is slightly misleading. It's NOT a StatusPage file -- it's rendered inline in `App/init.lua` as a `StudioPluginGui` with `isEphemeral = true`. The parenthetical note is correct but the placement under StatusPages implies a separate file.

### atlas-dedup.mdc - Issues

1. **Missing `encodeHelpers.lua`** from the Key Files Map table. This file is relevant because it contains the shared property filtering logic (`forEachEncodableProperty`) used during instance encoding for both two-way sync additions and live syncback.

## Changes

### Changes to `audit.md`

**Section "Sync System Layout" -- add Live Syncback Pipeline** after the Two-Way Sync section:

Add a third pipeline section covering:

- Plugin-side: `SYNCBACK_SERVICES` in `App/init.lua`, `encodeService.lua`, `performSyncback()`, `SyncbackConfirm` dialog
- Transport: `POST /api/syncback` (MessagePack)
- Server-side: `handle_api_syncback` in `api.rs`, `SyncbackSignal`/`LiveServer` in `mod.rs`, `build_dom_from_chunks`/`run_live_syncback` in `serve.rs`
- Wire types: `SyncbackRequest`, `ServiceChunk`, `SyncbackPayload` in `interface.rs`

**Section "Plugin --> Server" -- add `encodeHelpers.lua`** to the Change Batching subsection:

- Add entry for `plugin/src/ChangeBatcher/encodeHelpers.lua` describing `encodeAttributes`, `encodeTags`, `forEachEncodableProperty`
- Add entry for `plugin/src/ChangeBatcher/encodeService.lua` describing service-level encoding for live syncback

**Section "Shared Infrastructure" -- add new test files:**

- `tests/tests/live_syncback.rs` -- Live syncback parity, validation, lifecycle, edge cases
- `tests/tests/git_sync_defaults.rs` -- Git-based sync default selection tests
- `tests/tests/matching_fixtures.rs` -- Matching algorithm fixture tests

**Add new audit section 11n: Live Syncback Parity** covering:

- Service property encoding completeness (`encodeService.lua` vs `filter_properties`)
- ObjectValue carrier lifecycle (created, serialized, destroyed)
- rbxm blob reconstruction (`build_dom_from_chunks` child distribution)
- Clean mode semantics (always `incremental=false`)
- Server restart behavior (session invalidation, reconnection)
- `SYNCBACK_SERVICES` list consistency with `VISIBLE_SERVICES` in Rust
- Protocol/version/placeId validation
- Round-trip: live syncback output must match CLI syncback output

**Update Audit Reading Order** (step 9 area) to include live syncback flow.

### Changes to Rule MDC Files

`**atlas-api.mdc`:**

- Add `encodeHelpers.lua` row to ChangeBatcher file table with description: "Shared encoding helpers: `encodeAttributes`, `encodeTags`, `forEachEncodableProperty`. Used by `encodeInstance.lua` and `encodeService.lua`."
- Update `encodeInstance.lua` row to note it uses `encodeHelpers.forEachEncodableProperty()` for property iteration

`**atlas-plugin.mdc`:**

- Add `encodeHelpers.lua` to the "4. ChangeBatcher" file listing with description: "Shared property encoding helpers (`encodeAttributes`, `encodeTags`, `forEachEncodableProperty`)"
- Move `SyncbackConfirm` from the StatusPages list to its own note under the App section, clarifying it's rendered inline in `App/init.lua` as a floating `StudioPluginGui`, not a separate StatusPage file

`**atlas-dedup.mdc`:**

- Add `plugin/src/ChangeBatcher/encodeHelpers.lua` to the Key Files Map table with subsystem "Encoding" and purpose "Shared property filtering/encoding helpers used by `encodeInstance.lua` and `encodeService.lua`"

