local ChangeHistoryService = game:GetService("ChangeHistoryService")
local Players = game:GetService("Players")
local SerializationService = game:GetService("SerializationService")
local ServerStorage = game:GetService("ServerStorage")
local RunService = game:GetService("RunService")

local SYNCBACK_SERVICES = {
	"Lighting",
	"MaterialService",
	"ReplicatedFirst",
	"ReplicatedStorage",
	"ServerScriptService",
	"ServerStorage",
	"SoundService",
	"StarterGui",
	"StarterPack",
	"StarterPlayer",
	"Teams",
	"TextChatService",
	"VoiceChatService",
	"Workspace",
}

local Rojo = script:FindFirstAncestor("Rojo")
local Plugin = Rojo.Plugin
local Packages = Rojo.Packages

local Roact = require(Packages.Roact)
local Log = require(Packages.Log)
local Promise = require(Packages.Promise)

local Assets = require(Plugin.Assets)
local Version = require(Plugin.Version)
local Config = require(Plugin.Config)
local Settings = require(Plugin.Settings)
local strict = require(Plugin.strict)
local Dictionary = require(Plugin.Dictionary)
local ServeSession = require(Plugin.ServeSession)
local ApiContext = require(Plugin.ApiContext)
local McpStream = require(Plugin.McpStream)
local McpTools = require(Plugin.McpTools)
local PatchSet = require(Plugin.PatchSet)
local PatchTree = require(Plugin.PatchTree)
local preloadAssets = require(Plugin.preloadAssets)
local soundPlayer = require(Plugin.soundPlayer)
local ignorePlaceIds = require(Plugin.ignorePlaceIds)
local timeUtil = require(Plugin.timeUtil)
local Theme = require(script.Theme)

local Http = require(Packages.Http)

local encodeService = require(Plugin.ChangeBatcher.encodeService)
local encodeProperty = require(Plugin.ChangeBatcher.encodeProperty)
local SHA1 = require(Plugin.SHA1)
local RbxDom = require(Packages.RbxDom)
local decodeValue = require(Plugin.Reconciler.decodeValue)
local getProperty = require(Plugin.Reconciler.getProperty)
local trueEquals = require(Plugin.Reconciler.trueEquals)

local Page = require(script.Page)
local Notifications = require(script.Components.Notifications)
local TextButton = require(script.Components.TextButton)
local Tooltip = require(script.Components.Tooltip)
local StudioPluginAction = require(script.Components.Studio.StudioPluginAction)
local StudioToolbar = require(script.Components.Studio.StudioToolbar)
local StudioToggleButton = require(script.Components.Studio.StudioToggleButton)
local StudioPluginGui = require(script.Components.Studio.StudioPluginGui)
local StudioPluginContext = require(script.Components.Studio.StudioPluginContext)
local StatusPages = require(script.StatusPages)

local AppStatus = strict("AppStatus", {
	NotConnected = "NotConnected",
	Settings = "Settings",
	Connecting = "Connecting",
	Confirming = "Confirming",
	Connected = "Connected",
	Error = "Error",
})

local e = Roact.createElement

local App = Roact.Component:extend("App")

function App:init()
	preloadAssets()

	local priorSyncInfo = self:getPriorSyncInfo()
	self.host, self.setHost = Roact.createBinding(priorSyncInfo.host or "")
	self.port, self.setPort = Roact.createBinding(priorSyncInfo.port or "")

	self.confirmationBindable = Instance.new("BindableEvent")
	self.confirmationEvent = self.confirmationBindable.Event
	self.knownProjects = {}
	self.notifId = 0

	self.waypointConnection = ChangeHistoryService.OnUndo:Connect(function(action: string)
		if not string.find(action, "^Atlas: Patch") then
			return
		end

		local undoConnection, redoConnection = nil, nil
		local function cleanup()
			undoConnection:Disconnect()
			redoConnection:Disconnect()
		end

		Log.warn(
			string.format(
				"You've undone '%s'.\nIf this was not intended, please Redo in the topbar or with Ctrl/⌘+Y.",
				action
			)
		)
		local dismissNotif = self:addNotification({
			text = string.format("You've undone '%s'.\nIf this was not intended, please restore.", action),
			timeout = 10,
			onClose = function()
				cleanup()
			end,
			actions = {
				Restore = {
					text = "Restore",
					style = "Solid",
					layoutOrder = 1,
					onClick = function()
						ChangeHistoryService:Redo()
					end,
				},
				Dismiss = {
					text = "Dismiss",
					style = "Bordered",
					layoutOrder = 2,
				},
			},
		})

		undoConnection = ChangeHistoryService.OnUndo:Once(function()
			-- Our notif is now out of date- redoing will not restore the patch
			-- since we've undone even further. Dismiss the notif.
			cleanup()
			if dismissNotif then
				dismissNotif()
			end
		end)
		redoConnection = ChangeHistoryService.OnRedo:Once(function(redoneAction: string)
			if redoneAction == action then
				-- The user has restored the patch, so we can dismiss the notif
				cleanup()
				if dismissNotif then
					dismissNotif()
				end
			end
		end)
	end)

	self.disconnectUpdatesCheckChanged = Settings:onChanged("checkForUpdates", function()
		self:checkForUpdates()
	end)
	self.disconnectPrereleasesCheckChanged = Settings:onChanged("checkForPrereleases", function()
		self:checkForUpdates()
	end)

	self:setState({
		appStatus = AppStatus.NotConnected,
		guiEnabled = false,
		confirmData = {},
		patchData = {
			patch = PatchSet.newEmpty(),
			unapplied = PatchSet.newEmpty(),
			timestamp = os.time(),
		},
		notifications = {},
		toolbarIcon = Assets.Images.PluginButton,
	})

	if RunService:IsEdit() then
		self:checkForUpdates()

		self:startSyncReminderPolling()
		self.disconnectSyncReminderPollingChanged = Settings:onChanged("syncReminderPolling", function(enabled)
			if enabled then
				self:startSyncReminderPolling()
			else
				self:stopSyncReminderPolling()
			end
		end)

		self:tryAutoReconnect():andThen(function(didReconnect)
			if not didReconnect then
				self:checkSyncReminder()
			end
		end)
	end

	if self:isAutoConnectPlaytestServerAvailable() then
		self:useRunningConnectionInfo()
		self:startSession()
	end
	self.autoConnectPlaytestServerListener = Settings:onChanged("autoConnectPlaytestServer", function(enabled)
		if enabled then
			if self:isAutoConnectPlaytestServerWriteable() and self.serveSession ~= nil then
				-- Write the existing session
				local baseUrl = self.serveSession.__apiContext.__baseUrl
				self:setRunningConnectionInfo(baseUrl)
			end
		else
			self:clearRunningConnectionInfo()
		end
	end)

	if RunService:IsEdit() then
		self.mcpStream = McpStream.new({
			onSyncCommand = function(requestId, mode, overrides)
				return self:startMcpSync(requestId, mode, overrides)
			end,
			onGetScriptCommand = function(requestId, data)
				return self:handleMcpGetScript(requestId, data)
			end,
			onToolCommand = function(requestId, toolType, args)
				return Promise.new(function(resolve)
					local ok, result = pcall(McpTools.dispatch, toolType, args)
					if ok then
						resolve({
							requestId = requestId,
							status = "success",
							response = result or "",
						})
					else
						resolve({
							requestId = requestId,
							status = "error",
							response = tostring(result),
						})
					end
				end)
			end,
			getPluginConfig = function(key)
				return Settings:get(key)
			end,
		})
		self.mcpStream:start()
	end
