--[[
	Encodes a Roblox Studio instance into a format suitable for transmission
	to the Rojo server for syncback (creating files from Studio instances).
	
	Supports ALL instance types:
	- Scripts (ModuleScript, Script, LocalScript) -> .luau files or directories with init files if has children
	- Folders -> directories (with Attributes if present)
	- All other instances -> .model.json files or directories
	
	Children are recursively encoded and included.
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

encodeInstance = function(instance, parentId)
	local properties = {}

	-- Always try to encode Attributes for any instance type
	encodeAttributes(instance, properties)

	if SCRIPT_CLASS_NAMES[instance.ClassName] then
		-- For scripts, encode the Source property
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

	-- Recursively encode children
	local children = {}
	for _, child in ipairs(instance:GetChildren()) do
		local encodedChild = encodeInstance(child, nil) -- parentId will be resolved server-side
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

return encodeInstance
