--[[
	Defines the process for diffing a virtual DOM and the real DOM to compute a
	patch that can be later applied.
]]

local Packages = script.Parent.Parent.Parent.Packages
local Log = require(Packages.Log)

local Config = require(script.Parent.Parent.Config)
local invariant = require(script.Parent.Parent.invariant)
local getProperty = require(script.Parent.getProperty)
local Error = require(script.Parent.Error)
local decodeValue = require(script.Parent.decodeValue)

local trueEquals = require(script.Parent.trueEquals)

local function isEmpty(table)
	return next(table) == nil
end

local function shouldDeleteUnknownInstances(virtualInstance)
	if virtualInstance.Metadata ~= nil then
		return not virtualInstance.Metadata.ignoreUnknownInstances
	else
		return true
	end
end

-- Script class names that should always be eligible for deletion,
-- even when the parent has ignoreUnknownInstances: true (scripts-only mode)
local SCRIPT_CLASS_NAMES = {
	Script = true,
	LocalScript = true,
	ModuleScript = true,
}

local function shouldDeleteChild(virtualInstance, childInstance)
	-- If the parent allows deleting unknown instances, always allow
	if shouldDeleteUnknownInstances(virtualInstance) then
		return true
	end

	-- In scripts-only mode (where parent has ignoreUnknownInstances: true),
	-- we still want to allow deletion of scripts. This ensures deleted
	-- script files show up in the sync diff.
	if SCRIPT_CLASS_NAMES[childInstance.ClassName] then
		return true
	end

	return false
end

local DIFF_YIELD_INTERVAL = 1000

