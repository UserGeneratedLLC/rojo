--[[
	Defines the process of "hydration" -- matching up a virtual DOM with
	concrete instances and assigning them IDs.

	Uses the 3-pass matching algorithm to handle duplicate-named instances:
	Pass 1: unique name + ClassName narrowing
	Pass 2: Ref property discriminators (using pre-matched UIDs)
	Pass 3: structural fingerprinting + pairwise similarity
]]

local Packages = script.Parent.Parent.Parent.Packages
local Log = require(Packages.Log)

local invariant = require(script.Parent.Parent.invariant)
local Matching = require(script.Parent.matching)

local function hydrate(instanceMap, virtualInstances, rootId, rootInstance)
	local virtualInstance = virtualInstances[rootId]

	if virtualInstance == nil then
		invariant("Cannot hydrate an instance not present in virtualInstances\nID: {}", rootId)
	end

	instanceMap:insert(rootId, rootInstance)

	local existingChildren = rootInstance:GetChildren()
	local virtualChildIds = virtualInstance.Children

	-- Filter out virtual children missing from virtualInstances
	local validVirtualIds: { string } = {}
	for _, childId in ipairs(virtualChildIds) do
		if virtualInstances[childId] then
			table.insert(validVirtualIds, childId)
		else
			Log.warn(
				"Hydration: virtualInstances missing child ID {} (parent: {} '{}')",
				childId,
				rootId,
				virtualInstance.Name
			)
		end
	end

	-- Use the 3-pass matching algorithm to pair virtual ↔ studio children
	local result = Matching.matchChildren(
		validVirtualIds,
		existingChildren,
		virtualInstances,
		instanceMap
	)

	-- Recursively hydrate matched pairs
	for _, pair in result.matched do
		hydrate(instanceMap, virtualInstances, pair.virtualId, pair.studioInstance)
	end

	-- Log unmatched virtual children (no Studio counterpart found)
	for _, virtualId in result.unmatchedVirtual do
		local virtualChild = virtualInstances[virtualId]
		if virtualChild then
			Log.debug(
				"Hydration: No match for virtual child '{}' ({}) under '{}' ({})",
				virtualChild.Name,
				virtualChild.ClassName,
				virtualInstance.Name,
				rootInstance:GetFullName()
			)
		end
	end
end

return hydrate
