--[[
	Computes structured change metadata from a changeList.

	Produces both summary fields (for display tags and diff bar) and
	detail fields (for future merge dialogue). Replaces the ad-hoc
	inline metadata computation that was previously in PatchTree.build().
]]

local Rojo = script:FindFirstAncestor("Rojo")
local Plugin = Rojo.Plugin

local DiffUtil = require(Plugin.DiffUtil)

local ChangeMetadata = {}

--[[
	Compute metadata for an updated instance's changeList.
	changeList is an array of {prop, currentValue, incomingValue} entries
	(without the header row -- that is added separately by PatchTree).

	Returns a ChangeMetadata table with:
	  Summary (for display):
	    linesAdded, linesRemoved, isWhitespaceOnly, propChanges
	  Detail (for future merge dialogue):
	    hunks, propertyDetails
	  Tracking:
	    edits, failed (set later by updateMetadata)
]]
function ChangeMetadata.compute(changeList: { { any } }): { [string]: any }
	local linesAdded: number? = nil
	local linesRemoved: number? = nil
	local isWhitespaceOnly: boolean? = nil
	local hunks: { any }? = nil
	local propertyDetails: { any }? = nil
	local propCount = 0
	local totalEdits = #changeList

	for _, entry in changeList do
		local prop = entry[1]
		local currentValue = entry[2]
		local incomingValue = entry[3]

		if prop == "Source" and type(currentValue) == "string" and type(incomingValue) == "string" then
			-- Source property: compute line diff (always exact, no thresholds)
			local diffResult = DiffUtil.diffLines(currentValue, incomingValue)

			if diffResult.added > 0 or diffResult.removed > 0 then
				linesAdded = diffResult.added
				linesRemoved = diffResult.removed
				isWhitespaceOnly = diffResult.isWhitespaceOnly
				hunks = diffResult.hunks
			end
		else
			-- Non-Source property
			propCount += 1

			-- Build property detail for future merge dialogue
			if not propertyDetails then
				propertyDetails = {}
			end

			table.insert(propertyDetails, {
				name = prop,
				oldValue = currentValue,
				newValue = incomingValue,
				valueType = typeof(incomingValue) or typeof(currentValue) or "unknown",
			})
		end
	end

	return {
		-- Summary (for display tags and diff bar)
		linesAdded = linesAdded,
		linesRemoved = linesRemoved,
		isWhitespaceOnly = isWhitespaceOnly,
		propChanges = if propCount > 0 then propCount else nil,

		-- Tracking (edits used by added instances and failure tracking)
		edits = totalEdits,

		-- Detail (for future merge dialogue -- not consumed by UI yet)
		hunks = hunks,
		propertyDetails = propertyDetails,
	}
end

--[[
	Compute metadata for an added instance's changeList.
	These are all new properties, so there are no "current" values.
	changeList entries are {prop, "N/A", incomingValue}.
]]
function ChangeMetadata.computeForAddition(changeList: { { any } }): { [string]: any }
	local linesAdded: number? = nil
	local hunks: { any }? = nil
	local propertyDetails: { any }? = nil

	for _, entry in changeList do
		local prop = entry[1]
		local incomingValue = entry[3]

		if prop == "Source" and type(incomingValue) == "string" then
			-- New script: count lines being added
			local diffResult = DiffUtil.diffLines("", incomingValue)
			if diffResult.added > 0 then
				linesAdded = diffResult.added
				hunks = diffResult.hunks
			end
		end

		if not propertyDetails then
			propertyDetails = {}
		end

		table.insert(propertyDetails, {
			name = prop,
			oldValue = nil,
			newValue = incomingValue,
			valueType = typeof(incomingValue) or "unknown",
		})
	end

	return {
		edits = #changeList,
		linesAdded = if linesAdded then linesAdded else nil,
		linesRemoved = if linesAdded then 0 else nil,
		isWhitespaceOnly = false,
		hunks = hunks,
		propertyDetails = propertyDetails,
	}
end

--[[
	Compute metadata for a removed instance.
	Reads the Source property (if it exists) to show line counts.
	Returns nil if the instance has no Source (non-script removal).
]]
function ChangeMetadata.computeForRemoval(instance: Instance): { [string]: any }?
	local ok, source = pcall(function()
		return (instance :: any).Source
	end)

	if not ok or type(source) ~= "string" then
		return nil
	end

	local diffResult = DiffUtil.diffLines(source, "")

	if diffResult.removed == 0 then
		return nil
	end

	return {
		linesAdded = 0,
		linesRemoved = diffResult.removed,
		isWhitespaceOnly = false,
	}
end

return ChangeMetadata
