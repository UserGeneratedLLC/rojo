--[[
	Mock API context for testing ServeSession without real server.
	
	Supports:
	- Configurable responses
	- Error injection
	- Delay simulation
	- Request logging
]]

local HttpService = game:GetService("HttpService")

local MockApiContext = {}
MockApiContext.__index = MockApiContext

--[[
	Create a new mock API context.
	
	Options:
	- simulateDelay: Whether to add artificial delays (default: false)
	- delayMs: Delay in milliseconds (default: 100)
	- failOnConnect: Whether connect() should fail (default: false)
	- failOnRead: Whether read() should fail (default: false)
	- failOnWrite: Whether write() should fail (default: false)
]]
function MockApiContext.new(options)
	options = options or {}

	local self = setmetatable({
		-- Configuration
		simulateDelay = options.simulateDelay or false,
		delayMs = options.delayMs or 100,
		failOnConnect = options.failOnConnect or false,
		failOnRead = options.failOnRead or false,
		failOnWrite = options.failOnWrite or false,

		-- State
		connected = false,
		messageCursor = 0,
		webSocketConnected = false,

		-- Request logging
		requests = {},

		-- Configurable responses
		connectResponse = nil,
		readResponse = nil,
		writeResponse = nil,
		serializeResponse = nil,
		refPatchResponse = nil,

		-- Callbacks for custom behavior
		onConnect = nil,
		onRead = nil,
		onWrite = nil,
		onWebSocketMessage = nil,

		-- WebSocket simulation
		pendingMessages = {},
		webSocketCallback = nil,
	}, MockApiContext)

	return self
end

--[[
	Simulate a delay if configured.
]]
function MockApiContext:_maybeDelay()
	if self.simulateDelay then
		task.wait(self.delayMs / 1000)
	end
end

--[[
	Log a request.
]]
function MockApiContext:_logRequest(method, data)
	table.insert(self.requests, {
		method = method,
		data = data,
		timestamp = os.clock(),
	})
end

--[[
	Connect to the mock server.
]]
function MockApiContext:connect()
	self:_logRequest("connect", {})
	self:_maybeDelay()

	if self.failOnConnect then
		return false, "Mock connection failure"
	end

	if self.onConnect then
		local result = self.onConnect(self)
		if result ~= nil then
			return result
		end
	end

	self.connected = true

	if self.connectResponse then
		return true, self.connectResponse
	end

	-- Default connect response
	return true, {
		serverVersion = "0.0.0-mock",
		protocolVersion = 4,
		sessionId = HttpService:GenerateGUID(false),
		expectedPlaceIds = {},
		rootInstanceId = HttpService:GenerateGUID(false),
	}
end

--[[
	Read the current state from the mock server.
]]
function MockApiContext:read(ids)
	self:_logRequest("read", { ids = ids })
	self:_maybeDelay()

	if self.failOnRead then
		return false, "Mock read failure"
	end

	if self.onRead then
		local result = self.onRead(self, ids)
		if result ~= nil then
			return result
		end
	end

	if self.readResponse then
		return true, self.readResponse
	end

	-- Default empty read response
	return true, {
		messageCursor = self.messageCursor,
		instances = {},
	}
end

--[[
	Write changes to the mock server.
]]
function MockApiContext:write(patch)
	self:_logRequest("write", { patch = patch })
	self:_maybeDelay()

	if self.failOnWrite then
		return false, "Mock write failure"
	end

	if self.onWrite then
		local result = self.onWrite(self, patch)
		if result ~= nil then
			return result
		end
	end

	if self.writeResponse then
		return true, self.writeResponse
	end

	-- Default success response
	return true, { success = true }
end

--[[
	Connect WebSocket for real-time updates.
]]
function MockApiContext:connectWebSocket(callback)
	self:_logRequest("connectWebSocket", {})

	self.webSocketConnected = true
	self.webSocketCallback = callback

	-- Return a mock WebSocket object
	return {
		close = function()
			self.webSocketConnected = false
			self.webSocketCallback = nil
		end,
	}
end

--[[
	Simulate a WebSocket message from the server.
]]
function MockApiContext:simulateWebSocketMessage(message)
	if self.webSocketCallback then
		self.webSocketCallback(message)
	else
		table.insert(self.pendingMessages, message)
	end
end

--[[
	Simulate a patch message from the server.
]]
function MockApiContext:simulatePatchMessage(patch)
	self.messageCursor = self.messageCursor + 1
	self:simulateWebSocketMessage({
		type = "patch",
		messageCursor = self.messageCursor,
		patch = patch,
	})
end

--[[
	Serialize instances (mock implementation).
]]
function MockApiContext:serialize(ids)
	self:_logRequest("serialize", { ids = ids })
	self:_maybeDelay()

	if self.serializeResponse then
		return true, self.serializeResponse
	end

	-- Default empty serialization
	return true, { instances = {} }
end

--[[
	Get reference patch (mock implementation).
]]
function MockApiContext:refPatch(ids)
	self:_logRequest("refPatch", { ids = ids })
	self:_maybeDelay()

	if self.refPatchResponse then
		return true, self.refPatchResponse
	end

	-- Default empty patch
	return true, {
		removed = {},
		added = {},
		updated = {},
	}
end

--[[
	Set the message cursor.
]]
function MockApiContext:setMessageCursor(cursor)
	self:_logRequest("setMessageCursor", { cursor = cursor })
	self.messageCursor = cursor
end

--[[
	Open a script externally (mock - does nothing).
]]
function MockApiContext:open(id)
	self:_logRequest("open", { id = id })
	return true
end

--[[
	Disconnect from the mock server.
]]
function MockApiContext:disconnect()
	self:_logRequest("disconnect", {})
	self.connected = false
	self.webSocketConnected = false
	self.webSocketCallback = nil
end

--[[
	Get all logged requests.
]]
function MockApiContext:getRequests()
	return self.requests
end

--[[
	Get requests filtered by method.
]]
function MockApiContext:getRequestsByMethod(method)
	local filtered = {}
	for _, request in ipairs(self.requests) do
		if request.method == method then
			table.insert(filtered, request)
		end
	end
	return filtered
end

--[[
	Clear all logged requests.
]]
function MockApiContext:clearRequests()
	self.requests = {}
end

--[[
	Set a custom read response.
]]
function MockApiContext:setReadResponse(response)
	self.readResponse = response
end

--[[
	Set a custom write response.
]]
function MockApiContext:setWriteResponse(response)
	self.writeResponse = response
end

--[[
	Set a custom connect response.
]]
function MockApiContext:setConnectResponse(response)
	self.connectResponse = response
end

--[[
	Configure error injection.
]]
function MockApiContext:setFailures(options)
	options = options or {}
	self.failOnConnect = options.connect or false
	self.failOnRead = options.read or false
	self.failOnWrite = options.write or false
end

--[[
	Reset all state.
]]
function MockApiContext:reset()
	self.connected = false
	self.messageCursor = 0
	self.webSocketConnected = false
	self.requests = {}
	self.connectResponse = nil
	self.readResponse = nil
	self.writeResponse = nil
	self.serializeResponse = nil
	self.refPatchResponse = nil
	self.onConnect = nil
	self.onRead = nil
	self.onWrite = nil
	self.onWebSocketMessage = nil
	self.pendingMessages = {}
	self.webSocketCallback = nil
	self.failOnConnect = false
	self.failOnRead = false
	self.failOnWrite = false
end

return MockApiContext