end

function App:willUnmount()
	self:endSession()

	self.waypointConnection:Disconnect()
	self.confirmationBindable:Destroy()

	self.disconnectUpdatesCheckChanged()
	self.disconnectPrereleasesCheckChanged()
	if self.disconnectSyncReminderPollingChanged then
		self.disconnectSyncReminderPollingChanged()
	end

	self:stopSyncReminderPolling()

	self.autoConnectPlaytestServerListener()
	self:clearRunningConnectionInfo()

	if self.mcpStream then
		self.mcpStream:stop()
		self.mcpStream = nil
		self._lastMcpFromIds = nil
	end
end

function App:addNotification(notif: {
	text: string,
	isFullscreen: boolean?,
	timeout: number?,
	actions: { [string]: { text: string, style: string, layoutOrder: number, onClick: (any) -> ()? } }?,
	onClose: (any) -> ()?,
})
	if not Settings:get("showNotifications") then
		return
	end

	self.notifId += 1
	local id = self.notifId

	self:setState(function(prevState)
		local notifications = table.clone(prevState.notifications)
		notifications[id] = Dictionary.merge({
			timeout = notif.timeout or 5,
			isFullscreen = notif.isFullscreen or false,
		}, notif)

		return {
			notifications = notifications,
		}
	end)

	return function()
		self:closeNotification(id)
	end
end

function App:closeNotification(id: number)
	if not self.state.notifications[id] then
		return
	end

	self:setState(function(prevState)
		local notifications = table.clone(prevState.notifications)
		notifications[id] = nil

		return {
			notifications = notifications,
		}
	end)
end

function App:checkForUpdates()
	local updateMessage = Version.getUpdateMessage()

	if updateMessage then
		self:addNotification({
			text = updateMessage,
			timeout = 500,
			actions = {
				Dismiss = {
					text = "Dismiss",
					style = "Bordered",
					layoutOrder = 2,
				},
			},
		})
	end
end

function App:getPriorSyncInfo(): { host: string?, port: string?, projectName: string?, timestamp: number? }
	local priorSyncInfos = Settings:get("priorEndpoints")
	if not priorSyncInfos then
		return {}
	end

	local id = tostring(game.PlaceId)
	if ignorePlaceIds[id] then
		return {}
	end

	return priorSyncInfos[id] or {}
end

function App:setPriorSyncInfo(host: string, port: string, projectName: string)
	local priorSyncInfos = Settings:get("priorEndpoints")
	if not priorSyncInfos then
		priorSyncInfos = {}
	end

	local now = os.time()

	-- Clear any stale saves to avoid disc bloat
	for placeId, syncInfo in priorSyncInfos do
		if now - (syncInfo.timestamp or now) > 12_960_000 then
			priorSyncInfos[placeId] = nil
			Log.trace("Cleared stale saved endpoint for {}", placeId)
		end
	end

	local id = tostring(game.PlaceId)
	if ignorePlaceIds[id] then
		return
	end

	priorSyncInfos[id] = {
		host = if host ~= Config.defaultHost then host else nil,
		port = if port ~= Config.defaultPort then port else nil,
		projectName = projectName,
		timestamp = now,
	}
	Log.trace("Saved last used endpoint for {}", game.PlaceId)

	Settings:set("priorEndpoints", priorSyncInfos)
end

function App:forgetPriorSyncInfo()
	local priorSyncInfos = Settings:get("priorEndpoints")
	if not priorSyncInfos then
		priorSyncInfos = {}
	end

	local id = tostring(game.PlaceId)
	priorSyncInfos[id] = nil
	Log.trace("Erased last used endpoint for {}", game.PlaceId)

	Settings:set("priorEndpoints", priorSyncInfos)
end

function App:getHostAndPort()
	local host = self.host:getValue()
	local port = self.port:getValue()

	return if #host > 0 then host else Config.defaultHost, if #port > 0 then port else Config.defaultPort
end

function App:isSyncLockAvailable()
	if #Players:GetPlayers() == 0 then
		-- Team Create is not active, so no one can be holding the lock
		return true
	end

	local lock = ServerStorage:FindFirstChild("__Atlas_SessionLock")
	if not lock then
		-- No lock is made yet, so it is available
		return true
	end

	if lock.Value and lock.Value ~= Players.LocalPlayer and lock.Value.Parent then
		-- Someone else is holding the lock
		return false, lock.Value
	end

	-- The lock exists, but is not claimed
	return true
end

function App:claimSyncLock()
	if #Players:GetPlayers() == 0 then
		Log.trace("Skipping sync lock because this isn't in Team Create")
		return true
	end

	local isAvailable, priorOwner = self:isSyncLockAvailable()
	if not isAvailable then
		Log.trace("Skipping sync lock because it is already claimed")
		return false, priorOwner
	end

	local lock = ServerStorage:FindFirstChild("__Atlas_SessionLock")
	if not lock then
		lock = Instance.new("ObjectValue")
		lock.Name = "__Atlas_SessionLock"
		lock.Archivable = false
		lock.Value = Players.LocalPlayer
		lock.Parent = ServerStorage
		Log.trace("Created and claimed sync lock")
		return true
	end

	lock.Value = Players.LocalPlayer
	Log.trace("Claimed existing sync lock")
	return true
end

function App:releaseSyncLock()
	local lock = ServerStorage:FindFirstChild("__Atlas_SessionLock")
	if not lock then
		Log.trace("No sync lock found, assumed released")
		return
	end

	if lock.Value == Players.LocalPlayer then
		lock.Value = nil
		Log.trace("Released sync lock")
		return
	end

	Log.trace("Could not relase sync lock because it is owned by {}", lock.Value)
end

function App:findActiveServer()
	local host, port = self:getHostAndPort()
	local baseUrl = if string.find(host, "^https?://")
		then string.format("%s:%s", host, port)
		else string.format("http://%s:%s", host, port)

	Log.trace("Checking for active sync server at {}", baseUrl)

	local apiContext = ApiContext.new(baseUrl)
	return apiContext:connect():andThen(function(serverInfo)
		apiContext:disconnect()
		return serverInfo, host, port
	end)
end

function App:tryAutoReconnect()
	if not Settings:get("autoReconnect") then
		return Promise.resolve(false)
	end

	local priorSyncInfo = self:getPriorSyncInfo()
	if not priorSyncInfo.projectName then
		Log.trace("No prior sync info found, skipping auto-reconnect")
		return Promise.resolve(false)
	end

	return self:findActiveServer()
		:andThen(function(serverInfo)
			-- change
			if serverInfo.projectName == priorSyncInfo.projectName then
				Log.trace("Auto-reconnect found matching server, reconnecting...")
				self:addNotification({
					text = `Auto-reconnect discovered project '{serverInfo.projectName}'...`,
				})
				self:startSession()
				return true
			end
			Log.trace("Auto-reconnect found different server, not reconnecting")
			return false
		end)
		:catch(function()
			Log.trace("Auto-reconnect did not find a server, not reconnecting")
			return false
		end)
