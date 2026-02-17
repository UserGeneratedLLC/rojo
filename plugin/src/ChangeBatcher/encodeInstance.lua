--[[
	Encodes a Roblox Studio instance into a format suitable for transmission
	to the Rojo server for syncback (creating files from Studio instances).
	
	Supports ALL instance types:
	- Scripts (ModuleScript, Script, LocalScript) -> .luau files or directories with init files if has children
	- Folders -> directories (with Attributes if present)
	- All other instances -> .model.json files or directories
	
	Children are recursively encoded and included.
	
	DUPLICATE HANDLING:
	- Instances with duplicate-named siblings at ANY level of the path are skipped
	- This matches the behavior in ref_properties.rs::is_path_unique
	- A path is ambiguous if duplicates exist anywhere from the instance to the root
	- A warning is logged when duplicates are encountered
]]

local Packages = script.Parent.Parent.Parent.Packages
local Log = require(Packages.Log)
local RbxDom = require(Packages.RbxDom)

local encodeProperty = require(script.Parent.encodeProperty)
local UNENCODABLE_DATA_TYPES = require(script.Parent.propertyFilter)

-- Script class names that need Source property
local SCRIPT_CLASS_NAMES = {
	Script = true,
	LocalScript = true,
	ModuleScript = true,
}

-- Properties to always skip when encoding (internal Roblox properties that never serialize)
local SKIP_PROPERTIES = {
	Parent = true,
	Name = true, -- We handle this separately
	Archivable = true,
}

-- Properties to skip per class (matches filter_out_property in Rust syncback)
-- These are properties whose values are encoded in the file itself, not metadata
local SKIP_PROPERTIES_BY_CLASS = {
	Script = {
		Source = true, -- Encoded in the .luau file
		ScriptGuid = true, -- Internal
		RunContext = true, -- Encoded in file suffix (.server.luau, .client.luau, etc.)
	},
	LocalScript = {
		Source = true, -- Encoded in the .luau file
		ScriptGuid = true, -- Internal
	},
	ModuleScript = {
		Source = true, -- Encoded in the .luau file
		ScriptGuid = true, -- Internal
	},
	LocalizationTable = {
		Contents = true, -- Encoded in the .csv file
	},
	StringValue = {
		Value = true, -- Encoded in the .txt file
	},
}

-- Finds duplicate names among a list of instances
-- Returns a set of names that appear more than once
local function findDuplicateNames(children: { Instance }): { [string]: boolean }
	local nameCounts: { [string]: number } = {}
	local duplicates: { [string]: boolean } = {}

	for _, child in children do
		local name = child.Name
		nameCounts[name] = (nameCounts[name] or 0) + 1
		if nameCounts[name] > 1 then
			duplicates[name] = true
		end
	end

	return duplicates
end

-- Checks if an instance has duplicate-named siblings
local function hasDuplicateSiblings(instance: Instance): boolean
	local parent = instance.Parent
	if not parent then
		return false
	end

	local siblings = parent:GetChildren()
	local duplicates = findDuplicateNames(siblings)
	return duplicates[instance.Name] == true
end

-- Checks if the path to an instance is unique by walking up the tree.
-- Returns false if ANY ancestor (including the instance itself) has duplicate siblings.
-- This matches the behavior of is_path_unique in ref_properties.rs
local function isPathUnique(instance: Instance): boolean
	local current = instance

	while current do
		-- Check if current has duplicate-named siblings
		if hasDuplicateSiblings(current) then
			return false
		end

		-- Move up to parent
		current = current.Parent

		-- Stop at game (DataModel) - we've reached the root
		if current and current == game then
			break
		end
	end

	return true
end

-- Forward declaration for recursion
local encodeInstance

-- Encode Attributes if present on any instance
local function encodeAttributes(instance, properties)
	-- Try to get Attributes - this works for all instance types
	local success, attributes = pcall(function()
		return instance:GetAttributes()
	end)

	if success and attributes and next(attributes) then
		-- Attributes need to be encoded specially
		-- The server expects them in the properties map under "Attributes"
		local attributeDescriptor = RbxDom.findCanonicalPropertyDescriptor(instance.ClassName, "Attributes")
		if attributeDescriptor then
			local encodeSuccess, encodeResult = encodeProperty(instance, "Attributes", attributeDescriptor)
			if encodeSuccess and encodeResult ~= nil then
				properties.Attributes = encodeResult
			end
		end
	end
end

