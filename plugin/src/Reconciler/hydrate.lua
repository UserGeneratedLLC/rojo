--[[
	Defines the process of "hydration" -- matching up a virtual DOM with
	concrete instances and assigning them IDs.

	Uses the change-count matching algorithm to handle duplicate-named
	instances: groups by (Name, ClassName), then greedy-assigns pairs
	by fewest total reconciler changes (recursive into subtrees).
]]

local Packages = script.Parent.Parent.Parent.Packages
local Log = require(Packages.Log)

local invariant = require(script.Parent.Parent.invariant)
local Matching = require(script.Parent.matching)

local function hydrate(instanceMap, virtualInstances, rootId, rootInstance, session)
	if not session then
		session = Matching.newSession()
	end

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

	local result =
		Matching.matchChildren(session, validVirtualIds, existingChildren, virtualInstances, rootId, rootInstance)

	-- Recursively hydrate matched pairs
	for _, pair in result.matched do
		hydrate(instanceMap, virtualInstances, pair.virtualId, pair.studioInstance, session)
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
