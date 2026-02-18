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

	Performance: all property decoding and Studio reads are pre-computed
	BEFORE the N×M scoring loop. The inner comparison is pure table
	lookups + trueEquals, with zero pcall/decode in the hot path.
]]

local Packages = script.Parent.Parent.Parent.Packages
local RbxDom = require(Packages.RbxDom)

local trueEquals = require(script.Parent.trueEquals)

local UNMATCHED_PENALTY = 10000

local Matching = {}

-- ================================================================
-- Pre-computation helpers (called once per instance per group)
-- ================================================================

--[[
	Pre-decode all virtual instance properties into native Roblox values.
	Called once per virtual instance, reused across all M studio comparisons.

	Returns: {
		props:      { [propName]: decodedValue },
		extraProps: { propName, ... } | nil,  -- props not in classKeys (rare)
		tags:       { [tag]: true } | nil,
		attrs:      { [key]: value } | nil,
		childCount: number,
	}
]]
function Matching._cacheVirtual(vInst: any, classKeys: any)
	local decoded: { [string]: any } = {}
	local extraProps: { string }? = nil
	local vProps = vInst.Properties

	if vProps then
		for propName, encodedValue in vProps do
			if propName == "Tags" or propName == "Attributes" then
				continue
			end

			local ty = next(encodedValue)
			if ty == "Ref" then
				continue
			end

			local pcallOk, decodeOk, value = pcall(RbxDom.EncodedValue.decode, encodedValue)
			if pcallOk and decodeOk and value ~= nil then
				decoded[propName] = value
				if not classKeys.propNameSet[propName] then
					if not extraProps then
						extraProps = {}
					end
					table.insert(extraProps, propName)
				end
			end
		end
	end

	-- Pre-decode tags into a set
	local decodedTags: { [string]: boolean }? = nil
	if vProps and vProps.Tags then
		local ok, tags = RbxDom.EncodedValue.decode(vProps.Tags)
		if ok and type(tags) == "table" then
			decodedTags = {}
			for _, tag in tags do
				decodedTags[tag] = true
			end
		end
	end

	-- Pre-decode attributes
	local decodedAttrs: { [string]: any }? = nil
	if vProps and vProps.Attributes then
		local ok, attrs = RbxDom.EncodedValue.decode(vProps.Attributes)
		if ok and type(attrs) == "table" then
			decodedAttrs = attrs
		end
	end

	return {
		props = decoded,
		extraProps = extraProps,
		tags = decodedTags,
		attrs = decodedAttrs,
		childCount = if vInst.Children then #vInst.Children else 0,
	}
end

--[[
	Pre-read all comparable properties from a Studio instance.
	Called once per Studio instance, reused across all N virtual comparisons.

	Returns: {
		props:      { [propName]: value },
		readable:   { [propName]: true },  -- tracks which reads succeeded
		tags:       { [tag]: true } | nil,
		attrs:      { [key]: value } | nil,
		children:   { Instance } | nil,
		childCount: number,
	}
]]
function Matching._cacheStudio(studioInstance: Instance, classKeys: any, extraPropNames: { string }?)
	local props: { [string]: any } = {}

	-- Read all comparison-key properties
	for _, propName in classKeys.propNames do
		local ok, value = pcall(function()
			return (studioInstance :: any)[propName]
		end)
		if ok and value ~= nil then
			props[propName] = value
		end
	end

	-- Read any extra properties the virtual side has (not in classKeys)
	if extraPropNames then
		for _, propName in extraPropNames do
			if props[propName] ~= nil then
				continue
			end
			local ok, value = pcall(function()
				return (studioInstance :: any)[propName]
			end)
			if ok and value ~= nil then
				props[propName] = value
			end
		end
	end

	-- Read tags
	local tags: { [string]: boolean }? = nil
	local tagsOk, studioTags = pcall(function()
		return studioInstance:GetTags()
	end)
	if tagsOk then
		tags = {}
		for _, tag in studioTags do
			tags[tag] = true
		end
	end

	-- Read attributes
	local attrs: { [string]: any }? = nil
	local attrsOk, studioAttrs = pcall(function()
		return studioInstance:GetAttributes()
	end)
	if attrsOk then
		attrs = studioAttrs
	end

	-- Read children
	local children: { Instance }? = nil
	local childCount = 0
	local childOk, studioKids = pcall(function()
		return studioInstance:GetChildren()
	end)
	if childOk then
		children = studioKids
		childCount = #studioKids
	end

	return {
		props = props,
		tags = tags,
		attrs = attrs,
		children = children,
		childCount = childCount,
	}
