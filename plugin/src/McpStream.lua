local HttpService = game:GetService("HttpService")

local Rojo = script:FindFirstAncestor("Rojo")
local Packages = Rojo.Packages

local Log = require(Packages.Log)

local Config = require(script.Parent.Config)
local ConsoleOutput = require(script.Parent.McpTools.Utils.ConsoleOutput)
local strict = require(script.Parent.strict)

local RECONNECT_INTERVAL = 3

local PASSTHROUGH_TOOLS = {
	run_code = true,
	insert_model = true,
	get_console_output = true,
	get_studio_mode = true,
	start_stop_play = true,
	run_script_in_play_mode = true,
}

local McpStream = {}
McpStream.__index = McpStream

function McpStream.new(options)
	local self = setmetatable({}, McpStream)

	self._onSyncCommand = options.onSyncCommand
	self._onGetScriptCommand = options.onGetScriptCommand
	self._onToolCommand = options.onToolCommand
	self._getPluginConfig = options.getPluginConfig
	self._wsClient = nil
	self._running = false
	self._loopThread = nil

	return self
end

function McpStream:start()
	if self._running then
		return
	end
	self._running = true

	self._loopThread = task.spawn(function()
		while self._running do
			self:_tryConnect()
			if self._running then
				task.wait(RECONNECT_INTERVAL)
			end
		end
	end)
end

function McpStream:stop()
	self._running = false
	if self._loopThread then
		task.cancel(self._loopThread)
		self._loopThread = nil
	end
	self:_disconnect()
end

function McpStream:_disconnect()
	ConsoleOutput.onLogMessage = nil
	if self._wsClient then
		pcall(function()
			self._wsClient:Destroy()
		end)
		self._wsClient = nil
	end
end

function McpStream:_tryConnect()
	local host = Config.defaultHost
	local port = Config.defaultPort
	local url = ("ws://%s:%s/api/mcp/stream"):format(host, port)

	local success, wsClient =
		pcall(HttpService.CreateWebStreamClient, HttpService, Enum.WebStreamClientType.WebSocket, { Url = url })
	if not success then
		return
	end

	self._wsClient = wsClient
	Log.trace("MCP stream connected to {}", url)

	local greeting = HttpService:JSONEncode({
		type = "hello",
		twoWaySync = self._getPluginConfig("twoWaySync"),
		oneShotSync = self._getPluginConfig("oneShotSync"),
		confirmationBehavior = self._getPluginConfig("confirmationBehavior"),
		placeId = game.PlaceId,
	})
	pcall(function()
		wsClient:Send(greeting)
	end)

	ConsoleOutput.onLogMessage = function(message: string, messageType: Enum.MessageType)
		if not self._wsClient then
			return
		end
		pcall(function()
			self._wsClient:Send(HttpService:JSONEncode({
				type = "roblox_log",
				message = message,
				level = messageType.Value,
			}))
		end)
	end

	local done = false
	local closed, errored, received

	received = wsClient.MessageReceived:Connect(function(msg)
		local ok, data = pcall(HttpService.JSONDecode, HttpService, msg)
		if not ok then
			return
		end

		if data.type == "sync" and data.requestId then
			Log.info(
				"MCP stream: received sync command (requestId={}, mode={})",
				data.requestId,
				data.mode or "standard"
			)

			local resultPromise = self._onSyncCommand(data.requestId, data.mode or "standard", data.overrides or {})

			resultPromise
				:andThen(function(result)
					if self._wsClient then
						local json = HttpService:JSONEncode(result)
						pcall(function()
							self._wsClient:Send(json)
						end)
					end
				end)
				:catch(function(err)
					Log.warn("MCP stream: sync command failed: {}", tostring(err))
					if self._wsClient then
						local errorResult = HttpService:JSONEncode({
							requestId = data.requestId,
							status = "error",
							changes = {},
							message = tostring(err),
						})
						pcall(function()
							self._wsClient:Send(errorResult)
						end)
					end
				end)
		elseif data.type == "getScript" and data.requestId then
			Log.info("MCP stream: received getScript command (requestId={})", data.requestId)

			local resultPromise = self._onGetScriptCommand(data.requestId, data)

			resultPromise
				:andThen(function(result)
					if self._wsClient then
						local json = HttpService:JSONEncode(result)
						pcall(function()
							self._wsClient:Send(json)
						end)
					end
				end)
				:catch(function(err)
					Log.warn("MCP stream: getScript command failed: {}", tostring(err))
					if self._wsClient then
						local errorResult = HttpService:JSONEncode({
							requestId = data.requestId,
							status = "error",
							message = tostring(err),
						})
						pcall(function()
							self._wsClient:Send(errorResult)
						end)
					end
				end)
		elseif data.type and data.requestId and PASSTHROUGH_TOOLS[data.type] then
			Log.info("MCP stream: received {} command (requestId={})", data.type, data.requestId)

			local resultPromise = self._onToolCommand(data.requestId, data.type, data.args or {})

			resultPromise
				:andThen(function(result)
					if self._wsClient then
						local json = HttpService:JSONEncode(result)
						pcall(function()
							self._wsClient:Send(json)
						end)
					end
				end)
				:catch(function(err)
					Log.warn("MCP stream: {} command failed: {}", data.type, tostring(err))
					if self._wsClient then
						local errorResult = HttpService:JSONEncode({
							requestId = data.requestId,
							status = "error",
							response = tostring(err),
						})
						pcall(function()
							self._wsClient:Send(errorResult)
						end)
					end
				end)
		end
	end)

	closed = wsClient.Closed:Connect(function()
		done = true
		ConsoleOutput.onLogMessage = nil
		closed:Disconnect()
		errored:Disconnect()
		received:Disconnect()
		Log.trace("MCP stream disconnected")
	end)

	errored = wsClient.Error:Connect(function(_code, _msg)
		done = true
		ConsoleOutput.onLogMessage = nil
		closed:Disconnect()
		errored:Disconnect()
		received:Disconnect()
	end)

	while not done and self._running do
		task.wait(1)
	end

	self:_disconnect()
end

return strict("McpStream", {
	new = McpStream.new,
})
