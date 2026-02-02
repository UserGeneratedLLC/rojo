--[[
	Generates large instance hierarchies for stress testing.
	
	Supports:
	- Configurable depth and width
	- Mixed instance types
	- Property population
	- Duplicate name generation (for edge case testing)
]]

local HttpService = game:GetService("HttpService")

local LargeTreeGenerator = {}

-- Instance types to randomly select from
local INSTANCE_TYPES = {
	"Folder",
	"ModuleScript",
	"Script",
	"LocalScript",
	"Part",
	"Model",
	"StringValue",
	"NumberValue",
	"BoolValue",
	"ObjectValue",
	"Configuration",
}

-- Script-like types that have a Source property
local SCRIPT_TYPES = {
	ModuleScript = true,
	Script = true,
	LocalScript = true,
}

--[[
	Generate a unique ID for virtual instances
]]
function LargeTreeGenerator.generateId()
	return HttpService:GenerateGUID(false)
end

--[[
	Generate a random instance type
]]
function LargeTreeGenerator.randomInstanceType()
	return INSTANCE_TYPES[math.random(1, #INSTANCE_TYPES)]
end

--[[
	Create a real Roblox instance tree for testing.
	
	Options:
	- depth: How many levels deep to create (default: 3)
	- width: How many children per level (default: 5)
	- instanceType: Fixed type or "mixed" for random (default: "Folder")
	- withProperties: Whether to populate properties (default: false)
	- withDuplicates: Whether to create duplicate-named siblings (default: false)
	- duplicateCount: How many duplicates to create (default: 2)
	
	Returns: root instance
]]
function LargeTreeGenerator.createInstanceTree(options)
	options = options or {}
	local depth = options.depth or 3
	local width = options.width or 5
	local instanceType = options.instanceType or "Folder"
	local withProperties = options.withProperties or false
	local withDuplicates = options.withDuplicates or false
	local duplicateCount = options.duplicateCount or 2

	local function createInstance(name, level)
		local className = instanceType
		if instanceType == "mixed" then
			className = LargeTreeGenerator.randomInstanceType()
		end

		local instance = Instance.new(className)
		instance.Name = name

		if withProperties then
			LargeTreeGenerator.populateProperties(instance)
		end

		return instance
	end

	local function buildLevel(parent, currentDepth, prefix)
		if currentDepth > depth then
			return
		end

		local childrenToCreate = width

		-- Create regular children
		for i = 1, childrenToCreate do
			local childName = prefix .. "_" .. tostring(i)
			local child = createInstance(childName, currentDepth)
			child.Parent = parent
			buildLevel(child, currentDepth + 1, childName)
		end

		-- Optionally create duplicates
		if withDuplicates and currentDepth <= 2 then -- Only add duplicates at top levels
			for i = 1, duplicateCount do
				local duplicateName = prefix .. "_1" -- Same name as first child
				local duplicate = createInstance(duplicateName, currentDepth)
				duplicate.Parent = parent
			end
		end
	end

	local root = createInstance("Root", 0)
	buildLevel(root, 1, "Child")

	return root
end

--[[
	Populate an instance with test properties based on its class.
]]
function LargeTreeGenerator.populateProperties(instance)
	local className = instance.ClassName

	if SCRIPT_TYPES[className] then
		instance.Source = "-- Test script\nreturn {}"
	elseif className == "StringValue" then
		instance.Value = "Test string value " .. tostring(math.random(1000))
	elseif className == "NumberValue" then
		instance.Value = math.random() * 1000
	elseif className == "BoolValue" then
		instance.Value = math.random() > 0.5
	elseif className == "Part" then
		instance.Anchored = true
		instance.Size = Vector3.new(math.random(1, 10), math.random(1, 10), math.random(1, 10))
		instance.Position = Vector3.new(math.random(-100, 100), math.random(-100, 100), math.random(-100, 100))
		instance.BrickColor = BrickColor.random()
	end

	-- Add random attributes
	if math.random() > 0.7 then
		instance:SetAttribute("TestAttribute", "TestValue" .. tostring(math.random(1000)))
		instance:SetAttribute("NumericAttribute", math.random(1, 100))
	end

	-- Add random tags
	if math.random() > 0.8 then
		instance:AddTag("TestTag" .. tostring(math.random(1, 5)))
	end
end

--[[
	Create virtual instances (the format used by diff/reify).
	
	Options:
	- depth: How many levels deep (default: 3)
	- width: How many children per level (default: 5)
	- instanceType: Fixed type or "mixed" (default: "Folder")
	- withProperties: Whether to add properties (default: false)
	- withDuplicates: Whether to create duplicate names (default: false)
	
	Returns: { virtualInstances = {}, rootId = "..." }
]]
function LargeTreeGenerator.createVirtualTree(options)
	options = options or {}
	local depth = options.depth or 3
	local width = options.width or 5
	local instanceType = options.instanceType or "Folder"
	local withProperties = options.withProperties or false
	local withDuplicates = options.withDuplicates or false
	local duplicateCount = options.duplicateCount or 2

	local virtualInstances = {}

	local function createVirtualInstance(name, parentId, currentDepth, prefix)
		local id = LargeTreeGenerator.generateId()

		local className = instanceType
		if instanceType == "mixed" then
			className = LargeTreeGenerator.randomInstanceType()
		end

		local properties = {}
		if withProperties then
			if SCRIPT_TYPES[className] then
				properties.Source = { String = "-- Test script\nreturn {}" }
			elseif className == "StringValue" then
				properties.Value = { String = "Test value " .. tostring(math.random(1000)) }
			elseif className == "NumberValue" then
				properties.Value = { Float64 = math.random() * 1000 }
			elseif className == "BoolValue" then
				properties.Value = { Bool = math.random() > 0.5 }
			end
		end

		local children = {}

		-- Create children if not at max depth
		if currentDepth < depth then
			for i = 1, width do
				local childName = prefix .. "_" .. tostring(i)
				local childId = createVirtualInstance(childName, id, currentDepth + 1, childName)
				table.insert(children, childId)
			end

			-- Optionally create duplicates
			if withDuplicates and currentDepth <= 1 then
				for _ = 1, duplicateCount do
					local duplicateName = prefix .. "_1" -- Same as first child
					local duplicateId =
						createVirtualInstance(duplicateName, id, currentDepth + 1, duplicateName .. "_dup")
					table.insert(children, duplicateId)
				end
			end
		end

		virtualInstances[id] = {
			Id = id,
			ClassName = className,
			Name = name,
			Parent = parentId,
			Properties = properties,
			Children = children,
		}

		return id
	end

	local rootId = createVirtualInstance("Root", nil, 0, "Child")

	return {
		virtualInstances = virtualInstances,
		rootId = rootId,
	}
end

--[[
	Count the total number of instances in a tree (including the root).
]]
function LargeTreeGenerator.countInstances(root)
	local count = 1 -- Count the root
	for _, child in ipairs(root:GetDescendants()) do
		count = count + 1
	end
	return count
end

--[[
	Count the total number of virtual instances.
]]
function LargeTreeGenerator.countVirtualInstances(virtualInstances)
	local count = 0
	for _ in pairs(virtualInstances) do
		count = count + 1
	end
	return count
end

--[[
	Create a deep hierarchy (many levels, few children per level).
	
	Options:
	- depth: How many levels (default: 50)
	- instanceType: Type to use (default: "Folder")
	
	Returns: root instance
]]
function LargeTreeGenerator.createDeepTree(options)
	options = options or {}
	local depth = options.depth or 50
	local instanceType = options.instanceType or "Folder"

	local root = Instance.new(instanceType)
	root.Name = "DeepRoot"

	local current = root
	for i = 1, depth do
		local child = Instance.new(instanceType)
		child.Name = "Level" .. tostring(i)
		child.Parent = current
		current = child
	end

	return root
end

--[[
	Create a wide hierarchy (few levels, many children per level).
	
	Options:
	- width: How many siblings (default: 100)
	- levels: How many levels (default: 2)
	- instanceType: Type to use (default: "Folder")
	
	Returns: root instance
]]
function LargeTreeGenerator.createWideTree(options)
	options = options or {}
	local width = options.width or 100
	local levels = options.levels or 2
	local instanceType = options.instanceType or "Folder"

	local root = Instance.new(instanceType)
	root.Name = "WideRoot"

	local function addChildren(parent, level)
		if level > levels then
			return
		end
		for i = 1, width do
			local child = Instance.new(instanceType)
			child.Name = "Child_" .. tostring(level) .. "_" .. tostring(i)
			child.Parent = parent
			addChildren(child, level + 1)
		end
	end

	addChildren(root, 1)

	return root
end

--[[
	Create an instance with many properties set.
	
	Options:
	- propertyCount: Approximate number of properties to set (default: 50)
	
	Returns: Part instance with many properties
]]
function LargeTreeGenerator.createInstanceWithManyProperties(options)
	options = options or {}
	local propertyCount = options.propertyCount or 50

	-- Use Part as it has many properties
	local part = Instance.new("Part")
	part.Name = "ManyProperties"
	part.Anchored = true
	part.CanCollide = true
	part.CastShadow = true
	part.Size = Vector3.new(4, 1, 2)
	part.Position = Vector3.new(0, 5, 0)
	part.Orientation = Vector3.new(0, 45, 0)
	part.Color = Color3.new(1, 0, 0)
	part.Material = Enum.Material.Brick
	part.Transparency = 0.5
	part.Reflectance = 0.1
	part.Massless = false
	part.Locked = false

	-- Add many attributes to increase property count
	for i = 1, propertyCount do
		part:SetAttribute("Attr_" .. tostring(i), "Value_" .. tostring(i))
	end

	return part
end

--[[
	Cleanup helper - destroys an instance tree.
]]
function LargeTreeGenerator.cleanup(root)
	if root and root.Parent then
		root:Destroy()
	elseif root then
		root.Parent = nil
	end
end

return LargeTreeGenerator
