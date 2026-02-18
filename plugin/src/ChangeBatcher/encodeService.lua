local Packages = script.Parent.Parent.Parent.Packages
local Log = require(Packages.Log)
local RbxDom = require(Packages.RbxDom)

local encodeProperty = require(script.Parent.encodeProperty)
local UNENCODABLE_DATA_TYPES = require(script.Parent.propertyFilter)

local SKIP_PROPERTIES = {
	Parent = true,
	Name = true,
	Archivable = true,
}

return function(service: Instance)
	local properties = {}
	local refs = {}

	local classDescriptor = RbxDom.findClassDescriptor(service.ClassName)
	if classDescriptor then
		for propertyName, propertyMeta in pairs(classDescriptor.properties) do
			if SKIP_PROPERTIES[propertyName] then
				continue
			end
			if propertyName == "Attributes" or propertyName == "Tags" then
				continue
			end

			local isReadable = propertyMeta.scriptability == "ReadWrite"
				or propertyMeta.scriptability == "Read"
			local doesSerialize = propertyMeta.serialization ~= "DoesNotSerialize"

			if isReadable and doesSerialize then
				local descriptor = RbxDom.findCanonicalPropertyDescriptor(service.ClassName, propertyName)
				if descriptor then
					if UNENCODABLE_DATA_TYPES[descriptor.dataType] then
						continue
					end

					if descriptor.dataType == "Ref" then
						local readOk, target = descriptor:read(service)
						if readOk and target and target.Parent == service then
							refs[propertyName] = {
								name = target.Name,
								className = target.ClassName,
							}
						end
						continue
					end

					local encodeOk, encoded = encodeProperty(service, propertyName, descriptor)
					if encodeOk and encoded ~= nil then
						properties[propertyName] = encoded
					end
				end
			end
		end
	end

	local attrOk, attrMap = pcall(function()
		return service:GetAttributes()
	end)
	if attrOk and attrMap and next(attrMap) then
		local descriptor = RbxDom.findCanonicalPropertyDescriptor(service.ClassName, "Attributes")
		if descriptor then
			local encodeOk, encoded = encodeProperty(service, "Attributes", descriptor)
			if encodeOk and encoded ~= nil then
				properties.Attributes = encoded
			end
		end
	end

	local tagOk, tagList = pcall(function()
		return service:GetTags()
	end)
	if tagOk and tagList and #tagList > 0 then
		local descriptor = RbxDom.findCanonicalPropertyDescriptor(service.ClassName, "Tags")
		if descriptor then
			local encodeOk, encoded = encodeProperty(service, "Tags", descriptor)
			if encodeOk and encoded ~= nil then
				properties.Tags = encoded
			end
		end
	end

	local chunk = {
		className = service.ClassName,
		childCount = #service:GetChildren(),
	}
	if next(properties) then
		chunk.properties = properties
	end
	if next(refs) then
		chunk.refs = refs
	end
	return chunk
end
