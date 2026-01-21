--[[
	Defines the process for diffing a virtual DOM and the real DOM to compute a
	patch that can be later applied.
]]

local Packages = script.Parent.Parent.Parent.Packages
local Log = require(Packages.Log)

local invariant = require(script.Parent.Parent.invariant)
local getProperty = require(script.Parent.getProperty)
local Error = require(script.Parent.Error)
local decodeValue = require(script.Parent.decodeValue)

local function isEmpty(table)
	return next(table) == nil
end

-- Finds duplicate names among children and returns a set of names that are duplicated
local function findDuplicateNames(children: { Instance } | { string }, virtualInstances: { [string]: any }?): { [string]: boolean }
	local nameCounts: { [string]: number } = {}
	local duplicates: { [string]: boolean } = {}

	for _, child in children do
		local name: string
		if typeof(child) == "Instance" then
			name = child.Name
		elseif virtualInstances then
			-- child is a virtual instance ID
			local virtualChild = virtualInstances[child]
			name = virtualChild and virtualChild.Name or ""
		else
			continue
		end

		nameCounts[name] = (nameCounts[name] or 0) + 1
		if nameCounts[name] > 1 then
			duplicates[name] = true
		end
	end

	return duplicates
end

local function fuzzyEq(a: number, b: number, epsilon: number): boolean
	return math.abs(a - b) < epsilon
end

local function trueEquals(a, b): boolean
	-- Exit early for simple equality values
	if a == b then
		return true
	end

	-- Treat nil and { Ref = "000...0" } as equal
	if
		(a == nil and type(b) == "table" and b.Ref == "00000000000000000000000000000000")
		or (b == nil and type(a) == "table" and a.Ref == "00000000000000000000000000000000")
	then
		return true
	end

	local typeA, typeB = typeof(a), typeof(b)

	-- For tables, try recursive deep equality
	if typeA == "table" and typeB == "table" then
		local checkedKeys = {}
		for key, value in a do
			checkedKeys[key] = true
			if not trueEquals(value, b[key]) then
				return false
			end
		end
		for key, value in b do
			if checkedKeys[key] then
				continue
			end
			if not trueEquals(value, a[key]) then
				return false
			end
		end
		return true

	-- For numbers, compare with epsilon of 0.0001 to avoid floating point inequality
	elseif typeA == "number" and typeB == "number" then
		return fuzzyEq(a, b, 0.0001)

	-- For EnumItem->number, compare the EnumItem's value
	elseif typeA == "number" and typeB == "EnumItem" then
		return a == b.Value
	elseif typeA == "EnumItem" and typeB == "number" then
		return a.Value == b

	-- For Color3s, compare to RGB ints to avoid floating point inequality
	elseif typeA == "Color3" and typeB == "Color3" then
		local aR, aG, aB = math.floor(a.R * 255), math.floor(a.G * 255), math.floor(a.B * 255)
		local bR, bG, bB = math.floor(b.R * 255), math.floor(b.G * 255), math.floor(b.B * 255)
		return aR == bR and aG == bG and aB == bB

	-- For CFrames, compare to components with epsilon of 0.0001 to avoid floating point inequality
	elseif typeA == "CFrame" and typeB == "CFrame" then
		local aComponents, bComponents = { a:GetComponents() }, { b:GetComponents() }
		for i, aComponent in aComponents do
			if not fuzzyEq(aComponent, bComponents[i], 0.0001) then
				return false
			end
		end
		return true

	-- For Vector3s, compare to components with epsilon of 0.0001 to avoid floating point inequality
	elseif typeA == "Vector3" and typeB == "Vector3" then
		local aComponents, bComponents = { a.X, a.Y, a.Z }, { b.X, b.Y, b.Z }
		for i, aComponent in aComponents do
			if not fuzzyEq(aComponent, bComponents[i], 0.0001) then
				return false
			end
		end
		return true

	-- For Vector2s, compare to components with epsilon of 0.0001 to avoid floating point inequality
	elseif typeA == "Vector2" and typeB == "Vector2" then
		local aComponents, bComponents = { a.X, a.Y }, { b.X, b.Y }
		for i, aComponent in aComponents do
			if not fuzzyEq(aComponent, bComponents[i], 0.0001) then
				return false
			end
		end
		return true
	end

	return false
end

