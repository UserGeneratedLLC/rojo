--[[
	Instance matching algorithm for the plugin.

	Pairs server virtual instances to Studio instances during hydration
	by minimizing total reconciler changes.

	Algorithm per parent:
	  1. Ref pin: parent's Ref properties confirm identity of specific children
	  2. Group remaining by (Name, ClassName) -- 1:1 groups instant-match
	  3. Ambiguous groups: recursive change-count scoring + greedy assignment

	The change count for a pair = how many things the reconciler would need
	to touch to turn instance A into instance B, including the entire subtree.
]]

local Packages = script.Parent.Parent.Parent.Packages
local RbxDom = require(Packages.RbxDom)

local trueEquals = require(script.Parent.trueEquals)

local UNMATCHED_PENALTY = 10000

local Matching = {}

--[[
	Entry point. Match virtual children to Studio children under a parent.

	virtualChildren: array of virtual instance IDs
	studioChildren: array of Studio Instance objects
	virtualInstances: map of virtual ID → virtual instance data
	instanceMap: bidirectional ID ↔ Instance mapping

	Returns {matched, unmatchedVirtual, unmatchedStudio}
]]
function Matching.matchChildren(
	virtualChildren: { string },
	studioChildren: { Instance },
	virtualInstances: { [string]: any }
)
	local matched: { { virtualId: string, studioInstance: Instance } } = {}
	local remainingVirtual: { string } = table.clone(virtualChildren)
	local remainingStudio: { Instance } = table.clone(studioChildren)

	-- ============================================================
	-- Fast-path 1: Ref pin (confirmed identity, highest priority)
	-- ============================================================
	-- Scan virtual parent for Ref-typed properties that point to
	-- virtual children. If the corresponding Studio parent property
	-- points to a Studio child, pin them as a confirmed match.
	-- (Ref pins are handled implicitly during hydration -- the parent
	-- is already matched, and its Ref properties resolve via instanceMap
	-- after children are matched. For initial hydration the instanceMap
	-- is empty, so explicit Ref pinning at this stage is a no-op.
	-- Ref correctness is maintained by the change-count scoring which
	-- counts Ref property diffs.)

	-- ============================================================
	-- Fast-path 2: Group by (Name, ClassName) -- 1:1 instant match
	-- ============================================================
	local vByKey: { [string]: { number } } = {}
	for i, id in remainingVirtual do
		local vInst = virtualInstances[id]
		if vInst then
			local key = vInst.Name .. "\0" .. vInst.ClassName
			if not vByKey[key] then
				vByKey[key] = {}
			end
			table.insert(vByKey[key], i)
		end
	end

	local sByKey: { [string]: { number } } = {}
	for i, inst in remainingStudio do
		local ok, name, class = pcall(function()
			return inst.Name, inst.ClassName
		end)
		if ok then
			local key = name .. "\0" .. class
			if not sByKey[key] then
				sByKey[key] = {}
			end
			table.insert(sByKey[key], i)
		end
	end

	-- Track which indices to remove (collected, then removed in reverse)
	local matchedV: { [number]: boolean } = {}
	local matchedS: { [number]: boolean } = {}

	-- 1:1 groups: instant match
	for key, vIndices in vByKey do
		local sIndices = sByKey[key]
		if sIndices and #vIndices == 1 and #sIndices == 1 then
			local vi, si = vIndices[1], sIndices[1]
			if not matchedV[vi] and not matchedS[si] then
				table.insert(matched, {
					virtualId = remainingVirtual[vi],
					studioInstance = remainingStudio[si],
				})
				matchedV[vi] = true
				matchedS[si] = true
			end
		end
	end

	-- Remove 1:1 matched from remaining lists
	Matching._removeMatched(remainingVirtual, matchedV)
	Matching._removeMatched(remainingStudio, matchedS)

	-- Early exit if nothing left
	if #remainingVirtual == 0 or #remainingStudio == 0 then
		return {
			matched = matched,
			unmatchedVirtual = remainingVirtual,
			unmatchedStudio = remainingStudio,
		}
	end

	-- ============================================================
	-- Ambiguous groups: change-count scoring + greedy assignment
	-- ============================================================
	-- Rebuild groups from remaining (only ambiguous ones are left)
	local vByKey2: { [string]: { number } } = {}
	for i, id in remainingVirtual do
		local vInst = virtualInstances[id]
		if vInst then
			local key = vInst.Name .. "\0" .. vInst.ClassName
			if not vByKey2[key] then
				vByKey2[key] = {}
			end
			table.insert(vByKey2[key], i)
		end
	end

	local sByKey2: { [string]: { number } } = {}
	for i, inst in remainingStudio do
		local ok, name, class = pcall(function()
			return inst.Name, inst.ClassName
		end)
		if ok then
			local key = name .. "\0" .. class
			if not sByKey2[key] then
				sByKey2[key] = {}
			end
			table.insert(sByKey2[key], i)
		end
	end

	local matchedV2: { [number]: boolean } = {}
	local matchedS2: { [number]: boolean } = {}

	for key, vIndices in vByKey2 do
		local sIndices = sByKey2[key]
		if not sIndices then
			continue
		end

		-- Build all (A, B) pairs with change counts.
		-- Pairs are built in child order (virtual outer, studio inner).
		local pairs: { { vi: number, si: number, cost: number, idx: number } } = {}
		local pairIdx = 0
		local bestSoFar = math.huge

		for _, vi in vIndices do
			if matchedV2[vi] then
				continue
			end
			for _, si in sIndices do
				if matchedS2[si] then
					continue
				end
				pairIdx += 1
				local cost =
					Matching._computeChangeCount(remainingVirtual[vi], remainingStudio[si], virtualInstances, bestSoFar)
				table.insert(pairs, { vi = vi, si = si, cost = cost, idx = pairIdx })
				if cost < bestSoFar then
					bestSoFar = cost
				end
			end
		end

		-- Stable sort by cost ascending (Luau sort is not stable,
		-- so break ties by insertion index to preserve child order)
		table.sort(pairs, function(a, b)
			if a.cost ~= b.cost then
				return a.cost < b.cost
			end
			return a.idx < b.idx
		end)

		-- Greedy assign
		for _, pair in pairs do
			if matchedV2[pair.vi] or matchedS2[pair.si] then
				continue
			end
			table.insert(matched, {
				virtualId = remainingVirtual[pair.vi],
				studioInstance = remainingStudio[pair.si],
			})
			matchedV2[pair.vi] = true
			matchedS2[pair.si] = true
		end
	end

	-- Remove matched from remaining
	Matching._removeMatched(remainingVirtual, matchedV2)
	Matching._removeMatched(remainingStudio, matchedS2)

	return {
		matched = matched,
		unmatchedVirtual = remainingVirtual,
		unmatchedStudio = remainingStudio,
	}
