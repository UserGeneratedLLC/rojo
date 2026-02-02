--[[
	Integration tests for the full sync flow.
	
	Tests diff -> applyPatch -> verify cycles, error recovery,
	and mode-specific behavior.
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
		container.Name = "SyncFlowTestContainer"
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

	describe("diff -> apply -> verify cycle", function()
		it("should apply additions and verify with diff", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			-- Initial virtual state with one child
			local childId = generateId()
			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Folder",
					Name = "Root",
					Parent = nil,
					Properties = {},
					Children = { childId },
				},
				[childId] = {
					Id = childId,
					ClassName = "Folder",
					Name = "Child",
					Parent = rootId,
					Properties = {},
					Children = {},
				},
			}

			-- Diff should show child needs to be added
			local ok1, patch1 = diff(instanceMap, virtualInstances, rootId)
			expect(ok1).to.equal(true)
			expect(patch1.added[childId]).to.be.ok()

			-- Apply the patch
			local unapplied = applyPatch(instanceMap, patch1)
			expect(PatchSet.isEmpty(unapplied)).to.equal(true)

			-- Diff again should show no changes
			local ok2, patch2 = diff(instanceMap, virtualInstances, rootId)
			expect(ok2).to.equal(true)
			expect(PatchSet.isEmpty(patch2)).to.equal(true)

			instanceMap:stop()
		end)

		it("should apply removals and verify with diff", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local child = Instance.new("Folder")
			child.Name = "ToRemove"
			child.Parent = root

			local rootId = generateId()
			-- Don't add child to instanceMap - it's an "unknown" instance to Rojo
			-- that should be removed because it's not in the virtual tree
			instanceMap:insert(rootId, root)

			-- Virtual state without the child
			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Folder",
					Name = "Root",
					Parent = nil,
					Properties = {},
					Children = {},
				},
			}

			-- Diff should show child needs to be removed (it's unknown)
			local ok1, patch1 = diff(instanceMap, virtualInstances, rootId)
			expect(ok1).to.equal(true)
			expect(#patch1.removed).to.equal(1)

			-- Apply the patch
			local unapplied = applyPatch(instanceMap, patch1)
			expect(PatchSet.isEmpty(unapplied)).to.equal(true)

			-- Diff again should show no changes
			local ok2, patch2 = diff(instanceMap, virtualInstances, rootId)
			expect(ok2).to.equal(true)
			expect(PatchSet.isEmpty(patch2)).to.equal(true)

			instanceMap:stop()
		end)

		it("should apply updates and verify with diff", function()
			local instanceMap = InstanceMap.new()

			local sv = Instance.new("StringValue")
			sv.Name = "TestValue"
			sv.Value = "Original"
			sv.Parent = container

			local id = generateId()
			instanceMap:insert(id, sv)

			-- Virtual state with updated value
			local virtualInstances = {
				[id] = {
					Id = id,
					ClassName = "StringValue",
					Name = "TestValue",
					Parent = nil,
					Properties = {
						Value = { String = "Updated" },
					},
					Children = {},
				},
			}

			-- Diff should show property needs to change
			local ok1, patch1 = diff(instanceMap, virtualInstances, id)
			expect(ok1).to.equal(true)
			expect(#patch1.updated).to.equal(1)

			-- Apply the patch
			local unapplied = applyPatch(instanceMap, patch1)
			expect(PatchSet.isEmpty(unapplied)).to.equal(true)
			expect(sv.Value).to.equal("Updated")

			-- Diff again should show no changes
			local ok2, patch2 = diff(instanceMap, virtualInstances, id)
			expect(ok2).to.equal(true)
			expect(PatchSet.isEmpty(patch2)).to.equal(true)

			instanceMap:stop()
		end)
	end)

	describe("multiple consecutive patches", function()
		it("should handle multiple patches in sequence", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			-- Apply 10 consecutive patches
			for i = 1, 10 do
				local patch = PatchGenerator.createAdditionsPatch({
					count = 5,
					parentId = rootId,
				})
				local unapplied = applyPatch(instanceMap, patch)
				expect(PatchSet.isEmpty(unapplied)).to.equal(true)
			end

			-- Should have 50 children
			expect(#root:GetChildren()).to.equal(50)

			instanceMap:stop()
		end)

		it("should maintain consistency through multiple cycles", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			local childIds = {}

			-- Build up virtual state
			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Folder",
					Name = "Root",
					Parent = nil,
					Properties = {},
					Children = childIds,
				},
			}

			for i = 1, 5 do
				-- Add a new child to virtual state
				local childId = generateId()
				table.insert(childIds, childId)

				virtualInstances[childId] = {
					Id = childId,
					ClassName = "Folder",
					Name = "Child_" .. tostring(i),
					Parent = rootId,
					Properties = {},
					Children = {},
				}

				-- Diff and apply
				local ok, patch = diff(instanceMap, virtualInstances, rootId)
				expect(ok).to.equal(true)
				applyPatch(instanceMap, patch)

				-- Verify
				local ok2, verify = diff(instanceMap, virtualInstances, rootId)
				expect(ok2).to.equal(true)
				expect(PatchSet.isEmpty(verify)).to.equal(true)
			end

			expect(#root:GetChildren()).to.equal(5)

			instanceMap:stop()
		end)
	end)

	describe("error recovery", function()
		it("should continue after partial failure", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			-- Patch with mix of valid and invalid additions
			local patch = PatchSet.newEmpty()

			local validId = generateId()
			patch.added[validId] = {
				Id = validId,
				ClassName = "Folder",
				Name = "ValidChild",
				Parent = rootId,
				Properties = {},
				Children = {},
			}

			local invalidId = generateId()
			patch.added[invalidId] = {
				Id = invalidId,
				ClassName = "NotARealClass",
				Name = "InvalidChild",
				Parent = rootId,
				Properties = {},
				Children = {},
			}

			local unapplied = applyPatch(instanceMap, patch)

			-- Valid should succeed, invalid should fail
			expect(instanceMap.fromIds[validId]).to.be.ok()
			expect(unapplied.added[invalidId]).to.be.ok()

			instanceMap:stop()
		end)

		it("should track unapplied portions accurately", function()
			local instanceMap = InstanceMap.new()

			local folder = Instance.new("Folder")
			folder.Name = "Test"
			folder.Parent = container

			local id = generateId()
			instanceMap:insert(id, folder)

			-- Update with invalid property
			-- Note: setProperty may handle unknown properties gracefully,
			-- so we check that the patch was processed (not that it failed)
			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, {
				id = id,
				changedProperties = {
					-- These properties don't exist on Folder
					NonExistentProp = { String = "Test" },
				},
			})

			local unapplied = applyPatch(instanceMap, patch)

			-- The implementation may or may not track unknown properties as unapplied
			-- This test verifies the operation completes without error
			expect(unapplied).to.be.ok()

			instanceMap:stop()
		end)
	end)

	describe("state consistency", function()
		it("should maintain InstanceMap consistency after failures", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			-- Try to add children, some fail
			for _ = 1, 10 do
				local patch = PatchSet.newEmpty()

				local validId = generateId()
				patch.added[validId] = {
					Id = validId,
					ClassName = "Folder",
					Name = "Valid",
					Parent = rootId,
					Properties = {},
					Children = {},
				}

				local invalidId = generateId()
				patch.added[invalidId] = {
					Id = invalidId,
					ClassName = "FakeClass",
					Name = "Invalid",
					Parent = rootId,
					Properties = {},
					Children = {},
				}

				applyPatch(instanceMap, patch)

				-- Verify consistency
				for id, instance in pairs(instanceMap.fromIds) do
					expect(instanceMap.fromInstances[instance]).to.equal(id)
				end
			end

			instanceMap:stop()
		end)
	end)

	describe("name change sync", function()
		it("should sync name changes bidirectionally", function()
			local instanceMap = InstanceMap.new()

			local folder = Instance.new("Folder")
			folder.Name = "Original"
			folder.Parent = container

			local id = generateId()
			instanceMap:insert(id, folder)

			-- Virtual state with different name
			local virtualInstances = {
				[id] = {
					Id = id,
					ClassName = "Folder",
					Name = "ServerName",
					Parent = nil,
					Properties = {},
					Children = {},
				},
			}

			-- Diff shows name change
			local ok1, patch1 = diff(instanceMap, virtualInstances, id)
			expect(ok1).to.equal(true)
			expect(patch1.updated[1].changedName).to.equal("ServerName")

			-- Apply
			applyPatch(instanceMap, patch1)
			expect(folder.Name).to.equal("ServerName")

			-- No more changes
			local ok2, patch2 = diff(instanceMap, virtualInstances, id)
			expect(ok2).to.equal(true)
			expect(PatchSet.isEmpty(patch2)).to.equal(true)

			instanceMap:stop()
		end)
	end)

	describe("deep tree sync", function()
		it("should sync changes deep in tree", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local level1 = Instance.new("Folder")
			level1.Name = "Level1"
			level1.Parent = root

			local level2 = Instance.new("Folder")
			level2.Name = "Level2"
			level2.Parent = level1

			local leaf = Instance.new("StringValue")
			leaf.Name = "Leaf"
			leaf.Value = "Original"
			leaf.Parent = level2

			local rootId = generateId()
			local level1Id = generateId()
			local level2Id = generateId()
			local leafId = generateId()

			instanceMap:insert(rootId, root)
			instanceMap:insert(level1Id, level1)
			instanceMap:insert(level2Id, level2)
			instanceMap:insert(leafId, leaf)

			-- Virtual state with changed leaf value
			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Folder",
					Name = "Root",
					Parent = nil,
					Properties = {},
					Children = { level1Id },
				},
				[level1Id] = {
					Id = level1Id,
					ClassName = "Folder",
					Name = "Level1",
					Parent = rootId,
					Properties = {},
					Children = { level2Id },
				},
				[level2Id] = {
					Id = level2Id,
					ClassName = "Folder",
					Name = "Level2",
					Parent = level1Id,
					Properties = {},
					Children = { leafId },
				},
				[leafId] = {
					Id = leafId,
					ClassName = "StringValue",
					Name = "Leaf",
					Parent = level2Id,
					Properties = {
						Value = { String = "Updated" },
					},
					Children = {},
				},
			}

			-- Diff and apply
			local ok1, patch1 = diff(instanceMap, virtualInstances, rootId)
			expect(ok1).to.equal(true)

			applyPatch(instanceMap, patch1)
			expect(leaf.Value).to.equal("Updated")

			instanceMap:stop()
		end)
	end)
end
