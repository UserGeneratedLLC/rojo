local Packages = script.Parent.Parent.Parent.Packages
local Log = require(Packages.Log)
local RbxDom = require(Packages.RbxDom)

return function(instance, propertyName, propertyDescriptor)
	local readSuccess, readResult = propertyDescriptor:read(instance)

	if not readSuccess then
		Log.warn("Could not sync back property {:?}.{}: {}", instance, propertyName, readResult)
		return false, nil
	end

	local dataType = propertyDescriptor.dataType
	local pcallOk, encodeSuccess, encodeResult = pcall(RbxDom.EncodedValue.encode, readResult, dataType)

	if not pcallOk then
		-- Encoder threw an error (e.g., Ref, Region3, SharedString)
		Log.warn("Could not sync back property {:?}.{}: {}", instance, propertyName, encodeSuccess)
		return false, nil
	end

	if not encodeSuccess then
		Log.warn("Could not sync back property {:?}.{}: {}", instance, propertyName, encodeResult)
		return false, nil
	end

	return true, encodeResult
end
