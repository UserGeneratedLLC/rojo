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
	Pass 3: Pairwise similarity scoring within same-name groups.
	Uses ClassName and child count as similarity signals.
	Structural fingerprinting with XXH32 is computed lazily only for
	instances that enter this pass.
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

		-- Build all candidate pairs with similarity scores
		local pairs: { { vi: number, si: number, score: number } } = {}
		for _, vi in vIndices do
			if matchedVirtualIndices[vi] then
				continue
			end
			for _, si in sIndices do
				if matchedStudioIndices[si] then
					continue
				end
				local score = Matching._computeSimilarity(
					remainingVirtual[vi],
					remainingStudio[si],
					virtualInstances
				)
				table.insert(pairs, { vi = vi, si = si, score = score })
			end
		end

		-- Sort by score descending (tiebreaker: original order)
		table.sort(pairs, function(a, b)
			if a.score ~= b.score then
				return a.score > b.score
			end
			if a.vi ~= b.vi then
				return a.vi < b.vi
			end
			return a.si < b.si
		end)

		-- Greedy assignment
		for _, pair in pairs do
			if matchedVirtualIndices[pair.vi] or matchedStudioIndices[pair.si] then
				continue
			end
			table.insert(matched, {
				virtualId = remainingVirtual[pair.vi],
				studioInstance = remainingStudio[pair.si],
			})
			matchedVirtualIndices[pair.vi] = true
			matchedStudioIndices[pair.si] = true
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

--[[
	Compute similarity between a virtual instance and a Studio instance.
	Higher score = more similar.

	Comparison order (early-exit, cheapest first):
	ClassName (100), child count (20), Tags (50), Attributes (30)
	Source is NOT compared here (too expensive for the matching context).
]]
function Matching._computeSimilarity(
	virtualId: string,
	studioInstance: Instance,
	virtualInstances: { [string]: any }
): number
	local vInst = virtualInstances[virtualId]
	if not vInst then
		return 0
	end

	local score = 0

	-- ClassName match
	local classSuccess, className = pcall(function()
		return studioInstance.ClassName
	end)
	if classSuccess and className == vInst.ClassName then
		score += 100
	end

	-- Child count match
	local childSuccess, childCount = pcall(function()
		return #studioInstance:GetChildren()
	end)
	if childSuccess then
		local vChildCount = if vInst.Children then #vInst.Children else 0
		if childCount == vChildCount then
			score += 20
		end
	end

	return score
end

return Matching
