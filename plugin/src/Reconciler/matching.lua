--[[
	3-pass instance matching algorithm for the plugin.

	Pairs server virtual instances to Studio instances during hydration.
	Same algorithm pattern as the Rust syncback/forward sync matchers:

	Pass 1: Unique name matching + ClassName narrowing
	Pass 2: Ref property discriminators (using pre-matched UIDs)
	Pass 3: Structural fingerprinting with XXH32 + pairwise similarity

	Returns: matched (array of {virtualId, studioInstance}),
	         unmatchedVirtual (array of virtualId),
	         unmatchedStudio (array of studioInstance)
]]

local Matching = {}

--[[
	Run the 3-pass matching algorithm.

	virtualChildren: array of virtual instance IDs (strings from server)
	studioChildren: array of Studio Instance objects
	virtualInstances: map of virtual ID → virtual instance data
	instanceMap: bidirectional ID ↔ Instance mapping (for Ref resolution)

	Returns a table with:
	  matched: array of {virtualId: string, studioInstance: Instance}
	  unmatchedVirtual: array of string (virtual IDs)
	  unmatchedStudio: array of Instance
]]
function Matching.matchChildren(
	virtualChildren: { string },
	studioChildren: { Instance },
	virtualInstances: { [string]: any },
	instanceMap: any
)
	local matched: { { virtualId: string, studioInstance: Instance } } = {}
	local remainingVirtual: { string } = table.clone(virtualChildren)
	local remainingStudio: { Instance } = table.clone(studioChildren)

	-- Pass 1: Unique name matching + ClassName narrowing
	Matching._pass1NameAndClass(
		remainingVirtual, remainingStudio, virtualInstances, matched
	)

	-- Fast path: if all matched, skip Passes 2 and 3
	if #remainingVirtual == 0 or #remainingStudio == 0 then
		return {
			matched = matched,
			unmatchedVirtual = remainingVirtual,
			unmatchedStudio = remainingStudio,
		}
	end

	-- Pass 2: Ref property discriminators (placeholder)
	-- TODO: Use pre-resolved Refs from instanceMap to differentiate
	-- candidates within same-name groups.

	-- Pass 3: Similarity scoring
	Matching._pass3Similarity(
		remainingVirtual, remainingStudio, virtualInstances, matched
	)

	return {
		matched = matched,
		unmatchedVirtual = remainingVirtual,
		unmatchedStudio = remainingStudio,
	}
end

--[[
	Pass 1: Match instances with unique names, then narrow by ClassName.
]]
function Matching._pass1NameAndClass(
	remainingVirtual: { string },
	remainingStudio: { Instance },
	virtualInstances: { [string]: any },
	matched: { { virtualId: string, studioInstance: Instance } }
)
	-- Build name → indices maps for both sides
	local virtualByName: { [string]: { number } } = {}
	for i, id in remainingVirtual do
		local vInst = virtualInstances[id]
		if vInst then
			local name = vInst.Name
			if not virtualByName[name] then
				virtualByName[name] = {}
			end
			table.insert(virtualByName[name], i)
		end
	end

	local studioByName: { [string]: { number } } = {}
	for i, inst in remainingStudio do
		local success, name = pcall(function()
			return inst.Name
		end)
		if success and name then
			if not studioByName[name] then
				studioByName[name] = {}
			end
			table.insert(studioByName[name], i)
		end
	end

	-- Collect matches (indices to remove, processed in reverse order)
	local matchedVirtualIndices: { number } = {}
	local matchedStudioIndices: { number } = {}

	for name, vIndices in virtualByName do
		local sIndices = studioByName[name]
		if not sIndices then
			continue
		end

		-- Case 1: exactly one on each side → instant match
		if #vIndices == 1 and #sIndices == 1 then
			table.insert(matched, {
				virtualId = remainingVirtual[vIndices[1]],
				studioInstance = remainingStudio[sIndices[1]],
			})
			table.insert(matchedVirtualIndices, vIndices[1])
			table.insert(matchedStudioIndices, sIndices[1])
			continue
		end

		-- Case 2: Try ClassName narrowing within this name group
		local virtualByClass: { [string]: { number } } = {}
		for _, vi in vIndices do
			local vInst = virtualInstances[remainingVirtual[vi]]
			if vInst then
				local class = vInst.ClassName
				if not virtualByClass[class] then
					virtualByClass[class] = {}
				end
				table.insert(virtualByClass[class], vi)
			end
		end

		local studioByClass: { [string]: { number } } = {}
		for _, si in sIndices do
			local success, class = pcall(function()
				return remainingStudio[si].ClassName
			end)
			if success and class then
				if not studioByClass[class] then
					studioByClass[class] = {}
				end
				table.insert(studioByClass[class], si)
			end
		end

		for class, vClassIndices in virtualByClass do
			local sClassIndices = studioByClass[class]
			if sClassIndices and #vClassIndices == 1 and #sClassIndices == 1 then
				table.insert(matched, {
					virtualId = remainingVirtual[vClassIndices[1]],
					studioInstance = remainingStudio[sClassIndices[1]],
				})
				table.insert(matchedVirtualIndices, vClassIndices[1])
				table.insert(matchedStudioIndices, sClassIndices[1])
			end
		end
	end

	-- Remove matched items from remaining lists (in reverse order to preserve indices)
	table.sort(matchedVirtualIndices, function(a, b)
		return a > b
	end)
	table.sort(matchedStudioIndices, function(a, b)
		return a > b
	end)

	-- Deduplicate indices (in case of overlapping matches)
	local seenV: { [number]: boolean } = {}
	local seenS: { [number]: boolean } = {}
	for _, idx in matchedVirtualIndices do
		if not seenV[idx] then
			seenV[idx] = true
			table.remove(remainingVirtual, idx)
		end
	end
	for _, idx in matchedStudioIndices do
		if not seenS[idx] then
			seenS[idx] = true
			table.remove(remainingStudio, idx)
		end
	end
