--[[
	Integration tests for two-way sync scenarios.
	
	Tests bidirectional changes, conflict resolution, and concurrent operations.
]]

return function()
	local InstanceMap = require(script.Parent.Parent.InstanceMap)
	local PatchSet = require(script.Parent.Parent.PatchSet)
	local applyPatch = require(script.Parent.Parent.Reconciler.applyPatch)
	local diff = require(script.Parent.Parent.Reconciler.diff)
	local encodeInstance = require(script.Parent.Parent.ChangeBatcher.encodeInstance)
	local testUtils = require(script.Parent.Parent.testUtils)
	local PatchGenerator = testUtils.PatchGenerator

	local HttpService = game:GetService("HttpService")

	-- Test container for cleanup
	local container

	beforeEach(function()
		container = Instance.new("Folder")
		container.Name = "TwoWaySyncTestContainer"
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

	describe("push flow", function()
		it("should encode local changes for push", function()
			local folder = Instance.new("Folder")
			folder.Name = "LocalFolder"
			folder.Parent = container

			local child = Instance.new("ModuleScript")
			child.Name = "LocalModule"
			child.Source = "return 'local'"
			child.Parent = folder

			local encoded = encodeInstance(folder, "PARENT_ID")

			expect(encoded).to.be.ok()
			expect(encoded.name).to.equal("LocalFolder")
			expect(#encoded.children).to.equal(1)
			expect(encoded.children[1].name).to.equal("LocalModule")
			expect(encoded.children[1].className).to.equal("ModuleScript")
		end)

		it("should encode property changes", function()
			local sv = Instance.new("StringValue")
			sv.Name = "Changed"
			sv.Value = "NewValue"
			sv.Parent = container

			local encoded = encodeInstance(sv, "PARENT_ID")

			expect(encoded).to.be.ok()
			expect(encoded.className).to.equal("StringValue")
		end)
	end)

	describe("pull flow", function()
		it("should apply server additions", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			-- Server sends new child
			local patch = PatchGenerator.createAdditionsPatch({
				count = 1,
				parentId = rootId,
				className = "ModuleScript",
			})

			local unapplied = applyPatch(instanceMap, patch)

			expect(PatchSet.isEmpty(unapplied)).to.equal(true)
			expect(#root:GetChildren()).to.equal(1)

			instanceMap:stop()
		end)

		it("should apply server deletions", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local child = Instance.new("ModuleScript")
			child.Name = "ToDelete"
			child.Parent = root

			local rootId = generateId()
			local childId = generateId()
			instanceMap:insert(rootId, root)
			instanceMap:insert(childId, child)

			-- Server sends deletion
			local patch = PatchGenerator.createRemovalsPatch({ ids = { childId } })

			local unapplied = applyPatch(instanceMap, patch)

			expect(PatchSet.isEmpty(unapplied)).to.equal(true)
			expect(#root:GetChildren()).to.equal(0)

			instanceMap:stop()
		end)

		it("should apply server property updates", function()
			local instanceMap = InstanceMap.new()

			local sv = Instance.new("StringValue")
			sv.Name = "Test"
			sv.Value = "Original"
			sv.Parent = container

			local id = generateId()
			instanceMap:insert(id, sv)

			-- Server sends update
			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, {
				id = id,
				changedProperties = {
					Value = { String = "FromServer" },
				},
			})

			local unapplied = applyPatch(instanceMap, patch)

			expect(PatchSet.isEmpty(unapplied)).to.equal(true)
			expect(sv.Value).to.equal("FromServer")

			instanceMap:stop()
		end)
	end)

	describe("conflict scenarios", function()
		it("should handle same property changed on both sides", function()
			local instanceMap = InstanceMap.new()

			local sv = Instance.new("StringValue")
			sv.Name = "Conflict"
			sv.Value = "LocalValue" -- Local change
			sv.Parent = container

			local id = generateId()
			instanceMap:insert(id, sv)

			-- Server also changed the value
			local serverPatch = PatchSet.newEmpty()
			table.insert(serverPatch.updated, {
				id = id,
				changedProperties = {
					Value = { String = "ServerValue" },
				},
			})

			-- Apply server changes (server wins in this model)
			applyPatch(instanceMap, serverPatch)

			expect(sv.Value).to.equal("ServerValue")

			instanceMap:stop()
		end)

		it("should handle deletion vs modification conflict", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local child = Instance.new("StringValue")
			child.Name = "Conflicted"
			child.Value = "Modified" -- Local modification
			child.Parent = root

			local rootId = generateId()
			local childId = generateId()
			instanceMap:insert(rootId, root)
			instanceMap:insert(childId, child)

			-- Server deleted the child
			local serverPatch = PatchGenerator.createRemovalsPatch({ ids = { childId } })

			-- Apply server deletion
			applyPatch(instanceMap, serverPatch)

			-- Child should be deleted (server wins)
			expect(#root:GetChildren()).to.equal(0)

			instanceMap:stop()
		end)
	end)

	describe("mixed push/pull operations", function()
		it("should handle interleaved push and pull", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			-- Pull: Server adds a child
			local pullPatch = PatchGenerator.createAdditionsPatch({
				count = 1,
				parentId = rootId,
				className = "Folder",
			})
			applyPatch(instanceMap, pullPatch)

			expect(#root:GetChildren()).to.equal(1)

			-- Push: Local adds another child (simulated by direct creation)
			local localChild = Instance.new("ModuleScript")
			localChild.Name = "LocalChild"
			localChild.Parent = root

			-- Encode for push
			local encoded = encodeInstance(localChild, rootId)
			expect(encoded).to.be.ok()
			expect(encoded.name).to.equal("LocalChild")

			instanceMap:stop()
		end)
	end)

	describe("instance type handling", function()
		it("should handle script pull correctly", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			-- Server adds a script
			local scriptId = generateId()
			local patch = PatchSet.newEmpty()
			patch.added[scriptId] = {
				Id = scriptId,
				ClassName = "ModuleScript",
				Name = "ServerModule",
				Parent = rootId,
				Properties = {
					Source = { String = "return 'from server'" },
				},
				Children = {},
			}

			local unapplied = applyPatch(instanceMap, patch)

			expect(PatchSet.isEmpty(unapplied)).to.equal(true)

			local created = instanceMap.fromIds[scriptId]
			expect(created).to.be.ok()
			expect(created.ClassName).to.equal("ModuleScript")
			expect(created.Source).to.equal("return 'from server'")

			instanceMap:stop()
		end)

		it("should handle folder pull correctly", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			-- Server adds a folder with children
			local folderId = generateId()
			local childId = generateId()

			local patch = PatchSet.newEmpty()
			patch.added[folderId] = {
				Id = folderId,
				ClassName = "Folder",
				Name = "ServerFolder",
				Parent = rootId,
				Properties = {},
				Children = { childId },
			}
			patch.added[childId] = {
				Id = childId,
				ClassName = "ModuleScript",
				Name = "ChildModule",
				Parent = folderId,
				Properties = {
					Source = { String = "return {}" },
				},
				Children = {},
			}

			local unapplied = applyPatch(instanceMap, patch)

			expect(PatchSet.isEmpty(unapplied)).to.equal(true)

			local folder = instanceMap.fromIds[folderId]
			expect(folder).to.be.ok()
			expect(folder.ClassName).to.equal("Folder")
			expect(#folder:GetChildren()).to.equal(1)

			instanceMap:stop()
		end)
	end)

	describe("rapid bidirectional changes", function()
		it("should handle rapid alternating push/pull", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			for i = 1, 20 do
				-- Simulate pull (server add)
				local pullPatch = PatchGenerator.createAdditionsPatch({
					count = 1,
					parentId = rootId,
				})
				applyPatch(instanceMap, pullPatch)

				-- Simulate push (local add)
				local localChild = Instance.new("Folder")
				localChild.Name = "Local_" .. tostring(i)
				localChild.Parent = root

				local localId = generateId()
				instanceMap:insert(localId, localChild)
			end

			-- Should have 40 children (20 pulled + 20 local)
			expect(#root:GetChildren()).to.equal(40)

			instanceMap:stop()
		end)
	end)

	describe("encoding edge cases", function()
		it("should handle encoding empty folder", function()
			local folder = Instance.new("Folder")
			folder.Name = "Empty"
			folder.Parent = container

			local encoded = encodeInstance(folder, "PARENT_ID")

			expect(encoded).to.be.ok()
			expect(encoded.name).to.equal("Empty")
			expect(#encoded.children).to.equal(0)
		end)

		it("should handle encoding script with empty source", function()
			local module = Instance.new("ModuleScript")
			module.Name = "Empty"
			module.Source = ""
			module.Parent = container

			local encoded = encodeInstance(module, "PARENT_ID")

			expect(encoded).to.be.ok()
			expect(encoded.className).to.equal("ModuleScript")
		end)

		it("should handle encoding deep hierarchy", function()
			local level1 = Instance.new("Folder")
			level1.Name = "Level1"
			level1.Parent = container

			local level2 = Instance.new("Folder")
			level2.Name = "Level2"
			level2.Parent = level1

			local level3 = Instance.new("ModuleScript")
			level3.Name = "Level3"
			level3.Source = "return 'deep'"
			level3.Parent = level2

			local encoded = encodeInstance(level1, "PARENT_ID")

			expect(encoded).to.be.ok()
			expect(#encoded.children).to.equal(1)
			expect(encoded.children[1].name).to.equal("Level2")
			expect(#encoded.children[1].children).to.equal(1)
			expect(encoded.children[1].children[1].name).to.equal("Level3")
		end)
	end)
end