end

function App:checkSyncReminder()
	local syncReminderMode = Settings:get("syncReminderMode")
	if syncReminderMode == "None" then
		return
	end

	if self.serveSession ~= nil or not self:isSyncLockAvailable() then
		-- Already syncing or cannot sync, no reason to remind
		return
	end

	local priorSyncInfo = self:getPriorSyncInfo()

	self:findActiveServer()
		:andThen(function(serverInfo, host, port)
			self:sendSyncReminder(
				`Project '{serverInfo.projectName}' is serving at {host}:{port}.\nWould you like to connect?`,
				{ "Connect", "Dismiss" }
			)
		end)
		:catch(function()
			if priorSyncInfo.timestamp and priorSyncInfo.projectName then
				-- We didn't find an active server,
				-- but this place has a prior sync
				-- so we should remind the user to serve

				local timeSinceSync = timeUtil.elapsedToText(os.time() - priorSyncInfo.timestamp)
				self:sendSyncReminder(
					`You synced project '{priorSyncInfo.projectName}' to this place {timeSinceSync}.\nDid you mean to run 'atlas serve' and then connect?`,
					{ "Connect", "Forget", "Dismiss" }
				)
			end
		end)
end

function App:startSyncReminderPolling()
	-- Feature disabled: polling GET /api/rojo every 30s is expensive and
	-- the setting was hidden behind showNotifications anyway.
	do
		return
	end

	if
		self.syncReminderPollingThread ~= nil
		or Settings:get("syncReminderMode") == "None"
		or not Settings:get("syncReminderPolling")
	then
		return
	end

	Log.trace("Starting sync reminder polling thread")
	self.syncReminderPollingThread = task.spawn(function()
		while task.wait(30) do
			if self.syncReminderPollingThread == nil then
				-- The polling thread was stopped, so exit
				return
			end
			if self.dismissSyncReminder then
				-- There is already a sync reminder being shown
				task.wait(5)
				continue
			end
			self:checkSyncReminder()
		end
	end)
end

function App:stopSyncReminderPolling()
	if self.syncReminderPollingThread then
		Log.trace("Stopping sync reminder polling thread")
		task.cancel(self.syncReminderPollingThread)
		self.syncReminderPollingThread = nil
	end
end

function App:sendSyncReminder(message: string, shownActions: { string })
	local syncReminderMode = Settings:get("syncReminderMode")
	if syncReminderMode == "None" then
		return
	end

	local connectIndex = table.find(shownActions, "Connect")
	local forgetIndex = table.find(shownActions, "Forget")
	local dismissIndex = table.find(shownActions, "Dismiss")

	self.dismissSyncReminder = self:addNotification({
		text = message,
		timeout = 120,
		isFullscreen = Settings:get("syncReminderMode") == "Fullscreen",
		onClose = function()
			self.dismissSyncReminder = nil
		end,
		actions = {
			Connect = if connectIndex
				then {
					text = "Connect",
					style = "Solid",
					layoutOrder = connectIndex,
					onClick = function()
						self:startSession()
					end,
				}
				else nil,
			Forget = if forgetIndex
				then {
					text = "Forget",
					style = "Bordered",
					layoutOrder = forgetIndex,
					onClick = function()
						-- The user doesn't want to be reminded again about this sync
						self:forgetPriorSyncInfo()
					end,
				}
				else nil,
			Dismiss = if dismissIndex
				then {
					text = "Dismiss",
					style = "Bordered",
					layoutOrder = dismissIndex,
					onClick = function()
						-- If the user dismisses the reminder,
						-- then we don't need to remind them again
						self:stopSyncReminderPolling()
					end,
				}
				else nil,
		},
	})
end

function App:isAutoConnectPlaytestServerAvailable()
	return RunService:IsRunning()
		and RunService:IsStudio()
		and RunService:IsServer()
		and Settings:get("autoConnectPlaytestServer")
		and workspace:GetAttribute("__Atlas_ConnectionUrl")
end

function App:isAutoConnectPlaytestServerWriteable()
	return RunService:IsEdit() and Settings:get("autoConnectPlaytestServer")
end

function App:setRunningConnectionInfo(baseUrl: string)
	if not self:isAutoConnectPlaytestServerWriteable() then
		return
	end

	Log.trace("Setting connection info for play solo auto-connect")
	workspace:SetAttribute("__Atlas_ConnectionUrl", baseUrl)
end

function App:clearRunningConnectionInfo()
	if not RunService:IsEdit() then
		-- Only write connection info from edit mode
		return
	end

	Log.trace("Clearing connection info for play solo auto-connect")
	workspace:SetAttribute("__Atlas_ConnectionUrl", nil)
end

function App:useRunningConnectionInfo()
	local connectionInfo = workspace:GetAttribute("__Atlas_ConnectionUrl")
	if not connectionInfo then
		return
	end

	Log.trace("Using connection info for play solo auto-connect")
	local host, port = string.match(connectionInfo, "^(.+):(.-)$")

	self.setHost(host)
	self.setPort(port)
end

function App:performSyncback()
	self:setState({ showingSyncbackConfirm = false })

	local host = self.host:getValue()
	local port = self.port:getValue()

	if host == "" then
		host = Config.defaultHost
	end
	if port == "" then
		port = Config.defaultPort
	end

	local services = {}
	local allChildren = {}
	local allCarriers = {}
	for _, className in SYNCBACK_SERVICES do
		local ok, service = pcall(game.FindService, game, className)
		if ok and service then
			local chunk, refTargets = encodeService(service)
			for _, child in service:GetChildren() do
				table.insert(allChildren, child)
			end
			for _, carrier in refTargets do
				table.insert(allChildren, carrier)
				table.insert(allCarriers, carrier)
			end
			table.insert(services, chunk)
		end
	end

	local data = buffer.create(0)
	if #allChildren > 0 then
		data = SerializationService:SerializeInstancesAsync(allChildren)
	end

	for _, carrier in allCarriers do
		carrier:Destroy()
	end

	local url = ("http://%s:%s/api/syncback"):format(host, port)
	local body = Http.msgpackEncode({
		protocolVersion = Config.protocolVersion,
		serverVersion = Config.expectedServerVersionString,
		placeId = game.PlaceId,
		data = data,
		services = services,
	})

	Http.post(url, body)
		:andThen(function()
			Log.info("Syncback data sent to server.")
			self:addNotification({
				text = "Syncback data sent. Server is processing.",
				timeout = 10,
			})
		end)
		:catch(function(err)
			Log.warn("Syncback failed: " .. tostring(err))
			self:addNotification({
				text = "Syncback failed: " .. tostring(err),
				timeout = 10,
			})
		end)
end

