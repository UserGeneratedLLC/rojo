--[[
	Shared encoding helpers for ChangeBatcher encoders. Consolidates property
	filter logic so encodeInstance.lua and encodeService.lua stay in sync.
]]

local Packages = script.Parent.Parent.Parent.Packages
local RbxDom = require(Packages.RbxDom)

local encodeProperty = require(script.Parent.encodeProperty)
local UNENCODABLE_DATA_TYPES = require(script.Parent.propertyFilter)

local SKIP_PROPERTIES = {
	Parent = true,
	Name = true,
	Archivable = true,
}

local Helpers = {}

function Helpers.encodeAttributes(instance, properties)
	local success, attributes = pcall(function()
		return instance:GetAttributes()
	end)
	if success and attributes and next(attributes) then
		local descriptor = RbxDom.findCanonicalPropertyDescriptor(instance.ClassName, "Attributes")
		if descriptor then
			local ok, result = encodeProperty(instance, "Attributes", descriptor)
			if ok and result ~= nil then
				properties.Attributes = result
			end
		end
	end
end

function Helpers.encodeTags(instance, properties)
	local success, tags = pcall(function()
		return instance:GetTags()
	end)
	if success and tags and #tags > 0 then
		local descriptor = RbxDom.findCanonicalPropertyDescriptor(instance.ClassName, "Tags")
		if descriptor then
			local ok, result = encodeProperty(instance, "Tags", descriptor)
			if ok and result ~= nil then
				properties.Tags = result
			end
		end
	end
end

-- Iterates all encodable properties for a class, applying common filters:
-- SKIP_PROPERTIES, per-class skip map, Attributes/Tags, scriptability,
-- serialization, and UNENCODABLE_DATA_TYPES. Calls callback(propertyName,
-- descriptor) for each property that passes all filters.
function Helpers.forEachEncodableProperty(className, classSkipProps, callback)
	local classDescriptor = RbxDom.findClassDescriptor(className)
	if not classDescriptor then
		return
	end

	for propertyName, propertyMeta in pairs(classDescriptor.properties) do
		if SKIP_PROPERTIES[propertyName] then
			continue
		end
		if classSkipProps and classSkipProps[propertyName] then
			continue
		end
		if propertyName == "Attributes" or propertyName == "Tags" then
			continue
		end

		local isReadable = propertyMeta.scriptability == "ReadWrite"
			or propertyMeta.scriptability == "Read"
			or propertyMeta.scriptability == "Custom"
		local doesSerialize = propertyMeta.serialization ~= "DoesNotSerialize"

		if isReadable and doesSerialize then
			local descriptor = RbxDom.findCanonicalPropertyDescriptor(className, propertyName)
			if descriptor and not UNENCODABLE_DATA_TYPES[descriptor.dataType] then
				callback(propertyName, descriptor)
			end
		end
	end
end

return Helpers
