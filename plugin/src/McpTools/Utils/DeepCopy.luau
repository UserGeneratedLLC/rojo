--!strict
--!optimize 2

function DeepCopy<T>(value: T, cache_: { [any]: any }?): T
	if type(value) ~= "table" then
		return value
	end

	local cache: { [any]: any } = cache_ or {}

	local cachedResult = cache[value]
	if cachedResult then
		return cachedResult
	end

	local result = {}
	cache[value] = result

	for k, v in next, value do
		result[DeepCopy(k, cache)] = DeepCopy(v, cache)
	end

	local valueMT = getmetatable(value)
	if valueMT then
		setmetatable(result, valueMT :: any)
	end

	return result :: any
end

return DeepCopy
