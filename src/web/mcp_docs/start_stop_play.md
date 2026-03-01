Start or stop play mode in Roblox Studio.

Modes:
- `start_play` — enters play testing.
- `run_server` — starts a server without a client. Don't use unless you are sure no client/player is needed.
- `stop` — exits any active play session.

Starting play mode will temporarily disconnect the Atlas plugin. The plugin reconnects automatically when play mode stops.

If it returns "Previous call to start play session has not been completed", call with `stop` first, then retry the original mode.