local function shouldDeleteUnknownInstances(virtualInstance)
	if virtualInstance.Metadata ~= nil then
		return not virtualInstance.Metadata.ignoreUnknownInstances
	else
		return true
	end
end

local function diff(instanceMap, virtualInstances, rootId)
	local patch = {
		removed = {},
		added = {},
		updated = {},
	}

	-- Count of locations with duplicate-named siblings
	local skippedDuplicateCount = 0

	-- Pre-scan to find ALL instances that are in an ambiguous path
	-- (i.e., they or any of their ancestors have duplicate-named siblings)
	local ambiguousIds: { [string]: boolean } = {}

	local function markSubtreeAmbiguous(id)
		if ambiguousIds[id] then
			return
		end
		ambiguousIds[id] = true

		local virtualInstance = virtualInstances[id]
		if virtualInstance then
			for _, childId in ipairs(virtualInstance.Children) do
				markSubtreeAmbiguous(childId)
			end
		end
	end

	-- Recursively scan for duplicates and mark ambiguous subtrees
	local function scanForDuplicates(id)
		local virtualInstance = virtualInstances[id]
		if not virtualInstance then
			return
		end

		local instance = instanceMap.fromIds[id]

		-- Detect duplicate names among real DOM children (if instance exists)
		local realDuplicates: { [string]: boolean } = {}
		if instance then
			local realChildren = instance:GetChildren()
			realDuplicates = findDuplicateNames(realChildren)

			-- Count locations with duplicates
			if next(realDuplicates) then
				skippedDuplicateCount += 1
			end
		end

		-- Detect duplicate names among virtual DOM children
		local virtualDuplicates = findDuplicateNames(virtualInstance.Children, virtualInstances)

		-- Merge duplicates from both sides
		local allDuplicates: { [string]: boolean } = {}
		for name in realDuplicates do
			allDuplicates[name] = true
		end
		for name in virtualDuplicates do
			allDuplicates[name] = true
		end

		-- Mark all children with duplicate names (and their entire subtrees) as ambiguous
		for _, childId in ipairs(virtualInstance.Children) do
			local virtualChild = virtualInstances[childId]
			if virtualChild and allDuplicates[virtualChild.Name] then
				markSubtreeAmbiguous(childId)
			end
		end

		-- Recurse into non-ambiguous children to find deeper duplicates
		for _, childId in ipairs(virtualInstance.Children) do
			if not ambiguousIds[childId] then
				scanForDuplicates(childId)
			end
		end
	end

	-- First pass: scan entire tree to find all ambiguous paths
	scanForDuplicates(rootId)

	-- Add a virtual instance and all of its descendants to the patch, marked as
	-- being added. Skip ambiguous instances.
	local function markIdAdded(id)
		if ambiguousIds[id] then
			return
		end

		local virtualInstance = virtualInstances[id]
		patch.added[id] = virtualInstance

		for _, childId in ipairs(virtualInstance.Children) do
			markIdAdded(childId)
		end
	end

	-- Internal recursive kernel for diffing an instance with the given ID.
	local function diffInternal(id)
		-- Skip entirely if this instance is in an ambiguous path
		if ambiguousIds[id] then
			return true
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
			-- Skip TouchTransmitter (auto-created by Roblox when touch events are connected)
			if childInstance.ClassName == "TouchTransmitter" then
				continue
			end

			local childId = instanceMap.fromInstances[childInstance]

			-- Skip children in ambiguous paths
			if childId and ambiguousIds[childId] then
				continue
			end

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

				-- Check if this unmapped child has a duplicate name among siblings
				-- If so, skip it (we can't reliably determine what to do with it)
				local siblings = instance:GetChildren()
				local siblingDuplicates = findDuplicateNames(siblings)
				if siblingDuplicates[childInstance.Name] then
					continue
				end

				-- This is an existing instance not present in the virtual DOM.
				-- We can mark it for deletion unless the user has asked us not
				-- to delete unknown stuff.
				if shouldDeleteUnknownInstances(virtualInstance) then
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
			-- Skip children in ambiguous paths
			if ambiguousIds[childId] then
				continue
			end

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

	-- Log count of skipped duplicates
	if skippedDuplicateCount > 0 then
		Log.warn(
			"Skipped {} location(s) with duplicate-named siblings (cannot reliably sync)",
			skippedDuplicateCount
		)
	end

	if not diffSuccess then
		return false, err
	end

	return true, patch
end

return diff
