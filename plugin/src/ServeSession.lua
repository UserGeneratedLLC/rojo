local StudioService = game:GetService("StudioService")
local RunService = game:GetService("RunService")
local ChangeHistoryService = game:GetService("ChangeHistoryService")
local SerializationService = game:GetService("SerializationService")
local Selection = game:GetService("Selection")
local HttpService = game:GetService("HttpService")

local Packages = script.Parent.Parent.Packages
local Log = require(Packages.Log)
local Fmt = require(Packages.Fmt)
local t = require(Packages.t)
local Promise = require(Packages.Promise)
local Timer = require(script.Parent.Timer)

local ChangeBatcher = require(script.Parent.ChangeBatcher)
local encodePatchUpdate = require(script.Parent.ChangeBatcher.encodePatchUpdate)
local encodeInstance = require(script.Parent.ChangeBatcher.encodeInstance)
local InstanceMap = require(script.Parent.InstanceMap)
local PatchSet = require(script.Parent.PatchSet)
local Matching = require(script.Parent.Reconciler.matching)
local Reconciler = require(script.Parent.Reconciler)
local strict = require(script.Parent.strict)
local Settings = require(script.Parent.Settings)

local Status = strict("Session.Status", {
	NotStarted = "NotStarted",
	Connecting = "Connecting",
	Connected = "Connected",
	Disconnected = "Disconnected",
})

local function debugPatch(object)
	return Fmt.debugify(object, function(patch, output)
		output:writeLine("Patch {{")
		output:indent()

		for removed in ipairs(patch.removed) do
			output:writeLine("Remove ID {}", removed)
		end

		for id, added in pairs(patch.added) do
			output:writeLine("Add ID {} {:#?}", id, added)
		end

		for _, updated in ipairs(patch.updated) do
			output:writeLine("Update ID {} {:#?}", updated.id, updated)
		end

		output:unindent()
		output:write("}")
	end)
end

local function attemptReparent(instance, parent)
	return pcall(function()
		instance.Parent = parent
	end)
end

local ServeSession = {}
ServeSession.__index = ServeSession

ServeSession.Status = Status

local validateServeOptions = t.strictInterface({
	apiContext = t.table,
	twoWaySync = t.boolean,
})

function ServeSession.new(options)
	assert(validateServeOptions(options))

	-- Declare self ahead of time to capture it in a closure
	local self
	local function onInstanceChanged(instance, propertyName)
		if not self.__twoWaySync then
			return
		end

		self.__changeBatcher:add(instance, propertyName)
	end

	local function onChangesFlushed(patch)
		-- ONE-SHOT MODE: Block ALL automatic outgoing writes from the ChangeBatcher.
		-- The only writes allowed in One-shot mode are explicit "pull" selections
		-- from the confirmation dialog (handled directly in __confirmAndApplyInitialPatch).
		-- This ensures nothing can be pushed to the server without explicit user action.
		if Settings:get("oneShotSync") then
			Log.info("One-shot mode: blocking automatic outgoing write from ChangeBatcher")
			return
		end

		self.__apiContext:write(patch)
	end

	local instanceMap = InstanceMap.new(onInstanceChanged)
	local changeBatcher = ChangeBatcher.new(instanceMap, onChangesFlushed)
	local reconciler = Reconciler.new(instanceMap)

	local connections = {}

	local connection = StudioService:GetPropertyChangedSignal("ActiveScript"):Connect(function()
		local activeScript = StudioService.ActiveScript

		if activeScript ~= nil then
			self:__onActiveScriptChanged(activeScript)
		end
	end)
	table.insert(connections, connection)

	self = {
		__status = Status.NotStarted,
		__apiContext = options.apiContext,
		__twoWaySync = options.twoWaySync,
		__syncSourceOnly = false,
		__syncScriptsOnly = false,
		__reconciler = reconciler,
		__instanceMap = instanceMap,
		__changeBatcher = changeBatcher,
		__statusChangedCallback = nil,
		__userConfirmCallback = nil,
		__patchUpdateCallback = nil,
		__initialSyncCompleteCallback = nil,
		__serverInfo = nil,
		__confirmingPatch = nil,
		__isConfirming = false, -- Explicit confirmation state flag for defense-in-depth
		__connections = connections,
		__precommitCallbacks = {},
		__postcommitCallbacks = {},
		__updateLoadingText = function() end,
	}

	setmetatable(self, ServeSession)

	return self
