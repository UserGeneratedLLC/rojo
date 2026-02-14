--[[
	The ChangeBatcher is responsible for collecting and dispatching changes made
	to tracked instances during two-way sync.
]]

local RunService = game:GetService("RunService")

local Packages = script.Parent.Parent.Packages
local Log = require(Packages.Log)

local PatchSet = require(script.Parent.PatchSet)

local createPatchSet = require(script.createPatchSet)

local ChangeBatcher = {}
ChangeBatcher.__index = ChangeBatcher

local BATCH_INTERVAL = 0.2

function ChangeBatcher.new(instanceMap, onChangesFlushed)
	local self

	local renderSteppedConnection = RunService.RenderStepped:Connect(function(dt)
		self:__cycle(dt)
	end)

	self = setmetatable({
		__accumulator = 0,
		__renderSteppedConnection = renderSteppedConnection,
		__instanceMap = instanceMap,
		__onChangesFlushed = onChangesFlushed,
		__pendingPropertyChanges = {},
		__syncSourceOnly = false,
		__paused = false,

		-- Map of target Instance -> list of {sourceInstance, propertyName}
		-- for Ref properties that couldn't be encoded because the target
		-- wasn't in the InstanceMap yet. When the target appears (via
		-- InstanceMap.onInstanceInserted), the property changes are
		-- re-queued into __pendingPropertyChanges.
		__pendingUnresolvedRefs = {},
	}, ChangeBatcher)

	-- Wire up the InstanceMap callback to resolve pending Ref properties
	-- when new instances are tracked.
	instanceMap.onInstanceInserted = function(instance)
		self:__onTargetInstanceInserted(instance)
	end

	return self
end

function ChangeBatcher:setSyncSourceOnly(enabled)
	self.__syncSourceOnly = enabled
end

-- Pause the batcher to prevent change accumulation during confirmation
function ChangeBatcher:pause()
	self.__paused = true
end

-- Resume the batcher after confirmation
function ChangeBatcher:resume()
	self.__paused = false
end

-- Check if the batcher is paused
function ChangeBatcher:isPaused()
	return self.__paused
end

function ChangeBatcher:stop()
	self.__renderSteppedConnection:Disconnect()
	self.__pendingPropertyChanges = {}
	self.__pendingUnresolvedRefs = {}
	self.__instanceMap.onInstanceInserted = nil
end

--- Register a Ref property that couldn't be encoded because the target
--- Instance isn't in the InstanceMap yet. When the target appears, the
--- property change will be automatically re-queued.
function ChangeBatcher:deferUnresolvedRef(sourceInstance, propertyName, targetInstance)
	local pending = self.__pendingUnresolvedRefs[targetInstance]
	if not pending then
		pending = {}
		self.__pendingUnresolvedRefs[targetInstance] = pending
	end
	table.insert(pending, { sourceInstance = sourceInstance, propertyName = propertyName })
	Log.debug(
		"ChangeBatcher: deferred Ref {:?}.{} until target {:?} is tracked",
		sourceInstance,
		propertyName,
		targetInstance
	)
end

--- Called when a new instance is inserted into the InstanceMap. Checks if
--- any pending unresolved Ref properties targeted this instance and
--- re-queues them for the next batch cycle.
function ChangeBatcher:__onTargetInstanceInserted(instance)
	local pending = self.__pendingUnresolvedRefs[instance]
	if not pending then
		return
	end

	self.__pendingUnresolvedRefs[instance] = nil

	for _, entry in pending do
		local sourceInstance = entry.sourceInstance
		local propertyName = entry.propertyName

		-- Verify the source instance is still valid and tracked
		if sourceInstance.Parent and self.__instanceMap.fromInstances[sourceInstance] then
			Log.info("ChangeBatcher: resolving deferred Ref {:?}.{} -> {:?}", sourceInstance, propertyName, instance)
			self:add(sourceInstance, propertyName)
		end
	end
end

function ChangeBatcher:add(instance, propertyName)
	local properties = self.__pendingPropertyChanges[instance]

	if not properties then
		properties = {}
		self.__pendingPropertyChanges[instance] = properties
		Log.trace("ChangeBatcher: tracking changes to {}", instance:GetFullName())
	end

	Log.trace("ChangeBatcher: property '{}' changed on {}", propertyName, instance:GetFullName())
	properties[propertyName] = true
end

function ChangeBatcher:__cycle(dt)
	-- Skip processing when paused (during confirmation dialogue)
	if self.__paused then
		return
	end

	self.__accumulator += dt

	if self.__accumulator >= BATCH_INTERVAL then
		self.__accumulator -= BATCH_INTERVAL

		local patch = self:__flush()

		if patch then
			self.__onChangesFlushed(patch)
		end
	end

	self.__instanceMap:unpauseAllInstances()
end

function ChangeBatcher:__flush()
	if next(self.__pendingPropertyChanges) == nil then
		return nil
	end

	local patch = createPatchSet(
		self.__instanceMap,
		self.__pendingPropertyChanges,
		self.__syncSourceOnly,
		function(sourceInstance, propertyName, targetInstance)
			self:deferUnresolvedRef(sourceInstance, propertyName, targetInstance)
		end
	)
	self.__pendingPropertyChanges = {}

	if PatchSet.isEmpty(patch) then
		return nil
	end

	local addedCount = 0
	for _ in pairs(patch.added) do
		addedCount += 1
	end
	Log.info("Two-way sync: {} updates, {} additions, {} removals", #patch.updated, addedCount, #patch.removed)

	return patch
end

return ChangeBatcher
