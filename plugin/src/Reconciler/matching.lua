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
local MAX_SCORING_DEPTH = 3

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
	vCacheStore: { [string]: VCache },
}

type RefIdentity = {
	name: string,
	className: string,
}

type VCache = {
	props: { [string]: any },
	extraProps: { string },
	tags: { [string]: boolean },
	attrs: { [string]: any },
	refs: { [string]: RefIdentity },
	childCount: number,
	validChildren: { string },
}

type SCache = {
	instance: Instance,
	props: { [string]: any },
	tags: { [string]: boolean },
	attrs: { [string]: any },
	refs: { [string]: RefIdentity },
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

type ClassComparisonKeys = {
	propNames: { string },
	propNameSet: { [string]: boolean },
	defaults: { [string]: any },
}

-- ================================================================
-- Session constructor
-- ================================================================

local function newSession(): MatchingSession
	return {
		matchCache = {},
		costCache = {},
		vCacheStore = {},
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

local function cacheVirtual(
	vInst: VirtualInstance,
	classKeys: ClassComparisonKeys,
	virtualInstances: VirtualInstances
): VCache
	local decoded: { [string]: any } = {}
	local extraProps: { string } = {}
	local refs: { [string]: RefIdentity } = {}
	local vProps = vInst.Properties

	if vProps then
		for propName, encodedValue in pairs(vProps) do
			if propName == "Tags" or propName == "Attributes" then
				continue
			end

			local ty = next(encodedValue)
			if ty == "Ref" then
				local targetId = encodedValue.Ref
				if targetId then
					local target = virtualInstances[targetId]
					if target then
						refs[propName] = { name = target.Name, className = target.ClassName }
					end
				end
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

	local validChildren: { string } = {}
	if vInst.Children then
		for _, childId in ipairs(vInst.Children) do
			if virtualInstances[childId] then
				table.insert(validChildren, childId)
			end
		end
	end

	return {
		props = decoded,
		extraProps = extraProps,
		tags = decodedTags,
		attrs = decodedAttrs,
		refs = refs,
		childCount = if vInst.Children then #vInst.Children else 0,
		validChildren = validChildren,
	}
end

local function cacheStudio(
	studioInstance: Instance,
	classKeys: ClassComparisonKeys,
	extraPropNames: { string },
	refPropNames: { string }
): SCache
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

	local refs: { [string]: RefIdentity } = {}
	for _, propName in ipairs(refPropNames) do
		local ok, value = safeGet(studioInstance, propName)
		if ok and typeof(value) == "Instance" then
			refs[propName] = { name = (value :: Instance).Name, className = (value :: Instance).ClassName }
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
		refs = refs,
		children = children,
		childCount = #children,
	}
end

-- ================================================================
-- Hot-path scoring (ZERO decode, ZERO reflection lookup)
-- ================================================================

local function countOwnDiffs(vCache: VCache, sCache: SCache, classKeys: ClassComparisonKeys): number
	local cost = 0
	local vProps = vCache.props
	local sProps = sCache.props
	local defaults = classKeys.defaults

	for _, propName in ipairs(classKeys.propNames) do
		local vVal = vProps[propName]
		if vVal == nil then
			vVal = defaults[propName]
		end

		local sVal = sProps[propName]
		if vVal ~= sVal and not trueEquals(vVal, sVal) then
			cost += 1
		end
	end

	for _, propName in ipairs(vCache.extraProps) do
		local vVal = vProps[propName]
		local sVal = sProps[propName]
		if vVal ~= sVal and not trueEquals(vVal, sVal) then
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
		local sVal = sAttrs[key]
		if vVal ~= sVal and not trueEquals(vVal, sVal) then
			cost += 1
		end
	end
	for key, _ in pairs(sAttrs) do
		if vAttrs[key] == nil then
			cost += 1
		end
	end

	local vRefs = vCache.refs
	local sRefs = sCache.refs
	for propName, vRef in pairs(vRefs) do
		local sRef = sRefs[propName]
		if sRef then
			if vRef.name ~= sRef.name or vRef.className ~= sRef.className then
				cost += UNMATCHED_PENALTY
			end
		else
			cost += UNMATCHED_PENALTY
		end
	end
	for propName, _ in pairs(sRefs) do
		if not vRefs[propName] then
			cost += UNMATCHED_PENALTY
		end
	end

	if vCache.childCount ~= sCache.childCount then
		cost += 1
	end

	return cost
end

-- ================================================================
-- Optimal assignment via Hungarian (Kuhn-Munkres) algorithm
-- ================================================================

local function minCostAssignment(cost: { { number } }, rows: number, cols: number): { { number } }
	if rows == 0 or cols == 0 then
		return {}
	end

	local n = math.max(rows, cols)
	local big = UNMATCHED_PENALTY * 2

	local c: { { number } } = table.create(n)
	for i = 1, n do
		local row = table.create(n, big)
		for j = 1, n do
			if i <= rows and j <= cols then
				row[j] = cost[i][j]
			end
		end
		c[i] = row
	end

	local u = table.create(n + 1, 0)
	local v = table.create(n + 1, 0)
	local p = table.create(n + 1, 0)
	local way = table.create(n + 1, 0)

	local INF = math.huge

	for i = 1, n do
		p[0] = i
		local j0 = 0
		local minV = table.create(n + 1, INF)
		local used = table.create(n + 1, false)

		while true do
			used[j0] = true
			local i0 = p[j0]
			local delta = INF
			local j1 = 0

			for j = 1, n do
				if used[j] then
					continue
				end
				local cur = c[i0][j] - u[i0] - v[j]
				if cur < minV[j] then
					minV[j] = cur
					way[j] = j0
				end
				if minV[j] < delta then
					delta = minV[j]
					j1 = j
				end
			end

			for j = 0, n do
				if used[j] then
					u[p[j]] += delta
					v[j] -= delta
				else
					minV[j] -= delta
				end
			end

			j0 = j1
			if p[j0] == 0 then
				break
			end
		end

		while true do
			local j1 = way[j0]
			p[j0] = p[j1]
			j0 = j1
			if j0 == 0 then
				break
			end
		end
	end

	local result: { { number } } = {}
	for j = 1, n do
		if p[j] ~= 0 and p[j] <= rows and j <= cols then
			table.insert(result, { p[j], j })
		end
	end
	return result
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
	bestSoFar: number,
	depth: number
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
	local vCache = session.vCacheStore[virtualId]
	if not vCache then
		vCache = cacheVirtual(vInst, classKeys, virtualInstances)
		session.vCacheStore[virtualId] = vCache
	end
	local refPropNames: { string } = {}
	for propName, _ in pairs(vCache.refs) do
		table.insert(refPropNames, propName)
	end
	local sCache = cacheStudio(studioInstance, classKeys, vCache.extraProps, refPropNames)

	local cost = countOwnDiffs(vCache, sCache, classKeys)
	if cost >= bestSoFar or depth >= MAX_SCORING_DEPTH then
		return cost
	end

	local validVChildren = vCache.validChildren
	local studioKids = sCache.children

	if #validVChildren > 0 or #studioKids > 0 then
		if #validVChildren == 0 then
			cost += #studioKids * UNMATCHED_PENALTY
		elseif #studioKids == 0 then
			cost += #validVChildren * UNMATCHED_PENALTY
		else
			local childResult = matchChildren(
				session,
				validVChildren,
				studioKids,
				virtualInstances,
				virtualId,
				studioInstance,
				depth + 1
			)
			cost += childResult.totalCost
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
	parentStudioInstance: Instance?,
	_depth: number?
): MatchResult
	local depth = _depth or 0
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
	local matchedCosts: { number? } = {}
	local matchedV: { [number]: boolean } = {}
	local matchedS: { [number]: boolean } = {}

	-- ============================================================
	-- Group by (Name, ClassName) once
	-- ============================================================
	local vByKey: { [string]: { number } } = {}
	for i, id in ipairs(virtualChildren) do
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
	for i, inst in ipairs(studioChildren) do
		local key = inst.Name .. "\0" .. inst.ClassName
		local group = sByKey[key]
		if not group then
			group = {}
			sByKey[key] = group
		end
		table.insert(group, i)
	end

	-- ============================================================
	-- 1:1 instant match + ambiguous scoring in single pass
	-- ============================================================
	for key, vIndices in pairs(vByKey) do
		local sIndices = sByKey[key]
		if not sIndices then
			continue
		end

		if #vIndices == 1 and #sIndices == 1 then
			local vi, si = vIndices[1], sIndices[1]
			table.insert(matched, {
				virtualId = virtualChildren[vi],
				studioInstance = studioChildren[si],
			})
			table.insert(matchedCosts, nil)
			matchedV[vi] = true
			matchedS[si] = true
			continue
		end

		local firstVInst = virtualInstances[virtualChildren[vIndices[1]]]
		if not firstVInst then
			continue
		end
		local classKeys = RbxDom.getClassComparisonKeys(firstVInst.ClassName)

		local vCaches: { [number]: VCache } = {}
		local allExtraProps: { [string]: boolean } = {}
		local allRefProps: { [string]: boolean } = {}
		for _, vi in ipairs(vIndices) do
			local vid = virtualChildren[vi]
			local vInst = virtualInstances[vid]
			if not vInst then
				continue
			end
			local vCache = session.vCacheStore[vid]
			if not vCache then
				vCache = cacheVirtual(vInst, classKeys, virtualInstances)
				session.vCacheStore[vid] = vCache
			end
			vCaches[vi] = vCache
			for _, propName in ipairs(vCache.extraProps) do
				allExtraProps[propName] = true
			end
			for propName, _ in pairs(vCache.refs) do
				allRefProps[propName] = true
			end
		end

		local extraPropNamesArray: { string } = {}
		for propName, _ in pairs(allExtraProps) do
			table.insert(extraPropNamesArray, propName)
		end

		local refPropNamesArray: { string } = {}
		for propName, _ in pairs(allRefProps) do
			table.insert(refPropNamesArray, propName)
		end

		local sCaches: { [number]: SCache } = {}
		for _, si in ipairs(sIndices) do
			sCaches[si] = cacheStudio(studioChildren[si], classKeys, extraPropNamesArray, refPropNamesArray)
		end

		-- Build cost matrix for the Hungarian algorithm.
		local m = #vIndices
		local n = #sIndices
		local costMatrix: { { number } } = table.create(m)

		for ri, vi in ipairs(vIndices) do
			local vCache = vCaches[vi]
			local row = table.create(n, UNMATCHED_PENALTY)
			if vCache then
				for ci, si in ipairs(sIndices) do
					local sCache = sCaches[si]
					if not sCache then
						continue
					end

					local cost = countOwnDiffs(vCache, sCache, classKeys)

					if depth < MAX_SCORING_DEPTH then
						local validVChildren = vCache.validChildren
						local studioKids = sCache.children

						if #validVChildren > 0 or #studioKids > 0 then
							if #validVChildren == 0 then
								cost += #studioKids * UNMATCHED_PENALTY
							elseif #studioKids == 0 then
								cost += #validVChildren * UNMATCHED_PENALTY
							else
								local childResult = matchChildren(
									session,
									validVChildren,
									studioKids,
									virtualInstances,
									virtualChildren[vi],
									sCache.instance,
									depth + 1
								)
								cost += childResult.totalCost
							end
						end
					end

					row[ci] = cost
				end
			end
			costMatrix[ri] = row
		end

		-- Optimal assignment via the Hungarian algorithm.
		local assignment = minCostAssignment(costMatrix, m, n)
		for _, pair in ipairs(assignment) do
			local vi = vIndices[pair[1]]
			local si = sIndices[pair[2]]
			local cost = costMatrix[pair[1]][pair[2]]
			table.insert(matched, {
				virtualId = virtualChildren[vi],
				studioInstance = studioChildren[si],
			})
			table.insert(matchedCosts, cost)
			matchedV[vi] = true
			matchedS[si] = true
		end
	end

	-- ============================================================
	-- Build unmatched lists + compute totalCost
	-- ============================================================
	local unmatchedVirtual: { string } = {}
	for i, id in ipairs(virtualChildren) do
		if not matchedV[i] then
			table.insert(unmatchedVirtual, id)
		end
	end

	local unmatchedStudio: { Instance } = {}
	for i, inst in ipairs(studioChildren) do
		if not matchedS[i] then
			table.insert(unmatchedStudio, inst)
		end
	end

	local totalCost = 0
	for i, pair in ipairs(matched) do
		local precomputed = matchedCosts[i]
		if precomputed ~= nil then
			totalCost += precomputed
		else
			totalCost += computePairCost(
				session,
				pair.virtualId,
				pair.studioInstance,
				virtualInstances,
				math.huge,
				depth
			)
		end
	end
	totalCost += (#unmatchedVirtual + #unmatchedStudio) * UNMATCHED_PENALTY

	local result = {
		matched = matched,
		unmatchedVirtual = unmatchedVirtual,
		unmatchedStudio = unmatchedStudio,
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