end

function ServeSession:__fmtDebug(output)
	output:writeLine("ServeSession {{")
	output:indent()

	output:writeLine("API Context: {:#?}", self.__apiContext)
	output:writeLine("Instances: {:#?}", self.__instanceMap)

	output:unindent()
	output:write("}")
end

function ServeSession:getStatus()
	return self.__status
end

function ServeSession:onStatusChanged(callback)
	self.__statusChangedCallback = callback
end

function ServeSession:setConfirmCallback(callback)
	self.__userConfirmCallback = callback
end

function ServeSession:setPatchUpdateCallback(callback)
	self.__patchUpdateCallback = callback
end

function ServeSession:setInitialSyncCompleteCallback(callback)
	self.__initialSyncCompleteCallback = callback
end

function ServeSession:setUpdateLoadingTextCallback(callback)
	self.__updateLoadingText = callback
end

function ServeSession:setLoadingText(text: string)
	self.__updateLoadingText(text)
end

--[=[
	Hooks a function to run before patch application.
	The provided function is called with the incoming patch and an InstanceMap
	as parameters.
]=]
function ServeSession:hookPrecommit(callback)
	table.insert(self.__precommitCallbacks, callback)
	Log.trace("Added precommit callback: {}", callback)

	return function()
		-- Remove the callback from the list
		for i, cb in self.__precommitCallbacks do
			if cb == callback then
				table.remove(self.__precommitCallbacks, i)
				Log.trace("Removed precommit callback: {}", callback)
				break
			end
		end
	end
end

--[=[
	Hooks a function to run after patch application.
	The provided function is called with the applied patch, the current
	InstanceMap, and a PatchSet containing any unapplied changes.
]=]
function ServeSession:hookPostcommit(callback)
	table.insert(self.__postcommitCallbacks, callback)
	Log.trace("Added postcommit callback: {}", callback)

	return function()
		-- Remove the callback from the list
		for i, cb in self.__postcommitCallbacks do
			if cb == callback then
				table.remove(self.__postcommitCallbacks, i)
				Log.trace("Removed postcommit callback: {}", callback)
				break
			end
		end
	end
end

