--[[
	Chaos engineering stress tests.
	
	Tests random operations, rapid changes, and edge conditions
	to uncover potential race conditions and stability issues.
]]

return function()
	local InstanceMap = require(script.Parent.Parent.InstanceMap)
	local PatchSet = require(script.Parent.Parent.PatchSet)
	local applyPatch = require(script.Parent.Parent.Reconciler.applyPatch)
	local diff = require(script.Parent.Parent.Reconciler.diff)
	local testUtils = require(script.Parent.Parent.testUtils)
	local PatchGenerator = testUtils.PatchGenerator

	local HttpService = game:GetService("HttpService")

	-- Test container for cleanup
	local container

	beforeEach(function()
		container = Instance.new("Folder")
		container.Name = "ChaosTestContainer"
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

	describe("rapid add/remove cycles", function()
		it("should handle rapid sequential add/remove", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			for _ = 1, 50 do
				-- Add
				local addPatch = PatchGenerator.createAdditionsPatch({
					count = 5,
					parentId = rootId,
				})
				local unapplied1 = applyPatch(instanceMap, addPatch)
				expect(PatchSet.isEmpty(unapplied1)).to.equal(true)

				-- Get IDs of added instances
				local idsToRemove = {}
				for id in pairs(addPatch.added) do
					table.insert(idsToRemove, id)
				end

				-- Remove
				local removePatch = PatchGenerator.createRemovalsPatch({ ids = idsToRemove })
				local unapplied2 = applyPatch(instanceMap, removePatch)
				expect(PatchSet.isEmpty(unapplied2)).to.equal(true)
			end

			-- Root should have no children
			expect(#root:GetChildren()).to.equal(0)

			instanceMap:stop()
		end)

		it("should handle interleaved add/remove", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			local allIds = {}

			for _ = 1, 30 do
				-- Add some
				local addPatch = PatchGenerator.createAdditionsPatch({
					count = 3,
					parentId = rootId,
				})
				applyPatch(instanceMap, addPatch)

				for id in pairs(addPatch.added) do
					table.insert(allIds, id)
				end

				-- Remove some (if we have any)
				if #allIds > 5 then
					local toRemove = {}
					for _ = 1, 2 do
						if #allIds > 0 then
							table.insert(toRemove, table.remove(allIds, 1))
						end
					end
					if #toRemove > 0 then
						local removePatch = PatchGenerator.createRemovalsPatch({ ids = toRemove })
						applyPatch(instanceMap, removePatch)
					end
				end
			end

			instanceMap:stop()
		end)
	end)

	describe("property flapping", function()
		it("should handle rapid property changes", function()
			local instanceMap = InstanceMap.new()

			local sv = Instance.new("StringValue")
			sv.Name = "Flapping"
			sv.Value = "Initial"
			sv.Parent = container

			local id = generateId()
			instanceMap:insert(id, sv)

			for i = 1, 100 do
				local patch = PatchSet.newEmpty()
				table.insert(patch.updated, {
					id = id,
					changedProperties = {
						Value = { String = "Value_" .. tostring(i) },
					},
				})
				applyPatch(instanceMap, patch)
			end

			expect(sv.Value).to.equal("Value_100")

			instanceMap:stop()
		end)

		it("should handle rapid name changes", function()
			local instanceMap = InstanceMap.new()

			local folder = Instance.new("Folder")
			folder.Name = "Initial"
			folder.Parent = container

			local id = generateId()
			instanceMap:insert(id, folder)

			for i = 1, 50 do
				local patch = PatchSet.newEmpty()
				table.insert(patch.updated, {
					id = id,
					changedName = "Name_" .. tostring(i),
					changedProperties = {},
				})
				applyPatch(instanceMap, patch)
			end

			expect(folder.Name).to.equal("Name_50")

			instanceMap:stop()
		end)
	end)

	describe("tree mutations", function()
		it("should handle adding to different parts of tree", function()
			local instanceMap = InstanceMap.new()

			-- Create initial tree
			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			local branchIds = {}
			for i = 1, 5 do
				local branch = Instance.new("Folder")
				branch.Name = "Branch_" .. tostring(i)
				branch.Parent = root

				local branchId = generateId()
				instanceMap:insert(branchId, branch)
				table.insert(branchIds, branchId)
			end

			-- Add children to random branches
			for _ = 1, 50 do
				local parentId = branchIds[math.random(1, #branchIds)]
				local patch = PatchGenerator.createAdditionsPatch({
					count = 1,
					parentId = parentId,
				})
				applyPatch(instanceMap, patch)
			end

			-- Total should be root + 5 branches + 50 new = 56
			expect(instanceMap:size()).to.equal(56)

			instanceMap:stop()
		end)

		it("should handle moving instances between parents", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			local parent1 = Instance.new("Folder")
			parent1.Name = "Parent1"
			parent1.Parent = root

			local parent2 = Instance.new("Folder")
			parent2.Name = "Parent2"
			parent2.Parent = root

			local child = Instance.new("Folder")
			child.Name = "Child"
			child.Parent = parent1

			local parent1Id = generateId()
			local parent2Id = generateId()
			local childId = generateId()

			instanceMap:insert(parent1Id, parent1)
			instanceMap:insert(parent2Id, parent2)
			instanceMap:insert(childId, child)

			-- Move child back and forth
			for _ = 1, 20 do
				child.Parent = parent2
				child.Parent = parent1
			end

			-- Child should still be tracked
			expect(instanceMap.fromIds[childId]).to.equal(child)

			instanceMap:stop()
		end)
	end)

	describe("mixed operation stress", function()
		it("should handle random mix of operations", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			local knownIds = { rootId }

			-- Helper to clean up knownIds by removing IDs that no longer exist in instanceMap.
			-- This is necessary because when a parent is removed, its children are also
			-- destroyed by Roblox, but their IDs may still be in knownIds.
			local function cleanupKnownIds()
				local validIds = {}
				for _, id in ipairs(knownIds) do
					if instanceMap.fromIds[id] ~= nil then
						table.insert(validIds, id)
					end
				end
				knownIds = validIds
			end

			for _ = 1, 100 do
				local operation = math.random(1, 3)

				if operation == 1 and #knownIds < 500 then
					-- Add: verify parent still exists (may have been destroyed as descendant)
					local parentId = knownIds[math.random(1, #knownIds)]
					if instanceMap.fromIds[parentId] ~= nil then
						local patch = PatchGenerator.createAdditionsPatch({
							count = 1,
							parentId = parentId,
						})
						applyPatch(instanceMap, patch)
						for id in pairs(patch.added) do
							table.insert(knownIds, id)
						end
					end
				elseif operation == 2 and #knownIds > 1 then
					-- Remove (not root)
					local indexToRemove = math.random(2, #knownIds)
					local idToRemove = knownIds[indexToRemove]
					table.remove(knownIds, indexToRemove)

					local patch = PatchGenerator.createRemovalsPatch({ ids = { idToRemove } })
					applyPatch(instanceMap, patch)

					-- Clean up knownIds to remove any descendants that were destroyed
					cleanupKnownIds()
				elseif operation == 3 and #knownIds > 0 then
					-- Update: verify instance still exists
					local idToUpdate = knownIds[math.random(1, #knownIds)]
					if instanceMap.fromIds[idToUpdate] ~= nil then
						local patch = PatchSet.newEmpty()
						table.insert(patch.updated, {
							id = idToUpdate,
							changedName = "Updated_" .. tostring(os.clock()),
							changedProperties = {},
						})
						applyPatch(instanceMap, patch)
					end
				end
			end

			instanceMap:stop()
		end)
	end)

	describe("diff consistency", function()
		it("should produce consistent diffs", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local child1 = Instance.new("Folder")
			child1.Name = "Child1"
			child1.Parent = root

			local child2 = Instance.new("Folder")
			child2.Name = "Child2"
			child2.Parent = root

			local rootId = generateId()
			local child1Id = generateId()
			local child2Id = generateId()

			instanceMap:insert(rootId, root)
			instanceMap:insert(child1Id, child1)
			instanceMap:insert(child2Id, child2)

			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Folder",
					Name = "Root",
					Parent = nil,
					Properties = {},
					Children = { child1Id, child2Id },
				},
				[child1Id] = {
					Id = child1Id,
					ClassName = "Folder",
					Name = "Child1",
					Parent = rootId,
					Properties = {},
					Children = {},
				},
				[child2Id] = {
					Id = child2Id,
					ClassName = "Folder",
					Name = "Child2",
					Parent = rootId,
					Properties = {},
					Children = {},
				},
			}

			-- Diff multiple times, should be consistent
			for _ = 1, 10 do
				local ok, patch = diff(instanceMap, virtualInstances, rootId)
				expect(ok).to.equal(true)
				expect(PatchSet.isEmpty(patch)).to.equal(true)
			end

			instanceMap:stop()
		end)
	end)

	describe("large batch operations", function()
		it("should handle large batch addition and removal", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			-- Add 200 instances
			local addPatch = PatchGenerator.createAdditionsPatch({
				count = 200,
				parentId = rootId,
			})

			local startTime = os.clock()
			applyPatch(instanceMap, addPatch)
			local addTime = os.clock() - startTime

			expect(#root:GetChildren()).to.equal(200)
			expect(addTime < 5).to.equal(true)

			-- Remove all
			local idsToRemove = {}
			for id in pairs(addPatch.added) do
				table.insert(idsToRemove, id)
			end

			local removePatch = PatchGenerator.createRemovalsPatch({ ids = idsToRemove })

			startTime = os.clock()
			applyPatch(instanceMap, removePatch)
			local removeTime = os.clock() - startTime

			expect(#root:GetChildren()).to.equal(0)
			expect(removeTime < 5).to.equal(true)

			instanceMap:stop()
		end)
	end)

	describe("edge conditions", function()
		it("should handle empty patches gracefully", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			-- Apply many empty patches
			for _ = 1, 100 do
				local patch = PatchSet.newEmpty()
				local unapplied = applyPatch(instanceMap, patch)
				expect(PatchSet.isEmpty(unapplied)).to.equal(true)
			end

			instanceMap:stop()
		end)

		it("should handle patches with invalid IDs gracefully", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			-- Patch with updates to non-existent IDs
			local patch = PatchSet.newEmpty()
			for _ = 1, 10 do
				table.insert(patch.updated, {
					id = generateId(), -- Non-existent
					changedName = "NewName",
					changedProperties = {},
				})
			end

			local unapplied = applyPatch(instanceMap, patch)

			-- All updates should be unapplied
			expect(#unapplied.updated).to.equal(10)

			instanceMap:stop()
		end)
	end)
end