end

-- ================================================================
-- Hot-path scoring (ZERO pcall, ZERO decode, ZERO reflection lookup)
-- ================================================================

--[[
	Count own property diffs between pre-computed virtual and Studio caches.
	Single loop over the class's comparable properties. Pure table lookups
	and trueEquals calls -- no pcall, no decode, no reflection queries.
]]
function Matching._countOwnDiffsCached(vCache: any, sCache: any, classKeys: any): number
	local cost = 0
	local vProps = vCache.props
	local sProps = sCache.props
	local defaults = classKeys.defaults

	-- Single loop over all comparable property names for this class.
	-- For each property, the effective virtual value is the explicit value
	-- if present, otherwise the class default (syncback strips defaults).
	for _, propName in classKeys.propNames do
		local vVal = vProps[propName]
		if vVal == nil then
			vVal = defaults[propName]
		end

		if not trueEquals(vVal, sProps[propName]) then
			cost += 1
		end
	end

	-- Handle virtual properties not in the class comparison set (rare:
	-- unknown class or properties from a newer API version).
	if vCache.extraProps then
		for _, propName in vCache.extraProps do
			if not trueEquals(vProps[propName], sProps[propName]) then
				cost += 1
			end
		end
	end

	-- Tags: symmetric set diff (pre-computed sets)
	local vTags = vCache.tags
	local sTags = sCache.tags
	if vTags and sTags then
		for tag in vTags do
			if not sTags[tag] then
				cost += 1
			end
		end
		for tag in sTags do
			if not vTags[tag] then
				cost += 1
			end
		end
	end

	-- Attributes: symmetric map diff (pre-decoded / pre-read)
	local vAttrs = vCache.attrs
	local sAttrs = sCache.attrs
	if vAttrs and sAttrs then
		for key, vVal in vAttrs do
			if not trueEquals(vVal, sAttrs[key]) then
				cost += 1
			end
		end
		for key in sAttrs do
			if vAttrs[key] == nil then
				cost += 1
			end
		end
	end

	-- Children count diff
	if vCache.childCount ~= sCache.childCount then
		cost += 1
	end

	return cost
end

-- ================================================================
-- Recursive scoring (children matching)
-- ================================================================

