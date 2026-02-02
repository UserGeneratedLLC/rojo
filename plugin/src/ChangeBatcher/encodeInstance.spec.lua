return function()
	local encodeInstance = require(script.Parent.encodeInstance)

	describe("encodeInstance", function()
		describe("script encoding", function()
			it("should encode ModuleScript correctly", function()
				local moduleScript = Instance.new("ModuleScript")
				moduleScript.Name = "TestModule"
				moduleScript.Source = "return {}"

				local encoded = encodeInstance(moduleScript, "PARENT_ID")

				expect(encoded).to.be.ok()
				expect(encoded.parent).to.equal("PARENT_ID")
				expect(encoded.name).to.equal("TestModule")
				expect(encoded.className).to.equal("ModuleScript")
				expect(encoded.properties).to.be.ok()
				expect(encoded.properties.Source).to.be.ok()

				moduleScript:Destroy()
			end)

			it("should encode Script correctly", function()
				local script = Instance.new("Script")
				script.Name = "TestScript"
				script.Source = "print('hello')"

				local encoded = encodeInstance(script, "PARENT_ID")

				expect(encoded).to.be.ok()
				expect(encoded.name).to.equal("TestScript")
				expect(encoded.className).to.equal("Script")
				expect(encoded.properties.Source).to.be.ok()

				script:Destroy()
			end)

			it("should encode LocalScript correctly", function()
				local localScript = Instance.new("LocalScript")
				localScript.Name = "TestClient"
				localScript.Source = "print('client')"

				local encoded = encodeInstance(localScript, "PARENT_ID")

				expect(encoded).to.be.ok()
				expect(encoded.name).to.equal("TestClient")
				expect(encoded.className).to.equal("LocalScript")
				expect(encoded.properties.Source).to.be.ok()

				localScript:Destroy()
			end)
		end)

		describe("folder encoding", function()
			it("should encode Folder correctly", function()
				local folder = Instance.new("Folder")
				folder.Name = "TestFolder"

				local encoded = encodeInstance(folder, "PARENT_ID")

				expect(encoded).to.be.ok()
				expect(encoded.parent).to.equal("PARENT_ID")
				expect(encoded.name).to.equal("TestFolder")
				expect(encoded.className).to.equal("Folder")
				-- Folders have no special properties
				expect(encoded.properties).to.be.ok()

				folder:Destroy()
			end)
		end)

		describe("other instance encoding", function()
			it("should encode Part with properties", function()
				local part = Instance.new("Part")
				part.Name = "TestPart"
				part.Anchored = true

				local encoded = encodeInstance(part, "PARENT_ID")

				expect(encoded).to.be.ok()
				expect(encoded.name).to.equal("TestPart")
				expect(encoded.className).to.equal("Part")
				expect(encoded.properties).to.be.ok()
				-- Properties should be captured
				-- Note: Exact properties depend on RbxDom descriptors

				part:Destroy()
			end)

			it("should encode StringValue", function()
				local stringValue = Instance.new("StringValue")
				stringValue.Name = "TestValue"
				stringValue.Value = "Hello World"

				local encoded = encodeInstance(stringValue, "PARENT_ID")

				expect(encoded).to.be.ok()
				expect(encoded.name).to.equal("TestValue")
				expect(encoded.className).to.equal("StringValue")

				stringValue:Destroy()
			end)

			it("should encode Configuration", function()
				local config = Instance.new("Configuration")
				config.Name = "TestConfig"

				local encoded = encodeInstance(config, "PARENT_ID")

				expect(encoded).to.be.ok()
				expect(encoded.className).to.equal("Configuration")

				config:Destroy()
			end)
		end)

		describe("parent ID handling", function()
			it("should include parent ID in all encodings", function()
				local instances = {
					Instance.new("ModuleScript"),
					Instance.new("Folder"),
					Instance.new("Part"),
				}

				for _, instance in instances do
					local encoded = encodeInstance(instance, "UNIQUE_PARENT_ID")
					expect(encoded).to.be.ok()
					expect(encoded.parent).to.equal("UNIQUE_PARENT_ID")
					instance:Destroy()
				end
			end)
		end)

		describe("edge cases", function()
			it("should handle instances with special characters in name", function()
				local folder = Instance.new("Folder")
				folder.Name = "Test Folder (1)"

				local encoded = encodeInstance(folder, "PARENT")

				expect(encoded).to.be.ok()
				expect(encoded.name).to.equal("Test Folder (1)")

				folder:Destroy()
			end)

			it("should handle instances with empty name", function()
				local folder = Instance.new("Folder")
				folder.Name = ""

				local encoded = encodeInstance(folder, "PARENT")

				expect(encoded).to.be.ok()
				expect(encoded.name).to.equal("")

				folder:Destroy()
			end)
		end)

		describe("children encoding", function()
			it("should include children array even when empty", function()
				local moduleScript = Instance.new("ModuleScript")
				moduleScript.Name = "TestModule"
				moduleScript.Source = "return {}"

				local encoded = encodeInstance(moduleScript, "PARENT_ID")

				expect(encoded).to.be.ok()
				expect(encoded.children).to.be.ok()
				expect(type(encoded.children)).to.equal("table")
				expect(#encoded.children).to.equal(0)

				moduleScript:Destroy()
			end)

			it("should encode ModuleScript with children", function()
				local parent = Instance.new("ModuleScript")
				parent.Name = "ParentModule"
				parent.Source = "return {}"

				local child1 = Instance.new("ModuleScript")
				child1.Name = "ChildModule1"
				child1.Source = "return 1"
				child1.Parent = parent

				local child2 = Instance.new("ModuleScript")
				child2.Name = "ChildModule2"
				child2.Source = "return 2"
				child2.Parent = parent

				local encoded = encodeInstance(parent, "PARENT_ID")

				expect(encoded).to.be.ok()
				expect(encoded.children).to.be.ok()
				expect(#encoded.children).to.equal(2)

				-- Verify children are properly encoded
				local childNames = {}
				for _, child in encoded.children do
					childNames[child.name] = true
					expect(child.className).to.equal("ModuleScript")
					expect(child.properties.Source).to.be.ok()
				end
				expect(childNames["ChildModule1"]).to.equal(true)
				expect(childNames["ChildModule2"]).to.equal(true)

				parent:Destroy()
			end)

			it("should encode nested children recursively", function()
				local root = Instance.new("Folder")
				root.Name = "Root"

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
				expect(encoded.children[1].children[1].className).to.equal("ModuleScript")

				root:Destroy()
			end)

			it("should skip children with duplicate names", function()
				local parent = Instance.new("Folder")
				parent.Name = "Parent"

				local child1 = Instance.new("ModuleScript")
				child1.Name = "DuplicateName"
				child1.Source = "return 1"
				child1.Parent = parent

				local child2 = Instance.new("ModuleScript")
				child2.Name = "DuplicateName" -- Same name!
				child2.Source = "return 2"
				child2.Parent = parent

				local child3 = Instance.new("ModuleScript")
				child3.Name = "UniqueName"
				child3.Source = "return 3"
				child3.Parent = parent

				local encoded = encodeInstance(parent, "PARENT_ID")

				expect(encoded).to.be.ok()
				-- Should only have 1 child (UniqueName), duplicates skipped
				expect(#encoded.children).to.equal(1)
				expect(encoded.children[1].name).to.equal("UniqueName")

				parent:Destroy()
			end)

			it("should return empty children array when all children deleted", function()
				-- Simulate: parent had children, now they're all gone
				local parent = Instance.new("ModuleScript")
				parent.Name = "ParentModule"
				parent.Source = "return {}"
				-- No children parented

				local encoded = encodeInstance(parent, "PARENT_ID")

				expect(encoded).to.be.ok()
				expect(encoded.children).to.be.ok()
				expect(#encoded.children).to.equal(0)

				parent:Destroy()
			end)

			it("should encode Script with Folder child", function()
				-- This tests the init.server.luau + child directory scenario
				local script = Instance.new("Script")
				script.Name = "EventService"
				script.Source = "-- Event service"

				local events = Instance.new("Folder")
				events.Name = "Events"
				events.Parent = script

				local event1 = Instance.new("ModuleScript")
				event1.Name = "AcidRainEvent"
				event1.Source = "return {}"
				event1.Parent = events

				local encoded = encodeInstance(script, "PARENT_ID")

				expect(encoded).to.be.ok()
				expect(encoded.className).to.equal("Script")
				expect(#encoded.children).to.equal(1)
				expect(encoded.children[1].name).to.equal("Events")
				expect(encoded.children[1].className).to.equal("Folder")
				expect(#encoded.children[1].children).to.equal(1)
				expect(encoded.children[1].children[1].name).to.equal("AcidRainEvent")

				script:Destroy()
			end)
		end)
	end)
end
