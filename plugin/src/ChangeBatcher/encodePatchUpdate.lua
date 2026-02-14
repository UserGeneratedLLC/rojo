local Packages = script.Parent.Parent.Parent.Packages
local Log = require(Packages.Log)
local RbxDom = require(Packages.RbxDom)

local encodeProperty = require(script.Parent.encodeProperty)
local UNENCODABLE_DATA_TYPES = require(script.Parent.propertyFilter)

local NULL_REF = "00000000000000000000000000000000"

return function(instance, instanceId, properties, instanceMap)
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

			-- Ref properties are encoded using the InstanceMap to resolve
			-- Studio Instance references to server-side Ref IDs.
			if descriptor.dataType == "Ref" then
				local readSuccess, readResult = descriptor:read(instance)
				if not readSuccess then
					Log.debug("Could not read Ref property {:?}.{}: {}", instance, propertyName, readResult)
					continue
				end

				if readResult == nil then
					-- Nil ref: encode as null ref
					update.changedProperties[propertyName] = { Ref = NULL_REF }
				elseif instanceMap then
					local targetId = instanceMap.fromInstances[readResult]
					if targetId then
						update.changedProperties[propertyName] = { Ref = targetId }
					else
						Log.warn(
							"Cannot sync Ref property {:?}.{}: target {:?} is not tracked by Atlas",
							instance,
							propertyName,
							readResult
						)
					end
				else
					Log.warn("Cannot encode Ref property {:?}.{}: no InstanceMap provided", instance, propertyName)
				end
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