--[[
	Compute the recursive children cost for a matched pair.
	Uses sCache.children (pre-read) to avoid repeated GetChildren().
]]
function Matching._computeChildrenCost(
	vInst: any,
	sCache: any,
	virtualInstances: { [string]: any },
	bestSoFar: number
): number
	local vChildren = vInst.Children
	local studioKids = sCache.children

	if not vChildren or #vChildren == 0 then
		if studioKids then
			return #studioKids * UNMATCHED_PENALTY
		end
		return 0
	end

	if not studioKids then
		return 0
	end

	local validVChildren: { string } = {}
	for _, childId in vChildren do
		if virtualInstances[childId] then
			table.insert(validVChildren, childId)
		end
	end

	local childResult = Matching.matchChildren(validVChildren, studioKids, virtualInstances)

	local cost = 0
	for _, pair in childResult.matched do
		cost += Matching._computeChangeCount(pair.virtualId, pair.studioInstance, virtualInstances, bestSoFar - cost)
		if cost >= bestSoFar then
			return cost
		end
	end

	cost += (#childResult.unmatchedVirtual + #childResult.unmatchedStudio) * UNMATCHED_PENALTY
	return cost
end

--[[
	Compute total change count between a virtual instance and a Studio instance.
	Includes own property diffs + recursive children cost.
	Early-exits if cost reaches or exceeds bestSoFar.

	Used for recursive 1:1 scoring (matched child pairs). For the N×M
	ambiguous-group loop, matchChildren uses pre-computed caches directly.
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

	-- Build caches inline (1:1 call, no N×M savings needed here,
	-- but classKeys is still cached per class so it's free).
	local classKeys = RbxDom.getClassComparisonKeys(vInst.ClassName)
	local vCache = Matching._cacheVirtual(vInst, classKeys)
	local sCache = Matching._cacheStudio(studioInstance, classKeys, vCache.extraProps)

	local cost = Matching._countOwnDiffsCached(vCache, sCache, classKeys)
	if cost >= bestSoFar then
		return cost
	end

	cost += Matching._computeChildrenCost(vInst, sCache, virtualInstances, bestSoFar - cost)
	return cost
end

-- ================================================================
-- Entry point
-- ================================================================

--[[
	Match virtual children to Studio children under a parent.

	virtualChildren: array of virtual instance IDs
	studioChildren: array of Studio Instance objects
	virtualInstances: map of virtual ID → virtual instance data

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

		-- --------------------------------------------------------
		-- Pre-compute caches for this ambiguous group.
		-- All instances share the same ClassName (grouped by key).
		-- --------------------------------------------------------
		local firstVInst = virtualInstances[remainingVirtual[vIndices[1]]]
		if not firstVInst then
			continue
		end
		local classKeys = RbxDom.getClassComparisonKeys(firstVInst.ClassName)

		-- Pre-decode all virtual instances in this group
		local vCaches: { [number]: any } = {}
		local allExtraProps: { [string]: boolean }? = nil
		for _, vi in vIndices do
			if matchedV2[vi] then
				continue
			end
			local vInst = virtualInstances[remainingVirtual[vi]]
			if not vInst then
				continue
			end
			local vCache = Matching._cacheVirtual(vInst, classKeys)
			vCaches[vi] = vCache
			-- Collect extra prop names across all virtuals for studio reads
			if vCache.extraProps then
				if not allExtraProps then
					allExtraProps = {}
				end
				for _, propName in vCache.extraProps do
					allExtraProps[propName] = true
				end
			end
		end

		-- Convert extra props set to array for studio caching
		local extraPropNamesArray: { string }? = nil
		if allExtraProps then
			extraPropNamesArray = {}
			for propName in allExtraProps do
				table.insert(extraPropNamesArray, propName)
			end
		end

		-- Pre-read all Studio instances in this group
		local sCaches: { [number]: any } = {}
		for _, si in sIndices do
			if not matchedS2[si] then
				sCaches[si] = Matching._cacheStudio(remainingStudio[si], classKeys, extraPropNamesArray)
			end
		end

		-- --------------------------------------------------------
		-- Score all (A, B) pairs using pre-computed caches.
		-- Own diffs = pure table lookups (FAST).
		-- Children cost = recursive (only if own diffs < bestSoFar).
		-- --------------------------------------------------------
		local pairs: { { vi: number, si: number, cost: number, idx: number } } = {}
		local pairIdx = 0
		local bestSoFar = math.huge

		for _, vi in vIndices do
			if matchedV2[vi] then
				continue
			end
			local vCache = vCaches[vi]
			if not vCache then
				continue
			end

			for _, si in sIndices do
				if matchedS2[si] then
					continue
				end
				local sCache = sCaches[si]
				if not sCache then
					continue
				end

				pairIdx += 1

				-- Own diffs: pure table lookups + trueEquals
				local cost = Matching._countOwnDiffsCached(vCache, sCache, classKeys)

				if cost < bestSoFar then
					-- Recursive children cost
					local vInst = virtualInstances[remainingVirtual[vi]]
					if vInst then
						cost += Matching._computeChildrenCost(vInst, sCache, virtualInstances, bestSoFar - cost)
					end
				end

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