function ServeSession:__onWebSocketMessage(messagesPacket)
	if self.__status == Status.Disconnected then
		return
	end

	Log.info("Received {} messages from Rojo server", #messagesPacket.messages)

	-- Combine all messages into a single patch
	local combinedPatch = PatchSet.newEmpty()
	for _, message in messagesPacket.messages do
		PatchSet.assign(combinedPatch, message)
	end

	local addedCount = 0
	for _ in pairs(combinedPatch.added) do
		addedCount += 1
	end
	Log.info(
		"Combined patch: {} removals, {} additions, {} updates",
		#combinedPatch.removed,
		addedCount,
		#combinedPatch.updated
	)

	-- If we're already waiting for confirmation, merge into the patch being confirmed
	-- Changes flow into the dialogue even in one-shot mode so user can see them
	if self.__confirmingPatch ~= nil then
		Log.info("Already confirming, merging into current patch")

		-- Collect IDs of items that are being changed (for unselecting in UI)
		local changedIds = {}
		for _, update in ipairs(combinedPatch.updated) do
			changedIds[update.id] = true
		end
		for _, removed in ipairs(combinedPatch.removed) do
			local id = if type(removed) == "string" then removed else self.__instanceMap.fromInstances[removed]
			if id then
				changedIds[id] = true
			end
		end
		for id in pairs(combinedPatch.added) do
			changedIds[id] = true
		end

		PatchSet.merge(self.__confirmingPatch, combinedPatch, self.__instanceMap)
		-- Notify the UI to update the displayed patch and unselect changed items
		if self.__patchUpdateCallback ~= nil then
			self.__patchUpdateCallback(self.__instanceMap, self.__confirmingPatch, changedIds)
		end
		self.__apiContext:setMessageCursor(messagesPacket.messageCursor)
		return
	end

	-- Set confirmation state flags before spawning to prevent race condition
	-- where another message arrives before the spawned thread starts
	self.__isConfirming = true
	self.__confirmingPatch = combinedPatch

	-- Pause ChangeBatcher during confirmation to prevent outgoing changes
	self.__changeBatcher:pause()

	-- Spawn a new thread to handle potentially yielding confirmation
	task.spawn(function()
		Log.trace("Processing WebSocket patch, callback exists: {}", self.__userConfirmCallback ~= nil)
		Log.trace("ServerInfo exists: {}", self.__serverInfo ~= nil)

		local userDecision = "Accept"
		if self.__userConfirmCallback ~= nil then
			userDecision = self.__userConfirmCallback(self.__instanceMap, combinedPatch, self.__serverInfo)
		end

		-- Clear confirmation state flags
		self.__confirmingPatch = nil
		self.__isConfirming = false

		-- If the session was disconnected during the confirmation yield
		-- (e.g., WebSocket error, server crash), bail out immediately.
		if self.__status == Status.Disconnected then
			return
		end

		-- Resume ChangeBatcher after confirmation
		self.__changeBatcher:resume()

		Log.trace("WebSocket patch decision: {}", userDecision)

		if userDecision == "Abort" then
			self:__stopInternal("Aborted Atlas sync operation")
			return
		elseif userDecision == "Accept" then
			-- combinedPatch may have been updated with additional changes
			-- that arrived while we were waiting for confirmation
			self:__applyPatch(combinedPatch)
		end
		-- Note: Table-type responses ({ type = "Confirm", selections = ... }) from
		-- the selection-based UI are not yet handled here. Ongoing WebSocket patches
		-- with per-item selections would need the same logic as
		-- __confirmAndApplyInitialPatch to properly respect push/pull/ignore choices.
	end)

	self.__apiContext:setMessageCursor(messagesPacket.messageCursor)
end

function ServeSession:start()
	self:__setStatus(Status.Connecting)
	self:setLoadingText("Connecting to server...")

	self.__apiContext
		:connect()
		:andThen(function(serverInfo)
			self:setLoadingText("Loading initial data from server...")

			-- Configure sync mode based on server capabilities
			self.__syncSourceOnly = serverInfo.syncSourceOnly or false
			self.__changeBatcher:setSyncSourceOnly(self.__syncSourceOnly)

			self.__syncScriptsOnly = serverInfo.syncScriptsOnly or false
			self.__changeBatcher:setSyncScriptsOnly(self.__syncScriptsOnly)

			self.__serverInfo = serverInfo
			return self:__computeInitialPatch(serverInfo):andThen(function(catchUpPatch)
				self:setLoadingText("Starting sync loop...")

				-- Connect WebSocket BEFORE showing confirmation so changes
				-- during confirmation can be merged into the patch
				-- Note: connectWebSocket returns a Promise that only resolves when
				-- the connection closes, so we don't chain .andThen() on it
				self.__apiContext
					:connectWebSocket({
						["messages"] = function(messagesPacket)
							self:__onWebSocketMessage(messagesPacket)
						end,
					})
					:catch(function(err)
						if self.__status ~= Status.Disconnected then
							self:__stopInternal(err)
						end
					end)

				-- Now show confirmation and wait for user decision
				return self:__confirmAndApplyInitialPatch(catchUpPatch, serverInfo)
			end)
		end)
		:andThen(function()
			-- Initial sync is fully complete (including any writes to the server)
			if self.__initialSyncCompleteCallback ~= nil then
				self.__initialSyncCompleteCallback()
			end

			self:__connectScriptsOnlyWatchers()
		end)
		:catch(function(err)
			if self.__status ~= Status.Disconnected then
				self:__stopInternal(err)
			end
		end)
end

function ServeSession:stop()
	self:__stopInternal()
end

function ServeSession:__applyGameAndPlaceId(serverInfo)
	if serverInfo.gameId ~= nil then
		game:SetUniverseId(serverInfo.gameId)
	end

	if serverInfo.placeId ~= nil then
		game:SetPlaceId(serverInfo.placeId)
	end
end

function ServeSession:__onActiveScriptChanged(activeScript)
	if not Settings:get("openScriptsExternally") then
		Log.trace("Not opening script {} because feature not enabled.", activeScript)

		return
	end

	if self.__status ~= Status.Connected then
		Log.trace("Not opening script {} because session is not connected.", activeScript)

		return
	end

	local scriptId = self.__instanceMap.fromInstances[activeScript]
	if scriptId == nil then
		Log.trace("Not opening script {} because it is not known by Rojo.", activeScript)

		return
	end

	Log.debug("Trying to open script {} externally...", activeScript)

	-- Force-close the script inside Studio... with a small delay in the middle
	-- to prevent Studio from crashing.
	spawn(function()
		local existingParent = activeScript.Parent
		activeScript.Parent = nil

		for _ = 1, 3 do
			RunService.Heartbeat:Wait()
		end

		activeScript.Parent = existingParent
	end)

	-- Notify the Rojo server to open this script
	self.__apiContext:open(scriptId)
end

function ServeSession:__replaceInstances(idList)
	if #idList == 0 then
		return true, PatchSet.newEmpty()
	end
	-- It would be annoying if selection went away, so we try to preserve it.
	local selection = Selection:Get()
	local selectionMap = {}
	for i, instance in selection do
		selectionMap[instance] = i
	end

	-- TODO: Should we do this in multiple requests so we can more granularly mark failures?
	local modelSuccess, replacements = self.__apiContext
		:serialize(idList)
		:andThen(function(response)
			Log.debug("Deserializing results from serialize endpoint")
			local objects = SerializationService:DeserializeInstancesAsync(response.modelContents)
			if not objects[1] then
				return Promise.reject("Serialize endpoint did not deserialize into any Instances")
			end
			if #objects[1]:GetChildren() ~= #idList then
				return Promise.reject("Serialize endpoint did not return the correct number of Instances")
			end

			local instanceMap = {}
			for _, item in objects[1]:GetChildren() do
				instanceMap[item.Name] = item.Value
			end
			return instanceMap
		end)
		:await()

	local refSuccess, refPatch = self.__apiContext
		:refPatch(idList)
		:andThen(function(response)
			return response.patch
		end)
		:await()

	if not (modelSuccess and refSuccess) then
		return false
	end

	for id, replacement in replacements do
		local oldInstance = self.__instanceMap.fromIds[id]
		if not oldInstance then
			-- TODO: Why would this happen?
			Log.warn("Instance {} not found in InstanceMap during sync replacement", id)
			continue
		end

		self.__instanceMap:insert(id, replacement)
		Log.trace("Swapping Instance {} out via api/models/ endpoint", id)
		local oldParent = oldInstance.Parent
		for _, child in oldInstance:GetChildren() do
			-- Some children cannot be reparented, such as a TouchTransmitter
			local reparentSuccess, reparentError = attemptReparent(child, replacement)
			if not reparentSuccess then
				Log.warn(
					"Could not reparent child {} of instance {} during sync replacement: {}",
					child.Name,
					oldInstance.Name,
					reparentError
				)
			end
		end

		-- ChangeHistoryService doesn't like it if an Instance has been
		-- Destroyed. So, we have to accept the potential memory hit and
		-- just set the parent to `nil`.
		local deleteSuccess, deleteError = attemptReparent(oldInstance, nil)
		local replaceSuccess, replaceError = attemptReparent(replacement, oldParent)

		if not (deleteSuccess and replaceSuccess) then
			Log.warn(
				"Could not swap instances {} and {} during sync replacement: {}",
				oldInstance.Name,
				replacement.Name,
				(deleteError or "") .. "\n" .. (replaceError or "")
			)

			-- We need to revert the failed swap to avoid losing the old instance and children.
			for _, child in replacement:GetChildren() do
				attemptReparent(child, oldInstance)
			end
			attemptReparent(oldInstance, oldParent)

			-- Our replacement should never have existed in the first place, so we can just destroy it.
			replacement:Destroy()
			continue
		end

		if selectionMap[oldInstance] then
			-- This is a bit funky, but it saves the order of Selection
			-- which might matter for some use cases.
			selection[selectionMap[oldInstance]] = replacement
		end
	end

	local patchApplySuccess, unappliedPatch = pcall(self.__reconciler.applyPatch, self.__reconciler, refPatch)
	if patchApplySuccess then
		Selection:Set(selection)
		return true, unappliedPatch
	else
		error(unappliedPatch)
	end
end

function ServeSession:__applyPatch(patch)
	-- Defense-in-depth: Prevent unexpected patch application during confirmation
	-- The only valid case is applying the confirmingPatch itself after user approval
	if self.__isConfirming and patch ~= self.__confirmingPatch then
		Log.error("Attempted to apply unexpected patch while confirmation dialogue is open. This is a bug!")
		return
	end

	local patchTimestamp = DateTime.now():FormatLocalTime("LTS", "en-us")
	local historyRecording = ChangeHistoryService:TryBeginRecording("Atlas: Patch " .. patchTimestamp)
	if not historyRecording then
		-- There can only be one recording at a time
		Log.debug("Failed to begin history recording for " .. patchTimestamp .. ". Another recording is in progress.")
	end

	Timer.start("precommitCallbacks")
	-- Precommit callbacks must be serial in order to obey the contract that
	-- they execute before commit
	for _, callback in self.__precommitCallbacks do
		local success, err = pcall(callback, patch, self.__instanceMap)
		if not success then
			Log.warn("Precommit hook errored: {}", err)
		end
	end
	Timer.stop()

	local patchApplySuccess, unappliedPatch = pcall(self.__reconciler.applyPatch, self.__reconciler, patch)
	if not patchApplySuccess then
		if historyRecording then
			ChangeHistoryService:FinishRecording(historyRecording, Enum.FinishRecordingOperation.Commit)
		end
		-- This might make a weird stack trace but the only way applyPatch can
		-- fail is if a bug occurs so it's probably fine.
		error(unappliedPatch)
	end

	if Settings:get("enableSyncFallback") and not PatchSet.isEmpty(unappliedPatch) then
		-- Some changes did not apply, let's try replacing them instead
		local addedIdList = PatchSet.addedIdList(unappliedPatch)
		local updatedIdList = PatchSet.updatedIdList(unappliedPatch)

		Log.debug("ServeSession:__replaceInstances(unappliedPatch.added)")
		Timer.start("ServeSession:__replaceInstances(unappliedPatch.added)")
		local addSuccess, unappliedAddedRefs = self:__replaceInstances(addedIdList)
		Timer.stop()

		Log.debug("ServeSession:__replaceInstances(unappliedPatch.updated)")
		Timer.start("ServeSession:__replaceInstances(unappliedPatch.updated)")
		local updateSuccess, unappliedUpdateRefs = self:__replaceInstances(updatedIdList)
		Timer.stop()

		-- Update the unapplied patch to reflect which Instances were replaced successfully
		if addSuccess then
			table.clear(unappliedPatch.added)
			PatchSet.assign(unappliedPatch, unappliedAddedRefs)
		end
		if updateSuccess then
			table.clear(unappliedPatch.updated)
			PatchSet.assign(unappliedPatch, unappliedUpdateRefs)
		end
	end

	if not PatchSet.isEmpty(unappliedPatch) then
		Log.info(
			"Could not apply all changes requested by the Rojo server:\n{}",
			PatchSet.humanSummary(self.__instanceMap, unappliedPatch)
		)
	end

	Timer.start("postcommitCallbacks")
	-- Postcommit callbacks can be called with spawn since regardless of firing order, they are
	-- guaranteed to be called after the commit
	for _, callback in self.__postcommitCallbacks do
		task.spawn(function()
			local success, err = pcall(callback, patch, self.__instanceMap, unappliedPatch)
			if not success then
				Log.warn("Postcommit hook errored: {}", err)
			end
		end)
	end
	Timer.stop()

	if historyRecording then
		ChangeHistoryService:FinishRecording(historyRecording, Enum.FinishRecordingOperation.Commit)
	end
end

function ServeSession:__computeInitialPatch(serverInfo)
	return self.__apiContext:read({ serverInfo.rootInstanceId }):andThen(function(readResponseBody)
		-- Tell the API Context that we're up-to-date with the version of
		-- the tree defined in this response.
		self.__apiContext:setMessageCursor(readResponseBody.messageCursor)

		-- For any instances that line up with the Rojo server's view, start
		-- tracking them in the reconciler.
		Log.trace("Matching existing Roblox instances to Rojo IDs")
		self:setLoadingText("Hydrating instance map...")
		local matchingSession = Matching.newSession()
		self.__reconciler:hydrate(readResponseBody.instances, serverInfo.rootInstanceId, game, matchingSession)

		-- Calculate the initial patch to apply to the DataModel to catch us
		-- up to what Rojo thinks the place should look like.
		Log.trace("Computing changes that plugin needs to make to catch up to server...")
		self:setLoadingText("Finding differences between server and Studio...")
		local success, catchUpPatch =
			self.__reconciler:diff(readResponseBody.instances, serverInfo.rootInstanceId, serverInfo)

		if not success then
			Log.error("Could not compute a diff to catch up to the Rojo server: {:#?}", catchUpPatch)
		end

		for _, update in catchUpPatch.updated do
			if update.id == self.__instanceMap.fromInstances[game] and update.changedClassName ~= nil then
				-- Non-place projects will try to update the classname of game from DataModel to
				-- something like Folder, ModuleScript, etc. This would fail, so we exit with a clear
				-- message instead of crashing.
				return Promise.reject(
					"Cannot sync a model as a place."
						.. "\nEnsure Rojo is serving a project file that has a DataModel at the root of its tree and try again."
						.. "\nSee project file docs: https://rojo.space/docs/v7/project-format/"
				)
			end
		end

		Log.trace("Computed hydration patch: {:#?}", debugPatch(catchUpPatch))

		return catchUpPatch
	end)
end

function ServeSession:__confirmAndApplyInitialPatch(catchUpPatch, serverInfo)
	local userDecision = "Accept"
	if self.__userConfirmCallback ~= nil then
		-- Pause ChangeBatcher during confirmation to prevent change accumulation
		self.__changeBatcher:pause()

		-- Set confirmation state flags
		self.__isConfirming = true
		self.__confirmingPatch = catchUpPatch

		userDecision = self.__userConfirmCallback(self.__instanceMap, catchUpPatch, serverInfo)

		-- Clear confirmation state flags
		self.__confirmingPatch = nil
		self.__isConfirming = false

		-- If the session was disconnected during the confirmation yield
		-- (e.g., WebSocket error, server crash), bail out immediately.
		-- Don't resume batcher or apply patches on a dead session.
		if self.__status == Status.Disconnected then
			return Promise.reject("Session disconnected during confirmation")
		end

		-- Resume ChangeBatcher after confirmation
		self.__changeBatcher:resume()
	end

	if userDecision == "Abort" then
		return Promise.reject("Aborted Rojo sync operation")
	elseif userDecision == "Accept" then
		-- Legacy: Accept all changes (push all to Studio)
		self:__setStatus(Status.Connected, serverInfo.projectName)
		self:__applyGameAndPlaceId(serverInfo)
		self:__applyPatch(catchUpPatch)
		return Promise.resolve()
	elseif type(userDecision) == "table" and userDecision.type == "Confirm" then
		-- New: Apply based on per-item selections
		local selections = userDecision.selections or {}
		local autoSelectedIds = userDecision.autoSelectedIds or {}

		-- Log selection summary (debug level)
		local pushCount, pullCount, ignoreCount = 0, 0, 0
		for _, selection in pairs(selections) do
			if selection == "push" then
				pushCount += 1
			elseif selection == "pull" then
				pullCount += 1
			else
				ignoreCount += 1
			end
		end
		Log.info("User selections: {} push, {} pull, {} ignored", pushCount, pullCount, ignoreCount)

		-- Build partial patches based on selections
		local pushPatch = PatchSet.newEmpty() -- Items to apply to Studio
		local pullPatch = PatchSet.newEmpty() -- Items to send back to Rojo

		-- Build stage_ids: all push-accepted IDs + auto-selected pull-accepted IDs
		local stageIds = {}

		-- Process updated items
		for _, change in catchUpPatch.updated do
			local selection = selections[change.id]
			local instance = self.__instanceMap.fromIds[change.id]
			local instancePath = if instance then instance:GetFullName() else "ID:" .. tostring(change.id)

			if selection == "push" then
				-- Apply Rojo changes to Studio
				Log.info("[Push] Update: {}", instancePath)
				table.insert(pushPatch.updated, change)
				table.insert(stageIds, change.id)
			elseif selection == "pull" and self.__twoWaySync then
				-- Send Studio state back to Rojo
				if instance then
					local propertiesToSync = change.changedProperties
					if self.__syncSourceOnly then
						if propertiesToSync.Source then
							propertiesToSync = { Source = propertiesToSync.Source }
						else
							continue
						end
					end
					Log.info("[Pull] Update: {}", instancePath)
					local update = encodePatchUpdate(
						instance,
						change.id,
						propertiesToSync,
						self.__instanceMap,
						function(sourceInstance, propertyName, targetInstance)
							-- Defer unresolved Refs for retry via ChangeBatcher.
							-- After the pull patch is sent, the target may appear
							-- in the InstanceMap via forward-sync reconciliation.
							self.__changeBatcher:deferUnresolvedRef(sourceInstance, propertyName, targetInstance)
						end
					)
					if update then
						table.insert(pullPatch.updated, update)
						if autoSelectedIds[change.id] then
							table.insert(stageIds, change.id)
						end
					end
				end
			end
			-- "ignore" items are skipped
		end

		-- Process removed items (instances in Studio that don't exist in Rojo)
		for _, idOrInstance in catchUpPatch.removed do
			-- For removed items, idOrInstance is the Studio Instance
			-- Note: use typeof() for Roblox Instances, not type() which returns "userdata"
			local instance = if typeof(idOrInstance) == "Instance"
				then idOrInstance
				else self.__instanceMap.fromIds[idOrInstance]

			-- Try to get an ID for selection lookup
			local id = if typeof(idOrInstance) == "string"
				then idOrInstance
				else self.__instanceMap.fromInstances[idOrInstance]

			-- If we don't have an ID, use the instance itself as the key
			local selection = if id then selections[id] else selections[idOrInstance]

			local instancePath = if instance then instance:GetFullName() else "ID:" .. tostring(idOrInstance)
			local instanceClass = if instance then instance.ClassName else "unknown"

			if selection == "push" then
				-- Apply Rojo removal to Studio (delete the instance)
				Log.info("[Push] Delete: {}", instancePath)
				table.insert(pushPatch.removed, idOrInstance)
				if id then
					table.insert(stageIds, id)
				end
			elseif selection == "pull" and self.__twoWaySync then
				-- Syncback: Create file in Rojo from Studio instance
				if instance and instance.Parent then
					local parentId = self.__instanceMap.fromInstances[instance.Parent]
					if parentId then
						local encoded = encodeInstance(instance, parentId)
						if encoded then
							-- Generate a temporary ref for the new instance
							-- Use HttpService:GenerateGUID() and strip dashes to get 32-char hex
							local guid = HttpService:GenerateGUID(false)
							local tempRef = string.gsub(guid, "-", ""):lower()
							pullPatch.added[tempRef] = encoded
							if id and autoSelectedIds[id] then
								table.insert(stageIds, tempRef)
							end
							-- Note: We intentionally do NOT pre-insert into InstanceMap here.
							-- The VFS watcher will process the written files and assign a
							-- server-side Ref ID. Any Ref properties targeting this instance
							-- will be deferred by the ChangeBatcher and resolved once the
							-- server ID appears in the InstanceMap via forward-sync.
							-- Log at info level since this is a major file creation operation
							Log.info("[Pull] Create file: {} ({})", instancePath, instanceClass)
						end
					else
						Log.warn("Cannot syncback {:?}: parent not in Rojo tree", instance)
					end
				end
			end
			-- "skip"/"ignore" items are skipped
		end

		-- Process added items
		for id, change in catchUpPatch.added do
			local selection = selections[id]
			local instanceName = change.Name or "unknown"
			local instanceClass = change.ClassName or "unknown"
			local parentInstance = self.__instanceMap.fromIds[change.Parent]
			local parentPath = if parentInstance then parentInstance:GetFullName() else "ID:" .. tostring(change.Parent)

			if selection == "push" then
				-- Apply Rojo addition to Studio
				Log.info("[Push] Add: {}.{}", parentPath, instanceName)
				pushPatch.added[id] = change
				table.insert(stageIds, id)
			elseif selection == "pull" and self.__twoWaySync then
				-- Don't add in Studio, remove from Rojo
				-- Log at info level since this is a major file deletion operation
				Log.info("[Pull] Delete file: {}.{} ({})", parentPath, instanceName, instanceClass)
				table.insert(pullPatch.removed, id)
			end
			-- "ignore" items are skipped
		end

		-- Apply the changes
		self:__setStatus(Status.Connected, serverInfo.projectName)
		self:__applyGameAndPlaceId(serverInfo)

		-- Apply push items to Studio
		if not PatchSet.isEmpty(pushPatch) then
			local addCount = 0
			for _ in pairs(pushPatch.added) do
				addCount += 1
			end
			Log.info(
				"Applying to Studio: {} additions, {} removals, {} updates",
				addCount,
				#pushPatch.removed,
				#pushPatch.updated
			)
			self:__applyPatch(pushPatch)
		end

		-- Send pull items + stage requests to Rojo server
		local hasPullChanges = self.__twoWaySync and not PatchSet.isEmpty(pullPatch)
		local hasStageIds = #stageIds > 0

		if hasPullChanges or hasStageIds then
			if hasPullChanges then
				local addCount = 0
				for _ in pairs(pullPatch.added) do
					addCount += 1
				end
				Log.info(
					"Sending to Rojo: {} file creations, {} file deletions, {} updates",
					addCount,
					#pullPatch.removed,
					#pullPatch.updated
				)
			end
			if hasStageIds then
				Log.info("Requesting git stage for {} files", #stageIds)
			end
			return self.__apiContext:write(pullPatch, stageIds)
		end

		return Promise.resolve()
	else
		return Promise.reject("Invalid user decision: " .. tostring(userDecision))
	end
end

function ServeSession:__connectScriptsOnlyWatchers()
	if not self.__syncScriptsOnly then
		return
	end
	if not self.__twoWaySync then
		return
	end

	for _, instance in self.__instanceMap.fromIds do
		if instance.Parent ~= game then
			continue
		end

		local conn = instance.DescendantAdded:Connect(function(descendant)
			if not self.__twoWaySync then
				return
			end
			if self.__status ~= Status.Connected then
				return
			end
			if not descendant:IsA("LuaSourceContainer") then
				return
			end
			if self.__instanceMap.fromInstances[descendant] then
				return
			end

			local chain = {}
			local current = descendant.Parent
			while current and current ~= game do
				if self.__instanceMap.fromInstances[current] then
					break
				end
				table.insert(chain, 1, current)
				current = current.Parent
			end

			local mappedParent = current
			if not mappedParent then
				return
			end
			local mappedParentId = self.__instanceMap.fromInstances[mappedParent]
			if not mappedParentId then
				return
			end

			local patch = PatchSet.newEmpty()
			local prevParentId = mappedParentId

			for _, intermediate in chain do
				local tempId = string.gsub(HttpService:GenerateGUID(false), "-", ""):lower()
				patch.added[tempId] = {
					parent = prevParentId,
					name = intermediate.Name,
					className = intermediate.ClassName,
					properties = {},
					children = {},
				}
				self.__instanceMap:insert(tempId, intermediate)
				prevParentId = tempId
			end

			local scriptTempId = string.gsub(HttpService:GenerateGUID(false), "-", ""):lower()
			patch.added[scriptTempId] = encodeInstance(descendant, prevParentId)
			self.__instanceMap:insert(scriptTempId, descendant)

			Log.info(
				"Scripts-only: detected new script {} outside mapped tree, sending to server",
				descendant:GetFullName()
			)
			self.__apiContext:write(patch)
		end)
		table.insert(self.__connections, conn)
	end
end

function ServeSession:__stopInternal(err)
	self:__setStatus(Status.Disconnected, err)
	self.__apiContext:disconnect()
	self.__instanceMap:stop()
	self.__changeBatcher:stop()

	for _, connection in ipairs(self.__connections) do
		connection:Disconnect()
	end
	self.__connections = {}
end

function ServeSession:__setStatus(status, detail)
	self.__status = status

	if self.__statusChangedCallback ~= nil then
		self.__statusChangedCallback(status, detail)
	end
end

return ServeSession