-- Encode Tags if present on any instance
local function encodeTags(instance, properties)
	-- Try to get Tags - this works for all instance types
	local success, tags = pcall(function()
		return instance:GetTags()
	end)

	if success and tags and #tags > 0 then
		-- Tags need to be encoded specially
		local tagDescriptor = RbxDom.findCanonicalPropertyDescriptor(instance.ClassName, "Tags")
		if tagDescriptor then
			local encodeSuccess, encodeResult = encodeProperty(instance, "Tags", tagDescriptor)
			if encodeSuccess and encodeResult ~= nil then
				properties.Tags = encodeResult
			end
		end
	end
end

encodeInstance = function(instance, parentId, _skipPathCheck)
	-- Log duplicate-named siblings as a debug message (server handles them via
	-- dedup suffixes) but do NOT skip encoding.
	if not _skipPathCheck and not isPathUnique(instance) then
		Log.debug(
			"Instance '{}' ({}) has duplicate-named siblings in path (server will handle via dedup)",
			instance:GetFullName(),
			instance.ClassName
		)
	end

	-- Log encoding at trace level (very detailed)
	if not _skipPathCheck then
		Log.trace("Encoding for syncback: {} ({})", instance:GetFullName(), instance.ClassName)
	end

	local properties = {}

	-- Always try to encode Attributes and Tags for any instance type
	encodeAttributes(instance, properties)
	encodeTags(instance, properties)

	-- For scripts, encode the Source property first (required for script files)
	if SCRIPT_CLASS_NAMES[instance.ClassName] then
		local sourceDescriptor = RbxDom.findCanonicalPropertyDescriptor(instance.ClassName, "Source")
		if sourceDescriptor then
			local encodeSuccess, encodeResult = encodeProperty(instance, "Source", sourceDescriptor)
			if encodeSuccess then
				properties.Source = encodeResult
			else
				Log.warn("Failed to encode Source property for {:?}", instance)
				return nil
			end
		else
			Log.warn("Could not find Source property descriptor for {:?}", instance)
			return nil
		end
	end

	-- Encode ALL properties for ANY instance type (matching Rust syncback behavior)
	-- The server handles filtering defaults and serialization
	local classDescriptor = RbxDom.findClassDescriptor(instance.ClassName)
	if classDescriptor then
		-- Get class-specific properties to skip
		local classSkipProps = SKIP_PROPERTIES_BY_CLASS[instance.ClassName] or {}

		for propertyName, propertyMeta in pairs(classDescriptor.properties) do
			-- Skip universally skipped properties
			if SKIP_PROPERTIES[propertyName] then
				continue
			end

			-- Skip class-specific properties (Source, RunContext for scripts, etc.)
			if classSkipProps[propertyName] then
				continue
			end

			-- Skip Attributes and Tags since we handle them separately above
			if propertyName == "Attributes" or propertyName == "Tags" then
				continue
			end

			-- Only encode properties that:
			-- 1. Are readable (ReadWrite or Read scriptability)
			-- 2. Actually serialize (not "DoesNotSerialize")
			local isReadable = propertyMeta.scriptability == "ReadWrite" or propertyMeta.scriptability == "Read"
			local doesSerialize = propertyMeta.serialization ~= "DoesNotSerialize"

			if isReadable and doesSerialize then
				-- Get the full PropertyDescriptor for encoding
				local descriptor = RbxDom.findCanonicalPropertyDescriptor(instance.ClassName, propertyName)
				if descriptor then
					if UNENCODABLE_DATA_TYPES[descriptor.dataType] then
						continue
					end

					-- Ref properties cannot be encoded during instance addition
					-- because the target instance has no server ID yet. They are
					-- handled separately by encodePatchUpdate.lua during property
					-- update encoding via the InstanceMap.
					if descriptor.dataType == "Ref" then
						continue
					end

					local encodeSuccess, encodeResult = encodeProperty(instance, propertyName, descriptor)
					if encodeSuccess and encodeResult ~= nil then
						properties[propertyName] = encodeResult
					end
				end
			end
		end
	end

	-- Recursively encode children (including those with duplicate names;
	-- the server handles duplicates via dedup suffix system)
	local children = {}
	local childInstances = instance:GetChildren()

	for _, child in ipairs(childInstances) do
		local encodedChild = encodeInstance(child, nil, true)
		if encodedChild then
			table.insert(children, encodedChild)
		end
	end

	-- Log property count for debugging
	local propCount = 0
	for _ in pairs(properties) do
		propCount += 1
	end
	Log.trace("  Encoded {} with {} properties, {} children", instance.Name, propCount, #children)

	return {
		parent = parentId,
		name = instance.Name,
		className = instance.ClassName,
		properties = properties,
		children = children, -- Include children array (may be empty)
	}
end

return function(instance, parentId)
	return encodeInstance(instance, parentId, false)
end
