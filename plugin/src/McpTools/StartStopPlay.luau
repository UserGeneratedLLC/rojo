--!strict

local StudioTestService = game:GetService("StudioTestService")

local ConsoleOutput = require(script.Parent.Utils.ConsoleOutput)
local GameStopUtil = require(script.Parent.Utils.GameStopUtil)
local StudioModeState = require(script.Parent.Utils.StudioModeState)

type Args = {
	mode: StudioModeState.StartStopPlayMode,
}

local function callWithTimeout(callback: () -> any, timeout: number, timeoutResult: string): (string?, boolean)
	local BindableEvent = Instance.new("BindableEvent")
	local isDone = false
	local _success = true
	local result = timeoutResult
	local isTimeout = false

	task.spawn(function()
		_success, result = pcall(function()
			return callback()
		end)
		if not isDone then
			isDone = true
			BindableEvent:Fire()
		end
	end)
	task.spawn(function()
		task.wait(timeout)
		if not isDone then
			isDone = true
			isTimeout = true
			BindableEvent:Fire()
		end
	end)

	if not isDone then
		BindableEvent.Event:Wait()
	end
	return result, isTimeout
end

local function startPlay(): string?
	ConsoleOutput.outputMessage = ""
	StudioModeState.studioMode = "start_play"
	local result, isTimeout = callWithTimeout(function()
		return StudioTestService:ExecutePlayModeAsync({})
	end, 0.1, "Started play")
	if not isTimeout then
		return "Already in play mode"
	end
	return result
end

local function stop(): string?
	GameStopUtil.stopPlay()
	task.wait(1)
	StudioModeState.studioMode = "stop"
	return "Stopped"
end

local function runServer(): string?
	StudioModeState.studioMode = "run_server"
	local result, isTimeout = callWithTimeout(function()
		return StudioTestService:ExecuteRunModeAsync({})
	end, 0.1, "Ran server")
	if not isTimeout then
		return "Already in run_server mode"
	end
	return result
end

local function handleStartStopPlay(args: Args): string?
	assert(type(args.mode) == "string", "Missing mode in start_stop_play")

	if args.mode == "start_play" then
		return startPlay()
	elseif args.mode == "stop" then
		return stop()
	elseif args.mode == "run_server" then
		return runServer()
	end

	error("Invalid mode in start_stop_play, must be start_play, stop, or run_server")
end

return handleStartStopPlay
