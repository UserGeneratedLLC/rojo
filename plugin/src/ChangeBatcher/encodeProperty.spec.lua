--[[
	Tests for property encoding functionality.
	
	Tests all property types and edge cases.
]]

return function()
	local encodeProperty = require(script.Parent.encodeProperty)

	local Packages = script.Parent.Parent.Parent.Packages
	local RbxDom = require(Packages.RbxDom)

	-- Test container for cleanup
	local container

	beforeEach(function()
		container = Instance.new("Folder")
		container.Name = "EncodePropertyTestContainer"
		container.Parent = game:GetService("Workspace")
	end)

	afterEach(function()
		if container then
			container:Destroy()
			container = nil
		end
	end)

	local function getDescriptor(className, propertyName)
		return RbxDom.findCanonicalPropertyDescriptor(className, propertyName)
	end

	describe("string property encoding", function()
		it("should encode StringValue.Value", function()
			local sv = Instance.new("StringValue")
			sv.Value = "Test String"
			sv.Parent = container

			local descriptor = getDescriptor("StringValue", "Value")
			local success, encoded = encodeProperty(sv, "Value", descriptor)

			expect(success).to.equal(true)
			expect(encoded).to.be.ok()
		end)

		it("should encode empty string", function()
			local sv = Instance.new("StringValue")
			sv.Value = ""
			sv.Parent = container

			local descriptor = getDescriptor("StringValue", "Value")
			local success, encoded = encodeProperty(sv, "Value", descriptor)

			expect(success).to.equal(true)
			expect(encoded).to.be.ok()
		end)

		it("should encode unicode string", function()
			local sv = Instance.new("StringValue")
			sv.Value = "Hello 世界 🌍 مرحبا"
			sv.Parent = container

			local descriptor = getDescriptor("StringValue", "Value")
			local success, encoded = encodeProperty(sv, "Value", descriptor)

			expect(success).to.equal(true)
			expect(encoded).to.be.ok()
		end)

		it("should encode ModuleScript.Source", function()
			local module = Instance.new("ModuleScript")
			module.Source = "return {}"
			module.Parent = container

			local descriptor = getDescriptor("ModuleScript", "Source")
			local success, encoded = encodeProperty(module, "Source", descriptor)

			expect(success).to.equal(true)
			expect(encoded).to.be.ok()
		end)
	end)

	describe("number property encoding", function()
		it("should encode NumberValue.Value", function()
			local nv = Instance.new("NumberValue")
			nv.Value = 42.5
			nv.Parent = container

			local descriptor = getDescriptor("NumberValue", "Value")
			local success, encoded = encodeProperty(nv, "Value", descriptor)

			expect(success).to.equal(true)
			expect(encoded).to.be.ok()
		end)

		it("should encode zero", function()
			local nv = Instance.new("NumberValue")
			nv.Value = 0
			nv.Parent = container

			local descriptor = getDescriptor("NumberValue", "Value")
			local success, encoded = encodeProperty(nv, "Value", descriptor)

			expect(success).to.equal(true)
			expect(encoded).to.be.ok()
		end)

		it("should encode negative number", function()
			local nv = Instance.new("NumberValue")
			nv.Value = -42.5
			nv.Parent = container

			local descriptor = getDescriptor("NumberValue", "Value")
			local success, encoded = encodeProperty(nv, "Value", descriptor)

			expect(success).to.equal(true)
			expect(encoded).to.be.ok()
		end)

		it("should encode Part.Transparency", function()
			local part = Instance.new("Part")
			part.Transparency = 0.5
			part.Parent = container

			local descriptor = getDescriptor("Part", "Transparency")
			local success, encoded = encodeProperty(part, "Transparency", descriptor)

			expect(success).to.equal(true)
			expect(encoded).to.be.ok()
		end)
	end)

	describe("boolean property encoding", function()
		it("should encode BoolValue.Value true", function()
			local bv = Instance.new("BoolValue")
			bv.Value = true
			bv.Parent = container

			local descriptor = getDescriptor("BoolValue", "Value")
			local success, encoded = encodeProperty(bv, "Value", descriptor)

			expect(success).to.equal(true)
			expect(encoded).to.be.ok()
		end)

		it("should encode BoolValue.Value false", function()
			local bv = Instance.new("BoolValue")
			bv.Value = false
			bv.Parent = container

			local descriptor = getDescriptor("BoolValue", "Value")
			local success, encoded = encodeProperty(bv, "Value", descriptor)

			expect(success).to.equal(true)
			expect(encoded).to.be.ok()
		end)

		it("should encode Part.Anchored", function()
			local part = Instance.new("Part")
			part.Anchored = true
			part.Parent = container

			local descriptor = getDescriptor("Part", "Anchored")
			local success, encoded = encodeProperty(part, "Anchored", descriptor)

			expect(success).to.equal(true)
			expect(encoded).to.be.ok()
		end)
	end)

	describe("Vector3 property encoding", function()
		it("should encode Part.Size", function()
			local part = Instance.new("Part")
			part.Size = Vector3.new(4, 1, 2)
			part.Parent = container

			local descriptor = getDescriptor("Part", "Size")
			local success, encoded = encodeProperty(part, "Size", descriptor)

			expect(success).to.equal(true)
			expect(encoded).to.be.ok()
		end)

		it("should encode Part.Position", function()
			local part = Instance.new("Part")
			part.Position = Vector3.new(10, 20, 30)
			part.Parent = container

			local descriptor = getDescriptor("Part", "Position")
			local success, encoded = encodeProperty(part, "Position", descriptor)

			expect(success).to.equal(true)
			expect(encoded).to.be.ok()
		end)

		it("should encode negative Vector3", function()
			local part = Instance.new("Part")
			part.Position = Vector3.new(-10, -20, -30)
			part.Parent = container

			local descriptor = getDescriptor("Part", "Position")
			local success, encoded = encodeProperty(part, "Position", descriptor)

			expect(success).to.equal(true)
			expect(encoded).to.be.ok()
		end)
	end)

	describe("Color3 property encoding", function()
		it("should encode Part.Color", function()
			local part = Instance.new("Part")
			part.Color = Color3.new(1, 0, 0)
			part.Parent = container

			local descriptor = getDescriptor("Part", "Color")
			local success, encoded = encodeProperty(part, "Color", descriptor)

			expect(success).to.equal(true)
			expect(encoded).to.be.ok()
		end)

		it("should encode white color", function()
			local part = Instance.new("Part")
			part.Color = Color3.new(1, 1, 1)
			part.Parent = container

			local descriptor = getDescriptor("Part", "Color")
			local success, encoded = encodeProperty(part, "Color", descriptor)

			expect(success).to.equal(true)
			expect(encoded).to.be.ok()
		end)

		it("should encode black color", function()
			local part = Instance.new("Part")
			part.Color = Color3.new(0, 0, 0)
			part.Parent = container

			local descriptor = getDescriptor("Part", "Color")
			local success, encoded = encodeProperty(part, "Color", descriptor)

			expect(success).to.equal(true)
			expect(encoded).to.be.ok()
		end)
	end)

	describe("CFrame property encoding", function()
		it("should encode Part.CFrame", function()
			local part = Instance.new("Part")
			part.CFrame = CFrame.new(10, 20, 30)
			part.Parent = container

			local descriptor = getDescriptor("Part", "CFrame")
			local success, encoded = encodeProperty(part, "CFrame", descriptor)

			expect(success).to.equal(true)
			expect(encoded).to.be.ok()
		end)

		it("should encode rotated CFrame", function()
			local part = Instance.new("Part")
			part.CFrame = CFrame.new(0, 0, 0) * CFrame.Angles(math.pi / 4, 0, 0)
			part.Parent = container

			local descriptor = getDescriptor("Part", "CFrame")
			local success, encoded = encodeProperty(part, "CFrame", descriptor)

			expect(success).to.equal(true)
			expect(encoded).to.be.ok()
		end)
	end)

	describe("UDim2 property encoding", function()
		it("should encode Frame.Size", function()
			local frame = Instance.new("Frame")
			frame.Size = UDim2.new(0.5, 100, 0.5, 50)
			frame.Parent = container

			local descriptor = getDescriptor("Frame", "Size")
			local success, encoded = encodeProperty(frame, "Size", descriptor)

			expect(success).to.equal(true)
			expect(encoded).to.be.ok()
		end)

		it("should encode Frame.Position", function()
			local frame = Instance.new("Frame")
			frame.Position = UDim2.new(0, 0, 0, 0)
			frame.Parent = container

			local descriptor = getDescriptor("Frame", "Position")
			local success, encoded = encodeProperty(frame, "Position", descriptor)

			expect(success).to.equal(true)
			expect(encoded).to.be.ok()
		end)
	end)

	describe("Enum property encoding", function()
		it("should encode Part.Material", function()
			local part = Instance.new("Part")
			part.Material = Enum.Material.Brick
			part.Parent = container

			local descriptor = getDescriptor("Part", "Material")
			local success, encoded = encodeProperty(part, "Material", descriptor)

			expect(success).to.equal(true)
			expect(encoded).to.be.ok()
		end)

		it("should encode Part.Shape", function()
			local part = Instance.new("Part")
			part.Shape = Enum.PartType.Ball
			part.Parent = container

			local descriptor = getDescriptor("Part", "Shape")
			local success, encoded = encodeProperty(part, "Shape", descriptor)

			expect(success).to.equal(true)
			expect(encoded).to.be.ok()
		end)
	end)

	describe("Attributes encoding", function()
		it("should encode Attributes property", function()
			local folder = Instance.new("Folder")
			folder:SetAttribute("TestAttr", "TestValue")
			folder:SetAttribute("NumAttr", 42)
			folder.Parent = container

			local descriptor = getDescriptor("Folder", "Attributes")
			if descriptor then
				local success, encoded = encodeProperty(folder, "Attributes", descriptor)

				expect(success).to.equal(true)
				expect(encoded).to.be.ok()
			end
		end)
	end)

	describe("Tags encoding", function()
		it("should encode Tags property", function()
			local folder = Instance.new("Folder")
			folder:AddTag("Tag1")
			folder:AddTag("Tag2")
			folder.Parent = container

			local descriptor = getDescriptor("Folder", "Tags")
			if descriptor then
				local success, encoded = encodeProperty(folder, "Tags", descriptor)

				expect(success).to.equal(true)
				expect(encoded).to.be.ok()
			end
		end)
	end)

	describe("edge cases", function()
		it("should handle very large numbers", function()
			local nv = Instance.new("NumberValue")
			nv.Value = 1e15
			nv.Parent = container

			local descriptor = getDescriptor("NumberValue", "Value")
			local success, encoded = encodeProperty(nv, "Value", descriptor)

			expect(success).to.equal(true)
			expect(encoded).to.be.ok()
		end)

		it("should handle very small numbers", function()
			local nv = Instance.new("NumberValue")
			nv.Value = 1e-15
			nv.Parent = container

			local descriptor = getDescriptor("NumberValue", "Value")
			local success, encoded = encodeProperty(nv, "Value", descriptor)

			expect(success).to.equal(true)
			expect(encoded).to.be.ok()
		end)

		it("should handle long strings", function()
			local sv = Instance.new("StringValue")
			sv.Value = string.rep("a", 10000)
			sv.Parent = container

			local descriptor = getDescriptor("StringValue", "Value")
			local success, encoded = encodeProperty(sv, "Value", descriptor)

			expect(success).to.equal(true)
			expect(encoded).to.be.ok()
		end)
	end)
end
