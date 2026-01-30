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

-- Script class names that need Source property
local SCRIPT_CLASS_NAMES = {
	Script = true,
	LocalScript = true,
	ModuleScript = true,
}

-- Properties to skip when encoding (internal Roblox properties)
local SKIP_PROPERTIES = {
	Parent = true,
	Name = true, -- We handle this separately
	Archivable = true,
}

-- Classes that become directories but may have properties (like Attributes)
local DIRECTORY_CLASSES = {
	Folder = true,
	Configuration = true,
	Tool = true,
	ScreenGui = true,
	SurfaceGui = true,
	BillboardGui = true,
	AdGui = true,
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

-- Track skipped duplicates for logging
local skippedDuplicateCount = 0

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

encodeInstance = function(instance, parentId, _skipPathCheck)
	-- Check if the entire path to this instance is unique (unless we're in a recursive call)
	-- For top-level instances, we check the entire ancestor chain for duplicates
	-- For children, the parent already verified the path is unique up to that point,
	-- so we only need to check siblings at each level (done in the recursion below)
	if not _skipPathCheck and not isPathUnique(instance) then
		skippedDuplicateCount += 1
		Log.warn(
			"Skipped instance '{}' ({}) - path contains duplicate-named siblings (cannot reliably sync)",
			instance:GetFullName(),
			instance.ClassName
		)
		return nil
	end

	local properties = {}

	-- Always try to encode Attributes for any instance type
	encodeAttributes(instance, properties)

	if SCRIPT_CLASS_NAMES[instance.ClassName] then
		-- For scripts, encode the Source property (required)
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

		-- Also encode other script properties (Disabled, LinkedSource, etc.)
		-- These go into the .meta.json5 file
		local classDescriptor = RbxDom.findClassDescriptor(instance.ClassName)
		if classDescriptor then
			for propertyName, propertyDescriptor in classDescriptor.properties do
				-- Skip properties we handle separately or should ignore
				if SKIP_PROPERTIES[propertyName] then
					continue
				end
				if propertyName == "Source" or propertyName == "Attributes" then
					continue
				end

				-- Only encode serializable properties
				if propertyDescriptor.scriptability == "ReadWrite" or propertyDescriptor.scriptability == "Read" then
					local encodeSuccess, encodeResult = encodeProperty(instance, propertyName, propertyDescriptor)
					if encodeSuccess and encodeResult ~= nil then
						properties[propertyName] = encodeResult
					end
				end
			end
		end
	elseif not DIRECTORY_CLASSES[instance.ClassName] then
		-- For non-directory, non-script classes, encode all relevant properties
		local classDescriptor = RbxDom.findClassDescriptor(instance.ClassName)
		if classDescriptor then
			for propertyName, propertyDescriptor in classDescriptor.properties do
				if SKIP_PROPERTIES[propertyName] then
					continue
				end

				-- Skip Attributes here since we handle it separately above
				if propertyName == "Attributes" then
					continue
				end

				-- Only encode serializable properties
				if propertyDescriptor.scriptability == "ReadWrite" or propertyDescriptor.scriptability == "Read" then
					local encodeSuccess, encodeResult = encodeProperty(instance, propertyName, propertyDescriptor)
					if encodeSuccess and encodeResult ~= nil then
						properties[propertyName] = encodeResult
					end
				end
			end
		end
	end
	-- For directory classes (Folder, Configuration, etc.), we only encode Attributes (done above)

	-- Recursively encode children, skipping those with duplicate names
	local children = {}
	local childInstances = instance:GetChildren()
	local childDuplicates = findDuplicateNames(childInstances)

	for _, child in ipairs(childInstances) do
		-- Skip children with duplicate names
		if childDuplicates[child.Name] then
			skippedDuplicateCount += 1
			Log.warn(
				"Skipped child instance '{}' ({}) - has duplicate-named siblings (cannot reliably sync)",
				child:GetFullName(),
				child.ClassName
			)
			continue
		end

		-- Pass true to skip duplicate check since we already checked above
		local encodedChild = encodeInstance(child, nil, true)
		if encodedChild then
			table.insert(children, encodedChild)
		end
	end

	return {
		parent = parentId,
		name = instance.Name,
		className = instance.ClassName,
		properties = properties,
		children = children, -- Include children array (may be empty)
	}
end

-- Wrapper function that resets the duplicate count and returns the final count
local function encodeInstanceWithDuplicateTracking(instance, parentId)
	skippedDuplicateCount = 0
	local result = encodeInstance(instance, parentId, false)

	-- Log summary if any duplicates were skipped
	if skippedDuplicateCount > 0 then
		Log.warn(
			"Skipped {} location(s) with duplicate-named siblings during pull (cannot reliably sync)",
			skippedDuplicateCount
		)
	end

	return result, skippedDuplicateCount
end

return encodeInstanceWithDuplicateTracking