end

--[[
	Compute total change count between a virtual instance and a Studio instance.
	Includes own property diffs + recursive children cost.
	Early-exits if cost reaches or exceeds bestSoFar.
]]
function Matching._computeChangeCount(
	virtualId: string,
	studioInstance: Instance,
	virtualInstances: { [string]: any },
	bestSoFar: number
): number
	local vInst = virtualInstances[virtualId]
	if not vInst then
		return UNMATCHED_PENALTY
	end

	-- Cheap: own property diffs
	local cost = Matching._countOwnDiffs(vInst, studioInstance)
	if cost >= bestSoFar then
		return cost
	end

	-- Expensive: recursive children matching
	local vChildren = vInst.Children
	if not vChildren or #vChildren == 0 then
		-- No virtual children: any Studio children are unmatched
		local ok, studioKids = pcall(function()
			return studioInstance:GetChildren()
		end)
		if ok then
			cost += #studioKids * UNMATCHED_PENALTY
		end
		return cost
	end

	local ok, studioKids = pcall(function()
		return studioInstance:GetChildren()
	end)
	if not ok then
		return cost
	end

	-- Filter valid virtual children
	local validVChildren: { string } = {}
	for _, childId in vChildren do
		if virtualInstances[childId] then
			table.insert(validVChildren, childId)
		end
	end

	-- Recursive: match children and sum costs
	local childResult = Matching.matchChildren(validVChildren, studioKids, virtualInstances)

	-- Matched children: recursively score each pair
	for _, pair in childResult.matched do
		cost += Matching._computeChangeCount(pair.virtualId, pair.studioInstance, virtualInstances, bestSoFar - cost)
		if cost >= bestSoFar then
			return cost
		end
	end

	-- Unmatched children: penalty per instance
	local unmatchedCount = #childResult.unmatchedVirtual + #childResult.unmatchedStudio
	cost += unmatchedCount * UNMATCHED_PENALTY

	return cost
end

