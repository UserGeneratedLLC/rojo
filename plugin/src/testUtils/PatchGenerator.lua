--[[
	Generates patches for stress testing.
	
	Supports:
	- Random valid patches
	- Edge case patches (all adds, all removes, etc.)
	- Large batch patches
	- Invalid/malformed patches for error testing
]]

local HttpService = game:GetService("HttpService")

local PatchGenerator = {}

-- Common class names for generating patches
local CLASS_NAMES = {
	"Folder",
	"ModuleScript",
	"Script",
	"LocalScript",
	"Part",
	"Model",
	"StringValue",
	"NumberValue",
	"BoolValue",
}

-- Invalid class names for error testing
local INVALID_CLASS_NAMES = {
	"NotARealClass",
	"FakeInstance",
	"",
	"123Invalid",
	"Class With Spaces",
}

--[[
	Generate a unique ID
]]
function PatchGenerator.generateId()
	return HttpService:GenerateGUID(false)
end

--[[
	Create an empty patch
]]
function PatchGenerator.empty()
	return {
		removed = {},
		added = {},
		updated = {},
	}
end

--[[
	Create a patch with only additions.
	
	Options:
	- count: Number of instances to add (default: 10)
	- parentId: Parent ID for all additions (required)
	- className: Class to use or "mixed" (default: "Folder")
	- withChildren: Whether to add children to each instance (default: false)
	- childCount: Number of children per instance (default: 3)
	
	Returns: { patch, addedIds }
]]
function PatchGenerator.createAdditionsPatch(options)
	options = options or {}
	local count = options.count or 10
	local parentId = options.parentId
	local className = options.className or "Folder"
	local withChildren = options.withChildren or false
	local childCount = options.childCount or 3

	assert(parentId, "parentId is required for additions")

	local patch = PatchGenerator.empty()
	local addedIds = {}

	for i = 1, count do
		local id = PatchGenerator.generateId()
		local actualClassName = className
		if className == "mixed" then
			actualClassName = CLASS_NAMES[math.random(1, #CLASS_NAMES)]
		end

		local children = {}
		if withChildren then
			for j = 1, childCount do
				local childId = PatchGenerator.generateId()
				local childClassName = className == "mixed" and CLASS_NAMES[math.random(1, #CLASS_NAMES)] or "Folder"

				patch.added[childId] = {
					Id = childId,
					ClassName = childClassName,
					Name = "Child_" .. tostring(i) .. "_" .. tostring(j),
					Parent = id,
					Properties = {},
					Children = {},
				}
				table.insert(children, childId)
				table.insert(addedIds, childId)
			end
		end

		patch.added[id] = {
			Id = id,
			ClassName = actualClassName,
			Name = "Added_" .. tostring(i),
			Parent = parentId,
			Properties = {},
			Children = children,
		}
		table.insert(addedIds, id)
	end

	return patch, addedIds
end

--[[
	Create a patch with only removals.
	
	Options:
	- ids: List of IDs to remove (required)
	
	Returns: patch
]]
function PatchGenerator.createRemovalsPatch(options)
	options = options or {}
	local ids = options.ids

	assert(ids and #ids > 0, "ids are required for removals")

	local patch = PatchGenerator.empty()
	for _, id in ipairs(ids) do
		table.insert(patch.removed, id)
	end

	return patch
end

--[[
	Create a patch with only updates.
	
	Options:
	- ids: List of IDs to update (required)
	- changeNames: Whether to change names (default: false)
	- changeProperties: Whether to change properties (default: true)
	- properties: Specific properties to change (default: auto-detect)
	
	Returns: patch
]]
function PatchGenerator.createUpdatesPatch(options)
	options = options or {}
	local ids = options.ids
	local changeNames = options.changeNames or false
	local changeProperties = options.changeProperties ~= false

	assert(ids and #ids > 0, "ids are required for updates")

	local patch = PatchGenerator.empty()

	for i, id in ipairs(ids) do
		local update = {
			id = id,
			changedProperties = {},
		}

		if changeNames then
			update.changedName = "Updated_" .. tostring(i)
		end

		if changeProperties then
			-- Add a generic property change
			update.changedProperties.TestProperty = { String = "Updated value " .. tostring(i) }
		end

		table.insert(patch.updated, update)
	end

	return patch
end

--[[
	Create a mixed patch with adds, removes, and updates.
	
	Options:
	- addCount: Number of additions (default: 5)
	- removeIds: IDs to remove (default: {})
	- updateIds: IDs to update (default: {})
	- parentId: Parent for additions (required if addCount > 0)
	
	Returns: patch
]]
function PatchGenerator.createMixedPatch(options)
	options = options or {}
	local addCount = options.addCount or 5
	local removeIds = options.removeIds or {}
	local updateIds = options.updateIds or {}
	local parentId = options.parentId

	local patch = PatchGenerator.empty()

	-- Add additions
	if addCount > 0 then
		assert(parentId, "parentId required for additions")
		local addPatch = PatchGenerator.createAdditionsPatch({
			count = addCount,
			parentId = parentId,
		})
		for id, added in pairs(addPatch.added) do
			patch.added[id] = added
		end
	end

	-- Add removals
	for _, id in ipairs(removeIds) do
		table.insert(patch.removed, id)
	end

	-- Add updates
	if #updateIds > 0 then
		local updatePatch = PatchGenerator.createUpdatesPatch({ ids = updateIds })
		for _, update in ipairs(updatePatch.updated) do
			table.insert(patch.updated, update)
		end
	end

	return patch
end

--[[
	Create a patch with invalid class names for error testing.
	
	Options:
	- count: Number of invalid instances (default: 5)
	- parentId: Parent ID (required)
	
	Returns: patch
]]
function PatchGenerator.createInvalidClassPatch(options)
	options = options or {}
	local count = options.count or 5
	local parentId = options.parentId

	assert(parentId, "parentId is required")

	local patch = PatchGenerator.empty()

	for i = 1, count do
		local id = PatchGenerator.generateId()
		local invalidClassName = INVALID_CLASS_NAMES[((i - 1) % #INVALID_CLASS_NAMES) + 1]

		patch.added[id] = {
			Id = id,
			ClassName = invalidClassName,
			Name = "Invalid_" .. tostring(i),
			Parent = parentId,
			Properties = {},
			Children = {},
		}
	end

	return patch
end

--[[
	Create a patch with ref properties.
	
	Options:
	- parentId: Parent for new instances (required)
	- refTargetId: ID that refs should point to (required)
	- count: Number of instances with refs (default: 5)
	- invalidRefs: Whether to use invalid ref targets (default: false)
	
	Returns: patch
]]
function PatchGenerator.createRefPatch(options)
	options = options or {}
	local parentId = options.parentId
	local refTargetId = options.refTargetId
	local count = options.count or 5
	local invalidRefs = options.invalidRefs or false

	assert(parentId, "parentId is required")

	local patch = PatchGenerator.empty()

	for i = 1, count do
		local id = PatchGenerator.generateId()
		local targetRef = refTargetId

		if invalidRefs then
			targetRef = PatchGenerator.generateId() -- Non-existent ID
		end

		patch.added[id] = {
			Id = id,
			ClassName = "ObjectValue",
			Name = "RefHolder_" .. tostring(i),
			Parent = parentId,
			Properties = {
				Value = { Ref = targetRef },
			},
			Children = {},
		}
	end

	return patch
end

--[[
	Create a patch with className changes.
	
	Options:
	- id: ID of instance to change (required)
	- newClassName: New class name (default: "Model")
	- withProperties: Properties to set on new class (default: {})
	
	Returns: patch
]]
function PatchGenerator.createClassNameChangePatch(options)
	options = options or {}
	local id = options.id
	local newClassName = options.newClassName or "Model"
	local withProperties = options.withProperties or {}

	assert(id, "id is required")

	local patch = PatchGenerator.empty()

	table.insert(patch.updated, {
		id = id,
		changedClassName = newClassName,
		changedProperties = withProperties,
	})

	return patch
end

--[[
	Create a large batch patch for stress testing.
	
	Options:
	- addCount: Number of additions (default: 500)
	- updateCount: Number of updates (default: 500)
	- parentId: Parent for additions (required)
	- existingIds: IDs that exist for updates (required if updateCount > 0)
	
	Returns: patch
]]
function PatchGenerator.createLargeBatchPatch(options)
	options = options or {}
	local addCount = options.addCount or 500
	local updateCount = options.updateCount or 500
	local parentId = options.parentId
	local existingIds = options.existingIds or {}

	local patch = PatchGenerator.empty()

	-- Create additions
	if addCount > 0 then
		assert(parentId, "parentId required for additions")
		for i = 1, addCount do
			local id = PatchGenerator.generateId()
			patch.added[id] = {
				Id = id,
				ClassName = "Folder",
				Name = "BatchAdd_" .. tostring(i),
				Parent = parentId,
				Properties = {},
				Children = {},
			}
		end
	end

	-- Create updates
	local updateableIds = existingIds
	for i = 1, math.min(updateCount, #updateableIds) do
		table.insert(patch.updated, {
			id = updateableIds[i],
			changedName = "BatchUpdated_" .. tostring(i),
			changedProperties = {},
		})
	end

	return patch
end

--[[
	Create a patch with nested additions (parent and children in same patch).
	
	Options:
	- rootParentId: ID of existing parent (required)
	- depth: How deep to nest (default: 5)
	- width: Children per level (default: 3)
	
	Returns: { patch, rootAddedId }
]]
function PatchGenerator.createNestedAdditionsPatch(options)
	options = options or {}
	local rootParentId = options.rootParentId
	local depth = options.depth or 5
	local width = options.width or 3

	assert(rootParentId, "rootParentId is required")

	local patch = PatchGenerator.empty()

	local function addLevel(parentId, currentDepth, prefix)
		if currentDepth > depth then
			return {}
		end

		local children = {}
		for i = 1, width do
			local id = PatchGenerator.generateId()
			local childPrefix = prefix .. "_" .. tostring(i)
			local grandchildren = addLevel(id, currentDepth + 1, childPrefix)

			patch.added[id] = {
				Id = id,
				ClassName = "Folder",
				Name = childPrefix,
				Parent = parentId,
				Properties = {},
				Children = grandchildren,
			}

			table.insert(children, id)
		end

		return children
	end

	local rootId = PatchGenerator.generateId()
	local children = addLevel(rootId, 1, "Nested")

	patch.added[rootId] = {
		Id = rootId,
		ClassName = "Folder",
		Name = "NestedRoot",
		Parent = rootParentId,
		Properties = {},
		Children = children,
	}

	return patch, rootId
end

--[[
	Create a patch with property value edge cases.
	
	Options:
	- id: ID to update (required)
	
	Returns: patch with various edge case property values
]]
function PatchGenerator.createEdgeCasePropertiesPatch(options)
	options = options or {}
	local id = options.id

	assert(id, "id is required")

	local patch = PatchGenerator.empty()

	table.insert(patch.updated, {
		id = id,
		changedProperties = {
			-- Empty string
			EmptyString = { String = "" },
			-- Very long string
			LongString = { String = string.rep("a", 10000) },
			-- Unicode
			UnicodeString = { String = "Hello 世界 🌍 مرحبا" },
			-- Large number
			LargeNumber = { Float64 = 1e308 },
			-- Small number
			SmallNumber = { Float64 = 1e-308 },
			-- Zero
			Zero = { Float64 = 0 },
			-- Negative
			Negative = { Float64 = -12345.6789 },
		},
	})

	return patch
end

return PatchGenerator
