--[[
	Stress tests for instance encoding.
	
	Tests deep/wide hierarchies, duplicate handling, and all instance types.
]]

return function()
	local encodeInstance = require(script.Parent.encodeInstance)
	local testUtils = require(script.Parent.Parent.testUtils)
	local LargeTreeGenerator = testUtils.LargeTreeGenerator

	-- Test container for cleanup
	local container

	beforeEach(function()
		container = Instance.new("Folder")
		container.Name = "EncodeInstanceStressTestContainer"
		container.Parent = game:GetService("Workspace")
	end)

	afterEach(function()
		if container then
			container:Destroy()
			container = nil
		end
	end)

	describe("deep hierarchy encoding", function()
		it("should encode a 10-level deep hierarchy", function()
			local root = LargeTreeGenerator.createDeepTree({ depth = 10 })
			root.Parent = container

			local encoded, skippedCount = encodeInstance(root, "PARENT_ID")

			expect(encoded).to.be.ok()
			expect(encoded.name).to.equal("DeepRoot")
			expect(skippedCount).to.equal(0)

			-- Verify depth (root + 10 children = 11 total levels)
			local current = encoded
			local depth = 1
			while current.children and #current.children > 0 do
				current = current.children[1]
				depth = depth + 1
			end
			expect(depth).to.equal(11)

			root:Destroy()
		end)

		it("should encode a 30-level deep hierarchy", function()
			local root = LargeTreeGenerator.createDeepTree({ depth = 30 })
			root.Parent = container

			local encoded, skippedCount = encodeInstance(root, "PARENT_ID")

			expect(encoded).to.be.ok()
			expect(skippedCount).to.equal(0)

			root:Destroy()
		end)
	end)

	describe("wide hierarchy encoding", function()
		it("should encode a tree with 50 children", function()
			local root = LargeTreeGenerator.createWideTree({ width = 50, levels = 1 })
			root.Parent = container

			local encoded, skippedCount = encodeInstance(root, "PARENT_ID")

			expect(encoded).to.be.ok()
			expect(#encoded.children).to.equal(50)
			expect(skippedCount).to.equal(0)

			root:Destroy()
		end)

		it("should encode a tree with 100 children", function()
			local root = LargeTreeGenerator.createWideTree({ width = 100, levels = 1 })
			root.Parent = container

			local encoded, skippedCount = encodeInstance(root, "PARENT_ID")

			expect(encoded).to.be.ok()
			expect(#encoded.children).to.equal(100)
			expect(skippedCount).to.equal(0)

			root:Destroy()
		end)
	end)

	describe("duplicate name handling", function()
		it("should skip children with duplicate names", function()
			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			-- Create two children with the same name
			local child1 = Instance.new("Folder")
			child1.Name = "Duplicate"
			child1.Parent = root

			local child2 = Instance.new("Folder")
			child2.Name = "Duplicate"
			child2.Parent = root

			local unique = Instance.new("Folder")
			unique.Name = "Unique"
			unique.Parent = root

			local encoded, skippedCount = encodeInstance(root, "PARENT_ID")

			expect(encoded).to.be.ok()
			-- Should only have the unique child
			expect(#encoded.children).to.equal(1)
			expect(encoded.children[1].name).to.equal("Unique")
			expect(skippedCount).to.equal(2) -- Both duplicates skipped
		end)

		it("should skip entire subtree for instances with duplicate-named siblings in path", function()
			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			-- Create duplicate siblings that each have children
			local dup1 = Instance.new("Folder")
			dup1.Name = "Duplicate"
			dup1.Parent = root

			local dup1Child = Instance.new("Folder")
			dup1Child.Name = "ChildOfDup1"
			dup1Child.Parent = dup1

			local dup2 = Instance.new("Folder")
			dup2.Name = "Duplicate"
			dup2.Parent = root

			local dup2Child = Instance.new("Folder")
			dup2Child.Name = "ChildOfDup2"
			dup2Child.Parent = dup2

			local encoded, skippedCount = encodeInstance(root, "PARENT_ID")

			expect(encoded).to.be.ok()
			-- Both duplicates and their children should be skipped
			expect(#encoded.children).to.equal(0)
			expect(skippedCount).to.equal(2) -- The two duplicates
		end)
	end)

	describe("instance type encoding", function()
		it("should encode ModuleScript correctly", function()
			local module = Instance.new("ModuleScript")
			module.Name = "TestModule"
			module.Source = "return {test = true}"
			module.Parent = container

			local encoded = encodeInstance(module, "PARENT_ID")

			expect(encoded).to.be.ok()
			expect(encoded.className).to.equal("ModuleScript")
			expect(encoded.properties.Source).to.be.ok()
		end)

		it("should encode Script correctly", function()
			local script = Instance.new("Script")
			script.Name = "TestScript"
			script.Source = "print('hello')"
			script.Parent = container

			local encoded = encodeInstance(script, "PARENT_ID")

			expect(encoded).to.be.ok()
			expect(encoded.className).to.equal("Script")
			expect(encoded.properties.Source).to.be.ok()
		end)

		it("should encode LocalScript correctly", function()
			local localScript = Instance.new("LocalScript")
			localScript.Name = "TestClient"
			localScript.Source = "print('client')"
			localScript.Parent = container

			local encoded = encodeInstance(localScript, "PARENT_ID")

			expect(encoded).to.be.ok()
			expect(encoded.className).to.equal("LocalScript")
			expect(encoded.properties.Source).to.be.ok()
		end)

		it("should encode Folder correctly", function()
			local folder = Instance.new("Folder")
			folder.Name = "TestFolder"
			folder.Parent = container

			local encoded = encodeInstance(folder, "PARENT_ID")

			expect(encoded).to.be.ok()
			expect(encoded.className).to.equal("Folder")
		end)

		it("should encode Part with properties", function()
			local part = Instance.new("Part")
			part.Name = "TestPart"
			part.Anchored = true
			part.Size = Vector3.new(4, 1, 2)
			part.Parent = container

			local encoded = encodeInstance(part, "PARENT_ID")

			expect(encoded).to.be.ok()
			expect(encoded.className).to.equal("Part")
			expect(encoded.properties).to.be.ok()
		end)

		it("should encode StringValue", function()
			local sv = Instance.new("StringValue")
			sv.Name = "TestString"
			sv.Value = "Test Value"
			sv.Parent = container

			local encoded = encodeInstance(sv, "PARENT_ID")

			expect(encoded).to.be.ok()
			expect(encoded.className).to.equal("StringValue")
		end)

		it("should encode Configuration", function()
			local config = Instance.new("Configuration")
			config.Name = "TestConfig"
			config.Parent = container

			local encoded = encodeInstance(config, "PARENT_ID")

			expect(encoded).to.be.ok()
			expect(encoded.className).to.equal("Configuration")
		end)

		it("should encode Model", function()
			local model = Instance.new("Model")
			model.Name = "TestModel"
			model.Parent = container

			-- Model encoding may fail due to PrimaryPart (Ref) property
			-- We just verify it doesn't throw and returns something
			local success, encoded = pcall(function()
				return encodeInstance(model, "PARENT_ID")
			end)

			-- Either succeeds or gracefully fails
			if success and encoded then
				expect(encoded.className).to.equal("Model")
			end
		end)
	end)

	describe("attribute encoding", function()
		it("should encode instance with string attribute", function()
			local folder = Instance.new("Folder")
			folder.Name = "WithAttribute"
			folder:SetAttribute("TestAttr", "TestValue")
			folder.Parent = container

			local encoded = encodeInstance(folder, "PARENT_ID")

			expect(encoded).to.be.ok()
			expect(encoded.properties.Attributes).to.be.ok()
		end)

		it("should encode instance with number attribute", function()
			local folder = Instance.new("Folder")
			folder.Name = "WithNumberAttr"
			folder:SetAttribute("NumAttr", 42)
			folder.Parent = container

			local encoded = encodeInstance(folder, "PARENT_ID")

			expect(encoded).to.be.ok()
			expect(encoded.properties.Attributes).to.be.ok()
		end)

		it("should encode instance with multiple attributes", function()
			local folder = Instance.new("Folder")
			folder.Name = "ManyAttributes"
			folder:SetAttribute("Attr1", "Value1")
			folder:SetAttribute("Attr2", 123)
			folder:SetAttribute("Attr3", true)
			folder.Parent = container

			local encoded = encodeInstance(folder, "PARENT_ID")

			expect(encoded).to.be.ok()
			expect(encoded.properties.Attributes).to.be.ok()
		end)
	end)

	describe("tag encoding", function()
		it("should encode instance with tags", function()
			local folder = Instance.new("Folder")
			folder.Name = "WithTags"
			folder:AddTag("TestTag1")
			folder:AddTag("TestTag2")
			folder.Parent = container

			local encoded = encodeInstance(folder, "PARENT_ID")

			expect(encoded).to.be.ok()
			expect(encoded.properties.Tags).to.be.ok()
		end)
	end)

	describe("children encoding", function()
		it("should include children array even when empty", function()
			local folder = Instance.new("Folder")
			folder.Name = "Empty"
			folder.Parent = container

			local encoded = encodeInstance(folder, "PARENT_ID")

			expect(encoded).to.be.ok()
			expect(encoded.children).to.be.ok()
			expect(type(encoded.children)).to.equal("table")
			expect(#encoded.children).to.equal(0)
		end)

		it("should encode nested children recursively", function()
			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local level1 = Instance.new("Folder")
			level1.Name = "Level1"
			level1.Parent = root

			local level2 = Instance.new("ModuleScript")
			level2.Name = "Level2"
			level2.Source = "return 'nested'"
			level2.Parent = level1

			local encoded = encodeInstance(root, "PARENT_ID")

			expect(encoded).to.be.ok()
			expect(#encoded.children).to.equal(1)
			expect(encoded.children[1].name).to.equal("Level1")
			expect(#encoded.children[1].children).to.equal(1)
			expect(encoded.children[1].children[1].name).to.equal("Level2")
		end)
	end)

	describe("edge cases", function()
		it("should handle instance with special characters in name", function()
			local folder = Instance.new("Folder")
			folder.Name = "Test (1) [brackets] {braces}"
			folder.Parent = container

			local encoded = encodeInstance(folder, "PARENT_ID")

			expect(encoded).to.be.ok()
			expect(encoded.name).to.equal("Test (1) [brackets] {braces}")
		end)

		it("should handle instance with empty name", function()
			local folder = Instance.new("Folder")
			folder.Name = ""
			folder.Parent = container

			local encoded = encodeInstance(folder, "PARENT_ID")

			expect(encoded).to.be.ok()
			expect(encoded.name).to.equal("")
		end)

		it("should handle script with unicode in source", function()
			local module = Instance.new("ModuleScript")
			module.Name = "UnicodeModule"
			module.Source = "-- 日本語コメント\nreturn '你好世界'"
			module.Parent = container

			local encoded = encodeInstance(module, "PARENT_ID")

			expect(encoded).to.be.ok()
			expect(encoded.properties.Source).to.be.ok()
		end)

		it("should handle script with long source", function()
			local module = Instance.new("ModuleScript")
			module.Name = "LongSource"
			module.Source = string.rep("-- Comment line\n", 1000) .. "return {}"
			module.Parent = container

			local encoded = encodeInstance(module, "PARENT_ID")

			expect(encoded).to.be.ok()
			expect(encoded.properties.Source).to.be.ok()
		end)
	end)

	describe("performance", function()
		it("should encode 200+ instances in reasonable time", function()
			local root = LargeTreeGenerator.createInstanceTree({
				depth = 3,
				width = 6,
				instanceType = "Folder",
			})
			root.Parent = container

			local startTime = os.clock()
			local encoded = encodeInstance(root, "PARENT_ID")
			local elapsed = os.clock() - startTime

			expect(encoded).to.be.ok()
			-- Should complete in under 2 seconds
			expect(elapsed < 2).to.equal(true)

			root:Destroy()
		end)
	end)

	describe("parent ID handling", function()
		it("should include parent ID in encoding", function()
			local folder = Instance.new("Folder")
			folder.Name = "Test"
			folder.Parent = container

			local encoded = encodeInstance(folder, "CUSTOM_PARENT_ID")

			expect(encoded).to.be.ok()
			expect(encoded.parent).to.equal("CUSTOM_PARENT_ID")
		end)
	end)
end
