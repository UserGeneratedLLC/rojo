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
	end)
end
