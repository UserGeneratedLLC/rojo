--[[
	Tests for property reading functionality.
	
	Tests error paths, property types, and edge cases.
]]

return function()
	local getProperty = require(script.Parent.getProperty)
	local Error = require(script.Parent.Error)

	-- Test container for cleanup
	local container

	beforeEach(function()
		container = Instance.new("Folder")
		container.Name = "GetPropertyTestContainer"
		container.Parent = game:GetService("Workspace")
	end)

	afterEach(function()
		if container then
			container:Destroy()
			container = nil
		end
	end)

	describe("successful property reads", function()
		it("should read Name property", function()
			local folder = Instance.new("Folder")
			folder.Name = "TestFolder"
			folder.Parent = container

			local success, value = getProperty(folder, "Name")

			expect(success).to.equal(true)
			expect(value).to.equal("TestFolder")
		end)

		it("should read ClassName property", function()
			local folder = Instance.new("Folder")
			folder.Parent = container

			local success, value = getProperty(folder, "ClassName")

			expect(success).to.equal(true)
			expect(value).to.equal("Folder")
		end)

		it("should read StringValue.Value", function()
			local sv = Instance.new("StringValue")
			sv.Value = "TestValue"
			sv.Parent = container

			local success, value = getProperty(sv, "Value")

			expect(success).to.equal(true)
			expect(value).to.equal("TestValue")
		end)

		it("should read NumberValue.Value", function()
			local nv = Instance.new("NumberValue")
			nv.Value = 42.5
			nv.Parent = container

			local success, value = getProperty(nv, "Value")

			expect(success).to.equal(true)
			expect(value).to.equal(42.5)
		end)

		it("should read BoolValue.Value", function()
			local bv = Instance.new("BoolValue")
			bv.Value = true
			bv.Parent = container

			local success, value = getProperty(bv, "Value")

			expect(success).to.equal(true)
			expect(value).to.equal(true)
		end)

		it("should read Part.Anchored", function()
			local part = Instance.new("Part")
			part.Anchored = true
			part.Parent = container

			local success, value = getProperty(part, "Anchored")

			expect(success).to.equal(true)
			expect(value).to.equal(true)
		end)

		it("should read Part.Size (Vector3)", function()
			local part = Instance.new("Part")
			part.Size = Vector3.new(4, 1, 2)
			part.Parent = container

			local success, value = getProperty(part, "Size")

			expect(success).to.equal(true)
			expect(typeof(value)).to.equal("Vector3")
			expect(value.X).to.equal(4)
			expect(value.Y).to.equal(1)
			expect(value.Z).to.equal(2)
		end)

		it("should read Part.Position (Vector3)", function()
			local part = Instance.new("Part")
			part.Position = Vector3.new(10, 20, 30)
			part.Parent = container

			local success, value = getProperty(part, "Position")

			expect(success).to.equal(true)
			expect(typeof(value)).to.equal("Vector3")
		end)

		it("should read Part.CFrame", function()
			local part = Instance.new("Part")
			part.CFrame = CFrame.new(1, 2, 3)
			part.Parent = container

			local success, value = getProperty(part, "CFrame")

			expect(success).to.equal(true)
			expect(typeof(value)).to.equal("CFrame")
		end)

		it("should read Part.Color (Color3)", function()
			local part = Instance.new("Part")
			part.Color = Color3.new(1, 0, 0)
			part.Parent = container

			local success, value = getProperty(part, "Color")

			expect(success).to.equal(true)
			expect(typeof(value)).to.equal("Color3")
		end)

		it("should read Part.Material (Enum)", function()
			local part = Instance.new("Part")
			part.Material = Enum.Material.Brick
			part.Parent = container

			local success, value = getProperty(part, "Material")

			expect(success).to.equal(true)
			expect(typeof(value)).to.equal("EnumItem")
			expect(value).to.equal(Enum.Material.Brick)
		end)

		it("should read Part.Transparency (number)", function()
			local part = Instance.new("Part")
			part.Transparency = 0.5
			part.Parent = container

			local success, value = getProperty(part, "Transparency")

			expect(success).to.equal(true)
			expect(value).to.equal(0.5)
		end)
	end)

	describe("unknown property errors", function()
		it("should return error for non-existent property", function()
			local folder = Instance.new("Folder")
			folder.Parent = container

			local success, err = getProperty(folder, "NonExistentProperty")

			expect(success).to.equal(false)
			expect(err.kind).to.equal(Error.UnknownProperty)
		end)

		it("should return error for property on wrong class", function()
			local folder = Instance.new("Folder")
			folder.Parent = container

			-- Folders don't have a Value property
			local success, err = getProperty(folder, "Value")

			expect(success).to.equal(false)
			expect(err.kind).to.equal(Error.UnknownProperty)
		end)
	end)

	describe("various property types", function()
		it("should read UDim2 property", function()
			local frame = Instance.new("Frame")
			frame.Size = UDim2.new(0.5, 100, 0.5, 50)
			frame.Parent = container

			local success, value = getProperty(frame, "Size")

			expect(success).to.equal(true)
			expect(typeof(value)).to.equal("UDim2")
		end)

		it("should read UDim property", function()
			local uiCorner = Instance.new("UICorner")
			uiCorner.CornerRadius = UDim.new(0, 8)
			uiCorner.Parent = container

			local success, value = getProperty(uiCorner, "CornerRadius")

			expect(success).to.equal(true)
			expect(typeof(value)).to.equal("UDim")
		end)

		it("should read Vector2 property", function()
			local frame = Instance.new("ScrollingFrame")
			frame.CanvasSize = UDim2.new(0, 500, 0, 1000)
			frame.Parent = container

			local success, value = getProperty(frame, "CanvasPosition")

			expect(success).to.equal(true)
			expect(typeof(value)).to.equal("Vector2")
		end)

		it("should read NumberRange property", function()
			local emitter = Instance.new("ParticleEmitter")
			emitter.Lifetime = NumberRange.new(1, 5)
			emitter.Parent = container

			local success, value = getProperty(emitter, "Lifetime")

			expect(success).to.equal(true)
			expect(typeof(value)).to.equal("NumberRange")
		end)

		it("should read Rect property", function()
			local imageLabel = Instance.new("ImageLabel")
			imageLabel.SliceCenter = Rect.new(10, 10, 20, 20)
			imageLabel.Parent = container

			local success, value = getProperty(imageLabel, "SliceCenter")

			expect(success).to.equal(true)
			expect(typeof(value)).to.equal("Rect")
		end)
	end)

	describe("ref properties", function()
		it("should read ObjectValue.Value (ref)", function()
			local target = Instance.new("Part")
			target.Name = "Target"
			target.Parent = container

			local objValue = Instance.new("ObjectValue")
			objValue.Value = target
			objValue.Parent = container

			local success, value = getProperty(objValue, "Value")

			expect(success).to.equal(true)
			expect(value).to.equal(target)
		end)

		it("should read nil ref", function()
			local objValue = Instance.new("ObjectValue")
			objValue.Value = nil
			objValue.Parent = container

			local success, value = getProperty(objValue, "Value")

			expect(success).to.equal(true)
			expect(value).to.equal(nil)
		end)
	end)

	describe("edge cases", function()
		it("should read empty string value", function()
			local sv = Instance.new("StringValue")
			sv.Value = ""
			sv.Parent = container

			local success, value = getProperty(sv, "Value")

			expect(success).to.equal(true)
			expect(value).to.equal("")
		end)

		it("should read zero number value", function()
			local nv = Instance.new("NumberValue")
			nv.Value = 0
			nv.Parent = container

			local success, value = getProperty(nv, "Value")

			expect(success).to.equal(true)
			expect(value).to.equal(0)
		end)

		it("should read false bool value", function()
			local bv = Instance.new("BoolValue")
			bv.Value = false
			bv.Parent = container

			local success, value = getProperty(bv, "Value")

			expect(success).to.equal(true)
			expect(value).to.equal(false)
		end)

		it("should handle Archivable property", function()
			local folder = Instance.new("Folder")
			folder.Archivable = false
			folder.Parent = container

			local success, value = getProperty(folder, "Archivable")

			expect(success).to.equal(true)
			expect(value).to.equal(false)
		end)
	end)

	describe("script properties", function()
		it("should read ModuleScript.Source", function()
			local module = Instance.new("ModuleScript")
			module.Source = "return {}"
			module.Parent = container

			local success, value = getProperty(module, "Source")

			expect(success).to.equal(true)
			expect(value).to.equal("return {}")
		end)

		it("should read Script.Source", function()
			local script = Instance.new("Script")
			script.Source = "print('hello')"
			script.Parent = container

			local success, value = getProperty(script, "Source")

			expect(success).to.equal(true)
			expect(value).to.equal("print('hello')")
		end)

		it("should read LocalScript.Source", function()
			local localScript = Instance.new("LocalScript")
			localScript.Source = "print('client')"
			localScript.Parent = container

			local success, value = getProperty(localScript, "Source")

			expect(success).to.equal(true)
			expect(value).to.equal("print('client')")
		end)
	end)
end
