Run a script in Roblox Studio play mode and automatically stop play after the script finishes or times out.

Returns structured JSON output:
```
{ success: boolean, value: string, error: string, logs: [{ level, message, ts }], errors: [{ level, message, ts }], duration: number, isTimeout: boolean }
```

Starting play mode will temporarily disconnect the Atlas plugin. The plugin reconnects automatically after play stops.

Prefer using `start_stop_play` when you need manual control over the play session.

If it returns "Previous call to start play session has not been completed", call `start_stop_play` with mode `stop` first, then try again.