function App:startSession()
	-- If a session is already in progress, tear it down first.
	-- In one-shot mode the sync lock is skipped, so this is the only guard
	-- against overlapping sessions (e.g., auto-reconnect racing with a
	-- manual connect).
	if self.serveSession ~= nil then
		Log.trace("Ending existing session before starting a new one")
		self:endSession()
	end

	-- Skip session lock in one-shot mode since it's a quick sync-and-disconnect
	if not Settings:get("oneShotSync") then
		local claimedLock, priorOwner = self:claimSyncLock()
		if not claimedLock then
			local msg = string.format("Could not sync because user '%s' is already syncing", tostring(priorOwner))

			Log.warn(msg)
			self:addNotification({
				text = msg,
				timeout = 10,
			})
			self:setState({
				appStatus = AppStatus.Error,
				errorMessage = msg,
				toolbarIcon = Assets.Images.PluginButtonWarning,
			})

			return
		end
	end

	local host, port = self:getHostAndPort()

	local baseUrl = if string.find(host, "^https?://")
		then string.format("%s:%s", host, port)
		else string.format("http://%s:%s", host, port)
	local apiContext = ApiContext.new(baseUrl)

	local serveSession = ServeSession.new({
		apiContext = apiContext,
		twoWaySync = Settings:get("twoWaySync"),
	})

	serveSession:setUpdateLoadingTextCallback(function(text: string)
		self:setState({
			connectingText = text,
		})
	end)

	local cachedServerInfo = nil

	self.cleanupPrecommit = serveSession:hookPrecommit(function(patch, instanceMap)
		local gitMetadata = cachedServerInfo and cachedServerInfo.gitMetadata
		self:setState({
			patchTree = PatchTree.build(patch, instanceMap, { "Property", "Old", "New" }, gitMetadata),
		})
	end)
	self.cleanupPostcommit = serveSession:hookPostcommit(function(patch, instanceMap, unappliedPatch)
		local now = DateTime.now().UnixTimestamp
		self:setState(function(prevState)
			local oldPatchData = prevState.patchData
			local newPatchData = {
				patch = patch,
				unapplied = unappliedPatch,
				timestamp = now,
			}

			if PatchSet.isEmpty(patch) then
				-- Keep existing patch info, but use new timestamp
				newPatchData.patch = oldPatchData.patch
				newPatchData.unapplied = oldPatchData.unapplied
			elseif now - oldPatchData.timestamp < 2 then
				-- Patches that apply in the same second are combined for human clarity
				newPatchData.patch = PatchSet.assign(PatchSet.newEmpty(), oldPatchData.patch, patch)
				newPatchData.unapplied = PatchSet.assign(PatchSet.newEmpty(), oldPatchData.unapplied, unappliedPatch)
			end

			return {
				patchTree = PatchTree.updateMetadata(prevState.patchTree, patch, instanceMap, unappliedPatch),
				patchData = newPatchData,
			}
		end)
	end)

	serveSession:onStatusChanged(function(status, details)
		if status == ServeSession.Status.Connecting then
			if self.dismissSyncReminder then
				self.dismissSyncReminder()
				self.dismissSyncReminder = nil
			end

			self:setState({
				appStatus = AppStatus.Connecting,
				toolbarIcon = Assets.Images.PluginButton,
			})
			self:addNotification({
				text = "Connecting to session...",
			})
		elseif status == ServeSession.Status.Connected then
			self.knownProjects[details] = true
			self:setPriorSyncInfo(host, port, details)
			self:setRunningConnectionInfo(baseUrl)

			local address = ("%s:%s"):format(host, port)

			if Settings:get("oneShotSync") then
				-- One-shot mode: Don't show Connected page since we're about to disconnect.
				-- Show a loading state while the patch applies and writes complete.
				self:setState({
					appStatus = AppStatus.Connecting,
					connectingText = "Completing sync...",
					toolbarIcon = Assets.Images.PluginButton,
				})
				self:addNotification({
					text = string.format("Synced with '%s'. Completing...", details),
				})
				-- Safety: if endSession hasn't fired within 30s, force disconnect.
				-- This catches cases where the promise chain silently breaks
				-- (e.g., write request hangs, promise never resolves).
				-- Capture the current session so we only kill THIS session, not a
				-- different one that may have replaced it (e.g., auto-reconnect race).
				local timeoutSession = serveSession
				task.delay(30, function()
					if
						self.serveSession ~= nil
						and self.serveSession == timeoutSession
						and Settings:get("oneShotSync")
					then
						Log.warn("One-shot sync safety timeout: forcing disconnect")
						self:endSession()
					end
				end)
			else
				self:setState({
					appStatus = AppStatus.Connected,
					projectName = details,
					address = address,
					toolbarIcon = Assets.Images.PluginButtonConnected,
				})
				self:addNotification({
					text = string.format("Connected to session '%s' at %s.", details, address),
				})
			end
		elseif status == ServeSession.Status.Disconnected then
			self.serveSession = nil
			-- Only release lock if we claimed it (not in one-shot mode)
			if not Settings:get("oneShotSync") then
				self:releaseSyncLock()
			end
			self:clearRunningConnectionInfo()
			self:setState({
				patchData = {
					patch = PatchSet.newEmpty(),
					unapplied = PatchSet.newEmpty(),
					timestamp = os.time(),
				},
			})

			-- Details being present indicates that this
			-- disconnection was from an error.
			if details ~= nil then
				Log.warn("Disconnected from an error: {}", details)

				self:setState({
					appStatus = AppStatus.Error,
					errorMessage = tostring(details),
					toolbarIcon = Assets.Images.PluginButtonWarning,
				})
				self:addNotification({
					text = tostring(details),
					timeout = 10,
				})
			else
				self:setState({
					appStatus = AppStatus.NotConnected,
					toolbarIcon = Assets.Images.PluginButton,
				})
				self:addNotification({
					text = "Disconnected from session.",
					timeout = 10,
				})
			end
		end
	end)

	local initialSyncConfirmed = false

	serveSession:setConfirmCallback(function(instanceMap, patch, serverInfo)
		cachedServerInfo = serverInfo

		-- Filter out the DataModel name change from the patch
		-- The project name (DataModel.Name) is managed by Studio independently
		PatchSet.removeDataModelName(patch, instanceMap)

		local isOneShotMode = Settings:get("oneShotSync")

		-- ONE-SHOT MODE: After the initial sync is confirmed, skip any
		-- subsequent patches entirely. These are server echoes from processing
		-- our pull request (e.g. VFS re-snapshots after file deletions).
		-- The session is about to disconnect — applying them is pointless,
		-- and without this guard the echo triggers a second confirmation
		-- dialog that races with endSession() and gets stuck.
		if isOneShotMode and initialSyncConfirmed then
			Log.info("One-shot mode: skipping post-initial-sync echo patch")
			return "Skip"
		end

		-- ONE-SHOT MODE: Never auto-accept, always require explicit confirmation
		-- This ensures nothing can "sneak through" without user review
		if not isOneShotMode then
			if PatchSet.isEmpty(patch) then
				Log.trace("Accepting patch without confirmation because it is empty")
				return "Accept"
			end

			-- Play solo auto-connect does not require confirmation
			if self:isAutoConnectPlaytestServerAvailable() then
				Log.trace("Accepting patch without confirmation because play solo auto-connect is enabled")
				return "Accept"
			end

			local confirmationBehavior = Settings:get("confirmationBehavior")
			if confirmationBehavior ~= "Always" then
				if confirmationBehavior == "Initial" then
					-- Only confirm if we haven't synced this project yet this session
					if self.knownProjects[serverInfo.projectName] then
						Log.trace(
							"Accepting patch without confirmation because project has already been connected and behavior is set to Initial"
						)
						return "Accept"
					end
				elseif confirmationBehavior == "Large Changes" then
					-- Only confirm if the patch impacts many instances
					if PatchSet.countInstances(patch) < Settings:get("largeChangesConfirmationThreshold") then
						Log.trace(
							"Accepting patch without confirmation because patch is small and behavior is set to Large Changes"
						)
						return "Accept"
					end
				elseif confirmationBehavior == "Unlisted PlaceId" then
					-- Only confirm if the current placeId is not in the servePlaceIds allowlist
					if serverInfo.expectedPlaceIds then
						local isListed = table.find(serverInfo.expectedPlaceIds, game.PlaceId) ~= nil
						if isListed then
							Log.trace(
								"Accepting patch without confirmation because placeId is listed and behavior is set to Unlisted PlaceId"
							)
							return "Accept"
						end
					end
				elseif confirmationBehavior == "Never" then
					Log.trace("Accepting patch without confirmation because behavior is set to Never")
					return "Accept"
				end
			end
		else
			Log.info("One-shot mode: skipping all auto-accept paths, requiring explicit confirmation")
		end

		self:setState({
			connectingText = "Computing diff view...",
		})
		local patchTreeClock = os.clock()
		Log.debug("[TIMING] PatchTree.build() starting")
		local patchTree =
			PatchTree.build(patch, instanceMap, { "Property", "Current", "Incoming" }, serverInfo.gitMetadata)
		Log.debug(
			"[TIMING] PatchTree.build() completed ({} ms)",
			string.format("%.1f", (os.clock() - patchTreeClock) * 1000)
		)
		self:setState({
			appStatus = AppStatus.Confirming,
			patchTree = patchTree,
			confirmData = {
				serverInfo = serverInfo,
			},
			toolbarIcon = Assets.Images.PluginButton,
		})

		self:addNotification({
			text = "Please review and confirm the sync changes, or disconnect.",
			timeout = 7,
		})

		Log.debug("[TIMING] Waiting for user to confirm/reject patch...")
		local result = self.confirmationEvent:Wait()

		-- One-shot sync: Don't transition to Connected UI
		-- The actual disconnect is handled by setInitialSyncCompleteCallback
		-- to ensure any pending writes complete first
		if Settings:get("oneShotSync") then
			initialSyncConfirmed = true
			return result
		end

		-- Reset UI state back to Connected after confirmation
		-- This is needed for ongoing WebSocket patches where the session
		-- is already connected and won't trigger a status change
		if self.serveSession and self.serveSession:getStatus() == ServeSession.Status.Connected then
			self:setState({
				appStatus = AppStatus.Connected,
				toolbarIcon = Assets.Images.PluginButtonConnected,
				-- Clear patchTree to avoid animation issues when the
				-- PatchVisualizer unmounts while Flipper motors are running
				patchTree = nil,
			})
		end

		return result
	end)

	local pendingGitRefreshThread = nil
	local GIT_METADATA_DEBOUNCE = 0.5

	serveSession:setPatchUpdateCallback(function(instanceMap, patch, changedIds)
		-- If all changes have been reverted, auto-accept the empty patch
		if PatchSet.isEmpty(patch) then
			Log.trace("Patch became empty after merging, auto-accepting")
			-- Return empty selections to accept all (nothing to do)
			self.confirmationBindable:Fire({ type = "Confirm", selections = {} })
			return
		end

		-- Immediately rebuild with stale metadata so the UI reflects new items
		local gitMetadata = cachedServerInfo and cachedServerInfo.gitMetadata
		self:setState({
			patchTree = PatchTree.build(patch, instanceMap, { "Property", "Current", "Incoming" }, gitMetadata),
			changedIds = changedIds,
		})

		-- Debounce: schedule a fresh git metadata fetch after rapid patches settle
		if pendingGitRefreshThread then
			task.cancel(pendingGitRefreshThread)
		end
		pendingGitRefreshThread = task.delay(GIT_METADATA_DEBOUNCE, function()
			pendingGitRefreshThread = nil
			apiContext
				:getGitMetadata()
				:andThen(function(freshMetadata)
					if cachedServerInfo then
						cachedServerInfo.gitMetadata = freshMetadata
					end
					if self.state.appStatus == AppStatus.Confirming then
						self:setState({
							patchTree = PatchTree.build(
								patch,
								instanceMap,
								{ "Property", "Current", "Incoming" },
								freshMetadata
							),
						})
					end
				end)
				:catch(function(err)
					Log.warn("Failed to refresh git metadata: {}", tostring(err))
				end)
		end)
	end)

	-- One-shot sync: disconnect after initial sync is fully complete (including any writes)
	-- This ensures that "pull" changes are sent to the server before disconnecting
	serveSession:setInitialSyncCompleteCallback(function()
		if Settings:get("oneShotSync") and self.serveSession then
			Log.info("One-shot sync: disconnecting after initial sync complete")
			self:endSession()
		end
	end)

	serveSession:start()

	self.serveSession = serveSession
