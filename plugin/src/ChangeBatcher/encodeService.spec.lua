return function()
	local encodeService = require(script.Parent.encodeService)

	describe("chunk structure", function()
		it("should return className matching the service", function()
			local lighting = game:GetService("Lighting")
			local chunk = encodeService(lighting)

			expect(chunk.className).to.equal("Lighting")
		end)

		it("should return childCount matching GetChildren", function()
			local lighting = game:GetService("Lighting")
			local chunk = encodeService(lighting)

			expect(chunk.childCount).to.equal(#lighting:GetChildren())
		end)

		it("should omit properties when none encode", function()
			local folder = Instance.new("Folder")
			folder.Name = "EmptyService"

			local chunk = encodeService(folder)

			expect(chunk.className).to.equal("Folder")
			expect(chunk.childCount).to.equal(0)
			expect(chunk.refTargetCount).to.equal(0)
		end)
	end)

	describe("property encoding", function()
		it("should encode Lighting.TimeOfDay", function()
			local lighting = game:GetService("Lighting")
			local chunk = encodeService(lighting)

			expect(chunk.properties).to.be.ok()
			expect(chunk.properties.TimeOfDay).to.be.ok()
		end)

		it("should not include skipped properties (Parent, Name, Archivable)", function()
			local lighting = game:GetService("Lighting")
			local chunk = encodeService(lighting)

			if chunk.properties then
				expect(chunk.properties.Parent).to.equal(nil)
				expect(chunk.properties.Name).to.equal(nil)
				expect(chunk.properties.Archivable).to.equal(nil)
			end
		end)
	end)

	describe("EXCLUDE_PROPERTIES", function()
		it("should not include Lighting.ClockTime", function()
			local lighting = game:GetService("Lighting")
			local chunk = encodeService(lighting)

			if chunk.properties then
				expect(chunk.properties.ClockTime).to.equal(nil)
			end
		end)
	end)

	describe("Attributes and Tags", function()
		it("should encode Attributes when present", function()
			local folder = Instance.new("Folder")
			folder:SetAttribute("TestKey", 42)

			local chunk = encodeService(folder)

			expect(chunk.properties).to.be.ok()
			expect(chunk.properties.Attributes).to.be.ok()

			folder:Destroy()
		end)

		it("should encode Tags when present", function()
			local folder = Instance.new("Folder")
			folder:AddTag("SyncTag")

			local chunk = encodeService(folder)

			expect(chunk.properties).to.be.ok()
			expect(chunk.properties.Tags).to.be.ok()

			folder:Destroy()
		end)

		it("should omit Attributes when empty", function()
			local folder = Instance.new("Folder")
			local chunk = encodeService(folder)

			if chunk.properties then
				expect(chunk.properties.Attributes).to.equal(nil)
			end

			folder:Destroy()
		end)
	end)

	describe("Ref property carriers", function()
		it("should create ObjectValue carriers for Ref properties", function()
			local workspace = game:GetService("Workspace")
			local _, refTargets = encodeService(workspace)

			for _, carrier in refTargets do
				expect(carrier:IsA("ObjectValue")).to.equal(true)
				expect(typeof(carrier.Name)).to.equal("string")
			end

			for _, carrier in refTargets do
				carrier:Destroy()
			end
		end)

		it("should report correct refTargetCount", function()
			local workspace = game:GetService("Workspace")
			local chunk, refTargets = encodeService(workspace)

			expect(chunk.refTargetCount).to.equal(#refTargets)

			for _, carrier in refTargets do
				carrier:Destroy()
			end
		end)

		it("should map refs to 1-based carrier indices", function()
			local workspace = game:GetService("Workspace")
			local chunk, refTargets = encodeService(workspace)

			if chunk.refs then
				for _, idx in chunk.refs do
					expect(idx >= 1).to.equal(true)
					expect(idx <= #refTargets).to.equal(true)
				end
			end

			for _, carrier in refTargets do
				carrier:Destroy()
			end
		end)
	end)
end
