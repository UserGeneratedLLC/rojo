Pull the entire Roblox Studio place to the filesystem (Studio to Atlas).

This performs a full "clean mode" syncback: the plugin serializes every service and its children, sends the data to the server, and the server writes the complete filesystem tree. Unlike `atlas_sync` (which syncs individual changes bidirectionally), this replaces the entire project on disk with the current Studio state.

Use this when:
- You want to capture the full Studio state to disk (e.g., after building a level in Studio).
- The filesystem is out of date and you want to overwrite it with what's in Studio.
- You want a one-shot pull of everything rather than incremental sync.

No arguments are required. The plugin must be connected to the MCP stream.

Response includes the number of files written and removed.
