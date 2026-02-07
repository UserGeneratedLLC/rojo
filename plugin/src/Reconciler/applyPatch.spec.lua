return function()
	local applyPatch = require(script.Parent.applyPatch)

	local InstanceMap = require(script.Parent.Parent.InstanceMap)
	local PatchSet = require(script.Parent.Parent.PatchSet)

	local container = Instance.new("Folder")

	local tempContainer = Instance.new("Folder")
	local function wasRemoved(instance)
		-- If an instance was destroyed, its parent property is locked.
		-- If an instance was removed, its parent property is nil.
		-- We need to ensure we only remove, so that ChangeHistoryService can still Undo.

		local isParentUnlocked = pcall(function()
			local oldParent = instance.Parent
			instance.Parent = tempContainer
			instance.Parent = oldParent
		end)

		return instance.Parent == nil and isParentUnlocked
	end

	beforeEach(function()
		container:ClearAllChildren()
	end)

	afterAll(function()
		container:Destroy()
		tempContainer:Destroy()
	end)

	it("should return an empty patch if given an empty patch", function()
		local patch = applyPatch(InstanceMap.new(), PatchSet.newEmpty())
		assert(PatchSet.isEmpty(patch), "expected remaining patch to be empty")
	end)

	it("should remove instances listed for remove", function()
		local root = Instance.new("Folder")
		root.Name = "ROOT"
		root.Parent = container

		local child = Instance.new("Folder")
		child.Name = "Child"
		child.Parent = root

		local instanceMap = InstanceMap.new()
		instanceMap:insert("ROOT", root)
		instanceMap:insert("CHILD", child)

		local patch = PatchSet.newEmpty()
		table.insert(patch.removed, child)

		local unapplied = applyPatch(instanceMap, patch)
		assert(PatchSet.isEmpty(unapplied), "expected remaining patch to be empty")

		assert(not wasRemoved(root), "expected root to be left alone")
		assert(wasRemoved(child), "expected child to be removed")

		instanceMap:stop()
	end)

	it("should remove IDs listed for remove", function()
		local root = Instance.new("Folder")
		root.Name = "ROOT"
		root.Parent = container

		local child = Instance.new("Folder")
		child.Name = "Child"
		child.Parent = root

		local instanceMap = InstanceMap.new()
		instanceMap:insert("ROOT", root)
		instanceMap:insert("CHILD", child)

		local patch = PatchSet.newEmpty()
		table.insert(patch.removed, "CHILD")

		local unapplied = applyPatch(instanceMap, patch)
		assert(PatchSet.isEmpty(unapplied), "expected remaining patch to be empty")
		expect(instanceMap:size()).to.equal(1)

		assert(not wasRemoved(root), "expected root to be left alone")
		assert(wasRemoved(child), "expected child to be removed")

		instanceMap:stop()
	end)

	it("should add instances to the DOM", function()
		-- Many of the details of this functionality are instead covered by
		-- tests on reify, not here.

		local root = Instance.new("Folder")
		root.Name = "ROOT"
		root.Parent = container

		local instanceMap = InstanceMap.new()
		instanceMap:insert("ROOT", root)

		local patch = PatchSet.newEmpty()
		patch.added["CHILD"] = {
			Id = "CHILD",
			ClassName = "Model",
			Name = "Child",
			Parent = "ROOT",
			Children = { "GRANDCHILD" },
			Properties = {},
		}

		patch.added["GRANDCHILD"] = {
			Id = "GRANDCHILD",
			ClassName = "Part",
			Name = "Grandchild",
			Parent = "CHILD",
			Children = {},
			Properties = {},
		}

		local unapplied = applyPatch(instanceMap, patch)
		assert(PatchSet.isEmpty(unapplied), "expected remaining patch to be empty")
		expect(instanceMap:size()).to.equal(3)

		local child = root:FindFirstChild("Child")
		expect(child).to.be.ok()
		expect(child.ClassName).to.equal("Model")
		expect(child).to.equal(instanceMap.fromIds["CHILD"])

		local grandchild = child:FindFirstChild("Grandchild")
		expect(grandchild).to.be.ok()
		expect(grandchild.ClassName).to.equal("Part")
		expect(grandchild).to.equal(instanceMap.fromIds["GRANDCHILD"])
	end)

	it("should return unapplied additions when instances cannot be created", function()
		local root = Instance.new("Folder")
		root.Name = "ROOT"
		root.Parent = container

		local instanceMap = InstanceMap.new()
		instanceMap:insert("ROOT", root)

		local patch = PatchSet.newEmpty()
		patch.added["OOPSIE"] = {
			Id = "OOPSIE",
			-- Hopefully Roblox never makes an instance with this ClassName.
			ClassName = "UH OH",
			Name = "FUBAR",
			Parent = "ROOT",
			Children = {},
			Properties = {},
		}

		local unapplied = applyPatch(instanceMap, patch)
		expect(unapplied.added["OOPSIE"]).to.equal(patch.added["OOPSIE"])
		expect(instanceMap:size()).to.equal(1)
		expect(#root:GetChildren()).to.equal(0)
	end)

	it("should apply property changes to instances", function()
		local value = Instance.new("StringValue")
		value.Value = "HELLO"

		local instanceMap = InstanceMap.new()
		instanceMap:insert("VALUE", value)

		local patch = PatchSet.newEmpty()
		table.insert(patch.updated, {
			id = "VALUE",
			changedProperties = {
				Value = {
					String = "WORLD",
				},
			},
		})

		local unapplied = applyPatch(instanceMap, patch)
		assert(PatchSet.isEmpty(unapplied), "expected remaining patch to be empty")
		expect(value.Value).to.equal("WORLD")
	end)

	it("should recreate instances when changedClassName is set, preserving children", function()
		local root = Instance.new("Folder")
		root.Name = "Initial Root Name"
		root.Parent = container

		local child = Instance.new("Folder")
		child.Name = "Child"
		child.Parent = root

		local instanceMap = InstanceMap.new()
		instanceMap:insert("ROOT", root)
		instanceMap:insert("CHILD", child)

		local patch = PatchSet.newEmpty()
		table.insert(patch.updated, {
			id = "ROOT",
			changedName = "Updated Root Name",
			changedClassName = "StringValue",
			changedProperties = {
				Value = {
					String = "I am Root",
				},
			},
		})

		local unapplied = applyPatch(instanceMap, patch)
		assert(PatchSet.isEmpty(unapplied), "expected remaining patch to be empty")

		local newRoot = instanceMap.fromIds["ROOT"]
		assert(newRoot ~= root, "expected instance to be recreated")
		expect(newRoot.ClassName).to.equal("StringValue")
		expect(newRoot.Name).to.equal("Updated Root Name")
		expect(newRoot.Value).to.equal("I am Root")

		local newChild = newRoot:FindFirstChild("Child")
		assert(newChild ~= nil, "expected child to be present")
		assert(newChild == child, "expected child to be preserved")
	end)

	describe("missing instance during update", function()
		it("should return unapplied update when instance is not in map", function()
			local instanceMap = InstanceMap.new()

			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, {
				id = "MISSING",
				changedProperties = {
					Value = { String = "hello" },
				},
			})

			local unapplied = applyPatch(instanceMap, patch)

			expect(#unapplied.updated).to.equal(1)
			expect(unapplied.updated[1].id).to.equal("MISSING")

			instanceMap:stop()
		end)
	end)

	describe("ignored class names during removal", function()
		it("should skip Camera instances during removal", function()
			local root = Instance.new("Folder")
			root.Name = "ROOT"
			root.Parent = container

			local camera = Instance.new("Camera")
			camera.Parent = root

			local instanceMap = InstanceMap.new()
			instanceMap:insert("ROOT", root)
			instanceMap:insert("CAMERA", camera)

			local patch = PatchSet.newEmpty()
			table.insert(patch.removed, camera)

			local unapplied = applyPatch(instanceMap, patch)

			-- Camera should be skipped (not removed, not unapplied)
			assert(PatchSet.isEmpty(unapplied), "expected remaining patch to be empty")
			-- Camera should still exist
			expect(camera.Parent).to.equal(root)

			instanceMap:stop()
		end)
	end)

	describe("instance already exists during addition", function()
		it("should skip adding instance that already exists in map", function()
			local root = Instance.new("Folder")
			root.Name = "ROOT"
			root.Parent = container

			local existing = Instance.new("Folder")
			existing.Name = "Existing"
			existing.Parent = root

			local instanceMap = InstanceMap.new()
			instanceMap:insert("ROOT", root)
			instanceMap:insert("EXISTING", existing)

			local patch = PatchSet.newEmpty()
			patch.added["EXISTING"] = {
				Id = "EXISTING",
				ClassName = "Folder",
				Name = "Existing",
				Parent = "ROOT",
				Children = {},
				Properties = {},
			}

			local unapplied = applyPatch(instanceMap, patch)

			-- Should be skipped (instance already exists)
			assert(PatchSet.isEmpty(unapplied), "expected remaining patch to be empty")
			-- Should still be the same instance
			expect(instanceMap.fromIds["EXISTING"]).to.equal(existing)

			instanceMap:stop()
		end)
	end)

	describe("partial property failure", function()
		it("should track failed decode in unapplied while applying others", function()
			local value = Instance.new("StringValue")
			value.Name = "Test"
			value.Value = "Original"
			value.Parent = container

			local instanceMap = InstanceMap.new()
			instanceMap:insert("VALUE", value)

			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, {
				id = "VALUE",
				changedProperties = {
					Value = { String = "NewValue" },
					-- Invalid encoded type — decodeValue will fail
					Name = { BOGUS_ENCODED_TYPE = "Should fail to decode" },
				},
			})

			local unapplied = applyPatch(instanceMap, patch)

			-- Valid property should have been applied
			expect(value.Value).to.equal("NewValue")

			-- Failed property should be in unapplied
			expect(#unapplied.updated).to.equal(1)
			expect(unapplied.updated[1].changedProperties.Name).to.be.ok()
			-- Successfully applied property should NOT be in unapplied
			expect(unapplied.updated[1].changedProperties.Value).to.equal(nil)

			instanceMap:stop()
		end)

		it("should silently skip unknown properties (setProperty returns true)", function()
			-- setProperty returns true for unknown properties by design
			-- (they are assumed to be serialization-only, not reflected to Lua)
			local folder = Instance.new("Folder")
			folder.Name = "Test"
			folder.Parent = container

			local instanceMap = InstanceMap.new()
			instanceMap:insert("FOLDER", folder)

			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, {
				id = "FOLDER",
				changedProperties = {
					UnknownSerializationProperty = { String = "value" },
				},
			})

			local unapplied = applyPatch(instanceMap, patch)

			-- Unknown properties are silently skipped (treated as success)
			assert(PatchSet.isEmpty(unapplied), "expected remaining patch to be empty")

			instanceMap:stop()
		end)
	end)

	describe("name change", function()
		it("should apply name changes", function()
			local folder = Instance.new("Folder")
			folder.Name = "OldName"
			folder.Parent = container

			local instanceMap = InstanceMap.new()
			instanceMap:insert("FOLDER", folder)

			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, {
				id = "FOLDER",
				changedName = "NewName",
				changedProperties = {},
			})

			local unapplied = applyPatch(instanceMap, patch)
			assert(PatchSet.isEmpty(unapplied), "expected remaining patch to be empty")

			expect(folder.Name).to.equal("NewName")

			instanceMap:stop()
		end)
	end)

	describe("changedMetadata", function()
		it("should always return changedMetadata as unapplied", function()
			local folder = Instance.new("Folder")
			folder.Name = "Test"
			folder.Parent = container

			local instanceMap = InstanceMap.new()
			instanceMap:insert("META_FOLDER", folder)

			-- Verify the instance is actually in the map
			assert(instanceMap.fromIds["META_FOLDER"] == folder, "instance not in map")

			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, {
				id = "META_FOLDER",
				changedMetadata = {
					ignoreUnknownInstances = true,
				},
				changedProperties = {},
			})

			local unapplied = applyPatch(instanceMap, patch)

			-- changedMetadata is always unapplied (not yet supported)
			expect(#unapplied.updated).to.equal(1)
			expect(unapplied.updated[1].changedMetadata).to.be.ok()
			expect(unapplied.updated[1].changedMetadata.ignoreUnknownInstances).to.equal(true)

			instanceMap:stop()
		end)
	end)

	describe("removal edge cases", function()
		it("should silently handle removal of non-existent ID", function()
			local instanceMap = InstanceMap.new()

			-- Remove by ID with no corresponding instance
			-- destroyId silently succeeds (pcall doesn't error)
			local patch = PatchSet.newEmpty()
			table.insert(patch.removed, "NONEXISTENT_ID")

			local unapplied = applyPatch(instanceMap, patch)

			-- Should succeed silently (destroyId is a no-op for unknown IDs)
			assert(PatchSet.isEmpty(unapplied), "expected remaining patch to be empty")

			instanceMap:stop()
		end)

		it("should handle removal of multiple instances", function()
			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local child1 = Instance.new("Folder")
			child1.Name = "Child1"
			child1.Parent = root

			local child2 = Instance.new("Folder")
			child2.Name = "Child2"
			child2.Parent = root

			local instanceMap = InstanceMap.new()
			instanceMap:insert("ROOT", root)
			instanceMap:insert("CHILD1", child1)
			instanceMap:insert("CHILD2", child2)

			local patch = PatchSet.newEmpty()
			table.insert(patch.removed, "CHILD1")
			table.insert(patch.removed, "CHILD2")

			local unapplied = applyPatch(instanceMap, patch)
			assert(PatchSet.isEmpty(unapplied), "expected remaining patch to be empty")

			expect(wasRemoved(child1)).to.equal(true)
			expect(wasRemoved(child2)).to.equal(true)
			expect(instanceMap:size()).to.equal(1)

			instanceMap:stop()
		end)
	end)

	describe("multiple operations in one patch", function()
		it("should handle add, remove, and update in the same patch", function()
			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local toRemove = Instance.new("Folder")
			toRemove.Name = "ToRemove"
			toRemove.Parent = root

			local toUpdate = Instance.new("StringValue")
			toUpdate.Name = "ToUpdate"
			toUpdate.Value = "Old"
			toUpdate.Parent = root

			local instanceMap = InstanceMap.new()
			instanceMap:insert("ROOT", root)
			instanceMap:insert("REMOVE", toRemove)
			instanceMap:insert("UPDATE", toUpdate)

			local patch = PatchSet.newEmpty()

			-- Remove
			table.insert(patch.removed, "REMOVE")

			-- Add
			patch.added["NEW"] = {
				Id = "NEW",
				ClassName = "Part",
				Name = "NewPart",
				Parent = "ROOT",
				Children = {},
				Properties = {},
			}

			-- Update
			table.insert(patch.updated, {
				id = "UPDATE",
				changedProperties = {
					Value = { String = "New" },
				},
			})

			local unapplied = applyPatch(instanceMap, patch)
			assert(PatchSet.isEmpty(unapplied), "expected remaining patch to be empty")

			-- Removed
			expect(wasRemoved(toRemove)).to.equal(true)

			-- Added
			local newPart = root:FindFirstChild("NewPart")
			expect(newPart).to.be.ok()
			expect(newPart.ClassName).to.equal("Part")

			-- Updated
			expect(toUpdate.Value).to.equal("New")

			instanceMap:stop()
		end)
	end)
end
