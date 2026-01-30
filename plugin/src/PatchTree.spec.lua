return function()
	local PatchTree = require(script.Parent.PatchTree)
	local PatchSet = require(script.Parent.PatchSet)
	local InstanceMap = require(script.Parent.InstanceMap)
	local HttpService = game:GetService("HttpService")

	describe("build", function()
		it("should create a tree from an empty patch", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()

			local tree = PatchTree.build(patch, instanceMap, {})

			expect(tree).to.be.ok()
			instanceMap:stop()
		end)
	end)

	describe("buildInitialSelections", function()
		it("should return empty table for empty tree", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()
			local tree = PatchTree.build(patch, instanceMap, {})

			local selections = PatchTree.buildInitialSelections(tree)

			expect(selections).to.be.ok()
			expect(next(selections)).to.equal(nil)

			instanceMap:stop()
		end)

		it("should not include selections for nodes with nil defaultSelection", function()
			-- This test verifies that the new behavior (defaultSelection = nil)
			-- results in no automatic selections
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()

			-- Create a simple workspace for testing
			local workspace = game:GetService("Workspace")
			local testFolder = Instance.new("Folder")
			testFolder.Name = "PatchTreeTestFolder"
			testFolder.Parent = workspace

			-- Add the folder to instance map
			local folderId = HttpService:GenerateGUID(false)
			instanceMap:insert(folderId, testFolder)

			-- Add an update to the patch
			table.insert(patch.updated, {
				id = folderId,
				changedName = "NewName",
				changedProperties = {},
			})

			local tree = PatchTree.build(patch, instanceMap, { "Property", "Current", "Incoming" })
			local selections = PatchTree.buildInitialSelections(tree)

			-- With defaultSelection = nil, the selections table should be empty
			-- (no auto-selection to "push")
			expect(next(selections)).to.equal(nil)

			testFolder:Destroy()
			instanceMap:stop()
		end)
	end)

	describe("countUnselected", function()
		it("should return 0 for empty tree", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()
			local tree = PatchTree.build(patch, instanceMap, {})
			local selections = {}

			local count = PatchTree.countUnselected(tree, selections)

			expect(count).to.equal(0)

			instanceMap:stop()
		end)

		it("should count nodes without selections", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()

			-- Create test instances
			local workspace = game:GetService("Workspace")
			local testFolder = Instance.new("Folder")
			testFolder.Name = "CountTest"
			testFolder.Parent = workspace

			local id = HttpService:GenerateGUID(false)
			instanceMap:insert(id, testFolder)

			table.insert(patch.updated, {
				id = id,
				changedName = "UpdatedName",
				changedProperties = {},
			})

			local tree = PatchTree.build(patch, instanceMap, { "Property", "Current", "Incoming" })
			local selections = {} -- No selections

			local count = PatchTree.countUnselected(tree, selections)

			-- Should have at least 1 unselected item
			expect(count >= 1).to.equal(true)

			testFolder:Destroy()
			instanceMap:stop()
		end)

		it("should return 0 when all nodes are selected", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()

			local workspace = game:GetService("Workspace")
			local testFolder = Instance.new("Folder")
			testFolder.Name = "AllSelectedTest"
			testFolder.Parent = workspace

			local id = HttpService:GenerateGUID(false)
			instanceMap:insert(id, testFolder)

			table.insert(patch.updated, {
				id = id,
				changedName = "Selected",
				changedProperties = {},
			})

			local tree = PatchTree.build(patch, instanceMap, { "Property", "Current", "Incoming" })

			-- Select the node
			local selections = { [id] = "push" }

			local count = PatchTree.countUnselected(tree, selections)

			expect(count).to.equal(0)

			testFolder:Destroy()
			instanceMap:stop()
		end)

		it("should correctly count multiple unselected nodes", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()

			local workspace = game:GetService("Workspace")
			local testContainer = Instance.new("Folder")
			testContainer.Name = "MultiUnselectedTest"
			testContainer.Parent = workspace

			-- Create multiple test instances
			local ids = {}
			for i = 1, 3 do
				local folder = Instance.new("Folder")
				folder.Name = "TestFolder" .. i
				folder.Parent = testContainer

				local id = HttpService:GenerateGUID(false)
				instanceMap:insert(id, folder)
				table.insert(ids, id)

				table.insert(patch.updated, {
					id = id,
					changedName = "Updated" .. i,
					changedProperties = {},
				})
			end

			local tree = PatchTree.build(patch, instanceMap, { "Property", "Current", "Incoming" })

			-- Select only the first one
			local selections = { [ids[1]] = "push" }

			local count = PatchTree.countUnselected(tree, selections)

			-- Should have 2 unselected (the ones we didn't select)
			expect(count).to.equal(2)

			testContainer:Destroy()
			instanceMap:stop()
		end)
	end)

	describe("allNodesSelected", function()
		it("should return true for empty tree", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()
			local tree = PatchTree.build(patch, instanceMap, {})
			local selections = {}

			local result = PatchTree.allNodesSelected(tree, selections)

			expect(result).to.equal(true)

			instanceMap:stop()
		end)

		it("should return false when some nodes are unselected", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()

			local workspace = game:GetService("Workspace")
			local testFolder = Instance.new("Folder")
			testFolder.Name = "PartialSelectTest"
			testFolder.Parent = workspace

			local id = HttpService:GenerateGUID(false)
			instanceMap:insert(id, testFolder)

			table.insert(patch.updated, {
				id = id,
				changedName = "Updated",
				changedProperties = {},
			})

			local tree = PatchTree.build(patch, instanceMap, { "Property", "Current", "Incoming" })
			local selections = {} -- No selections

			local result = PatchTree.allNodesSelected(tree, selections)

			expect(result).to.equal(false)

			testFolder:Destroy()
			instanceMap:stop()
		end)

		it("should return true when all nodes are selected", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()

			local workspace = game:GetService("Workspace")
			local testFolder = Instance.new("Folder")
			testFolder.Name = "AllSelectTest"
			testFolder.Parent = workspace

			local id = HttpService:GenerateGUID(false)
			instanceMap:insert(id, testFolder)

			table.insert(patch.updated, {
				id = id,
				changedName = "Updated",
				changedProperties = {},
			})

			local tree = PatchTree.build(patch, instanceMap, { "Property", "Current", "Incoming" })
			local selections = { [id] = "push" }

			local result = PatchTree.allNodesSelected(tree, selections)

			expect(result).to.equal(true)

			testFolder:Destroy()
			instanceMap:stop()
		end)
	end)

	describe("getSelectableNodeIds", function()
		it("should return empty array for empty tree", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()
			local tree = PatchTree.build(patch, instanceMap, {})

			local ids = PatchTree.getSelectableNodeIds(tree)

			expect(#ids).to.equal(0)

			instanceMap:stop()
		end)

		it("should return IDs of all selectable nodes", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()

			local workspace = game:GetService("Workspace")
			local testContainer = Instance.new("Folder")
			testContainer.Name = "GetSelectableTest"
			testContainer.Parent = workspace

			local folder1 = Instance.new("Folder")
			folder1.Name = "Folder1"
			folder1.Parent = testContainer

			local id1 = HttpService:GenerateGUID(false)
			instanceMap:insert(id1, folder1)

			table.insert(patch.updated, {
				id = id1,
				changedName = "Updated1",
				changedProperties = {},
			})

			local tree = PatchTree.build(patch, instanceMap, { "Property", "Current", "Incoming" })
			local ids = PatchTree.getSelectableNodeIds(tree)

			expect(#ids >= 1).to.equal(true)

			testContainer:Destroy()
			instanceMap:stop()
		end)
	end)
end
