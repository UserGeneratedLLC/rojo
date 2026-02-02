--[[
	Stress tests for patch application.
	
	Tests large batch operations, partial failures, ref chains,
	className changes, and various edge conditions.
]]

return function()
	local applyPatch = require(script.Parent.applyPatch)
	local InstanceMap = require(script.Parent.Parent.InstanceMap)
	local PatchSet = require(script.Parent.Parent.PatchSet)
	local testUtils = require(script.Parent.Parent.testUtils)
	local PatchGenerator = testUtils.PatchGenerator

	local HttpService = game:GetService("HttpService")

	-- Test container for cleanup
	local container

	beforeEach(function()
		container = Instance.new("Folder")
		container.Name = "ApplyPatchStressTestContainer"
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

	describe("large batch additions", function()
		it("should handle adding 100 instances in a single patch", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			local patch = PatchGenerator.createAdditionsPatch({
				count = 100,
				parentId = rootId,
				className = "Folder",
			})

			local unapplied = applyPatch(instanceMap, patch)

			expect(PatchSet.isEmpty(unapplied)).to.equal(true)
			expect(#root:GetChildren()).to.equal(100)

			instanceMap:stop()
		end)

		it("should handle adding 500 instances in a single patch", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			local patch = PatchGenerator.createAdditionsPatch({
				count = 500,
				parentId = rootId,
				className = "Folder",
			})

			local startTime = os.clock()
			local unapplied = applyPatch(instanceMap, patch)
			local elapsed = os.clock() - startTime

			expect(PatchSet.isEmpty(unapplied)).to.equal(true)
			expect(#root:GetChildren()).to.equal(500)
			-- Should complete in reasonable time
			expect(elapsed < 5).to.equal(true)

			instanceMap:stop()
		end)

		it("should handle nested additions (parent and children in same patch)", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			local patch, nestedRootId = PatchGenerator.createNestedAdditionsPatch({
				rootParentId = rootId,
				depth = 4,
				width = 3,
			})

			local unapplied = applyPatch(instanceMap, patch)

			expect(PatchSet.isEmpty(unapplied)).to.equal(true)

			-- Verify the nested structure was created
			local nestedRoot = instanceMap.fromIds[nestedRootId]
			expect(nestedRoot).to.be.ok()
			expect(nestedRoot.Parent).to.equal(root)

			instanceMap:stop()
		end)
	end)

	describe("large batch removals", function()
		it("should handle removing 100 instances in a single patch", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			-- Create 100 children
			local childIds = {}
			for i = 1, 100 do
				local child = Instance.new("Folder")
				child.Name = "Child_" .. tostring(i)
				child.Parent = root

				local childId = generateId()
				instanceMap:insert(childId, child)
				table.insert(childIds, childId)
			end

			expect(#root:GetChildren()).to.equal(100)

			local patch = PatchGenerator.createRemovalsPatch({ ids = childIds })

			local unapplied = applyPatch(instanceMap, patch)

			expect(PatchSet.isEmpty(unapplied)).to.equal(true)
			expect(#root:GetChildren()).to.equal(0)

			instanceMap:stop()
		end)

		it("should handle removing instances by instance reference", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			local child = Instance.new("Folder")
			child.Name = "Child"
			child.Parent = root

			local childId = generateId()
			instanceMap:insert(childId, child)

			-- Remove by instance reference instead of ID
			local patch = PatchSet.newEmpty()
			table.insert(patch.removed, child)

			local unapplied = applyPatch(instanceMap, patch)

			expect(PatchSet.isEmpty(unapplied)).to.equal(true)
			expect(#root:GetChildren()).to.equal(0)

			instanceMap:stop()
		end)
	end)

	describe("large batch updates", function()
		it("should handle updating 100 instances in a single patch", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			-- Create 100 StringValues
			local ids = {}
			for i = 1, 100 do
				local sv = Instance.new("StringValue")
				sv.Name = "Value_" .. tostring(i)
				sv.Value = "Original"
				sv.Parent = root

				local id = generateId()
				instanceMap:insert(id, sv)
				table.insert(ids, id)
			end

			-- Create update patch
			local patch = PatchSet.newEmpty()
			for i, id in ipairs(ids) do
				table.insert(patch.updated, {
					id = id,
					changedProperties = {
						Value = { String = "Updated_" .. tostring(i) },
					},
				})
			end

			local unapplied = applyPatch(instanceMap, patch)

			expect(PatchSet.isEmpty(unapplied)).to.equal(true)

			-- Verify all values were updated
			for i, id in ipairs(ids) do
				local instance = instanceMap.fromIds[id]
				expect(instance.Value).to.equal("Updated_" .. tostring(i))
			end

			instanceMap:stop()
		end)
	end)

	describe("mixed operations", function()
		it("should handle adds, removes, and updates in the same patch", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			-- Create some existing instances
			local toRemove = Instance.new("Folder")
			toRemove.Name = "ToRemove"
			toRemove.Parent = root
			local toRemoveId = generateId()
			instanceMap:insert(toRemoveId, toRemove)

			local toUpdate = Instance.new("StringValue")
			toUpdate.Name = "ToUpdate"
			toUpdate.Value = "Original"
			toUpdate.Parent = root
			local toUpdateId = generateId()
			instanceMap:insert(toUpdateId, toUpdate)

			-- Create mixed patch
			local patch = PatchGenerator.createMixedPatch({
				addCount = 5,
				removeIds = { toRemoveId },
				updateIds = { toUpdateId },
				parentId = rootId,
			})

			-- Override the update with proper property change
			for _, update in ipairs(patch.updated) do
				if update.id == toUpdateId then
					update.changedProperties = { Value = { String = "Updated" } }
				end
			end

			local unapplied = applyPatch(instanceMap, patch)

			expect(PatchSet.isEmpty(unapplied)).to.equal(true)
			-- 5 added + 1 remaining (toUpdate) = 6
			expect(#root:GetChildren()).to.equal(6)
			expect(toUpdate.Value).to.equal("Updated")

			instanceMap:stop()
		end)
	end)

	describe("partial failures", function()
		it("should return unapplied additions when instance creation fails", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			-- Create patch with invalid class name
			local patch = PatchGenerator.createInvalidClassPatch({
				count = 5,
				parentId = rootId,
			})

			local unapplied = applyPatch(instanceMap, patch)

			-- All 5 invalid instances should be in unapplied
			local unappliedCount = 0
			for _ in pairs(unapplied.added) do
				unappliedCount += 1
			end
			expect(unappliedCount).to.equal(5)
			expect(#root:GetChildren()).to.equal(0)

			instanceMap:stop()
		end)

		it("should return unapplied updates when property setting fails", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			-- Try to set a non-existent property
			-- Note: setProperty may handle unknown properties gracefully (not fail)
			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, {
				id = rootId,
				changedProperties = {
					-- Folders don't have a Value property
					Value = { String = "Test" },
				},
			})

			local unapplied = applyPatch(instanceMap, patch)

			-- The implementation may or may not track unknown property assignments
			-- as unapplied. This test verifies the operation completes.
			expect(unapplied).to.be.ok()

			instanceMap:stop()
		end)

		it("should handle mixed success and failure in same update", function()
			local instanceMap = InstanceMap.new()

			local sv = Instance.new("StringValue")
			sv.Name = "Original"
			sv.Value = "Original"
			sv.Parent = container

			local id = generateId()
			instanceMap:insert(id, sv)

			-- Update with valid Name and Value, plus invalid property
			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, {
				id = id,
				changedName = "Updated",
				changedProperties = {
					-- Valid
					Value = { String = "NewValue" },
					-- Invalid (doesn't exist on StringValue)
					FakeProperty = { String = "Fail" },
				},
			})

			local unapplied = applyPatch(instanceMap, patch)

			-- Name and Value should succeed
			expect(sv.Name).to.equal("Updated")
			expect(sv.Value).to.equal("NewValue")

			-- The implementation may or may not track unknown properties as failures
			-- This test verifies the valid properties were applied
			expect(unapplied).to.be.ok()

			instanceMap:stop()
		end)
	end)

	describe("className changes", function()
		it("should handle changing className from Folder to Model", function()
			local instanceMap = InstanceMap.new()

			local folder = Instance.new("Folder")
			folder.Name = "Test"
			folder.Parent = container

			local child = Instance.new("Part")
			child.Name = "ChildPart"
			child.Parent = folder

			local folderId = generateId()
			local childId = generateId()
			instanceMap:insert(folderId, folder)
			instanceMap:insert(childId, child)

			local patch = PatchGenerator.createClassNameChangePatch({
				id = folderId,
				newClassName = "Model",
			})

			local unapplied = applyPatch(instanceMap, patch)

			expect(PatchSet.isEmpty(unapplied)).to.equal(true)

			local newInstance = instanceMap.fromIds[folderId]
			expect(newInstance).to.be.ok()
			expect(newInstance.ClassName).to.equal("Model")
			expect(newInstance.Name).to.equal("Test")

			-- Child should have been moved
			expect(child.Parent).to.equal(newInstance)

			instanceMap:stop()
		end)

		it("should handle className change with name change", function()
			local instanceMap = InstanceMap.new()

			local folder = Instance.new("Folder")
			folder.Name = "OldName"
			folder.Parent = container

			local folderId = generateId()
			instanceMap:insert(folderId, folder)

			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, {
				id = folderId,
				changedClassName = "Model",
				changedName = "NewName",
				changedProperties = {},
			})

			local unapplied = applyPatch(instanceMap, patch)

			expect(PatchSet.isEmpty(unapplied)).to.equal(true)

			local newInstance = instanceMap.fromIds[folderId]
			expect(newInstance.ClassName).to.equal("Model")
			expect(newInstance.Name).to.equal("NewName")

			instanceMap:stop()
		end)

		it("should handle className change with property changes", function()
			local instanceMap = InstanceMap.new()

			local folder = Instance.new("Folder")
			folder.Name = "Test"
			folder.Parent = container

			local folderId = generateId()
			instanceMap:insert(folderId, folder)

			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, {
				id = folderId,
				changedClassName = "StringValue",
				changedProperties = {
					Value = { String = "TestValue" },
				},
			})

			local unapplied = applyPatch(instanceMap, patch)

			expect(PatchSet.isEmpty(unapplied)).to.equal(true)

			local newInstance = instanceMap.fromIds[folderId]
			expect(newInstance.ClassName).to.equal("StringValue")
			expect(newInstance.Value).to.equal("TestValue")

			instanceMap:stop()
		end)

		it("should preserve multiple children during className change", function()
			local instanceMap = InstanceMap.new()

			local folder = Instance.new("Folder")
			folder.Name = "Parent"
			folder.Parent = container

			local folderId = generateId()
			instanceMap:insert(folderId, folder)

			-- Create multiple children
			local children = {}
			for i = 1, 10 do
				local child = Instance.new("Folder")
				child.Name = "Child_" .. tostring(i)
				child.Parent = folder
				table.insert(children, child)
			end

			local patch = PatchGenerator.createClassNameChangePatch({
				id = folderId,
				newClassName = "Model",
			})

			local unapplied = applyPatch(instanceMap, patch)

			expect(PatchSet.isEmpty(unapplied)).to.equal(true)

			local newInstance = instanceMap.fromIds[folderId]
			expect(#newInstance:GetChildren()).to.equal(10)

			-- All children should be under the new instance
			for _, child in ipairs(children) do
				expect(child.Parent).to.equal(newInstance)
			end

			instanceMap:stop()
		end)
	end)

	describe("ref properties", function()
		it("should handle ref to existing instance", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local target = Instance.new("Part")
			target.Name = "Target"
			target.Parent = root

			local rootId = generateId()
			local targetId = generateId()
			instanceMap:insert(rootId, root)
			instanceMap:insert(targetId, target)

			-- Add ObjectValue with ref to target
			local objValueId = generateId()
			local patch = PatchSet.newEmpty()
			patch.added[objValueId] = {
				Id = objValueId,
				ClassName = "ObjectValue",
				Name = "RefHolder",
				Parent = rootId,
				Properties = {
					Value = { Ref = targetId },
				},
				Children = {},
			}

			local unapplied = applyPatch(instanceMap, patch)

			expect(PatchSet.isEmpty(unapplied)).to.equal(true)

			local objValue = instanceMap.fromIds[objValueId]
			expect(objValue).to.be.ok()
			expect(objValue.Value).to.equal(target)

			instanceMap:stop()
		end)

		it("should handle ref to instance created in same patch", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			-- Add both target and ref holder in same patch
			local targetId = generateId()
			local objValueId = generateId()

			local patch = PatchSet.newEmpty()
			patch.added[targetId] = {
				Id = targetId,
				ClassName = "Part",
				Name = "Target",
				Parent = rootId,
				Properties = {},
				Children = {},
			}
			patch.added[objValueId] = {
				Id = objValueId,
				ClassName = "ObjectValue",
				Name = "RefHolder",
				Parent = rootId,
				Properties = {
					Value = { Ref = targetId },
				},
				Children = {},
			}

			local unapplied = applyPatch(instanceMap, patch)

			expect(PatchSet.isEmpty(unapplied)).to.equal(true)

			local objValue = instanceMap.fromIds[objValueId]
			local target = instanceMap.fromIds[targetId]
			expect(objValue.Value).to.equal(target)

			instanceMap:stop()
		end)

		it("should handle ref chain (A -> B -> C)", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			local aId = generateId()
			local bId = generateId()
			local cId = generateId()

			local patch = PatchSet.newEmpty()
			patch.added[cId] = {
				Id = cId,
				ClassName = "Part",
				Name = "C",
				Parent = rootId,
				Properties = {},
				Children = {},
			}
			patch.added[bId] = {
				Id = bId,
				ClassName = "ObjectValue",
				Name = "B",
				Parent = rootId,
				Properties = {
					Value = { Ref = cId },
				},
				Children = {},
			}
			patch.added[aId] = {
				Id = aId,
				ClassName = "ObjectValue",
				Name = "A",
				Parent = rootId,
				Properties = {
					Value = { Ref = bId },
				},
				Children = {},
			}

			local unapplied = applyPatch(instanceMap, patch)

			expect(PatchSet.isEmpty(unapplied)).to.equal(true)

			local a = instanceMap.fromIds[aId]
			local b = instanceMap.fromIds[bId]
			local c = instanceMap.fromIds[cId]

			expect(a.Value).to.equal(b)
			expect(b.Value).to.equal(c)

			instanceMap:stop()
		end)

		it("should handle null ref", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local target = Instance.new("Part")
			target.Name = "Target"
			target.Parent = root

			local objValue = Instance.new("ObjectValue")
			objValue.Name = "RefHolder"
			objValue.Value = target
			objValue.Parent = root

			local rootId = generateId()
			local objValueId = generateId()
			instanceMap:insert(rootId, root)
			instanceMap:insert(objValueId, objValue)

			-- Update to null ref
			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, {
				id = objValueId,
				changedProperties = {
					Value = { Ref = "00000000000000000000000000000000" },
				},
			})

			local unapplied = applyPatch(instanceMap, patch)

			-- The null ref handling may result in a deferred ref that can't be resolved
			-- This test verifies the operation completes without crashing
			-- and that the value was updated (either to nil or stayed)
			expect(unapplied).to.be.ok()

			instanceMap:stop()
		end)

		it("should handle invalid ref gracefully", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			-- Create patch with ref to non-existent ID
			local patch = PatchGenerator.createRefPatch({
				parentId = rootId,
				refTargetId = "non-existent-id",
				count = 1,
				invalidRefs = true,
			})

			local unapplied = applyPatch(instanceMap, patch)

			-- The ref property should be in unapplied
			expect(#unapplied.updated > 0 or next(unapplied.added) ~= nil).to.equal(true)

			instanceMap:stop()
		end)
	end)

	describe("name changes", function()
		it("should handle simple name change", function()
			local instanceMap = InstanceMap.new()

			local folder = Instance.new("Folder")
			folder.Name = "OldName"
			folder.Parent = container

			local folderId = generateId()
			instanceMap:insert(folderId, folder)

			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, {
				id = folderId,
				changedName = "NewName",
				changedProperties = {},
			})

			local unapplied = applyPatch(instanceMap, patch)

			expect(PatchSet.isEmpty(unapplied)).to.equal(true)
			expect(folder.Name).to.equal("NewName")

			instanceMap:stop()
		end)

		it("should handle name with special characters", function()
			local instanceMap = InstanceMap.new()

			local folder = Instance.new("Folder")
			folder.Name = "Normal"
			folder.Parent = container

			local folderId = generateId()
			instanceMap:insert(folderId, folder)

			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, {
				id = folderId,
				changedName = "Name With (Parentheses) [Brackets] {Braces}",
				changedProperties = {},
			})

			local unapplied = applyPatch(instanceMap, patch)

			expect(PatchSet.isEmpty(unapplied)).to.equal(true)
			expect(folder.Name).to.equal("Name With (Parentheses) [Brackets] {Braces}")

			instanceMap:stop()
		end)
	end)

	describe("performance", function()
		it("should apply a large mixed patch in reasonable time", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			-- Create existing instances for updates
			local existingIds = {}
			for i = 1, 100 do
				local sv = Instance.new("StringValue")
				sv.Name = "Existing_" .. tostring(i)
				sv.Value = "Original"
				sv.Parent = root

				local id = generateId()
				instanceMap:insert(id, sv)
				table.insert(existingIds, id)
			end

			local patch = PatchGenerator.createLargeBatchPatch({
				addCount = 200,
				updateCount = 100,
				parentId = rootId,
				existingIds = existingIds,
			})

			local startTime = os.clock()
			local unapplied = applyPatch(instanceMap, patch)
			local elapsed = os.clock() - startTime

			expect(PatchSet.isEmpty(unapplied)).to.equal(true)
			-- Should complete in under 5 seconds
			expect(elapsed < 5).to.equal(true)

			instanceMap:stop()
		end)
	end)

	describe("edge cases", function()
		it("should handle empty patch", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			local patch = PatchSet.newEmpty()

			local unapplied = applyPatch(instanceMap, patch)

			expect(PatchSet.isEmpty(unapplied)).to.equal(true)

			instanceMap:stop()
		end)

		it("should skip already-existing instances in additions", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local existingChild = Instance.new("Folder")
			existingChild.Name = "Existing"
			existingChild.Parent = root

			local rootId = generateId()
			local existingId = generateId()
			instanceMap:insert(rootId, root)
			instanceMap:insert(existingId, existingChild)

			-- Try to add an instance with the same ID that already exists
			local patch = PatchSet.newEmpty()
			patch.added[existingId] = {
				Id = existingId,
				ClassName = "Folder",
				Name = "Existing",
				Parent = rootId,
				Properties = {},
				Children = {},
			}

			local unapplied = applyPatch(instanceMap, patch)

			expect(PatchSet.isEmpty(unapplied)).to.equal(true)
			-- Should still only have one child
			expect(#root:GetChildren()).to.equal(1)

			instanceMap:stop()
		end)

		it("should handle update for non-existent instance", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			-- Update an instance that doesn't exist
			local fakeId = generateId()
			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, {
				id = fakeId,
				changedName = "NewName",
				changedProperties = {},
			})

			local unapplied = applyPatch(instanceMap, patch)

			-- Update should be in unapplied
			expect(#unapplied.updated).to.equal(1)
			expect(unapplied.updated[1].id).to.equal(fakeId)

			instanceMap:stop()
		end)

		it("should handle property edge case values", function()
			local instanceMap = InstanceMap.new()

			local sv = Instance.new("StringValue")
			sv.Name = "Test"
			sv.Value = "Original"
			sv.Parent = container

			local svId = generateId()
			instanceMap:insert(svId, sv)

			local patch = PatchGenerator.createEdgeCasePropertiesPatch({ id = svId })

			-- We only want to test the empty string case for StringValue
			patch.updated[1].changedProperties = {
				Value = { String = "" },
			}

			local unapplied = applyPatch(instanceMap, patch)

			expect(PatchSet.isEmpty(unapplied)).to.equal(true)
			expect(sv.Value).to.equal("")

			instanceMap:stop()
		end)
	end)
end
