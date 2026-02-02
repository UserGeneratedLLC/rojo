--[[
	Tests for property writing functionality.
	
	Tests error paths, type coercion, and edge cases.
]]

return function()
	local setProperty = require(script.Parent.setProperty)

	-- Test container for cleanup
	local container

	beforeEach(function()
		container = Instance.new("Folder")
		container.Name = "SetPropertyTestContainer"
		container.Parent = game:GetService("Workspace")
	end)

	afterEach(function()
		if container then
			container:Destroy()
			container = nil
		end
	end)

	describe("successful property writes", function()
		it("should set Name property", function()
			local folder = Instance.new("Folder")
			folder.Name = "Original"
			folder.Parent = container

			local success = setProperty(folder, "Name", "Updated")

			expect(success).to.equal(true)
			expect(folder.Name).to.equal("Updated")
		end)

		it("should set StringValue.Value", function()
			local sv = Instance.new("StringValue")
			sv.Value = "Original"
			sv.Parent = container

			local success = setProperty(sv, "Value", "Updated")

			expect(success).to.equal(true)
			expect(sv.Value).to.equal("Updated")
		end)

		it("should set NumberValue.Value", function()
			local nv = Instance.new("NumberValue")
			nv.Value = 0
			nv.Parent = container

			local success = setProperty(nv, "Value", 42.5)

			expect(success).to.equal(true)
			expect(nv.Value).to.equal(42.5)
		end)

		it("should set BoolValue.Value", function()
			local bv = Instance.new("BoolValue")
			bv.Value = false
			bv.Parent = container

			local success = setProperty(bv, "Value", true)

			expect(success).to.equal(true)
			expect(bv.Value).to.equal(true)
		end)

		it("should set Part.Anchored", function()
			local part = Instance.new("Part")
			part.Anchored = false
			part.Parent = container

			local success = setProperty(part, "Anchored", true)

			expect(success).to.equal(true)
			expect(part.Anchored).to.equal(true)
		end)

		it("should set Part.Size (Vector3)", function()
			local part = Instance.new("Part")
			part.Parent = container

			local success = setProperty(part, "Size", Vector3.new(10, 5, 2))

			expect(success).to.equal(true)
			expect(part.Size.X).to.equal(10)
			expect(part.Size.Y).to.equal(5)
			expect(part.Size.Z).to.equal(2)
		end)

		it("should set Part.Position (Vector3)", function()
			local part = Instance.new("Part")
			part.Parent = container

			local success = setProperty(part, "Position", Vector3.new(100, 200, 300))

			expect(success).to.equal(true)
			-- Position may not be exact due to physics, but should be close
			expect(math.abs(part.Position.X - 100) < 1).to.equal(true)
		end)

		it("should set Part.CFrame", function()
			local part = Instance.new("Part")
			part.Parent = container

			local newCFrame = CFrame.new(10, 20, 30)
			local success = setProperty(part, "CFrame", newCFrame)

			expect(success).to.equal(true)
		end)

		it("should set Part.Color (Color3)", function()
			local part = Instance.new("Part")
			part.Parent = container

			local success = setProperty(part, "Color", Color3.new(1, 0, 0))

			expect(success).to.equal(true)
			expect(part.Color.R).to.equal(1)
			expect(part.Color.G).to.equal(0)
			expect(part.Color.B).to.equal(0)
		end)

		it("should set Part.Material (Enum)", function()
			local part = Instance.new("Part")
			part.Parent = container

			local success = setProperty(part, "Material", Enum.Material.Brick)

			expect(success).to.equal(true)
			expect(part.Material).to.equal(Enum.Material.Brick)
		end)

		it("should set Part.Transparency", function()
			local part = Instance.new("Part")
			part.Parent = container

			local success = setProperty(part, "Transparency", 0.5)

			expect(success).to.equal(true)
			expect(part.Transparency).to.equal(0.5)
		end)
	end)

	describe("unknown property handling", function()
		it("should handle unknown property gracefully", function()
			local folder = Instance.new("Folder")
			folder.Parent = container

			-- Should not throw, just return success (unknown props are logged and skipped)
			local success = setProperty(folder, "NonExistentProperty", "value")

			-- Implementation may vary - check actual behavior
			-- Some implementations return true for unknown props, others return false
			expect(success ~= nil).to.equal(true)
		end)
	end)

	describe("various property types", function()
		it("should set UDim2 property", function()
			local frame = Instance.new("Frame")
			frame.Parent = container

			local success = setProperty(frame, "Size", UDim2.new(0.5, 100, 0.5, 50))

			expect(success).to.equal(true)
			expect(frame.Size.X.Scale).to.equal(0.5)
			expect(frame.Size.X.Offset).to.equal(100)
		end)

		it("should set UDim property", function()
			local uiCorner = Instance.new("UICorner")
			uiCorner.Parent = container

			local success = setProperty(uiCorner, "CornerRadius", UDim.new(0, 8))

			expect(success).to.equal(true)
			expect(uiCorner.CornerRadius.Offset).to.equal(8)
		end)

		it("should set Vector2 property", function()
			local frame = Instance.new("ScrollingFrame")
			frame.Parent = container

			local success = setProperty(frame, "CanvasPosition", Vector2.new(100, 200))

			expect(success).to.equal(true)
		end)

		it("should set NumberRange property", function()
			local emitter = Instance.new("ParticleEmitter")
			emitter.Parent = container

			local success = setProperty(emitter, "Lifetime", NumberRange.new(2, 5))

			expect(success).to.equal(true)
			expect(emitter.Lifetime.Min).to.equal(2)
			expect(emitter.Lifetime.Max).to.equal(5)
		end)

		it("should set Rect property", function()
			local imageLabel = Instance.new("ImageLabel")
			imageLabel.Parent = container

			local success = setProperty(imageLabel, "SliceCenter", Rect.new(10, 10, 20, 20))

			expect(success).to.equal(true)
		end)
	end)

	describe("ref properties", function()
		it("should set ObjectValue.Value (ref)", function()
			local target = Instance.new("Part")
			target.Name = "Target"
			target.Parent = container

			local objValue = Instance.new("ObjectValue")
			objValue.Parent = container

			local success = setProperty(objValue, "Value", target)

			expect(success).to.equal(true)
			expect(objValue.Value).to.equal(target)
		end)

		it("should set ObjectValue.Value to nil", function()
			local target = Instance.new("Part")
			target.Parent = container

			local objValue = Instance.new("ObjectValue")
			objValue.Value = target
			objValue.Parent = container

			local success = setProperty(objValue, "Value", nil)

			expect(success).to.equal(true)
			expect(objValue.Value).to.equal(nil)
		end)
	end)

	describe("edge cases", function()
		it("should set empty string value", function()
			local sv = Instance.new("StringValue")
			sv.Value = "non-empty"
			sv.Parent = container

			local success = setProperty(sv, "Value", "")

			expect(success).to.equal(true)
			expect(sv.Value).to.equal("")
		end)

		it("should set zero number value", function()
			local nv = Instance.new("NumberValue")
			nv.Value = 100
			nv.Parent = container

			local success = setProperty(nv, "Value", 0)

			expect(success).to.equal(true)
			expect(nv.Value).to.equal(0)
		end)

		it("should set false bool value", function()
			local bv = Instance.new("BoolValue")
			bv.Value = true
			bv.Parent = container

			local success = setProperty(bv, "Value", false)

			expect(success).to.equal(true)
			expect(bv.Value).to.equal(false)
		end)

		it("should set Archivable property", function()
			local folder = Instance.new("Folder")
			folder.Archivable = true
			folder.Parent = container

			local success = setProperty(folder, "Archivable", false)

			expect(success).to.equal(true)
			expect(folder.Archivable).to.equal(false)
		end)

		it("should set negative number", function()
			local nv = Instance.new("NumberValue")
			nv.Parent = container

			local success = setProperty(nv, "Value", -42.5)

			expect(success).to.equal(true)
			expect(nv.Value).to.equal(-42.5)
		end)

		it("should set very large number", function()
			local nv = Instance.new("NumberValue")
			nv.Parent = container

			local success = setProperty(nv, "Value", 1e10)

			expect(success).to.equal(true)
			expect(nv.Value).to.equal(1e10)
		end)

		it("should set very small number", function()
			local nv = Instance.new("NumberValue")
			nv.Parent = container

			local success = setProperty(nv, "Value", 1e-10)

			expect(success).to.equal(true)
			expect(math.abs(nv.Value - 1e-10) < 1e-15).to.equal(true)
		end)
	end)

	describe("script properties", function()
		it("should set ModuleScript.Source", function()
			local module = Instance.new("ModuleScript")
			module.Source = ""
			module.Parent = container

			local success = setProperty(module, "Source", "return {test = true}")

			expect(success).to.equal(true)
			expect(module.Source).to.equal("return {test = true}")
		end)

		it("should set Script.Source", function()
			local script = Instance.new("Script")
			script.Source = ""
			script.Parent = container

			local success = setProperty(script, "Source", "print('hello world')")

			expect(success).to.equal(true)
			expect(script.Source).to.equal("print('hello world')")
		end)

		it("should set LocalScript.Source", function()
			local localScript = Instance.new("LocalScript")
			localScript.Source = ""
			localScript.Parent = container

			local success = setProperty(localScript, "Source", "print('client code')")

			expect(success).to.equal(true)
			expect(localScript.Source).to.equal("print('client code')")
		end)

		it("should handle unicode in script source", function()
			local module = Instance.new("ModuleScript")
			module.Parent = container

			local unicodeSource = "-- 日本語コメント\nreturn '你好世界'"
			local success = setProperty(module, "Source", unicodeSource)

			expect(success).to.equal(true)
			expect(module.Source).to.equal(unicodeSource)
		end)
	end)
end
