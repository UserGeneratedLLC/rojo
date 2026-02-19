local encodeProperty = require(script.Parent.encodeProperty)
local Helpers = require(script.Parent.encodeHelpers)

-- ClockTime and TimeOfDay on Lighting are linked properties representing
-- the same value (float vs string). Exclude ClockTime so only TimeOfDay
-- is sent, avoiding redundant/conflicting representations.
local EXCLUDE_PROPERTIES = {
	Lighting = { ClockTime = true },
}

return function(service: Instance)
	local properties = {}
	local refs = {}
	local children = service:GetChildren()
	local refTargets = {}

	local classExcludes = EXCLUDE_PROPERTIES[service.ClassName]
	Helpers.forEachEncodableProperty(service.ClassName, classExcludes, function(propertyName, descriptor)
		if descriptor.dataType == "Ref" then
			local readOk, target = descriptor:read(service)
			if readOk and target then
				local carrier = Instance.new("ObjectValue")
				carrier.Name = propertyName
				carrier.Value = target
				table.insert(refTargets, carrier)
				refs[propertyName] = #refTargets
			end
			return
		end

		local encodeOk, encoded = encodeProperty(service, propertyName, descriptor)
		if encodeOk and encoded ~= nil then
			properties[propertyName] = encoded
		end
	end)

	Helpers.encodeAttributes(service, properties)
	Helpers.encodeTags(service, properties)

	local chunk = {
		className = service.ClassName,
		childCount = #children,
		refTargetCount = #refTargets,
	}
	if next(properties) then
		chunk.properties = properties
	end
	if next(refs) then
		chunk.refs = refs
	end
	return chunk, refTargets
end
