local Packages = script.Parent.Parent.Packages
local HttpService = game:GetService("HttpService")
local Http = require(Packages.Http)
local Log = require(Packages.Log)
local Promise = require(Packages.Promise)

local Config = require(script.Parent.Config)
local Types = require(script.Parent.Types)
local Version = require(script.Parent.Version)

local validateApiInfo = Types.ifEnabled(Types.ApiInfoResponse)
local validateApiRead = Types.ifEnabled(Types.ApiReadResponse)
local validateApiSocketPacket = Types.ifEnabled(Types.ApiSocketPacket)
local validateApiSerialize = Types.ifEnabled(Types.ApiSerializeResponse)
local validateApiRefPatch = Types.ifEnabled(Types.ApiRefPatchResponse)

local function rejectFailedRequests(response)
	if response.code >= 400 then
		local message = string.format("HTTP %s:\n%s", tostring(response.code), response.body)

		return Promise.reject(message)
	end

	return response
end

local function rejectWrongVersion(infoResponseBody)
	local pluginVersion = Version.display(Config.version)
	local serverVersion = infoResponseBody.serverVersion

	if serverVersion ~= pluginVersion then
		local message = (
			"Found a server, but it's a different version than this plugin."
			.. "\nThe Atlas plugin requires an exact version match with the server."
			.. "\n\nYour plugin is version %s."
			.. "\nYour server is version %s."
			.. "\n\nPlease update your plugin or server so both are the same version."
		):format(pluginVersion, serverVersion)

		return Promise.reject(message)
	end

	if infoResponseBody.protocolVersion ~= Config.protocolVersion then
		local message = (
			"Found a server with the same version, but a different protocol version."
			.. "\nThis should not happen — please report this as a bug."
			.. "\n\nPlugin protocol: %s, Server protocol: %s"
		):format(Config.protocolVersion, infoResponseBody.protocolVersion)

		return Promise.reject(message)
	end

	return Promise.resolve(infoResponseBody)
end

local function rejectWrongFork(infoResponseBody)
	if infoResponseBody.serverFork ~= "atlas" then
		local message = (
			"Found a Rojo server, but this plugin requires an Atlas server."
			.. "\nThe stock Rojo server is not compatible with the Atlas plugin."
			.. "\nMake sure you are running 'atlas serve' instead of 'rojo serve'."
		)

		return Promise.reject(message)
	end

	return Promise.resolve(infoResponseBody)
end

local function rejectWrongPlaceId(infoResponseBody)
	if infoResponseBody.expectedPlaceIds ~= nil then
		local foundId = table.find(infoResponseBody.expectedPlaceIds, game.PlaceId)

		if not foundId then
			local idList = {}
			for _, id in ipairs(infoResponseBody.expectedPlaceIds) do
				table.insert(idList, "- " .. tostring(id))
			end

			local message = (
				"Found a Rojo server, but its project is set to only be used with a specific list of places."
				.. "\nYour place ID is %u, but needs to be one of these:"
				.. "\n%s"
				.. "\n\nTo change this list, edit 'servePlaceIds' in your .project.json file."
			):format(game.PlaceId, table.concat(idList, "\n"))

			return Promise.reject(message)
		end
	end

	if infoResponseBody.unexpectedPlaceIds ~= nil then
		local foundId = table.find(infoResponseBody.unexpectedPlaceIds, game.PlaceId)

		if foundId then
			local idList = {}
			for _, id in ipairs(infoResponseBody.unexpectedPlaceIds) do
				table.insert(idList, "- " .. tostring(id))
			end

			local message = (
				"Found a Rojo server, but its project is set to not be used with a specific list of places."
				.. "\nYour place ID is %u, but needs to not be one of these:"
				.. "\n%s"
				.. "\n\nTo change this list, edit 'blockedPlaceIds' in your .project.json file."
			):format(game.PlaceId, table.concat(idList, "\n"))

			return Promise.reject(message)
		end
	end

	return Promise.resolve(infoResponseBody)
end

local ApiContext = {}
ApiContext.__index = ApiContext

function ApiContext.new(baseUrl)
	assert(type(baseUrl) == "string", "baseUrl must be a string")

	local self = {
		__baseUrl = baseUrl,
		__sessionId = nil,
		__messageCursor = -1,
		__wsClient = nil,
		__connected = true,
		__activeRequests = {},
	}

	return setmetatable(self, ApiContext)
end

function ApiContext:__fmtDebug(output)
	output:writeLine("ApiContext {{")
	output:indent()

	output:writeLine("Connected: {}", self.__connected)
	output:writeLine("Base URL: {}", self.__baseUrl)
	output:writeLine("Session ID: {}", self.__sessionId)
	output:writeLine("Message Cursor: {}", self.__messageCursor)

	output:unindent()
	output:write("}")
end

function ApiContext:disconnect()
	self.__connected = false
	for request in self.__activeRequests do
		Log.trace("Cancelling request {}", request)
		request:cancel()
	end
	self.__activeRequests = {}

	if self.__wsClient then
		Log.trace("Closing WebSocket client")
		self.__wsClient:Close()
	end
	self.__wsClient = nil
end

function ApiContext:setMessageCursor(index)
	self.__messageCursor = index
end

function ApiContext:connect()
	local url = ("%s/api/rojo"):format(self.__baseUrl)

	return Http.get(url)
		:andThen(rejectFailedRequests)
		:andThen(Http.Response.msgpack)
		:andThen(rejectWrongVersion)
		:andThen(rejectWrongFork)
		:andThen(function(body)
			assert(validateApiInfo(body))

			return body
		end)
		:andThen(rejectWrongPlaceId)
		:andThen(function(body)
			self.__sessionId = body.sessionId

			return body
		end)