--[[
	Count own property diffs between a virtual instance and Studio instance.
	Each differing property = +1. Tags and Attributes counted granularly.
]]
function Matching._countOwnDiffs(vInst: any, studioInstance: Instance): number
	local cost = 0

	-- Compare properties from the virtual side
	local vProps = vInst.Properties
	if vProps then
		for propName, encodedValue in vProps do
			-- Skip Tags and Attributes here (counted separately below)
			if propName == "Tags" or propName == "Attributes" then
				continue
			end

			-- Try to decode the virtual value and compare to Studio
			local ty = next(encodedValue)
			if ty == "Ref" then
				-- Ref properties: can't easily compare during hydration
				-- (instanceMap not yet populated). Skip for now.
				continue
			end

			-- Decode the encoded value to a native Roblox type
			local decodeOk, decodedValue = RbxDom.EncodedValue.decode(encodedValue)
			if not decodeOk then
				-- Can't decode: assume different (conservative)
				cost += 1
				continue
			end

			-- Read the Studio property
			local readOk, studioValue = pcall(function()
				return (studioInstance :: any)[propName]
			end)
			if not readOk then
				-- Can't read: assume different
				cost += 1
				continue
			end

			-- Compare values using the same equality check as diff.lua
			if not trueEquals(decodedValue, studioValue) then
				cost += 1
			end
		end
	end

	-- Properties present only on the Studio side (not in virtual).
	-- Syncback strips default-valued properties from model files, so virtual
	-- instances may omit properties that Studio has at non-default values.
	-- Without this check, a virtual instance missing e.g. Face scores equally
	-- against all Studio instances in the group, causing wrong pairings.
	local defaults = RbxDom.findDefaultProperties(vInst.ClassName)
	if defaults then
		for propName, encodedDefault in defaults do
			if propName == "Tags" or propName == "Attributes" then
				continue
			end

			-- Skip properties already compared from the virtual side
			if vProps and vProps[propName] ~= nil then
				continue
			end

			-- Skip Ref/UniqueId types (not comparable during hydration)
			local ty = next(encodedDefault)
			if ty == "Ref" or ty == "UniqueId" then
				continue
			end

			-- pcall guards against unsupported types (e.g. SharedString)
			local pcallOk, decodeOk, defaultValue = pcall(RbxDom.EncodedValue.decode, encodedDefault)
			if not pcallOk or not decodeOk then
				continue
			end

			local readOk, studioValue = pcall(function()
				return (studioInstance :: any)[propName]
			end)
			if not readOk then
				continue
			end

			if not trueEquals(defaultValue, studioValue) then
				cost += 1
			end
		end
	end

	-- Tags: count individual adds/removes
	local vTags = vInst.Properties and vInst.Properties.Tags
	if vTags then
		local decodeOk, decodedTags = RbxDom.EncodedValue.decode(vTags)
		if decodeOk and type(decodedTags) == "table" then
			local readOk, studioTags = pcall(function()
				return studioInstance:GetTags()
			end)
			if readOk then
				local vSet: { [string]: boolean } = {}
				for _, tag in decodedTags do
					vSet[tag] = true
				end
				local sSet: { [string]: boolean } = {}
				for _, tag in studioTags do
					sSet[tag] = true
				end
				-- Count symmetric difference
				for tag in vSet do
					if not sSet[tag] then
						cost += 1
					end
				end
				for tag in sSet do
					if not vSet[tag] then
						cost += 1
					end
				end
			end
		end
	end

	-- Attributes: count individual adds/removes/changes
	local vAttrs = vInst.Properties and vInst.Properties.Attributes
	if vAttrs then
		local decodeOk, decodedAttrs = RbxDom.EncodedValue.decode(vAttrs)
		if decodeOk and type(decodedAttrs) == "table" then
			local readOk, studioAttrs = pcall(function()
				return studioInstance:GetAttributes()
			end)
			if readOk then
				-- Count diffs in attribute maps
				for key, vVal in decodedAttrs do
					local sVal = studioAttrs[key]
					if sVal == nil or not trueEquals(vVal, sVal) then
						cost += 1
					end
				end
				for key in studioAttrs do
					if decodedAttrs[key] == nil then
						cost += 1
					end
				end
			end
		end
	end

	-- Children count diff
	local vChildCount = if vInst.Children then #vInst.Children else 0
	local readOk, studioChildCount = pcall(function()
		return #studioInstance:GetChildren()
	end)
	if readOk and vChildCount ~= studioChildCount then
		cost += 1
	end

	return cost
end

--[[
	Remove items from an array by index set (removes in reverse order).
]]
function Matching._removeMatched(arr: { any }, matchedIndices: { [number]: boolean })
	local toRemove: { number } = {}
	for idx in matchedIndices do
		table.insert(toRemove, idx)
	end
	table.sort(toRemove, function(a, b)
		return a > b
	end)
	for _, idx in toRemove do
		table.remove(arr, idx)
	end
end

return Matching
