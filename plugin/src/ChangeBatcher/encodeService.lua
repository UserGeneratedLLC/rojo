local SerializationService = game:GetService("SerializationService")

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

	local children = service:GetChildren()
	local data = buffer.create(0)
	if #children > 0 then
		local serializeOk, result = pcall(SerializationService.SerializeInstancesAsync, SerializationService, children)
		if serializeOk then
			data = result
		else
			Log.warn("Failed to serialize children of {}: {}", service.ClassName, tostring(result))
		end
	end

	return {
		className = service.ClassName,
		data = data,
		properties = properties,
		refs = refs,
	}
end
