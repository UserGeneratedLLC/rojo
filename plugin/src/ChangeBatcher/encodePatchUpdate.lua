local Packages = script.Parent.Parent.Parent.Packages
local Log = require(Packages.Log)
local RbxDom = require(Packages.RbxDom)

local encodeProperty = require(script.Parent.encodeProperty)
local UNENCODABLE_DATA_TYPES = require(script.Parent.propertyFilter)

return function(instance, instanceId, properties)
	local update = {
		id = instanceId,
		changedProperties = {},
	}

	for propertyName in properties do
		if propertyName == "Name" then
			update.changedName = instance.Name
		else
			local descriptor = RbxDom.findCanonicalPropertyDescriptor(instance.ClassName, propertyName)

			if not descriptor then
				Log.debug("Could not sync back property {:?}.{}", instance, propertyName)
				continue
			end

			if UNENCODABLE_DATA_TYPES[descriptor.dataType] then
				continue
			end

			local encodeSuccess, encodeResult = encodeProperty(instance, propertyName, descriptor)

			if not encodeSuccess then
				Log.debug("Could not sync back property {:?}.{}: {}", instance, propertyName, encodeResult)
				continue
			end

			update.changedProperties[propertyName] = encodeResult
		end
	end

	if next(update.changedProperties) == nil and update.changedName == nil then
		return nil
	end

	return update
end
