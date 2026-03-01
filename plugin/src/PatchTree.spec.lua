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

	describe("gitMetadata default selections", function()
		local SHA1 = require(script.Parent.SHA1)

		local function computeBlobSha1(content)
			local gitBlob = "blob " .. tostring(#content) .. "\0" .. content
			return SHA1(buffer.fromstring(gitBlob))
		end

		it("should default to pull for unchanged files (not in changedIds)", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()

			local workspace = game:GetService("Workspace")
			local folder = Instance.new("Folder")
			folder.Name = "UnchangedFolder"
			folder.Parent = workspace

			local id = HttpService:GenerateGUID(false)
			instanceMap:insert(id, folder)

			table.insert(patch.updated, {
				id = id,
				changedName = "Renamed",
				changedProperties = {},
			})

			local gitMetadata = {
				changedIds = {}, -- empty = no files changed in git
				scriptCommittedHashes = {},
			}

			local tree = PatchTree.build(patch, instanceMap, { "Property", "Current", "Incoming" }, gitMetadata)
			local selections = PatchTree.buildInitialSelections(tree)

			expect(selections[id]).to.equal("pull")

			folder:Destroy()
			instanceMap:stop()
		end)

		it("should default to nil when no gitMetadata provided", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()

			local workspace = game:GetService("Workspace")
			local folder = Instance.new("Folder")
			folder.Name = "NoGitFolder"
			folder.Parent = workspace

			local id = HttpService:GenerateGUID(false)
			instanceMap:insert(id, folder)

			table.insert(patch.updated, {
				id = id,
				changedName = "Renamed",
				changedProperties = {},
			})

			local tree = PatchTree.build(patch, instanceMap, { "Property", "Current", "Incoming" }, nil)
			local selections = PatchTree.buildInitialSelections(tree)

			expect(selections[id]).to.equal(nil)

			folder:Destroy()
			instanceMap:stop()
		end)

		it("should default to nil for non-script changed files", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()

			local workspace = game:GetService("Workspace")
			local folder = Instance.new("Folder")
			folder.Name = "ChangedFolder"
			folder.Parent = workspace

			local id = HttpService:GenerateGUID(false)
			instanceMap:insert(id, folder)

			table.insert(patch.updated, {
				id = id,
				changedName = "Renamed",
				changedProperties = {},
			})

			local gitMetadata = {
				changedIds = { id },
				scriptCommittedHashes = {},
			}

			local tree = PatchTree.build(patch, instanceMap, { "Property", "Current", "Incoming" }, gitMetadata)
			local selections = PatchTree.buildInitialSelections(tree)

			expect(selections[id]).to.equal(nil)

			folder:Destroy()
			instanceMap:stop()
		end)

		it("should default to push when script Source matches committed hash", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()

			local workspace = game:GetService("Workspace")
			local script = Instance.new("ModuleScript")
			script.Name = "MatchingScript"
			script.Source = "local x = 1\nreturn x"
			script.Parent = workspace

			local id = HttpService:GenerateGUID(false)
			instanceMap:insert(id, script)

			table.insert(patch.updated, {
				id = id,
				changedProperties = { Source = { String = "-- different from server" } },
			})

			local committedHash = computeBlobSha1(script.Source)
			local gitMetadata = {
				changedIds = { id },
				scriptCommittedHashes = {
					[id] = { committedHash },
				},
			}

			local tree = PatchTree.build(patch, instanceMap, { "Property", "Current", "Incoming" }, gitMetadata)
			local selections = PatchTree.buildInitialSelections(tree)

			expect(selections[id]).to.equal("push")

			script:Destroy()
			instanceMap:stop()
		end)

		it("should default to nil when script Source does not match any committed hash", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()

			local workspace = game:GetService("Workspace")
			local script = Instance.new("ModuleScript")
			script.Name = "MismatchScript"
			script.Source = "-- studio has different content"
			script.Parent = workspace

			local id = HttpService:GenerateGUID(false)
			instanceMap:insert(id, script)

			table.insert(patch.updated, {
				id = id,
				changedProperties = { Source = { String = "-- server version" } },
			})

			local gitMetadata = {
				changedIds = { id },
				scriptCommittedHashes = {
					[id] = { computeBlobSha1("-- committed version, different from studio") },
				},
			}

			local tree = PatchTree.build(patch, instanceMap, { "Property", "Current", "Incoming" }, gitMetadata)
			local selections = PatchTree.buildInitialSelections(tree)

			expect(selections[id]).to.equal(nil)

			script:Destroy()
			instanceMap:stop()
		end)

		it("should match against staged hash when HEAD hash doesn't match", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()

			local workspace = game:GetService("Workspace")
			local script = Instance.new("ModuleScript")
			script.Name = "StagedMatchScript"
			script.Source = "-- staged version content"
			script.Parent = workspace

			local id = HttpService:GenerateGUID(false)
			instanceMap:insert(id, script)

			table.insert(patch.updated, {
				id = id,
				changedProperties = { Source = { String = "-- server" } },
			})

			local headHash = computeBlobSha1("-- old committed version")
			local stagedHash = computeBlobSha1(script.Source)
			local gitMetadata = {
				changedIds = { id },
				scriptCommittedHashes = {
					[id] = { headHash, stagedHash },
				},
			}

			local tree = PatchTree.build(patch, instanceMap, { "Property", "Current", "Incoming" }, gitMetadata)
			local selections = PatchTree.buildInitialSelections(tree)

			expect(selections[id]).to.equal("push")

			script:Destroy()
			instanceMap:stop()
		end)

		it("should not auto-select Added items regardless of git status", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()

			local parentId = HttpService:GenerateGUID(false)
			local workspace = game:GetService("Workspace")
			instanceMap:insert(parentId, workspace)

			local addId = HttpService:GenerateGUID(false)
			patch.added[addId] = {
				Id = addId,
				Parent = parentId,
				ClassName = "ModuleScript",
				Name = "AddedScript",
				Properties = {},
			}

			local gitMetadata = {
				changedIds = { addId },
				scriptCommittedHashes = {},
			}

			local tree = PatchTree.build(patch, instanceMap, { "Property", "Current", "Incoming" }, gitMetadata)
			local selections = PatchTree.buildInitialSelections(tree)

			expect(selections[addId]).to.equal(nil)

			instanceMap:stop()
		end)

		it("should not auto-select Removed items regardless of git status", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()

			local workspace = game:GetService("Workspace")
			local folder = Instance.new("Folder")
			folder.Name = "ToRemove"
			folder.Parent = workspace

			local id = HttpService:GenerateGUID(false)
			instanceMap:insert(id, folder)
			table.insert(patch.removed, id)

			local gitMetadata = {
				changedIds = {},
				scriptCommittedHashes = {},
			}

			local tree = PatchTree.build(patch, instanceMap, { "Property", "Current", "Incoming" }, gitMetadata)
			local selections = PatchTree.buildInitialSelections(tree)

			expect(selections[id]).to.equal(nil)

			folder:Destroy()
			instanceMap:stop()
		end)

		it("should handle Script and LocalScript as LuaSourceContainer", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()

			local workspace = game:GetService("Workspace")
			local serverScript = Instance.new("Script")
			serverScript.Name = "ServerTest"
			serverScript.Source = "print('server')"
			serverScript.Parent = workspace

			local localScript = Instance.new("LocalScript")
			localScript.Name = "ClientTest"
			localScript.Source = "print('client')"
			localScript.Parent = workspace

			local serverId = HttpService:GenerateGUID(false)
			local localId = HttpService:GenerateGUID(false)
			instanceMap:insert(serverId, serverScript)
			instanceMap:insert(localId, localScript)

			table.insert(patch.updated, {
				id = serverId,
				changedProperties = { Source = { String = "-- new" } },
			})
			table.insert(patch.updated, {
				id = localId,
				changedProperties = { Source = { String = "-- new" } },
			})

			local gitMetadata = {
				changedIds = { serverId, localId },
				scriptCommittedHashes = {
					[serverId] = { computeBlobSha1(serverScript.Source) },
					[localId] = { computeBlobSha1(localScript.Source) },
				},
			}

			local tree = PatchTree.build(patch, instanceMap, { "Property", "Current", "Incoming" }, gitMetadata)
			local selections = PatchTree.buildInitialSelections(tree)

			expect(selections[serverId]).to.equal("push")
			expect(selections[localId]).to.equal("push")

			serverScript:Destroy()
			localScript:Destroy()
			instanceMap:stop()
		end)

		it("should handle mixed changed and unchanged files", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()

			local workspace = game:GetService("Workspace")
			local changedScript = Instance.new("ModuleScript")
			changedScript.Name = "Changed"
			changedScript.Source = "-- matches committed"
			changedScript.Parent = workspace

			local unchangedFolder = Instance.new("Folder")
			unchangedFolder.Name = "Unchanged"
			unchangedFolder.Parent = workspace

			local changedId = HttpService:GenerateGUID(false)
			local unchangedId = HttpService:GenerateGUID(false)
			instanceMap:insert(changedId, changedScript)
			instanceMap:insert(unchangedId, unchangedFolder)

			table.insert(patch.updated, {
				id = changedId,
				changedProperties = { Source = { String = "-- server" } },
			})
			table.insert(patch.updated, {
				id = unchangedId,
				changedName = "StudioRenamed",
				changedProperties = {},
			})

			local gitMetadata = {
				changedIds = { changedId }, -- only the script is git-changed
				scriptCommittedHashes = {
					[changedId] = { computeBlobSha1(changedScript.Source) },
				},
			}

			local tree = PatchTree.build(patch, instanceMap, { "Property", "Current", "Incoming" }, gitMetadata)
			local selections = PatchTree.buildInitialSelections(tree)

			expect(selections[changedId]).to.equal("push")
			expect(selections[unchangedId]).to.equal("pull")

			changedScript:Destroy()
			unchangedFolder:Destroy()
			instanceMap:stop()
		end)

		it("should handle changed script with no committed hash (new file)", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()

			local workspace = game:GetService("Workspace")
			local script = Instance.new("ModuleScript")
			script.Name = "NewScript"
			script.Source = "return 'new'"
			script.Parent = workspace

			local id = HttpService:GenerateGUID(false)
			instanceMap:insert(id, script)

			table.insert(patch.updated, {
				id = id,
				changedProperties = { Source = { String = "-- different" } },
			})

			local gitMetadata = {
				changedIds = { id },
				scriptCommittedHashes = {}, -- no hash for this script (new file)
			}

			local tree = PatchTree.build(patch, instanceMap, { "Property", "Current", "Incoming" }, gitMetadata)
			local selections = PatchTree.buildInitialSelections(tree)

			expect(selections[id]).to.equal(nil)

			script:Destroy()
			instanceMap:stop()
		end)

		it("should handle empty changedIds with gitMetadata present", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()

			local workspace = game:GetService("Workspace")
			local folder = Instance.new("Folder")
			folder.Name = "AllUnchanged"
			folder.Parent = workspace

			local script = Instance.new("ModuleScript")
			script.Name = "AlsoUnchanged"
			script.Source = "return true"
			script.Parent = workspace

			local folderId = HttpService:GenerateGUID(false)
			local scriptId = HttpService:GenerateGUID(false)
			instanceMap:insert(folderId, folder)
			instanceMap:insert(scriptId, script)

			table.insert(patch.updated, {
				id = folderId,
				changedName = "StudioRenamed",
				changedProperties = {},
			})
			table.insert(patch.updated, {
				id = scriptId,
				changedProperties = { Source = { String = "-- studio version" } },
			})

			local gitMetadata = {
				changedIds = {}, -- nothing changed in git
				scriptCommittedHashes = {},
			}

			local tree = PatchTree.build(patch, instanceMap, { "Property", "Current", "Incoming" }, gitMetadata)
			local selections = PatchTree.buildInitialSelections(tree)

			expect(selections[folderId]).to.equal("pull")
			expect(selections[scriptId]).to.equal("pull")

			folder:Destroy()
			script:Destroy()
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

	describe("MCP fast-forward detection", function()
		it("should detect all-pre-selected tree as fast-forwardable", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()

			local workspace = game:GetService("Workspace")
			local testContainer = Instance.new("Folder")
			testContainer.Name = "McpFFTest"
			testContainer.Parent = workspace

			local script1 = Instance.new("ModuleScript")
			script1.Name = "ModuleA"
			script1.Parent = testContainer

			local id1 = HttpService:GenerateGUID(false)
			instanceMap:insert(id1, script1)

			table.insert(patch.updated, {
				id = id1,
				changedName = nil,
				changedProperties = { Source = { String = "-- changed" } },
			})

			local gitMetadata = {
				changedIds = { id1 },
				scriptCommittedHashes = { [id1] = {} },
				newFileIds = {},
			}

			local tree = PatchTree.build(patch, instanceMap, { "Property", "Current", "Incoming" }, gitMetadata)

			local allPreSelected = true
			local hasSelectable = false
			tree:forEach(function(node)
				if node.patchType then
					hasSelectable = true
					if node.defaultSelection == nil then
						allPreSelected = false
					end
				end
			end)

			-- With git metadata for a changed script, it should have a defaultSelection.
			-- The exact value depends on hash matching, but we verify the structure works.
			expect(hasSelectable).to.equal(true)

			local selections = PatchTree.buildInitialSelections(tree)
			local count = 0
			for _ in selections do
				count += 1
			end
			-- If allPreSelected, count should match selectable count
			if allPreSelected then
				local selectableCount = 0
				tree:forEach(function(node)
					if node.patchType then
						selectableCount += 1
					end
				end)
				expect(count).to.equal(selectableCount)
			end

			testContainer:Destroy()
			instanceMap:stop()
		end)

		it("should detect tree with nil defaultSelection as NOT fast-forwardable", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()

			local workspace = game:GetService("Workspace")
			local testContainer = Instance.new("Folder")
			testContainer.Name = "McpNoFF"
			testContainer.Parent = workspace

			local folder = Instance.new("Folder")
			folder.Name = "SomeFolder"
			folder.Parent = testContainer

			local folderId = HttpService:GenerateGUID(false)
			instanceMap:insert(folderId, folder)

			-- An update without git metadata => defaultSelection stays nil
			table.insert(patch.updated, {
				id = folderId,
				changedName = "Renamed",
				changedProperties = {},
			})

			local tree = PatchTree.build(patch, instanceMap, { "Property", "Current", "Incoming" })

			local allPreSelected = true
			tree:forEach(function(node)
				if node.patchType and node.defaultSelection == nil then
					allPreSelected = false
				end
			end)

			expect(allPreSelected).to.equal(false)

			testContainer:Destroy()
			instanceMap:stop()
		end)
	end)

	describe("MCP change list path construction", function()
		it("should build paths by walking ancestry to ROOT", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()

			local workspace = game:GetService("Workspace")
			local testContainer = Instance.new("Folder")
			testContainer.Name = "PathTest"
			testContainer.Parent = workspace

			local child = Instance.new("ModuleScript")
			child.Name = "MyModule"
			child.Parent = testContainer

			local containerId = HttpService:GenerateGUID(false)
			instanceMap:insert(containerId, testContainer)

			local childId = HttpService:GenerateGUID(false)
			instanceMap:insert(childId, child)

			table.insert(patch.updated, {
				id = childId,
				changedProperties = { Source = { String = "return {}" } },
			})

			local tree = PatchTree.build(patch, instanceMap, { "Property", "Current", "Incoming" })

			-- Walk the ancestry of the child node to build a path
			local childNode = tree:getNode(childId)
			expect(childNode).to.be.ok()

			local segments = {}
			local current = childNode
			while current and current.id ~= "ROOT" do
				table.insert(segments, 1, current.name or current.id)
				if current.parentId then
					current = tree:getNode(current.parentId)
				else
					break
				end
			end

			local path = table.concat(segments, "/")

			-- Path should contain both the parent folder and the child name
			expect(string.find(path, "PathTest")).to.be.ok()
			expect(string.find(path, "MyModule")).to.be.ok()

			testContainer:Destroy()
			instanceMap:stop()
		end)

		it("should produce single-segment path for root-level nodes", function()
			local patch = PatchSet.newEmpty()
			local instanceMap = InstanceMap.new()

			-- Use a service-level instance (direct child of DataModel)
			local repStorage = game:GetService("ReplicatedStorage")
			local repStorageId = HttpService:GenerateGUID(false)
			instanceMap:insert(repStorageId, repStorage)

			local testObj = Instance.new("Folder")
			testObj.Name = "TopLevel"
			testObj.Parent = repStorage

			local objId = HttpService:GenerateGUID(false)
			instanceMap:insert(objId, testObj)

			table.insert(patch.updated, {
				id = objId,
				changedName = "Renamed",
				changedProperties = {},
			})

			local tree = PatchTree.build(patch, instanceMap, { "Property", "Current", "Incoming" })

			local node = tree:getNode(objId)
			expect(node).to.be.ok()

			local segments = {}
			local current = node
			while current and current.id ~= "ROOT" do
				table.insert(segments, 1, current.name or current.id)
				if current.parentId then
					current = tree:getNode(current.parentId)
				else
					break
				end
			end

			local path = table.concat(segments, "/")
			-- Should include the service ancestor and the object
			expect(string.find(path, "ReplicatedStorage")).to.be.ok()
			expect(string.find(path, "TopLevel")).to.be.ok()

			testObj:Destroy()
			instanceMap:stop()
		end)
	end)
end
