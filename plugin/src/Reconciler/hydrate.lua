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

local HYDRATE_YIELD_INTERVAL = 1000

local function hydrate(instanceMap, virtualInstances, rootId, rootInstance, session)
	local isRoot = not session or not session.hydrateCount

	if not session then
		session = Matching.newSession()
	end

	if not session.hydrateCount then
		session.hydrateCount = 0
		session.hydrateClock = os.clock()
	end
	session.hydrateCount += 1
	if session.hydrateCount % HYDRATE_YIELD_INTERVAL == 0 then
		task.wait()
	end

	local virtualInstance = virtualInstances[rootId]

	if virtualInstance == nil then
		invariant("Cannot hydrate an instance not present in virtualInstances\nID: {}", rootId)
	end

	if isRoot then
		Log.trace("[TIMING] hydrate() starting at root '{}' ({})", virtualInstance.Name, rootInstance:GetFullName())
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

	local matchClock = os.clock()
	local result =
		Matching.matchChildren(session, validVirtualIds, existingChildren, virtualInstances, rootId, rootInstance)
	local matchElapsed = (os.clock() - matchClock) * 1000

	if matchElapsed > 10 then
		Log.trace(
			"[TIMING] matchChildren for '{}' took {:.1} ms ({} virtual, {} studio, {} matched)",
			virtualInstance.Name,
			matchElapsed,
			#validVirtualIds,
			#existingChildren,
			#result.matched
		)
	end

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

	if isRoot then
		Log.trace(
			"[TIMING] hydrate() completed: {} instances processed, {} matched ({:.1} ms total)",
			session.hydrateCount,
			#result.matched,
			(os.clock() - session.hydrateClock) * 1000
		)
	end
end

return hydrate
