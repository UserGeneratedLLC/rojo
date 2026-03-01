--!strict

export type TestMode = "start_play" | "run_server"
export type StartStopPlayMode = TestMode | "stop"

return {
	studioMode = "stop" :: StartStopPlayMode,
}