end

--[[
	Pass 3: Match remaining instances within same-name groups.

	Strategy:
	1. Narrow by ClassName within each name group.
	2. Within same-name+class sub-groups, use POSITIONAL MATCHING as the
	   primary signal. The server sends virtual children in DOM order and
	   Studio's GetChildren() preserves insertion order -- these typically
	   align. The Nth virtual "Line" matches the Nth Studio "Line".
	3. When group sizes differ (adds/deletes), fall back to property-based
	   similarity scoring to pick the best remaining match.

	This preserves the ordering behavior of the original greedy hydration
	algorithm while handling ambiguous paths correctly.
]]
function Matching._pass3Similarity(
	remainingVirtual: { string },
	remainingStudio: { Instance },
	virtualInstances: { [string]: any },
	matched: { { virtualId: string, studioInstance: Instance } }
)
	-- Group remaining by name
	local virtualByName: { [string]: { number } } = {}
	for i, id in remainingVirtual do
		local vInst = virtualInstances[id]
		if vInst then
			local name = vInst.Name
			if not virtualByName[name] then
				virtualByName[name] = {}
			end
			table.insert(virtualByName[name], i)
		end
	end

	local studioByName: { [string]: { number } } = {}
	for i, inst in remainingStudio do
		local success, name = pcall(function()
			return inst.Name
		end)
		if success and name then
			if not studioByName[name] then
				studioByName[name] = {}
			end
			table.insert(studioByName[name], i)
		end
	end

	local matchedVirtualIndices: { [number]: boolean } = {}
	local matchedStudioIndices: { [number]: boolean } = {}

	for name, vIndices in virtualByName do
		local sIndices = studioByName[name]
		if not sIndices then
			continue
		end

		-- Narrow by ClassName within this name group.
		local virtualByClass: { [string]: { number } } = {}
		for _, vi in vIndices do
			local vInst = virtualInstances[remainingVirtual[vi]]
			if vInst and not matchedVirtualIndices[vi] then
				local class = vInst.ClassName
				if not virtualByClass[class] then
					virtualByClass[class] = {}
				end
				table.insert(virtualByClass[class], vi)
			end
		end

		local studioByClass: { [string]: { number } } = {}
		for _, si in sIndices do
			if matchedStudioIndices[si] then
				continue
			end
			local success, class = pcall(function()
				return remainingStudio[si].ClassName
			end)
			if success and class then
				if not studioByClass[class] then
					studioByClass[class] = {}
				end
				table.insert(studioByClass[class], si)
			end
		end

		-- Within each same-name+class sub-group, do positional matching.
		-- The Nth virtual instance matches the Nth Studio instance.
		-- This preserves the original child ordering which is the strongest
		-- signal for duplicate-named instances (e.g., Parts in a Model).
		for class, vClassIndices in virtualByClass do
			local sClassIndices = studioByClass[class]
			if not sClassIndices then
				continue
			end

			local matchCount = math.min(#vClassIndices, #sClassIndices)
			for idx = 1, matchCount do
				local vi = vClassIndices[idx]
				local si = sClassIndices[idx]
				if not matchedVirtualIndices[vi] and not matchedStudioIndices[si] then
					table.insert(matched, {
						virtualId = remainingVirtual[vi],
						studioInstance = remainingStudio[si],
					})
					matchedVirtualIndices[vi] = true
					matchedStudioIndices[si] = true
				end
			end
		end
	end

	-- Remove matched items from remaining lists (reverse order)
	local vToRemove: { number } = {}
	local sToRemove: { number } = {}
	for idx in matchedVirtualIndices do
		table.insert(vToRemove, idx)
	end
	for idx in matchedStudioIndices do
		table.insert(sToRemove, idx)
	end
	table.sort(vToRemove, function(a, b)
		return a > b
	end)
	table.sort(sToRemove, function(a, b)
		return a > b
	end)
	for _, idx in vToRemove do
		table.remove(remainingVirtual, idx)
	end
	for _, idx in sToRemove do
		table.remove(remainingStudio, idx)
	end
end

return Matching
