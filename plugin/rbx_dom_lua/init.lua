local database = require(script.database)
local Error = require(script.Error)
local PropertyDescriptor = require(script.PropertyDescriptor)

-- Returns a class descriptor with all property names and metadata (including inherited ones)
-- Returns nil if the class doesn't exist in the database
-- The properties table maps property names to metadata (scriptability, serialization status)
-- To get a full PropertyDescriptor for encoding, use findCanonicalPropertyDescriptor
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
	Error = Error,
	EncodedValue = require(script.EncodedValue),
}
