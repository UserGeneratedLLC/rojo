--!strict

local ServerScriptService = game:GetService("ServerScriptService")
local StudioTestService = game:GetService("StudioTestService")

local ConsoleOutput = require(script.Parent.Utils.ConsoleOutput)
local StartStopPlay = require(script.Parent.StartStopPlay)
local StudioModeState = require(script.Parent.Utils.StudioModeState)

type Args = {
	code: string,
	timeout: number?,
	mode: StudioModeState.TestMode,
}

local TEST_SCRIPT_NAME = "AtlasTestScript"

local function buildTestRunnerSource(userCode: string, timeout: number): string
	local timeoutStr = tostring(timeout)
	return [[
local StudioTestService = game:GetService("StudioTestService")
local LogService = game:GetService("LogService")
local RunService = game:GetService("RunService")

if not RunService:IsRunning() then
	return
end

local capturedLogs = {}
local logConnection = nil

local MESSAGE_TYPE_MAP = {
	[Enum.MessageType.MessageOutput] = "output",
	[Enum.MessageType.MessageInfo] = "info",
	[Enum.MessageType.MessageWarning] = "warning",
	[Enum.MessageType.MessageError] = "error",
}

logConnection = LogService.MessageOut:Connect(function(message, messageType)
	if string.sub(message, 1, 7) == "[Atlas]" then return end
	table.insert(capturedLogs, {
		level = MESSAGE_TYPE_MAP[messageType] or "output",
		message = message,
		ts = os.clock(),
	})
end)

local startTime = os.clock()
local isTimeout = false

local function endTest()
	local duration = os.clock() - startTime
	task.wait(0.1)

	if logConnection then logConnection:Disconnect() end

	local errors = {}
	for _, log in ipairs(capturedLogs) do
		if log.level == "error" or log.level == "warning" then
			table.insert(errors, log)
		end
	end

	StudioTestService:EndTest({
		success = ok,
		value = if ok then tostring(result) else nil,
		error = if not ok then tostring(result) else nil,
		logs = capturedLogs,
		errors = errors,
		duration = duration,
		isTimeout = isTimeout,
	})
end


task.spawn(function()
	task.wait(]] .. timeoutStr .. [[)
	isTimeout = true
	endTest()
end)

local ok, result = pcall(function()
]] .. userCode .. [[

end)

endTest()
]]
end

local function removeTestScript()
	local testScript = ServerScriptService:FindFirstChild(TEST_SCRIPT_NAME)
	if testScript then
		testScript:Destroy()
	end
end

local function injectTestScript(args: Args)
	removeTestScript()

	local source = buildTestRunnerSource(args.code, args.timeout or 1_000_000)
	local testScript = Instance.new("Script")
	testScript.Name = TEST_SCRIPT_NAME
	testScript.Parent = ServerScriptService
	testScript.Source = source
end

local function runScriptInPlayMode(args: Args): string?
	injectTestScript(args)

	ConsoleOutput.outputMessage = ""
	local success, result = pcall(function()
		if args.mode == "start_play" then
			StudioModeState.studioMode = "start_play"
			return StudioTestService:ExecutePlayModeAsync({})
		else
			StudioModeState.studioMode = "run_server"
			return StudioTestService:ExecuteRunModeAsync({})
		end
	end)

	StudioModeState.studioMode = "stop"
	removeTestScript()

	if not success then
		error(`Failed to run script in play mode: {result}`)
	end

	return result
end

local function handleRunScriptInPlayMode(args: Args): string?
	assert(type(args.code) == "string" and args.code ~= "", "Missing code in run_script_in_play_mode")
	assert(
		args.timeout == nil or (type(args.timeout) == "number" and args.timeout >= 0),
		"Timeout must be a number >= 0"
	)

	StartStopPlay({ mode = "stop" })
	task.wait(0.2)

	return runScriptInPlayMode(args)
end

return handleRunScriptInPlayMode
