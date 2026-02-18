--!strict
--!native
--!optimize 2
--[[
	Instance matching algorithm for the plugin.

	Pairs server virtual instances to Studio instances during hydration
	by minimizing total reconciler changes. matchChildren is fundamentally
	computeSubtreeCost -- it finds the cost-minimizing pairing of children
	and returns the total cost. The match assignments are a bonus output.

	Two-function mutual recursion:
	  matchChildren: given two child sets, find optimal pairing + totalCost
	  computePairCost: given one virtual + one studio instance, compute
	    total cost including subtree (calls matchChildren for children)

	A MatchingSession holds caches keyed by instance identity so that
	recursive scoring results are reused when hydrate recurses into the
	same children that were already scored.
]]

local Packages = script.Parent.Parent.Parent.Packages
local RbxDom = require(Packages.RbxDom)

local trueEquals = require(script.Parent.trueEquals)

local UNMATCHED_PENALTY = 10000

-- ================================================================
-- Types
-- ================================================================

type MatchPair = {
	virtualId: string,
	studioInstance: Instance,
}

type MatchResult = {
	matched: { MatchPair },
	unmatchedVirtual: { string },
	unmatchedStudio: { Instance },
	totalCost: number,
}

type MatchingSession = {
	matchCache: { [string]: { [Instance]: MatchResult } },
	costCache: { [string]: { [Instance]: number } },
}

type VCache = {
	props: { [string]: any },
	extraProps: { string },
	tags: { [string]: boolean },
	attrs: { [string]: any },
	childCount: number,
}

type SCache = {
	instance: Instance,
	props: { [string]: any },
	tags: { [string]: boolean },
	attrs: { [string]: any },
	children: { Instance },
	childCount: number,
}

type VirtualInstance = {
	Name: string,
	ClassName: string,
	Children: { string },
	Properties: { [string]: any }?,
}

type VirtualInstances = { [string]: VirtualInstance }

type ScoredPair = {
	vi: number,
	si: number,
	cost: number,
	idx: number,
}

-- ================================================================
-- Session constructor
-- ================================================================

local function newSession(): MatchingSession
	return {
		matchCache = {},
		costCache = {},
	}
end

-- ================================================================
-- Utilities
-- ================================================================

local function rawIndex(inst: any, key: string): any
	return inst[key]
end

local function safeGet(inst: Instance, key: string): (boolean, any)
	return pcall(rawIndex, inst :: any, key)
end

-- ================================================================
-- Pre-computation helpers (called once per instance per group)
-- ================================================================

local function cacheVirtual(vInst: VirtualInstance, classKeys: any): VCache
	local decoded: { [string]: any } = {}
	local extraProps: { string } = {}
	local vProps = vInst.Properties

	if vProps then
		for propName, encodedValue in pairs(vProps) do
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
					table.insert(extraProps, propName)
				end
			end
		end
	end

	local decodedTags: { [string]: boolean } = {}
	if vProps and vProps.Tags then
		local ok, tags = RbxDom.EncodedValue.decode(vProps.Tags)
		if ok and type(tags) == "table" then
			for _, tag in ipairs(tags) do
				decodedTags[tag] = true
			end
		end
	end

	local decodedAttrs: { [string]: any } = {}
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

local function cacheStudio(studioInstance: Instance, classKeys: any, extraPropNames: { string }): SCache
	local props: { [string]: any } = {}

	for _, propName in ipairs(classKeys.propNames) do
		local ok, value = safeGet(studioInstance, propName)
		if ok then
			props[propName] = value
		end
	end

	for _, propName in ipairs(extraPropNames) do
		if props[propName] ~= nil then
			continue
		end
		local ok, value = safeGet(studioInstance, propName)
		if ok then
			props[propName] = value
		end
	end

	local tags: { [string]: boolean } = {}
	for _, tag in ipairs(studioInstance:GetTags()) do
		tags[tag] = true
	end

	local children = studioInstance:GetChildren()

	return {
		instance = studioInstance,
		props = props,
		tags = tags,
		attrs = studioInstance:GetAttributes(),
		children = children,
		childCount = #children,
	}
end

-- ================================================================
-- Hot-path scoring (ZERO decode, ZERO reflection lookup)
-- ================================================================

local function countOwnDiffs(vCache: VCache, sCache: SCache, classKeys: any): number
	local cost = 0
	local vProps = vCache.props
	local sProps = sCache.props
	local defaults = classKeys.defaults

	for _, propName in ipairs(classKeys.propNames) do
		local vVal = vProps[propName]
		if vVal == nil then
			vVal = defaults[propName]
		end

		if not trueEquals(vVal, sProps[propName]) then
			cost += 1
		end
	end

	for _, propName in ipairs(vCache.extraProps) do
		if not trueEquals(vProps[propName], sProps[propName]) then
			cost += 1
		end
	end

	local vTags = vCache.tags
	local sTags = sCache.tags
	for tag, _ in pairs(vTags) do
		if not sTags[tag] then
			cost += 1
		end
	end
	for tag, _ in pairs(sTags) do
		if not vTags[tag] then
			cost += 1
		end
	end

	local vAttrs = vCache.attrs
	local sAttrs = sCache.attrs
	for key, vVal in pairs(vAttrs) do
		if not trueEquals(vVal, sAttrs[key]) then
			cost += 1
		end
	end
	for key, _ in pairs(sAttrs) do
		if vAttrs[key] == nil then
			cost += 1
		end
	end

	if vCache.childCount ~= sCache.childCount then
		cost += 1
	end

	return cost
