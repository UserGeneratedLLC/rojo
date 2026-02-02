--[[
	Tests for value decoding functionality.
	
	Tests ref scenarios, various types, and failure cases.
]]

return function()
	local decodeValue = require(script.Parent.decodeValue)
	local InstanceMap = require(script.Parent.Parent.InstanceMap)
	local Error = require(script.Parent.Error)

	local HttpService = game:GetService("HttpService")

	-- Test container for cleanup
	local container

	beforeEach(function()
		container = Instance.new("Folder")
		container.Name = "DecodeValueTestContainer"
		container.Parent = game:GetService("Workspace")
	end)

	afterEach(function()
		if container then
			container:Destroy()
			container = nil
		end
	end)

	local function generateId()
		return HttpService:GenerateGUID(false)
	end

	describe("string decoding", function()
		it("should decode String type", function()
			local instanceMap = InstanceMap.new()
			local encoded = { String = "Hello, World!" }

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			expect(value).to.equal("Hello, World!")

			instanceMap:stop()
		end)

		it("should decode empty string", function()
			local instanceMap = InstanceMap.new()
			local encoded = { String = "" }

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			expect(value).to.equal("")

			instanceMap:stop()
		end)

		it("should decode unicode string", function()
			local instanceMap = InstanceMap.new()
			local encoded = { String = "Hello 世界 🌍" }

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			expect(value).to.equal("Hello 世界 🌍")

			instanceMap:stop()
		end)
	end)

	describe("number decoding", function()
		it("should decode Float32", function()
			local instanceMap = InstanceMap.new()
			local encoded = { Float32 = 3.14159 }

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			expect(math.abs(value - 3.14159) < 0.0001).to.equal(true)

			instanceMap:stop()
		end)

		it("should decode Float64", function()
			local instanceMap = InstanceMap.new()
			local encoded = { Float64 = 3.141592653589793 }

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			expect(value).to.equal(3.141592653589793)

			instanceMap:stop()
		end)

		it("should decode Int32", function()
			local instanceMap = InstanceMap.new()
			local encoded = { Int32 = 42 }

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			expect(value).to.equal(42)

			instanceMap:stop()
		end)

		it("should decode negative Int32", function()
			local instanceMap = InstanceMap.new()
			local encoded = { Int32 = -42 }

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			expect(value).to.equal(-42)

			instanceMap:stop()
		end)

		it("should decode zero", function()
			local instanceMap = InstanceMap.new()
			local encoded = { Float64 = 0 }

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			expect(value).to.equal(0)

			instanceMap:stop()
		end)
	end)

	describe("boolean decoding", function()
		it("should decode true", function()
			local instanceMap = InstanceMap.new()
			local encoded = { Bool = true }

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			expect(value).to.equal(true)

			instanceMap:stop()
		end)

		it("should decode false", function()
			local instanceMap = InstanceMap.new()
			local encoded = { Bool = false }

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			expect(value).to.equal(false)

			instanceMap:stop()
		end)
	end)

	describe("ref decoding", function()
		it("should decode valid ref to existing instance", function()
			local instanceMap = InstanceMap.new()

			local target = Instance.new("Part")
			target.Name = "Target"
			target.Parent = container

			local targetId = generateId()
			instanceMap:insert(targetId, target)

			local encoded = { Ref = targetId }

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			expect(value).to.equal(target)

			instanceMap:stop()
		end)

		it("should decode null ref", function()
			local instanceMap = InstanceMap.new()
			local encoded = { Ref = "00000000000000000000000000000000" }

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			expect(value).to.equal(nil)

			instanceMap:stop()
		end)

		it("should return error for invalid ref", function()
			local instanceMap = InstanceMap.new()
			local encoded = { Ref = "non-existent-id" }

			local success, err = decodeValue(encoded, instanceMap)

			expect(success).to.equal(false)
			expect(err.kind).to.equal(Error.RefDidNotExist)

			instanceMap:stop()
		end)
	end)

	describe("Vector3 decoding", function()
		it("should decode Vector3", function()
			local instanceMap = InstanceMap.new()
			local encoded = { Vector3 = { 10, 20, 30 } }

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			expect(typeof(value)).to.equal("Vector3")
			expect(value.X).to.equal(10)
			expect(value.Y).to.equal(20)
			expect(value.Z).to.equal(30)

			instanceMap:stop()
		end)

		it("should decode Vector3 with negative values", function()
			local instanceMap = InstanceMap.new()
			local encoded = { Vector3 = { -10, -20, -30 } }

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			expect(value.X).to.equal(-10)
			expect(value.Y).to.equal(-20)
			expect(value.Z).to.equal(-30)

			instanceMap:stop()
		end)

		it("should decode Vector3 zero", function()
			local instanceMap = InstanceMap.new()
			local encoded = { Vector3 = { 0, 0, 0 } }

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			expect(value).to.equal(Vector3.zero)

			instanceMap:stop()
		end)
	end)

	describe("Vector2 decoding", function()
		it("should decode Vector2", function()
			local instanceMap = InstanceMap.new()
			local encoded = { Vector2 = { 100, 200 } }

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			expect(typeof(value)).to.equal("Vector2")
			expect(value.X).to.equal(100)
			expect(value.Y).to.equal(200)

			instanceMap:stop()
		end)
	end)

	describe("Color3 decoding", function()
		it("should decode Color3", function()
			local instanceMap = InstanceMap.new()
			local encoded = { Color3 = { 1, 0, 0 } }

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			expect(typeof(value)).to.equal("Color3")
			expect(value.R).to.equal(1)
			expect(value.G).to.equal(0)
			expect(value.B).to.equal(0)

			instanceMap:stop()
		end)

		it("should decode Color3 white", function()
			local instanceMap = InstanceMap.new()
			local encoded = { Color3 = { 1, 1, 1 } }

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			expect(value.R).to.equal(1)
			expect(value.G).to.equal(1)
			expect(value.B).to.equal(1)

			instanceMap:stop()
		end)

		it("should decode Color3 black", function()
			local instanceMap = InstanceMap.new()
			local encoded = { Color3 = { 0, 0, 0 } }

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			expect(value.R).to.equal(0)
			expect(value.G).to.equal(0)
			expect(value.B).to.equal(0)

			instanceMap:stop()
		end)
	end)

	describe("UDim decoding", function()
		it("should decode UDim", function()
			local instanceMap = InstanceMap.new()
			local encoded = { UDim = { 0.5, 100 } }

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			expect(typeof(value)).to.equal("UDim")
			expect(value.Scale).to.equal(0.5)
			expect(value.Offset).to.equal(100)

			instanceMap:stop()
		end)
	end)

	describe("UDim2 decoding", function()
		it("should decode UDim2", function()
			local instanceMap = InstanceMap.new()
			local encoded = { UDim2 = { { 0.5, 100 }, { 0.5, 50 } } }

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			expect(typeof(value)).to.equal("UDim2")
			expect(value.X.Scale).to.equal(0.5)
			expect(value.X.Offset).to.equal(100)
			expect(value.Y.Scale).to.equal(0.5)
			expect(value.Y.Offset).to.equal(50)

			instanceMap:stop()
		end)
	end)

	describe("Enum decoding", function()
		it("should decode Enum value", function()
			local instanceMap = InstanceMap.new()
			local encoded = { Enum = 256 } -- Plastic material

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			-- The decoded value is typically the number, not the EnumItem
			expect(value == 256 or (typeof(value) == "EnumItem" and value.Value == 256)).to.equal(true)

			instanceMap:stop()
		end)
	end)

	describe("NumberRange decoding", function()
		it("should decode NumberRange", function()
			local instanceMap = InstanceMap.new()
			local encoded = { NumberRange = { 1, 5 } }

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			expect(typeof(value)).to.equal("NumberRange")
			expect(value.Min).to.equal(1)
			expect(value.Max).to.equal(5)

			instanceMap:stop()
		end)
	end)

	describe("Rect decoding", function()
		it("should decode Rect", function()
			local instanceMap = InstanceMap.new()
			local encoded = { Rect = { { 10, 10 }, { 20, 20 } } }

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			expect(typeof(value)).to.equal("Rect")

			instanceMap:stop()
		end)
	end)

	describe("edge cases", function()
		it("should handle large numbers", function()
			local instanceMap = InstanceMap.new()
			local encoded = { Float64 = 1e15 }

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			expect(value).to.equal(1e15)

			instanceMap:stop()
		end)

		it("should handle very small numbers", function()
			local instanceMap = InstanceMap.new()
			local encoded = { Float64 = 1e-15 }

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			expect(math.abs(value - 1e-15) < 1e-20).to.equal(true)

			instanceMap:stop()
		end)

		it("should handle long strings", function()
			local instanceMap = InstanceMap.new()
			local longString = string.rep("a", 10000)
			local encoded = { String = longString }

			local success, value = decodeValue(encoded, instanceMap)

			expect(success).to.equal(true)
			expect(#value).to.equal(10000)

			instanceMap:stop()
		end)
	end)
end
