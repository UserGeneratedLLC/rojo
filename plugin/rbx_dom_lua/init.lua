local database = require(script.database)
local Error = require(script.Error)
local PropertyDescriptor = require(script.PropertyDescriptor)
local EncodedValue = require(script.EncodedValue)

-- Cache for default properties per class (includes inherited defaults).
local defaultPropertiesCache: { [string]: { [string]: any } } = {}

-- Returns a table mapping property names to their encoded default values
-- for a given class, including inherited defaults from parent classes.
-- Subclass overrides take precedence. Results are cached per class.
local function findDefaultProperties(className: string): { [string]: any }
	local cached = defaultPropertiesCache[className]
	if cached then
		return cached
	end

	local defaults: { [string]: any } = {}
	local currentClassName: string? = className

	repeat
		local currentClass = database.Classes[currentClassName]
		if currentClass == nil then
			break
		end

		local classDefaults = currentClass.DefaultProperties
		if classDefaults then
			for propName, encodedValue in classDefaults do
				if defaults[propName] == nil then
					defaults[propName] = encodedValue
				end
			end
		end

		currentClassName = currentClass.Superclass
	until currentClassName == nil

	defaultPropertiesCache[className] = defaults
	return defaults
end

-- Property names excluded from comparison (handled separately).
local EXCLUDED_PROP_NAMES: { [string]: boolean } = {
	Tags = true,
	Attributes = true,
	Name = true,
}

-- Encoded value type keys that cannot be compared during matching.
local EXCLUDED_ENCODED_TYPES: { [string]: boolean } = {
	Ref = true,
	UniqueId = true,
	SharedString = true,
	Region3 = true,
}

-- Scriptabilities that allow reading from Studio instances.
local READABLE_SCRIPTABILITY: { [string]: boolean } = {
	ReadWrite = true,
	Read = true,
}

-- Cache for class comparison keys (propNames + decoded defaults).
-- Computed once per ClassName, lives for the plugin's lifetime.
local classComparisonKeysCache: { [string]: any } = {}

-- Returns a pre-computed comparison key set for a class:
--   propNames:   ordered array of comparable property names
--   propNameSet: O(1) lookup set of the same names
--   defaults:    map of propName → decoded native default value
--
-- Filtered to: canonical, scriptable (readable), serializable, and
-- decodable. Excludes Tags, Attributes, Ref, UniqueId, SharedString.
-- All defaults are pre-decoded to native Roblox values.
local function getClassComparisonKeys(className: string)
	local cached = classComparisonKeysCache[className]
	if cached then
		return cached
	end

	local propNames: { string } = {}
	local propNameSet: { [string]: boolean } = {}
	local seenProps: { [string]: boolean } = {}

	-- Walk the class hierarchy for property metadata
	local currentClassName: string? = className
	repeat
		local currentClass = database.Classes[currentClassName]
		if currentClass == nil then
			break
		end

		for propertyName, propertyData in currentClass.Properties do
			if seenProps[propertyName] then
				continue
			end
			seenProps[propertyName] = true

			if EXCLUDED_PROP_NAMES[propertyName] then
				continue
			end
			if not propertyData.Kind or not propertyData.Kind.Canonical then
				continue
			end
			if not READABLE_SCRIPTABILITY[propertyData.Scriptability] then
				continue
			end
			if propertyData.Kind.Canonical.Serialization == "DoesNotSerialize" then
				continue
			end

			table.insert(propNames, propertyName)
			propNameSet[propertyName] = true
		end

		currentClassName = currentClass.Superclass
	until currentClassName == nil

	-- Pre-decode defaults for every comparable property
	local defaults: { [string]: any } = {}
	local encodedDefaults = findDefaultProperties(className)
	for _, propName in propNames do
		local encoded = encodedDefaults[propName]
		if encoded then
			local ty = next(encoded)
			if not EXCLUDED_ENCODED_TYPES[ty] then
				local pcallOk, decodeOk, decoded = pcall(EncodedValue.decode, encoded)
				if pcallOk and decodeOk and decoded ~= nil then
					defaults[propName] = decoded
				end
			end
		end
	end

	local result = table.freeze({
		propNames = table.freeze(propNames),
		propNameSet = table.freeze(propNameSet),
		defaults = table.freeze(defaults),
	})
	classComparisonKeysCache[className] = result
	return result
end

local function findClassDescriptor(className)
	local classData = database.Classes[className]
	if classData == nil then
		return nil
	end

	-- Build a table of all properties, walking up the inheritance chain
	local allProperties = {}
	local currentClassName = className

	repeat
		local currentClass = database.Classes[currentClassName]
		if currentClass == nil then
			break
		end

		-- Add properties from this class (don't override if already set by subclass)
		for propertyName, propertyData in pairs(currentClass.Properties) do
			if allProperties[propertyName] == nil then
				-- Only include canonical properties (not aliases)
				if propertyData.Kind and propertyData.Kind.Canonical then
					allProperties[propertyName] = {
						name = propertyName,
						scriptability = propertyData.Scriptability,
						dataType = propertyData.DataType,
						serialization = propertyData.Kind.Canonical.Serialization,
					}
				end
			end
		end

		currentClassName = currentClass.Superclass
	until currentClassName == nil

	return {
		name = classData.Name,
		superclass = classData.Superclass,
		properties = allProperties,
	}
end

local function findCanonicalPropertyDescriptor(className, propertyName)
	local currentClassName = className

	repeat
		local currentClass = database.Classes[currentClassName]

		if currentClass == nil then
			return currentClass
		end

		local propertyData = currentClass.Properties[propertyName]
		if propertyData ~= nil then
			local canonicalData = propertyData.Kind.Canonical
			if canonicalData ~= nil then
				return PropertyDescriptor.fromRaw(propertyData, currentClassName, propertyName)
			end

			local aliasData = propertyData.Kind.Alias
			if aliasData ~= nil then
				return PropertyDescriptor.fromRaw(
					currentClass.Properties[aliasData.AliasFor],
					currentClassName,
					aliasData.AliasFor
				)
			end

			return nil
		end

		currentClassName = currentClass.Superclass
	until currentClassName == nil

	return nil
end

local function readProperty(instance, propertyName)
	local descriptor = findCanonicalPropertyDescriptor(instance.ClassName, propertyName)

	if descriptor == nil then
		local fullName = ("%s.%s"):format(instance.className, propertyName)

		return false, Error.new(Error.Kind.UnknownProperty, fullName)
	end

	return descriptor:read(instance)
end

local function writeProperty(instance, propertyName, value)
	local descriptor = findCanonicalPropertyDescriptor(instance.ClassName, propertyName)

	if descriptor == nil then
		local fullName = ("%s.%s"):format(instance.className, propertyName)

		return false, Error.new(Error.Kind.UnknownProperty, fullName)
	end

	return descriptor:write(instance, value)
end

return {
	readProperty = readProperty,
	writeProperty = writeProperty,
	findCanonicalPropertyDescriptor = findCanonicalPropertyDescriptor,
	findClassDescriptor = findClassDescriptor,
	findDefaultProperties = findDefaultProperties,
	getClassComparisonKeys = getClassComparisonKeys,
	Error = Error,
	EncodedValue = EncodedValue,
}