local function diff(instanceMap, virtualInstances, rootId, serverInfo)
	local patch = {
		removed = {},
		added = {},
		updated = {},
	}

	local diffCount = 0

	-- Build a lookup table for visible services (for ignoreHiddenServices check)
	local visibleServicesSet: { [string]: boolean } = {}
	if serverInfo and serverInfo.ignoreHiddenServices and serverInfo.visibleServices then
		for _, serviceName in ipairs(serverInfo.visibleServices) do
			visibleServicesSet[serviceName] = true
		end
	end

	-- Add a virtual instance and all of its descendants to the patch, marked as
	-- being added.
	local function markIdAdded(id)
		local virtualInstance = virtualInstances[id]
		patch.added[id] = virtualInstance

		for _, childId in ipairs(virtualInstance.Children) do
			markIdAdded(childId)
		end
	end

	-- Internal recursive kernel for diffing an instance with the given ID.
	local function diffInternal(id)
		diffCount += 1
		if diffCount % DIFF_YIELD_INTERVAL == 0 then
			task.wait()
		end
		local virtualInstance = virtualInstances[id]
		local instance = instanceMap.fromIds[id]

		if virtualInstance == nil then
			invariant("Cannot diff an instance not present in virtualInstances\nID: {}", id)
		end

		if instance == nil then
			invariant("Cannot diff an instance not present in InstanceMap\nID: {}", id)
		end

		local changedClassName = nil
		if virtualInstance.ClassName ~= instance.ClassName then
			changedClassName = virtualInstance.ClassName
		end

		local changedName = nil
		if virtualInstance.Name ~= instance.Name then
			changedName = virtualInstance.Name
		end

		local changedProperties = {}
		for propertyName, virtualValue in pairs(virtualInstance.Properties) do
			-- Skip CanvasPosition on ScrollingFrame (runtime state, not meaningful for sync)
			if propertyName == "CanvasPosition" and virtualInstance.ClassName == "ScrollingFrame" then
				continue
			end

			local getProperySuccess, existingValueOrErr = getProperty(instance, propertyName)

			if getProperySuccess then
				local existingValue = existingValueOrErr
				local decodeSuccess, decodedValue

				-- If `virtualValue` is a ref then instead of decoding it to an instance,
				-- we change `existingValue` to be a ref. This is because `virtualValue`
				-- may point to an Instance which doesn't exist yet and therefore
				-- decoding it may throw an error.
				if next(virtualValue) == "Ref" then
					decodeSuccess, decodedValue = true, virtualValue

					if existingValue and typeof(existingValue) == "Instance" then
						local existingValueRef = instanceMap.fromInstances[existingValue]
						if existingValueRef then
							existingValue = { Ref = existingValueRef }
						end
					end
				else
					decodeSuccess, decodedValue = decodeValue(virtualValue, instanceMap)
				end

				if decodeSuccess then
					if not trueEquals(existingValue, decodedValue) then
						Log.debug(
							"{}.{} changed from '{}' to '{}'",
							instance:GetFullName(),
							propertyName,
							existingValue,
							decodedValue
						)
						changedProperties[propertyName] = virtualValue
					end
				else
					Log.warn(
						"Failed to decode property {}.{}. Encoded property was: {:#?}",
						virtualInstance.ClassName,
						propertyName,
						virtualValue
					)
				end
			else
				local err = existingValueOrErr

				if err.kind == Error.UnknownProperty then
					Log.trace("Skipping unknown property {}.{}", err.details.className, err.details.propertyName)
				else
					Log.trace("Skipping unreadable property {}.{}", err.details.className, err.details.propertyName)
				end
			end
		end

		if changedName ~= nil or changedClassName ~= nil or not isEmpty(changedProperties) then
			table.insert(patch.updated, {
				id = id,
				changedName = changedName,
				changedClassName = changedClassName,
				changedProperties = changedProperties,
				changedMetadata = nil,
			})
		end

		-- Traverse the list of children in the DOM. Any instance that has no
		-- corresponding virtual instance should be removed. Any instance that
		-- does have a corresponding virtual instance is recursively diffed.
		for _, childInstance in ipairs(instance:GetChildren()) do
			-- Skip auto-created instances that shouldn't be synced
			if Config.ignoredClassNames[childInstance.ClassName] then
				continue
			end

			local childId = instanceMap.fromInstances[childInstance]

			if childId == nil then
				-- pcall to avoid security permission errors
				local success, skip = pcall(function()
					-- We don't remove instances that aren't going to be saved anyway,
					-- such as the Rojo session lock value.
					return childInstance.Archivable == false
				end)
				if success and skip then
					continue
				end

				-- Skip hidden services when ignoreHiddenServices is enabled.
				-- Services are direct children of game (DataModel), and if they're
				-- not in the visible services list, they weren't synced back to disk,
				-- so we shouldn't mark them for deletion during forward sync.
				if serverInfo and serverInfo.ignoreHiddenServices and instance == game then
					if not visibleServicesSet[childInstance.Name] then
						Log.trace("Skipping hidden service {} (ignoreHiddenServices is enabled)", childInstance.Name)
						continue
					end
				end

				-- This is an existing instance not present in the virtual DOM.
				-- We can mark it for deletion unless the user has asked us not
				-- to delete unknown stuff. Scripts are always eligible for deletion
				-- even in scripts-only mode (where ignoreUnknownInstances is true).
				if shouldDeleteChild(virtualInstance, childInstance) then
					table.insert(patch.removed, childInstance)
				end
			else
				local diffSuccess, err = diffInternal(childId)

				if not diffSuccess then
					return false, err
				end
			end
		end

		-- Traverse the list of children in the virtual DOM. Any virtual
		-- instance that has no corresponding real instance should be created.
		for _, childId in ipairs(virtualInstance.Children) do
			local childInstance = instanceMap.fromIds[childId]

			if childInstance == nil then
				-- This instance is present in the virtual DOM, but doesn't
				-- exist in the real DOM.
				markIdAdded(childId)
			end
		end

		return true
	end

	local diffSuccess, err = diffInternal(rootId)

	if not diffSuccess then
		return false, err
	end

	return true, patch
end

return diff
