--[[
	Defines the process of "hydration" -- matching up a virtual DOM with
	concrete instances and assigning them IDs.
]]

local Packages = script.Parent.Parent.Parent.Packages
local Log = require(Packages.Log)

local invariant = require(script.Parent.Parent.invariant)

local function hydrate(instanceMap, virtualInstances, rootId, rootInstance)
	local virtualInstance = virtualInstances[rootId]

	if virtualInstance == nil then
		invariant("Cannot hydrate an instance not present in virtualInstances\nID: {}", rootId)
	end

	instanceMap:insert(rootId, rootInstance)

	local existingChildren = rootInstance:GetChildren()

	-- For each existing child, we'll track whether it's been paired with an
	-- instance that the Rojo server knows about.
	local isExistingChildVisited = {}
	for i = 1, #existingChildren do
		isExistingChildVisited[i] = false
	end

	for _, childId in ipairs(virtualInstance.Children) do
		local virtualChild = virtualInstances[childId]

		if virtualChild == nil then
			Log.warn(
				"Hydration: virtualInstances missing child ID {} (parent: {} '{}')",
				childId,
				rootId,
				virtualInstance.Name
			)
			continue
		end

		local matched = false
		for childIndex, childInstance in existingChildren do
			if not isExistingChildVisited[childIndex] then
				-- We guard accessing Name and ClassName in order to avoid
				-- tripping over children of DataModel that Rojo won't have
				-- permissions to access at all.
				local accessSuccess, name, className = pcall(function()
					return childInstance.Name, childInstance.ClassName
				end)

				-- This rule is very conservative and could be loosened in the
				-- future, or more heuristics could be introduced.
				if accessSuccess and name == virtualChild.Name and className == virtualChild.ClassName then
					isExistingChildVisited[childIndex] = true
					hydrate(instanceMap, virtualInstances, childId, childInstance)
					matched = true
					break
				end
			end
		end

		if not matched then
			-- Log why the match failed to help diagnose hydration issues
			Log.debug(
				"Hydration: No match for virtual child '{}' ({}) under '{}' ({})",
				virtualChild.Name,
				virtualChild.ClassName,
				virtualInstance.Name,
				rootInstance:GetFullName()
			)
			-- List real children for comparison
			local realChildNames = {}
			for _, child in existingChildren do
				local success, childName, childClass = pcall(function()
					return child.Name, child.ClassName
				end)
				if success then
					table.insert(realChildNames, string.format("%s (%s)", childName, childClass))
				end
			end
			Log.trace("  Real children: {}", table.concat(realChildNames, ", "))
		end
	end
end

return hydrate
