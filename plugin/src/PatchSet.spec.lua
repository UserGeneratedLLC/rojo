return function()
	local PatchSet = require(script.Parent.PatchSet)
	local InstanceMap = require(script.Parent.InstanceMap)

	describe("newEmpty", function()
		it("should create an empty patch", function()
			local patch = PatchSet.newEmpty()
			expect(PatchSet.isEmpty(patch)).to.equal(true)
		end)
	end)

	describe("isEmpty", function()
		it("should return true for empty patches", function()
			local patch = PatchSet.newEmpty()
			expect(PatchSet.isEmpty(patch)).to.equal(true)
		end)

		it("should return false when patch has removals", function()
			local patch = PatchSet.newEmpty()
			table.insert(patch.removed, "some-id")
			expect(PatchSet.isEmpty(patch)).to.equal(false)
		end)

		it("should return false when patch has additions", function()
			local patch = PatchSet.newEmpty()
			patch.added["some-id"] = { Id = "some-id", ClassName = "Folder", Name = "Test" }
			expect(PatchSet.isEmpty(patch)).to.equal(false)
		end)

		it("should return false when patch has updates", function()
			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, { id = "some-id", changedProperties = {} })
			expect(PatchSet.isEmpty(patch)).to.equal(false)
		end)
	end)

	describe("merge", function()
		it("should merge additions from source into target", function()
			local target = PatchSet.newEmpty()
			local source = PatchSet.newEmpty()

			source.added["CHILD"] = {
				Id = "CHILD",
				ClassName = "Folder",
				Name = "Child",
				Parent = "ROOT",
				Children = {},
				Properties = {},
			}

			PatchSet.merge(target, source)

			expect(target.added["CHILD"]).to.be.ok()
			expect(target.added["CHILD"].Name).to.equal("Child")
		end)

		it("should merge removals from source into target", function()
			local target = PatchSet.newEmpty()
			local source = PatchSet.newEmpty()

			table.insert(source.removed, "CHILD")

			PatchSet.merge(target, source)

			expect(#target.removed).to.equal(1)
			expect(target.removed[1]).to.equal("CHILD")
		end)

		it("should cancel additions when source removes them", function()
			local target = PatchSet.newEmpty()
			target.added["CHILD"] = {
				Id = "CHILD",
				ClassName = "Folder",
				Name = "Child",
				Parent = "ROOT",
				Children = {},
				Properties = {},
			}

			local source = PatchSet.newEmpty()
			table.insert(source.removed, "CHILD")

			PatchSet.merge(target, source)

			-- The addition should be cancelled
			expect(target.added["CHILD"]).to.equal(nil)
			-- And no removal should be added since it was never actually applied
			expect(#target.removed).to.equal(0)
		end)

		it("should merge updates from source into target", function()
			local target = PatchSet.newEmpty()
			local source = PatchSet.newEmpty()

			table.insert(source.updated, {
				id = "INSTANCE",
				changedName = "NewName",
				changedProperties = {},
			})

			PatchSet.merge(target, source)

			expect(#target.updated).to.equal(1)
			expect(target.updated[1].changedName).to.equal("NewName")
		end)

		it("should combine updates for the same instance", function()
			local target = PatchSet.newEmpty()
			table.insert(target.updated, {
				id = "INSTANCE",
				changedName = "FirstName",
				changedProperties = {
					Value = { String = "First" },
				},
			})

			local source = PatchSet.newEmpty()
			table.insert(source.updated, {
				id = "INSTANCE",
				changedName = "SecondName",
				changedProperties = {
					OtherValue = { String = "Second" },
				},
			})

			PatchSet.merge(target, source)

			-- Should still only have one update entry
			expect(#target.updated).to.equal(1)
			-- Name should be overwritten by the newer update
			expect(target.updated[1].changedName).to.equal("SecondName")
			-- Both properties should be present
			expect(target.updated[1].changedProperties.Value).to.be.ok()
			expect(target.updated[1].changedProperties.OtherValue).to.be.ok()
		end)

		it("should remove update when reverted to current instance state", function()
			-- Create an instance to compare against
			local testInstance = Instance.new("StringValue")
			testInstance.Name = "OriginalName"
			testInstance.Value = "OriginalValue"

			local instanceMap = InstanceMap.new()
			instanceMap:insert("INSTANCE", testInstance)

			-- Target has an update changing the name
			local target = PatchSet.newEmpty()
			table.insert(target.updated, {
				id = "INSTANCE",
				changedName = "ChangedName",
				changedProperties = {},
			})

			-- Source reverts the name back to original
			local source = PatchSet.newEmpty()
			table.insert(source.updated, {
				id = "INSTANCE",
				changedName = "OriginalName",
				changedProperties = {},
			})

			PatchSet.merge(target, source, instanceMap)

			-- The update should be removed entirely since it matches current state
			expect(#target.updated).to.equal(0)

			testInstance:Destroy()
			instanceMap:stop()
		end)

		it("should remove individual property changes when reverted", function()
			-- Create an instance to compare against
			local testInstance = Instance.new("StringValue")
			testInstance.Name = "Test"
			testInstance.Value = "OriginalValue"

			local instanceMap = InstanceMap.new()
			instanceMap:insert("INSTANCE", testInstance)

			-- Target has property change
			local target = PatchSet.newEmpty()
			table.insert(target.updated, {
				id = "INSTANCE",
				changedName = "NewName",
				changedProperties = {
					Value = { String = "ChangedValue" },
				},
			})

			-- Source reverts the property back to original
			local source = PatchSet.newEmpty()
			table.insert(source.updated, {
				id = "INSTANCE",
				changedProperties = {
					Value = { String = "OriginalValue" },
				},
			})

			PatchSet.merge(target, source, instanceMap)

			-- Should still have update for the name change
			expect(#target.updated).to.equal(1)
			expect(target.updated[1].changedName).to.equal("NewName")
			-- But the Value property should be removed since it matches current
			expect(target.updated[1].changedProperties.Value).to.equal(nil)

			testInstance:Destroy()
			instanceMap:stop()
		end)

		it("should keep updates for different instances separate", function()
			local target = PatchSet.newEmpty()
			table.insert(target.updated, {
				id = "INSTANCE_A",
				changedName = "NameA",
				changedProperties = {},
			})

			local source = PatchSet.newEmpty()
			table.insert(source.updated, {
				id = "INSTANCE_B",
				changedName = "NameB",
				changedProperties = {},
			})

			PatchSet.merge(target, source)

			expect(#target.updated).to.equal(2)
		end)
	end)

	describe("assign", function()
		it("should merge multiple patches additively", function()
			local target = PatchSet.newEmpty()

			local source1 = PatchSet.newEmpty()
			source1.added["A"] = { Id = "A", ClassName = "Folder", Name = "A" }

			local source2 = PatchSet.newEmpty()
			source2.added["B"] = { Id = "B", ClassName = "Folder", Name = "B" }

			PatchSet.assign(target, source1, source2)

			expect(target.added["A"]).to.be.ok()
			expect(target.added["B"]).to.be.ok()
		end)
	end)

	describe("countChanges", function()
		it("should count property changes in additions", function()
			local patch = PatchSet.newEmpty()
			patch.added["A"] = {
				Id = "A",
				ClassName = "StringValue",
				Name = "A",
				Properties = {
					Value = { String = "test" },
					MaxValue = { Float64 = 100 },
				},
			}

			expect(PatchSet.countChanges(patch)).to.equal(2)
		end)

		it("should count removals as single changes", function()
			local patch = PatchSet.newEmpty()
			table.insert(patch.removed, "A")
			table.insert(patch.removed, "B")

			expect(PatchSet.countChanges(patch)).to.equal(2)
		end)

		it("should count property updates", function()
			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, {
				id = "A",
				changedProperties = {
					Value = { String = "test" },
				},
			})

			expect(PatchSet.countChanges(patch)).to.equal(1)
		end)

		it("should count name changes", function()
			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, {
				id = "A",
				changedName = "NewName",
				changedProperties = {},
			})

			expect(PatchSet.countChanges(patch)).to.equal(1)
		end)

		it("should count className changes", function()
			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, {
				id = "A",
				changedClassName = "Model",
				changedProperties = {},
			})

			expect(PatchSet.countChanges(patch)).to.equal(1)
		end)
	end)

	describe("countInstances", function()
		it("should count all affected instances", function()
			local patch = PatchSet.newEmpty()
			patch.added["A"] = { Id = "A", ClassName = "Folder", Name = "A" }
			patch.added["B"] = { Id = "B", ClassName = "Folder", Name = "B" }
			table.insert(patch.removed, "C")
			table.insert(patch.updated, { id = "D", changedProperties = {} })

			expect(PatchSet.countInstances(patch)).to.equal(4)
		end)
	end)

	describe("isEqual", function()
		it("should return true for identical patches", function()
			local patchA = PatchSet.newEmpty()
			patchA.added["A"] = { Id = "A", ClassName = "Folder", Name = "A" }

			local patchB = PatchSet.newEmpty()
			patchB.added["A"] = { Id = "A", ClassName = "Folder", Name = "A" }

			expect(PatchSet.isEqual(patchA, patchB)).to.equal(true)
		end)

		it("should return false for different patches", function()
			local patchA = PatchSet.newEmpty()
			patchA.added["A"] = { Id = "A", ClassName = "Folder", Name = "A" }

			local patchB = PatchSet.newEmpty()
			patchB.added["B"] = { Id = "B", ClassName = "Folder", Name = "B" }

			expect(PatchSet.isEqual(patchA, patchB)).to.equal(false)
		end)
	end)

	describe("hasRemoves", function()
		it("should return false for empty patch", function()
			expect(PatchSet.hasRemoves(PatchSet.newEmpty())).to.equal(false)
		end)

		it("should return true when patch has removals", function()
			local patch = PatchSet.newEmpty()
			table.insert(patch.removed, "some-id")
			expect(PatchSet.hasRemoves(patch)).to.equal(true)
		end)
	end)

	describe("hasAdditions", function()
		it("should return false for empty patch", function()
			expect(PatchSet.hasAdditions(PatchSet.newEmpty())).to.equal(false)
		end)

		it("should return true when patch has additions", function()
			local patch = PatchSet.newEmpty()
			patch.added["A"] = { Id = "A", ClassName = "Folder", Name = "A" }
			expect(PatchSet.hasAdditions(patch)).to.equal(true)
		end)
	end)

	describe("hasUpdates", function()
		it("should return false for empty patch", function()
			expect(PatchSet.hasUpdates(PatchSet.newEmpty())).to.equal(false)
		end)

		it("should return true when patch has updates", function()
			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, { id = "A", changedProperties = {} })
			expect(PatchSet.hasUpdates(patch)).to.equal(true)
		end)
	end)

	describe("containsId", function()
		it("should find id in additions", function()
			local instanceMap = InstanceMap.new()
			local patch = PatchSet.newEmpty()
			patch.added["TARGET"] = { Id = "TARGET", ClassName = "Folder", Name = "T" }

			expect(PatchSet.containsId(patch, instanceMap, "TARGET")).to.equal(true)

			instanceMap:stop()
		end)

		it("should find id in removals", function()
			local instanceMap = InstanceMap.new()
			local patch = PatchSet.newEmpty()
			table.insert(patch.removed, "TARGET")

			expect(PatchSet.containsId(patch, instanceMap, "TARGET")).to.equal(true)

			instanceMap:stop()
		end)

		it("should find id in updates", function()
			local instanceMap = InstanceMap.new()
			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, { id = "TARGET", changedProperties = {} })

			expect(PatchSet.containsId(patch, instanceMap, "TARGET")).to.equal(true)

			instanceMap:stop()
		end)

		it("should return false for non-existent id", function()
			local instanceMap = InstanceMap.new()
			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, { id = "OTHER", changedProperties = {} })

			expect(PatchSet.containsId(patch, instanceMap, "TARGET")).to.equal(false)

			instanceMap:stop()
		end)

		it("should find instance-based removal by resolving to id", function()
			local instanceMap = InstanceMap.new()
			local instance = Instance.new("Folder")
			instanceMap:insert("TARGET", instance)

			local patch = PatchSet.newEmpty()
			table.insert(patch.removed, instance)

			expect(PatchSet.containsId(patch, instanceMap, "TARGET")).to.equal(true)

			instance:Destroy()
			instanceMap:stop()
		end)
	end)

	describe("containsInstance", function()
		it("should find instance in patch via instanceMap", function()
			local instanceMap = InstanceMap.new()
			local instance = Instance.new("Folder")
			instanceMap:insert("INST", instance)

			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, { id = "INST", changedProperties = {} })

			expect(PatchSet.containsInstance(patch, instanceMap, instance)).to.equal(true)

			instance:Destroy()
			instanceMap:stop()
		end)

		it("should return false for instance not in instanceMap", function()
			local instanceMap = InstanceMap.new()
			local instance = Instance.new("Folder")

			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, { id = "INST", changedProperties = {} })

			expect(PatchSet.containsInstance(patch, instanceMap, instance)).to.equal(false)

			instance:Destroy()
			instanceMap:stop()
		end)
	end)

	describe("containsOnlyId", function()
		it("should return true when patch only affects the given id", function()
			local instanceMap = InstanceMap.new()
			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, { id = "TARGET", changedProperties = {} })

			expect(PatchSet.containsOnlyId(patch, instanceMap, "TARGET")).to.equal(true)

			instanceMap:stop()
		end)

		it("should return false when patch affects other ids too", function()
			local instanceMap = InstanceMap.new()
			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, { id = "TARGET", changedProperties = {} })
			table.insert(patch.updated, { id = "OTHER", changedProperties = {} })

			expect(PatchSet.containsOnlyId(patch, instanceMap, "TARGET")).to.equal(false)

			instanceMap:stop()
		end)

		it("should return false when id is not in patch at all", function()
			local instanceMap = InstanceMap.new()
			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, { id = "OTHER", changedProperties = {} })

			expect(PatchSet.containsOnlyId(patch, instanceMap, "TARGET")).to.equal(false)

			instanceMap:stop()
		end)
	end)

	describe("getUpdateForId", function()
		it("should return the update for a given id", function()
			local patch = PatchSet.newEmpty()
			local update = { id = "TARGET", changedName = "NewName", changedProperties = {} }
			table.insert(patch.updated, update)

			expect(PatchSet.getUpdateForId(patch, "TARGET")).to.equal(update)
		end)

		it("should return nil for non-existent id", function()
			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, { id = "OTHER", changedProperties = {} })

			expect(PatchSet.getUpdateForId(patch, "TARGET")).to.equal(nil)
		end)
	end)

	describe("getUpdateForInstance", function()
		it("should return the update for a given instance", function()
			local instanceMap = InstanceMap.new()
			local instance = Instance.new("Folder")
			instanceMap:insert("INST", instance)

			local patch = PatchSet.newEmpty()
			local update = { id = "INST", changedName = "New", changedProperties = {} }
			table.insert(patch.updated, update)

			expect(PatchSet.getUpdateForInstance(patch, instanceMap, instance)).to.equal(update)

			instance:Destroy()
			instanceMap:stop()
		end)

		it("should return nil for instance not in map", function()
			local instanceMap = InstanceMap.new()
			local instance = Instance.new("Folder")

			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, { id = "INST", changedProperties = {} })

			expect(PatchSet.getUpdateForInstance(patch, instanceMap, instance)).to.equal(nil)

			instance:Destroy()
			instanceMap:stop()
		end)
	end)

	describe("addedIdList", function()
		it("should return list of added IDs", function()
			local patch = PatchSet.newEmpty()
			patch.added["A"] = { Id = "A", ClassName = "Folder", Name = "A" }
			patch.added["B"] = { Id = "B", ClassName = "Folder", Name = "B" }

			local ids = PatchSet.addedIdList(patch)

			expect(#ids).to.equal(2)
			-- IDs should contain A and B (order may vary due to pairs iteration)
			local idSet = {}
			for _, id in ids do
				idSet[id] = true
			end
			expect(idSet["A"]).to.equal(true)
			expect(idSet["B"]).to.equal(true)
		end)

		it("should return empty list for patch with no additions", function()
			local patch = PatchSet.newEmpty()
			table.insert(patch.removed, "some-id")

			local ids = PatchSet.addedIdList(patch)
			expect(#ids).to.equal(0)
		end)
	end)

	describe("updatedIdList", function()
		it("should return list of updated IDs", function()
			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, { id = "X", changedProperties = {} })
			table.insert(patch.updated, { id = "Y", changedProperties = {} })

			local ids = PatchSet.updatedIdList(patch)

			expect(#ids).to.equal(2)
			expect(ids[1]).to.equal("X")
			expect(ids[2]).to.equal("Y")
		end)

		it("should return empty list for patch with no updates", function()
			local patch = PatchSet.newEmpty()
			table.insert(patch.removed, "some-id")

			local ids = PatchSet.updatedIdList(patch)
			expect(#ids).to.equal(0)
		end)
	end)

	describe("removeDataModelName", function()
		it("should remove changedName from DataModel update", function()
			local instanceMap = InstanceMap.new()
			instanceMap:insert("DM_ID", game)

			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, {
				id = "DM_ID",
				changedName = "SomeProjectName",
				changedClassName = "DataModel",
				changedProperties = {},
			})

			PatchSet.removeDataModelName(patch, instanceMap)

			expect(#patch.updated).to.equal(1)
			expect(patch.updated[1].changedName).to.equal(nil)

			instanceMap:stop()
		end)

		it("should remove entire update if only Name was changed", function()
			local instanceMap = InstanceMap.new()
			instanceMap:insert("DM_ID", game)

			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, {
				id = "DM_ID",
				changedName = "SomeProjectName",
				changedProperties = {},
			})

			PatchSet.removeDataModelName(patch, instanceMap)

			expect(#patch.updated).to.equal(0)

			instanceMap:stop()
		end)

		it("should do nothing if DataModel is not in instanceMap", function()
			local instanceMap = InstanceMap.new()

			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, {
				id = "SOME_ID",
				changedName = "Test",
				changedProperties = {},
			})

			PatchSet.removeDataModelName(patch, instanceMap)

			expect(#patch.updated).to.equal(1)
			expect(patch.updated[1].changedName).to.equal("Test")

			instanceMap:stop()
		end)

		it("should not affect non-DataModel updates", function()
			local instanceMap = InstanceMap.new()
			instanceMap:insert("DM_ID", game)

			local otherInstance = Instance.new("Folder")
			instanceMap:insert("OTHER", otherInstance)

			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, {
				id = "OTHER",
				changedName = "NewName",
				changedProperties = {},
			})

			PatchSet.removeDataModelName(patch, instanceMap)

			expect(#patch.updated).to.equal(1)
			expect(patch.updated[1].changedName).to.equal("NewName")

			otherInstance:Destroy()
			instanceMap:stop()
		end)
	end)

	describe("validate", function()
		it("should validate a well-formed patch", function()
			local patch = PatchSet.newEmpty()
			table.insert(patch.updated, { id = "A", changedProperties = {} })
			patch.added["B"] = {
				Id = "B",
				ClassName = "Folder",
				Name = "B",
				Parent = "A",
				Properties = {},
				Children = {},
			}
			table.insert(patch.removed, "C")

			expect(PatchSet.validate(patch)).to.equal(true)
		end)

		it("should validate an empty patch", function()
			expect(PatchSet.validate(PatchSet.newEmpty())).to.equal(true)
		end)
	end)
end