end

function App:endSession()
	if self.serveSession == nil then
		-- Safety: if the session reference was cleared (e.g., by a Disconnected handler)
		-- but the UI is still stuck on Connected/Connecting, force reset to NotConnected.
		-- This handles race conditions where onStatusChanged(Connected) fires after
		-- onStatusChanged(Disconnected) due to a coroutine resuming on a dead session.
		if
			self.state.appStatus ~= AppStatus.NotConnected
			and self.state.appStatus ~= AppStatus.Error
			and self.state.appStatus ~= AppStatus.Settings
		then
			self:setState({
				appStatus = AppStatus.NotConnected,
				toolbarIcon = Assets.Images.PluginButton,
			})
		end
		return
	end

	Log.trace("Disconnecting session")

	self.serveSession:stop()
	self.serveSession = nil
	self:setState({
		appStatus = AppStatus.NotConnected,
	})

	if self.cleanupPrecommit ~= nil then
		self.cleanupPrecommit()
	end
	if self.cleanupPostcommit ~= nil then
		self.cleanupPostcommit()
	end

	Log.trace("Session terminated by user")
end

function App:startMcpSync(requestId, mode, overrides)
	mode = mode or "standard"
	overrides = overrides or {}

	return Promise.new(function(resolve, _reject)
		if self.state.appStatus == AppStatus.Connected then
			resolve({
				requestId = requestId,
				status = "already_connected",
				changes = {},
				message = "Atlas is already connected in live sync mode. All changes sync automatically.",
			})
			return
		end

		if self.state.appStatus == AppStatus.Connecting or self.state.appStatus == AppStatus.Confirming then
			resolve({
				requestId = requestId,
				status = "sync_in_progress",
				changes = {},
				message = "A sync operation is already in progress.",
			})
			return
		end

		if self.serveSession ~= nil then
			Log.trace("MCP sync: ending existing session before starting")
			self:endSession()
		end

		local host = Config.defaultHost
		local port = Config.defaultPort
		local baseUrl = ("http://%s:%s"):format(host, port)
		local apiContext = ApiContext.new(baseUrl)

		local serveSession = ServeSession.new({
			apiContext = apiContext,
			twoWaySync = false,
		})

		local _mcpPatchTree = nil
		local mcpSyncResolved = false

		local function resolveOnce(result)
			if mcpSyncResolved then
				return
			end
			mcpSyncResolved = true
			result.requestId = requestId
			resolve(result)
		end

		serveSession:setUpdateLoadingTextCallback(function(text: string)
			self:setState({
				connectingText = text,
			})
		end)

		local _cachedServerInfo = nil

		serveSession:setConfirmCallback(function(instanceMap, patch, serverInfo)
			_cachedServerInfo = serverInfo

			local retainedFromIds = table.clone(instanceMap.fromIds)
			self._lastMcpFromIds = retainedFromIds

			PatchSet.removeDataModelName(patch, instanceMap)

			if PatchSet.isEmpty(patch) then
				Log.info("MCP sync: no changes to sync")
				resolveOnce({
					status = "empty",
					changes = {},
				})
				return "Accept"
			end

			local gitMetadata = serverInfo.gitMetadata
			local patchTree = PatchTree.build(patch, instanceMap, { "Property", "Current", "Incoming" }, gitMetadata)
			_mcpPatchTree = patchTree

			-- Apply overrides: look up each override by id in the tree, verify, set selection
			local overrideSelections = {}
			for _, override in overrides do
				local node = patchTree:getNode(override.id)
				if not node or not node.patchType then
					Log.trace("MCP sync: override id {} not found in patch tree, skipping", override.id)
					continue
				end

				local verified = true

				if override.studioHash and node.instance and node.instance:IsA("LuaSourceContainer") then
					local source = node.instance.Source
					local gitBlob = "blob " .. tostring(#source) .. "\0" .. source
					local currentHash = SHA1(buffer.fromstring(gitBlob))
					if currentHash ~= override.studioHash then
						Log.info(
							"MCP sync: override {} studioHash mismatch (expected={}, got={})",
							override.id,
							override.studioHash,
							currentHash
						)
						verified = false
					end
				end

				if verified and override.expectedProperties and node.instance then
					for propName, expectedEncoded in override.expectedProperties do
						local decodeOk, expectedValue = decodeValue(expectedEncoded, instanceMap)
						if not decodeOk then
							Log.info(
								"MCP sync: override {} could not decode expected value for {}",
								override.id,
								propName
							)
							verified = false
							break
						end
						local readOk, currentValue = getProperty(node.instance, propName)
						if not readOk then
							Log.info("MCP sync: override {} could not read property {}", override.id, propName)
							verified = false
							break
						end
						if not trueEquals(expectedValue, currentValue) then
							Log.info("MCP sync: override {} property {} mismatch", override.id, propName)
							verified = false
							break
						end
					end
				end

				if verified then
					overrideSelections[override.id] = override.direction
				end
			end

			-- Compute resolution state: check defaults + overrides
			local allResolved = true
			local hasSelectableNodes = false
			patchTree:forEach(function(node)
				if node.patchType then
					hasSelectableNodes = true
					if node.defaultSelection == nil and overrideSelections[node.id] == nil then
						allResolved = false
					end
				end
			end)

			if not hasSelectableNodes then
				Log.info("MCP sync: no selectable changes, auto-accepting")
				resolveOnce({
					status = "empty",
					changes = {},
				})
				return "Accept"
			end

			-- DRYRUN: return full change list without applying
			if mode == "dryrun" then
				Log.info("MCP sync: dryrun mode, returning changes without applying")
				local changes = self:_buildMcpChangeList(patchTree, patch, instanceMap, nil, true)
				resolveOnce({
					status = "dryrun",
					changes = changes,
				})
				return "Abort"
			end

			-- FASTFAIL: if unresolved changes remain, fail immediately
			if mode == "fastfail" and not allResolved then
				Log.info("MCP sync: fastfail mode, unresolved changes exist")
				local changes = self:_buildMcpChangeList(patchTree, patch, instanceMap, nil, true)
				resolveOnce({
					status = "fastfail_unresolved",
					changes = changes,
					message = "Unresolved changes exist. Provide overrides or use standard mode for user review.",
				})
				return "Abort"
			end

			-- Merge overrides into default selections
			local selections = PatchTree.buildInitialSelections(patchTree)
			for nodeId, direction in overrideSelections do
				selections[nodeId] = direction
			end

			-- Check if all nodes are now resolved (defaults + overrides)
			local allPreSelected = true
			patchTree:forEach(function(node)
				if node.patchType and selections[node.id] == nil then
					allPreSelected = false
				end
			end)

			-- MANUAL: always show UI
			if mode == "manual" then
				allPreSelected = false
			end

			if allPreSelected then
				Log.info("MCP sync: all changes resolved, fast-forwarding")
				local changes = self:_buildMcpChangeList(patchTree, patch, instanceMap, selections, false)

				local autoSelectedIds = {}
				patchTree:forEach(function(node)
					if node.patchType and selections[node.id] then
						autoSelectedIds[node.id] = true
					end
				end)

				resolveOnce({
					status = "success",
					changes = changes,
				})

				return {
					type = "Confirm",
					selections = selections,
					autoSelectedIds = autoSelectedIds,
				}
			end

			Log.info("MCP sync: changes require user review, showing confirmation UI")
			self:setState({
				appStatus = AppStatus.Confirming,
				patchTree = patchTree,
				confirmData = {
					serverInfo = serverInfo,
				},
				toolbarIcon = Assets.Images.PluginButton,
			})

			self:addNotification({
				text = "An AI agent requested a sync. Please review and confirm the changes.",
				timeout = 10,
			})

			local result = self.confirmationEvent:Wait()

			if result == "Abort" then
				local changes = self:_buildMcpChangeList(patchTree, patch, instanceMap, nil, true)
				resolveOnce({
					status = "rejected",
					changes = changes,
					message = "User rejected the sync changes.",
				})
				return result
			end

			if type(result) == "table" and result.type == "Confirm" then
				local changes = self:_buildMcpChangeList(patchTree, patch, instanceMap, result.selections, false)
				resolveOnce({
					status = "success",
					changes = changes,
				})
				return result
			end

			resolveOnce({
				status = "success",
				changes = {},
			})
			return result
		end)

		serveSession:onStatusChanged(function(status, details)
			if status == ServeSession.Status.Connecting then
				self:setState({
					appStatus = AppStatus.Connecting,
					toolbarIcon = Assets.Images.PluginButton,
				})
			elseif status == ServeSession.Status.Connected then
				self:setState({
					appStatus = AppStatus.Connecting,
					connectingText = "Completing sync...",
					toolbarIcon = Assets.Images.PluginButton,
				})
			elseif status == ServeSession.Status.Disconnected then
				self.serveSession = nil
				self:setState({
					appStatus = AppStatus.NotConnected,
					toolbarIcon = Assets.Images.PluginButton,
					patchTree = nil,
				})
				if details ~= nil then
					Log.warn("MCP sync disconnected with error: {}", details)
					resolveOnce({
						status = "error",
						changes = {},
						message = tostring(details),
					})
				end
			end
		end)

		serveSession:setInitialSyncCompleteCallback(function()
			Log.info("MCP sync: initial sync complete, disconnecting")
			if self.serveSession and self.serveSession == serveSession then
				self:endSession()
			end
		end)

		serveSession:start()
		self.serveSession = serveSession
	end)
end

local SCRIPT_CLASS_SET = {
	Script = true,
	LocalScript = true,
	ModuleScript = true,
}

function App:_buildMcpChangeList(patchTree, patch, _instanceMap, selections, includeAll)
	local changes = {}

	local patchUpdatedLookup = nil
	if patch and patch.updated then
		patchUpdatedLookup = {}
		for _, change in patch.updated do
			patchUpdatedLookup[change.id] = change
		end
	end

	patchTree:forEach(function(node)
		if not node.patchType then
			return
		end

		local direction = nil
		if includeAll then
			if selections and selections[node.id] then
				direction = selections[node.id]
			elseif node.defaultSelection then
				direction = node.defaultSelection
			end
		elseif selections then
			local sel = selections[node.id]
			if sel == "push" then
				direction = "push"
			elseif sel == "pull" then
				direction = "pull"
			else
				return
			end
		else
			direction = node.defaultSelection or "push"
		end

		local segments = {}
		local current = node
		while current and current.id ~= "ROOT" do
			table.insert(segments, 1, current.name or current.id)
			if current.parentId then
				current = patchTree:getNode(current.parentId)
			else
				break
			end
		end

		local path = table.concat(segments, "/")

		local entry: any = {
			path = path,
			direction = direction or "unresolved",
			id = node.id,
			className = node.className,
			patchType = node.patchType,
			defaultSelection = node.defaultSelection,
		}

		if node.instance and node.instance:IsA("LuaSourceContainer") then
			local source = node.instance.Source
			local gitBlob = "blob " .. tostring(#source) .. "\0" .. source
			entry.studioHash = SHA1(buffer.fromstring(gitBlob))
		end

		if node.patchType == "Edit" and patchUpdatedLookup and node.instance then
			local patchChange = patchUpdatedLookup[node.id]
			if patchChange and patchChange.changedProperties then
				local props = {}
				local isScript = SCRIPT_CLASS_SET[node.className]

				for propName, incomingEncoded in patchChange.changedProperties do
					if isScript and propName == "Source" then
						continue
					end

					local descriptor = RbxDom.findCanonicalPropertyDescriptor(node.className, propName)
					if not descriptor then
						continue
					end

					local currentOk, currentEncoded = encodeProperty(node.instance, propName, descriptor)
					if not currentOk then
						continue
					end

					props[propName] = {
						current = currentEncoded,
						incoming = incomingEncoded,
					}
				end

				if next(props) then
					entry.properties = props
				end
			end
		end

		table.insert(changes, entry)
	end)

	return changes
end

function App:handleMcpGetScript(requestId, params)
	local ScriptEditorService = game:GetService("ScriptEditorService")

	return Promise.resolve():andThen(function()
		local fromIds = self._lastMcpFromIds
		if not fromIds then
			return {
				requestId = requestId,
				status = "error",
				message = 'No previous sync session. Run atlas_sync(mode: "dryrun") first to establish the instance mapping.',
			}
		end

		local id = params.id
		if not id then
			return {
				requestId = requestId,
				status = "error",
				message = "No id provided. The server should have resolved fsPath to an id.",
			}
		end

		local instance = fromIds[id]
		if not instance then
			return {
				requestId = requestId,
				status = "error",
				message = "Instance not found by id. The sync session may be stale, run atlas_sync again.",
			}
		end
		if instance.Parent == nil then
			return {
				requestId = requestId,
				status = "error",
				message = "Instance was deleted since last sync. Run atlas_sync again.",
			}
		end

		if not instance:IsA("LuaSourceContainer") then
			return {
				requestId = requestId,
				status = "error",
				message = `Instance is not a script (class: {instance.ClassName})`,
			}
		end

		local source
		local isDraft = params.fromDraft == true
		if isDraft then
			local ok, draft = pcall(ScriptEditorService.GetEditorSource, ScriptEditorService, instance)
			if ok then
				source = draft
			else
				source = instance.Source
				isDraft = false
			end
		else
			source = instance.Source
		end

		local gitBlob = "blob " .. tostring(#source) .. "\0" .. source
		local studioHash = SHA1(buffer.fromstring(gitBlob))

		local fullName = instance:GetFullName()
		local instancePath = string.gsub(fullName, "^game%.", "")
		instancePath = string.gsub(instancePath, "%.", "/")

		return {
			requestId = requestId,
			status = "success",
			source = source,
			studioHash = studioHash,
			className = instance.ClassName,
			instancePath = instancePath,
			isDraft = isDraft,
		}
	end)
end

function App:render()
	local pluginName = "Atlas " .. Version.display(Config.version)

	local function createPageElement(appStatus, additionalProps)
		additionalProps = additionalProps or {}

		local props = Dictionary.merge(additionalProps, {
			component = StatusPages[appStatus],
			active = self.state.appStatus == appStatus,
		})

		return e(Page, props)
	end

	return e(StudioPluginContext.Provider, {
		value = self.props.plugin,
	}, {
		e(Theme.StudioProvider, nil, {
			tooltip = e(Tooltip.Provider, nil, {
				gui = e(StudioPluginGui, {
					id = pluginName,
					title = pluginName,
					active = self.state.guiEnabled,
					isEphemeral = false,

					initDockState = Enum.InitialDockState.Right,
					overridePreviousState = false,
					floatingSize = Vector2.new(320, 210),
					minimumSize = Vector2.new(300, 210),

					zIndexBehavior = Enum.ZIndexBehavior.Sibling,

					onInitialState = function(initialState)
						self:setState({
							guiEnabled = initialState,
						})
					end,

					onClose = function()
						self:setState({
							guiEnabled = false,
						})
					end,
				}, {
					Tooltips = e(Tooltip.Container, nil),

					NotConnectedPage = createPageElement(AppStatus.NotConnected, {
						host = self.host,
						onHostChange = self.setHost,
						port = self.port,
						onPortChange = self.setPort,

						onConnect = function()
							self:startSession()
						end,

						onSyncback = function()
							self:setState({
								showingSyncbackConfirm = true,
							})
						end,

						onNavigateSettings = function()
							self.backPage = AppStatus.NotConnected
							self:setState({
								appStatus = AppStatus.Settings,
							})
						end,
					}),

					ConfirmingPage = createPageElement(AppStatus.Confirming, {
						confirmData = self.state.confirmData,
						patchTree = self.state.patchTree,
						changedIds = self.state.changedIds,
						createPopup = not self.state.guiEnabled,

						onAbort = function()
							self.confirmationBindable:Fire("Abort")
						end,
						onConfirm = function(selections)
							local autoSelectedIds = {}
							if self.state.patchTree then
								self.state.patchTree:forEach(function(node)
									if
										node.patchType
										and node.defaultSelection ~= nil
										and selections[node.id] == node.defaultSelection
									then
										autoSelectedIds[node.id] = true
									end
								end)
							end
							self.confirmationBindable:Fire({
								type = "Confirm",
								selections = selections,
								autoSelectedIds = autoSelectedIds,
							})
						end,
					}),

					Connecting = createPageElement(AppStatus.Connecting, {
						text = self.state.connectingText,
					}),

					Connected = createPageElement(AppStatus.Connected, {
						projectName = self.state.projectName,
						address = self.state.address,
						patchTree = self.state.patchTree,
						patchData = self.state.patchData,
						serveSession = self.serveSession,

						onDisconnect = function()
							self:endSession()
						end,

						onNavigateSettings = function()
							self.backPage = AppStatus.Connected
							self:setState({
								appStatus = AppStatus.Settings,
							})
						end,
					}),

					Settings = createPageElement(AppStatus.Settings, {
						syncActive = self.serveSession ~= nil
							and self.serveSession:getStatus() == ServeSession.Status.Connected,

						onBack = function()
							self:setState({
								appStatus = self.backPage or AppStatus.NotConnected,
							})
						end,
					}),

					Error = createPageElement(AppStatus.Error, {
						errorMessage = self.state.errorMessage,

						onClose = function()
							self:setState({
								appStatus = AppStatus.NotConnected,
								toolbarIcon = Assets.Images.PluginButton,
							})
						end,
					}),
				}),

				RojoNotifications = e("ScreenGui", {
					ZIndexBehavior = Enum.ZIndexBehavior.Sibling,
					ResetOnSpawn = false,
					DisplayOrder = 100,
				}, {
					Notifications = e(Notifications, {
						soundPlayer = self.props.soundPlayer,
						notifications = self.state.notifications,
						onClose = function(id)
							self:closeNotification(id)
						end,
					}),
				}),
			}),

			SyncbackConfirm = e(Theme.StudioProvider, nil, {
				e(StudioPluginGui, {
					id = "Atlas_SyncbackConfirm",
					title = "⚠️ Full Syncback (BETA) ⚠️",
					active = self.state.showingSyncbackConfirm == true,
					isEphemeral = true,

					initDockState = Enum.InitialDockState.Float,
					overridePreviousState = true,
					floatingSize = Vector2.new(400, 250),
					minimumSize = Vector2.new(300, 220),

					zIndexBehavior = Enum.ZIndexBehavior.Sibling,

					onClose = function()
						self:setState({ showingSyncbackConfirm = false })
					end,
				}, {
					Content = Theme.with(function(theme)
						local noTransparency = Roact.createBinding(0)

						return e("Frame", {
							Size = UDim2.fromScale(1, 1),
							BackgroundColor3 = theme.BackgroundColor,
							BorderSizePixel = 0,
						}, {
							Padding = e("UIPadding", {
								PaddingLeft = UDim.new(0, 16),
								PaddingRight = UDim.new(0, 16),
								PaddingTop = UDim.new(0, 16),
								PaddingBottom = UDim.new(0, 16),
							}),
							Layout = e("UIListLayout", {
								FillDirection = Enum.FillDirection.Vertical,
								SortOrder = Enum.SortOrder.LayoutOrder,
								Padding = UDim.new(0, 16),
								VerticalAlignment = Enum.VerticalAlignment.Center,
							}),
							Message = e("TextLabel", {
								Text = "This will overwrite your project files with the current Studio state. This cannot be undone.\n\nNote: Live syncback produces slightly different formatting than CLI syncback (atlas syncback). This may cause minor git diffs even when the data is identical. Pick one method and stick with it to avoid unnecessary churn.",
								TextWrapped = true,
								FontFace = theme.Font.Main,
								TextSize = theme.TextSize.Body,
								TextColor3 = theme.TextColor,
								Size = UDim2.new(1, 0, 0, 0),
								AutomaticSize = Enum.AutomaticSize.Y,
								BackgroundTransparency = 1,
								LayoutOrder = 1,
							}),
							Buttons = e("Frame", {
								Size = UDim2.new(1, 0, 0, 34),
								BackgroundTransparency = 1,
								LayoutOrder = 2,
							}, {
								Layout = e("UIListLayout", {
									HorizontalAlignment = Enum.HorizontalAlignment.Right,
									FillDirection = Enum.FillDirection.Horizontal,
									SortOrder = Enum.SortOrder.LayoutOrder,
									Padding = UDim.new(0, 10),
								}),
								Cancel = e(TextButton, {
									text = "Cancel",
									style = "Bordered",
									transparency = noTransparency,
									layoutOrder = 1,
									onClick = function()
										self:setState({ showingSyncbackConfirm = false })
									end,
								}),
								Confirm = e(TextButton, {
									text = "Syncback",
									style = "Danger",
									transparency = noTransparency,
									layoutOrder = 2,
									onClick = function()
										self:performSyncback()
									end,
								}),
							}),
						})
					end),
				}),
			}),

			toggleAction = e(StudioPluginAction, {
				name = "AtlasConnection",
				title = "Atlas: Connect/Disconnect",
				description = "Toggles the server for an Atlas sync session",
				icon = Assets.Images.PluginButton,
				bindable = true,
				onTriggered = function()
					if self.serveSession == nil or self.serveSession:getStatus() == ServeSession.Status.NotStarted then
						self:startSession()
					elseif
						self.serveSession ~= nil and self.serveSession:getStatus() == ServeSession.Status.Connected
					then
						self:endSession()
					end
				end,
			}),

			connectAction = e(StudioPluginAction, {
				name = "AtlasConnect",
				title = "Atlas: Connect",
				description = "Connects the server for an Atlas sync session",
				icon = Assets.Images.PluginButton,
				bindable = true,
				onTriggered = function()
					if self.serveSession == nil or self.serveSession:getStatus() == ServeSession.Status.NotStarted then
						self:startSession()
					end
				end,
			}),

			disconnectAction = e(StudioPluginAction, {
				name = "AtlasDisconnect",
				title = "Atlas: Disconnect",
				description = "Disconnects the server for an Atlas sync session",
				icon = Assets.Images.PluginButton,
				bindable = true,
				onTriggered = function()
					if self.serveSession ~= nil and self.serveSession:getStatus() == ServeSession.Status.Connected then
						self:endSession()
					end
				end,
			}),

			toolbar = e(StudioToolbar, {
				name = pluginName,
			}, {
				button = e(StudioToggleButton, {
					name = "Atlas",
					tooltip = "Show or hide the Atlas panel",
					icon = self.state.toolbarIcon,
					active = self.state.guiEnabled,
					enabled = true,
					onClick = function()
						self:setState(function(state)
							return {
								guiEnabled = not state.guiEnabled,
							}
						end)
					end,
				}),
			}),
		}),
	})
end

return function(props)
	local mergedProps = Dictionary.merge(props, {
		soundPlayer = soundPlayer.new(Settings),
	})

	return e(App, mergedProps)
end