end

-- ================================================================
-- Utilities
-- ================================================================

local function removeMatched(arr: { any }, matchedIndices: { [number]: boolean }): ()
	local write = 1
	for read = 1, #arr do
		if not matchedIndices[read] then
			arr[write] = arr[read]
			write += 1
		end
	end
	for i = write, #arr do
		arr[i] = nil
	end
end

-- ================================================================
-- Core: 2-function mutual recursion
-- ================================================================

local matchChildren -- forward declare

local function computePairCost(
	session: MatchingSession,
	virtualId: string,
	studioInstance: Instance,
	virtualInstances: VirtualInstances,
	bestSoFar: number
): number
	local vc = session.costCache[virtualId]
	if vc then
		local cached = vc[studioInstance]
		if cached ~= nil then
			return cached
		end
	end

	local vInst = virtualInstances[virtualId]
	if not vInst then
		return UNMATCHED_PENALTY
	end

	local classKeys = RbxDom.getClassComparisonKeys(vInst.ClassName)
	local vCache = cacheVirtual(vInst, classKeys)
	local sCache = cacheStudio(studioInstance, classKeys, vCache.extraProps)

	local cost = countOwnDiffs(vCache, sCache, classKeys)
	if cost >= bestSoFar then
		return cost
	end

	local vChildren = vInst.Children
	local studioKids = sCache.children

	if (not vChildren or #vChildren == 0) and #studioKids == 0 then
		-- leaf
	elseif not vChildren or #vChildren == 0 then
		cost += #studioKids * UNMATCHED_PENALTY
	elseif #studioKids == 0 then
		for _, childId in ipairs(vChildren) do
			if virtualInstances[childId] then
				cost += UNMATCHED_PENALTY
			end
		end
	else
		local validVChildren: { string } = {}
		for _, childId in ipairs(vChildren) do
			if virtualInstances[childId] then
				table.insert(validVChildren, childId)
			end
		end
		if #validVChildren > 0 then
			local childResult =
				matchChildren(session, validVChildren, studioKids, virtualInstances, virtualId, studioInstance)
			cost += childResult.totalCost
		else
			cost += #studioKids * UNMATCHED_PENALTY
		end
	end

	if cost < bestSoFar then
		if not session.costCache[virtualId] then
			session.costCache[virtualId] = {}
		end
		session.costCache[virtualId][studioInstance] = cost
	end

	return cost
end

matchChildren = function(
	session: MatchingSession,
	virtualChildren: { string },
	studioChildren: { Instance },
	virtualInstances: VirtualInstances,
	parentVirtualId: string?,
	parentStudioInstance: Instance?
): MatchResult
	-- Cache lookup
	if parentVirtualId and parentStudioInstance then
		local pc = session.matchCache[parentVirtualId]
		if pc then
			local cached = pc[parentStudioInstance]
			if cached then
				return cached
			end
		end
	end

	local matched: { MatchPair } = {}
	local remainingVirtual: { string } = table.clone(virtualChildren)
	local remainingStudio: { Instance } = table.clone(studioChildren)

	-- ============================================================
	-- Group by (Name, ClassName) -- direct property access, no pcall
	-- ============================================================
	local vByKey: { [string]: { number } } = {}
	for i, id in ipairs(remainingVirtual) do
		local vInst = virtualInstances[id]
		if vInst then
			local key = vInst.Name .. "\0" .. vInst.ClassName
			local group = vByKey[key]
			if not group then
				group = {}
				vByKey[key] = group
			end
			table.insert(group, i)
		end
	end

	local sByKey: { [string]: { number } } = {}
	for i, inst in ipairs(remainingStudio) do
		local key = inst.Name .. "\0" .. inst.ClassName
		local group = sByKey[key]
		if not group then
			group = {}
			sByKey[key] = group
		end
		table.insert(group, i)
	end

	-- ============================================================
	-- 1:1 instant match
	-- ============================================================
	local matchedV: { [number]: boolean } = {}
	local matchedS: { [number]: boolean } = {}

	for key, vIndices in pairs(vByKey) do
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

	removeMatched(remainingVirtual, matchedV)
	removeMatched(remainingStudio, matchedS)

	-- ============================================================
	-- Ambiguous groups: change-count scoring + greedy assignment
	-- ============================================================
	if #remainingVirtual > 0 and #remainingStudio > 0 then
		local vByKey2: { [string]: { number } } = {}
		for i, id in ipairs(remainingVirtual) do
			local vInst = virtualInstances[id]
			if vInst then
				local key = vInst.Name .. "\0" .. vInst.ClassName
				local group = vByKey2[key]
				if not group then
					group = {}
					vByKey2[key] = group
				end
				table.insert(group, i)
			end
		end

		local sByKey2: { [string]: { number } } = {}
		for i, inst in ipairs(remainingStudio) do
			local key = inst.Name .. "\0" .. inst.ClassName
			local group = sByKey2[key]
			if not group then
				group = {}
				sByKey2[key] = group
			end
			table.insert(group, i)
		end

		local matchedV2: { [number]: boolean } = {}
		local matchedS2: { [number]: boolean } = {}

		for key, vIndices in pairs(vByKey2) do
			local sIndices = sByKey2[key]
			if not sIndices then
				continue
			end

			local firstVInst = virtualInstances[remainingVirtual[vIndices[1]]]
			if not firstVInst then
				continue
			end
			local classKeys = RbxDom.getClassComparisonKeys(firstVInst.ClassName)

			local vCaches: { [number]: VCache } = {}
			local allExtraProps: { [string]: boolean } = {}
			for _, vi in ipairs(vIndices) do
				if matchedV2[vi] then
					continue
				end
				local vInst = virtualInstances[remainingVirtual[vi]]
				if not vInst then
					continue
				end
				local vCache = cacheVirtual(vInst, classKeys)
				vCaches[vi] = vCache
				for _, propName in ipairs(vCache.extraProps) do
					allExtraProps[propName] = true
				end
			end

			local extraPropNamesArray: { string } = {}
			for propName, _ in pairs(allExtraProps) do
				table.insert(extraPropNamesArray, propName)
			end

			local sCaches: { [number]: SCache } = {}
			for _, si in ipairs(sIndices) do
				if not matchedS2[si] then
					sCaches[si] = cacheStudio(remainingStudio[si], classKeys, extraPropNamesArray)
				end
			end

			-- Score all (A, B) pairs
			local scoredPairs: { ScoredPair } = {}
			local pairIdx = 0
			local bestSoFar = math.huge

			for _, vi in ipairs(vIndices) do
				if matchedV2[vi] then
					continue
				end
				local vCache = vCaches[vi]
				if not vCache then
					continue
				end

				for _, si in ipairs(sIndices) do
					if matchedS2[si] then
						continue
					end
					local sCache = sCaches[si]
					if not sCache then
						continue
					end

					pairIdx += 1

					local cost = countOwnDiffs(vCache, sCache, classKeys)

					if cost < bestSoFar then
						local vInst = virtualInstances[remainingVirtual[vi]]
						if vInst then
							local vChildren = vInst.Children
							local studioKids = sCache.children

							if (not vChildren or #vChildren == 0) and #studioKids == 0 then
								-- leaf
							elseif not vChildren or #vChildren == 0 then
								cost += #studioKids * UNMATCHED_PENALTY
							elseif #studioKids == 0 then
								for _, childId in ipairs(vChildren) do
									if virtualInstances[childId] then
										cost += UNMATCHED_PENALTY
									end
								end
							else
								local validVChildren: { string } = {}
								for _, childId in ipairs(vChildren) do
									if virtualInstances[childId] then
										table.insert(validVChildren, childId)
									end
								end
								if #validVChildren > 0 then
									local childResult = matchChildren(
										session,
										validVChildren,
										studioKids,
										virtualInstances,
										remainingVirtual[vi],
										sCache.instance
									)
									cost += childResult.totalCost
								else
									cost += #studioKids * UNMATCHED_PENALTY
								end
							end
						end
					end

					table.insert(scoredPairs, { vi = vi, si = si, cost = cost, idx = pairIdx })
					if cost < bestSoFar then
						bestSoFar = cost
					end
				end
			end

			table.sort(scoredPairs, function(a: ScoredPair, b: ScoredPair): boolean
				if a.cost ~= b.cost then
					return a.cost < b.cost
				end
				return a.idx < b.idx
			end)

			for _, pair in ipairs(scoredPairs) do
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

		removeMatched(remainingVirtual, matchedV2)
		removeMatched(remainingStudio, matchedS2)
	end

	-- ============================================================
	-- Compute totalCost for ALL matched pairs (session cache helps)
	-- ============================================================
	local totalCost = 0
	for _, pair in ipairs(matched) do
		totalCost += computePairCost(session, pair.virtualId, pair.studioInstance, virtualInstances, math.huge)
	end
	totalCost += (#remainingVirtual + #remainingStudio) * UNMATCHED_PENALTY

	local result = {
		matched = matched,
		unmatchedVirtual = remainingVirtual,
		unmatchedStudio = remainingStudio,
		totalCost = totalCost,
	}

	if parentVirtualId and parentStudioInstance then
		if not session.matchCache[parentVirtualId] then
			session.matchCache[parentVirtualId] = {}
		end
		session.matchCache[parentVirtualId][parentStudioInstance] = result
	end

	return result
end

return {
	newSession = newSession,
	matchChildren = matchChildren,
}
