--[[
	Stress tests for PatchSet operations.
	
	Tests merge operations, conflict resolution, and scale.
]]

return function()
	local PatchSet = require(script.Parent.PatchSet)
	local InstanceMap = require(script.Parent.InstanceMap)
	local testUtils = require(script.Parent.testUtils)
	local PatchGenerator = testUtils.PatchGenerator

	local HttpService = game:GetService("HttpService")

	-- Test container for cleanup
	local container

	beforeEach(function()
		container = Instance.new("Folder")
		container.Name = "PatchSetStressTestContainer"
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

	describe("merge operations", function()
		it("should merge two empty patches", function()
			local target = PatchSet.newEmpty()
			local source = PatchSet.newEmpty()

			PatchSet.merge(target, source)

			expect(PatchSet.isEmpty(target)).to.equal(true)
		end)

		it("should merge additions from source into target", function()
			local target = PatchSet.newEmpty()
			local source = PatchSet.newEmpty()

			local id = generateId()
			source.added[id] = {
				Id = id,
				ClassName = "Folder",
				Name = "Added",
				Parent = "ROOT",
				Properties = {},
				Children = {},
			}

			PatchSet.merge(target, source)

			expect(target.added[id]).to.be.ok()
		end)

		it("should merge removals from source into target", function()
			local target = PatchSet.newEmpty()
			local source = PatchSet.newEmpty()

			local id = generateId()
			table.insert(source.removed, id)

			PatchSet.merge(target, source)

			expect(#target.removed).to.equal(1)
			expect(target.removed[1]).to.equal(id)
		end)

		it("should merge updates from source into target", function()
			local target = PatchSet.newEmpty()
			local source = PatchSet.newEmpty()

			local id = generateId()
			table.insert(source.updated, {
				id = id,
				changedName = "NewName",
				changedProperties = {},
			})

			PatchSet.merge(target, source)

			expect(#target.updated).to.equal(1)
			expect(target.updated[1].id).to.equal(id)
		end)

		it("should cancel addition when removal for same ID is merged", function()
			local target = PatchSet.newEmpty()
			local source = PatchSet.newEmpty()

			local id = generateId()
			target.added[id] = {
				Id = id,
				ClassName = "Folder",
				Name = "ToCancel",
				Parent = "ROOT",
				Properties = {},
				Children = {},
			}

			table.insert(source.removed, id)

			PatchSet.merge(target, source)

			-- Addition should be cancelled
			expect(target.added[id]).to.equal(nil)
			-- Removal should NOT be added since the instance was never created
			-- (Implementation may vary - check actual behavior)
		end)

		it("should merge updates for same instance", function()
			local target = PatchSet.newEmpty()
			local source = PatchSet.newEmpty()

			local id = generateId()

			table.insert(target.updated, {
				id = id,
				changedName = "FirstName",
				changedProperties = {
					Prop1 = { String = "Value1" },
				},
			})

			table.insert(source.updated, {
				id = id,
				changedName = "SecondName",
				changedProperties = {
					Prop2 = { String = "Value2" },
				},
			})

			PatchSet.merge(target, source)

			-- Should have only one update for this ID
			expect(#target.updated).to.equal(1)
			-- Name should be from second (source) update
			expect(target.updated[1].changedName).to.equal("SecondName")
			-- Both properties should be present
			expect(target.updated[1].changedProperties.Prop1).to.be.ok()
			expect(target.updated[1].changedProperties.Prop2).to.be.ok()
		end)
	end)

	describe("large patch operations", function()
		it("should merge patches with 100 additions each", function()
			local target = PatchSet.newEmpty()
			local source = PatchSet.newEmpty()

			for i = 1, 100 do
				local id = generateId()
				target.added[id] = {
					Id = id,
					ClassName = "Folder",
					Name = "Target_" .. tostring(i),
					Parent = "ROOT",
					Properties = {},
					Children = {},
				}
			end

			for i = 1, 100 do
				local id = generateId()
				source.added[id] = {
					Id = id,
					ClassName = "Folder",
					Name = "Source_" .. tostring(i),
					Parent = "ROOT",
					Properties = {},
					Children = {},
				}
			end

			local startTime = os.clock()
			PatchSet.merge(target, source)
			local elapsed = os.clock() - startTime

			-- Should have 200 additions
			local count = 0
			for _ in pairs(target.added) do
				count = count + 1
			end
			expect(count).to.equal(200)

			-- Should be fast
			expect(elapsed < 0.1).to.equal(true)
		end)

		it("should merge patches with 500 updates each", function()
			local target = PatchSet.newEmpty()
			local source = PatchSet.newEmpty()

			local ids = {}
			for i = 1, 500 do
				local id = generateId()
				table.insert(ids, id)

				table.insert(target.updated, {
					id = id,
					changedName = "Target_" .. tostring(i),
					changedProperties = {},
				})
			end

			for i = 1, 500 do
				local id = ids[i] -- Same IDs for conflict testing
				table.insert(source.updated, {
					id = id,
					changedProperties = {
						Value = { String = "Updated_" .. tostring(i) },
					},
				})
			end

			local startTime = os.clock()
			PatchSet.merge(target, source)
			local elapsed = os.clock() - startTime

			-- Should still have 500 updates (merged, not duplicated)
			expect(#target.updated).to.equal(500)

			-- Should be reasonably fast
			expect(elapsed < 1).to.equal(true)
		end)
	end)

	describe("assign operations", function()
		it("should assign multiple patches additively", function()
			local target = PatchSet.newEmpty()
			local source1 = PatchSet.newEmpty()
			local source2 = PatchSet.newEmpty()

			local id1 = generateId()
			local id2 = generateId()

			source1.added[id1] = {
				Id = id1,
				ClassName = "Folder",
				Name = "FromSource1",
				Parent = "ROOT",
				Properties = {},
				Children = {},
			}

			source2.added[id2] = {
				Id = id2,
				ClassName = "Folder",
				Name = "FromSource2",
				Parent = "ROOT",
				Properties = {},
				Children = {},
			}

			PatchSet.assign(target, source1, source2)

			expect(target.added[id1]).to.be.ok()
			expect(target.added[id2]).to.be.ok()
		end)

		it("should handle empty sources in assign", function()
			local target = PatchSet.newEmpty()
			local source1 = PatchSet.newEmpty()
			local source2 = PatchSet.newEmpty()

			PatchSet.assign(target, source1, source2)

			expect(PatchSet.isEmpty(target)).to.equal(true)
		end)
	end)

	describe("countChanges and countInstances", function()
		it("should count changes in additions", function()
			local patch = PatchSet.newEmpty()

			patch.added[generateId()] = {
				Id = generateId(),
				ClassName = "StringValue",
				Name = "Test",
				Parent = "ROOT",
				Properties = {
					Value = { String = "Test" },
				},
				Children = {},
			}

			local count = PatchSet.countChanges(patch)

			-- 1 property (Value)
			expect(count).to.equal(1)
		end)

		it("should count changes in removals", function()
			local patch = PatchSet.newEmpty()

			table.insert(patch.removed, generateId())
			table.insert(patch.removed, generateId())
			table.insert(patch.removed, generateId())

			local count = PatchSet.countChanges(patch)

			expect(count).to.equal(3)
		end)

		it("should count changes in updates", function()
			local patch = PatchSet.newEmpty()

			table.insert(patch.updated, {
				id = generateId(),
				changedName = "NewName",
				changedProperties = {
					Prop1 = { String = "Value1" },
					Prop2 = { String = "Value2" },
				},
			})

			local count = PatchSet.countChanges(patch)

			-- 1 name change + 2 property changes = 3
			expect(count).to.equal(3)
		end)

		it("should count instances affected", function()
			local patch = PatchSet.newEmpty()

			patch.added[generateId()] = {
				Id = generateId(),
				ClassName = "Folder",
				Name = "Added",
				Parent = "ROOT",
				Properties = {},
				Children = {},
			}

			table.insert(patch.removed, generateId())

			table.insert(patch.updated, {
				id = generateId(),
				changedName = "Updated",
				changedProperties = {},
			})

			local count = PatchSet.countInstances(patch)

			expect(count).to.equal(3)
		end)
	end)

	describe("isEmpty", function()
		it("should return true for empty patch", function()
			local patch = PatchSet.newEmpty()

			expect(PatchSet.isEmpty(patch)).to.equal(true)
		end)

		it("should return false when has additions", function()
			local patch = PatchSet.newEmpty()
			patch.added[generateId()] = {}

			expect(PatchSet.isEmpty(patch)).to.equal(false)
		end)

		it("should return false when has removals", function()
			local patch = PatchSet.newEmpty()
			table.insert(patch.removed, generateId())

			expect(PatchSet.isEmpty(patch)).to.equal(false)
		end)

		it("should return false when has updates", function()
			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, { id = generateId() })

			expect(PatchSet.isEmpty(patch)).to.equal(false)
		end)
	end)

	describe("containsId", function()
		it("should find ID in additions", function()
			local instanceMap = InstanceMap.new()
			local patch = PatchSet.newEmpty()

			local id = generateId()
			patch.added[id] = {}

			expect(PatchSet.containsId(patch, instanceMap, id)).to.equal(true)

			instanceMap:stop()
		end)

		it("should find ID in removals", function()
			local instanceMap = InstanceMap.new()
			local patch = PatchSet.newEmpty()

			local id = generateId()
			table.insert(patch.removed, id)

			expect(PatchSet.containsId(patch, instanceMap, id)).to.equal(true)

			instanceMap:stop()
		end)

		it("should find ID in updates", function()
			local instanceMap = InstanceMap.new()
			local patch = PatchSet.newEmpty()

			local id = generateId()
			table.insert(patch.updated, { id = id })

			expect(PatchSet.containsId(patch, instanceMap, id)).to.equal(true)

			instanceMap:stop()
		end)

		it("should return false for missing ID", function()
			local instanceMap = InstanceMap.new()
			local patch = PatchSet.newEmpty()

			expect(PatchSet.containsId(patch, instanceMap, generateId())).to.equal(false)

			instanceMap:stop()
		end)
	end)

	describe("isEqual", function()
		it("should return true for identical patches", function()
			local patch1 = PatchSet.newEmpty()
			local patch2 = PatchSet.newEmpty()

			local id = generateId()
			patch1.added[id] = {
				Id = id,
				ClassName = "Folder",
				Name = "Test",
				Parent = "ROOT",
				Properties = {},
				Children = {},
			}
			patch2.added[id] = {
				Id = id,
				ClassName = "Folder",
				Name = "Test",
				Parent = "ROOT",
				Properties = {},
				Children = {},
			}

			expect(PatchSet.isEqual(patch1, patch2)).to.equal(true)
		end)

		it("should return false for different patches", function()
			local patch1 = PatchSet.newEmpty()
			local patch2 = PatchSet.newEmpty()

			patch1.added[generateId()] = {}
			patch2.added[generateId()] = {}

			expect(PatchSet.isEqual(patch1, patch2)).to.equal(false)
		end)

		it("should return true for both empty", function()
			local patch1 = PatchSet.newEmpty()
			local patch2 = PatchSet.newEmpty()

			expect(PatchSet.isEqual(patch1, patch2)).to.equal(true)
		end)
	end)

	describe("performance", function()
		it("should handle merging 1000+ item patches in reasonable time", function()
			local target = PatchSet.newEmpty()
			local source = PatchSet.newEmpty()

			for i = 1, 1000 do
				local id = generateId()
				target.added[id] = {
					Id = id,
					ClassName = "Folder",
					Name = "Item_" .. tostring(i),
					Parent = "ROOT",
					Properties = {},
					Children = {},
				}
			end

			for i = 1, 1000 do
				local id = generateId()
				source.added[id] = {
					Id = id,
					ClassName = "Folder",
					Name = "SourceItem_" .. tostring(i),
					Parent = "ROOT",
					Properties = {},
					Children = {},
				}
			end

			local startTime = os.clock()
			PatchSet.merge(target, source)
			local elapsed = os.clock() - startTime

			-- Should complete in under 0.5 seconds
			expect(elapsed < 0.5).to.equal(true)
		end)
	end)
end