end

function ApiContext:read(ids)
	local url = ("%s/api/read/%s"):format(self.__baseUrl, table.concat(ids, ","))

	return Http.get(url):andThen(rejectFailedRequests):andThen(Http.Response.msgpack):andThen(function(body)
		if body.sessionId ~= self.__sessionId then
			return Promise.reject("Server changed ID")
		end

		assert(validateApiRead(body))

		return body
	end)
end

function ApiContext:write(patch, stageIds)
	local url = ("%s/api/write"):format(self.__baseUrl)

	-- Log what we're sending to the server (syncback/pull)
	local addedCount = 0
	for _ in pairs(patch.added) do
		addedCount += 1
	end

	if #patch.removed > 0 or addedCount > 0 or #patch.updated > 0 then
		Log.info("Sending to server: {} removals, {} additions, {} updates", #patch.removed, addedCount, #patch.updated)
	end

	for _, removed in ipairs(patch.removed) do
		Log.info("[Syncback] Remove ID: {}", tostring(removed))
	end

	for _, addedInstance in pairs(patch.added) do
		local instanceName = addedInstance.name or addedInstance.Name or "unknown"
		local instanceClass = addedInstance.className or addedInstance.ClassName or "unknown"
		Log.info("[Syncback] Add {} ({})", instanceName, instanceClass)
	end

	for _, update in ipairs(patch.updated) do
		local propCount = 0
		if update.changedProperties then
			for _ in pairs(update.changedProperties) do
				propCount += 1
			end
		end
		Log.info("[Syncback] Update ID {} ({} properties)", update.id, propCount)
	end

	local updated = {}
	for _, update in ipairs(patch.updated) do
		local fixedUpdate = {
			id = update.id,
			changedName = update.changedName,
		}

		if next(update.changedProperties) ~= nil then
			fixedUpdate.changedProperties = update.changedProperties
		end

		table.insert(updated, fixedUpdate)
	end

	-- Only add the 'added' field if the table is non-empty, or else the msgpack
	-- encode implementation will turn the table into an array instead of a map,
	-- causing API validation to fail.
	local added
	if next(patch.added) ~= nil then
		added = patch.added
	end

	local body = {
		sessionId = self.__sessionId,
		removed = patch.removed,
		updated = updated,
		added = added,
		stageIds = stageIds,
	}

	body = Http.msgpackEncode(body)

	return Http.post(url, body)
		:andThen(rejectFailedRequests)
		:andThen(Http.Response.msgpack)
		:andThen(function(responseBody)
			Log.info("Write response: {:?}", responseBody)

			return responseBody
		end)
end

function ApiContext:connectWebSocket(packetHandlers)
	local url = ("%s/api/socket/%s"):format(self.__baseUrl, self.__messageCursor)
	-- Convert HTTP/HTTPS URL to WS/WSS
	url = url:gsub("^http://", "ws://"):gsub("^https://", "wss://")

	return Promise.new(function(resolve, reject)
		local success, wsClient =
			pcall(HttpService.CreateWebStreamClient, HttpService, Enum.WebStreamClientType.WebSocket, {
				Url = url,
			})
		if not success then
			reject("Failed to create WebSocket client: " .. tostring(wsClient))
			return
		end
		self.__wsClient = wsClient

		local closed, errored, received

		received = self.__wsClient.MessageReceived:Connect(function(msg)
			local data = Http.msgpackDecode(msg)
			if data.sessionId ~= self.__sessionId then
				Log.warn("Received message with wrong session ID; ignoring")
				return
			end

			assert(validateApiSocketPacket(data))

			Log.trace("Received websocket packet: {:#?}", data)

			local handler = packetHandlers[data.packetType]
			if handler then
				local ok, err = pcall(handler, data.body)
				if not ok then
					Log.error("Error in WebSocket packet handler for type '%s': %s", data.packetType, err)
				end
			else
				Log.warn("No handler for WebSocket packet type '%s'", data.packetType)
			end
		end)

		closed = self.__wsClient.Closed:Connect(function()
			closed:Disconnect()
			errored:Disconnect()
			received:Disconnect()

			if self.__connected then
				reject("WebSocket connection closed unexpectedly")
			else
				resolve()
			end
		end)

		errored = self.__wsClient.Error:Connect(function(code, msg)
			closed:Disconnect()
			errored:Disconnect()
			received:Disconnect()

			reject("WebSocket error: " .. code .. " - " .. msg)
		end)
	end)
end

function ApiContext:open(id)
	local url = ("%s/api/open/%s"):format(self.__baseUrl, id)

	return Http.post(url, ""):andThen(rejectFailedRequests):andThen(Http.Response.msgpack):andThen(function(body)
		if body.sessionId ~= self.__sessionId then
			return Promise.reject("Server changed ID")
		end

		return nil
	end)
end

function ApiContext:serialize(ids: { string })
	local url = ("%s/api/serialize/%s"):format(self.__baseUrl, table.concat(ids, ","))

	return Http.get(url):andThen(rejectFailedRequests):andThen(Http.Response.msgpack):andThen(function(body)
		if body.sessionId ~= self.__sessionId then
			return Promise.reject("Server changed ID")
		end

		assert(validateApiSerialize(body))

		return body
	end)
end

function ApiContext:refPatch(ids: { string })
	local url = ("%s/api/ref-patch/%s"):format(self.__baseUrl, table.concat(ids, ","))

	return Http.get(url):andThen(rejectFailedRequests):andThen(Http.Response.msgpack):andThen(function(body)
		if body.sessionId ~= self.__sessionId then
			return Promise.reject("Server changed ID")
		end

		assert(validateApiRefPatch(body))

		return body
	end)
end

return ApiContext
