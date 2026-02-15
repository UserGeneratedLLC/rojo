return function()
	local PatchSet = require(script.Parent.Parent.PatchSet)
	local InstanceMap = require(script.Parent.Parent.InstanceMap)

	local createPatchSet = require(script.Parent.createPatchSet)

	it("should return a patch", function()
		local patch = createPatchSet(InstanceMap.new(), {})

		assert(PatchSet.validate(patch))
	end)

	it("should contain updates for every instance with property changes", function()
		local instanceMap = InstanceMap.new()

		local part1 = Instance.new("Part")
		instanceMap:insert("PART_1", part1)

		local part2 = Instance.new("Part")
		instanceMap:insert("PART_2", part2)

		local changes = {
			[part1] = {
				Position = true,
				Size = true,
				Color = true,
			},
			[part2] = {
				CFrame = true,
				Velocity = true,
				Transparency = true,
			},
		}

		local patch = createPatchSet(instanceMap, changes)

		expect(#patch.updated).to.equal(2)
	end)

	it("should not contain any updates for removed instances", function()
		local instanceMap = InstanceMap.new()

		local part1 = Instance.new("Part")
		instanceMap:insert("PART_1", part1)

		local changes = {
			[part1] = {
				Parent = true,
				Position = true,
				Size = true,
			},
		}

		local patch = createPatchSet(instanceMap, changes)

		expect(#patch.removed).to.equal(1)
		expect(#patch.updated).to.equal(0)
	end)

	it("should remove instances from the property change table", function()
		local instanceMap = InstanceMap.new()

		local part1 = Instance.new("Part")
		instanceMap:insert("PART_1", part1)

		local changes = {
			[part1] = {},
		}

		createPatchSet(instanceMap, changes)

		expect(next(changes)).to.equal(nil)
	end)

	-- -------------------------------------------------------------------
	-- Ref property in patch creation
	-- -------------------------------------------------------------------

	describe("Ref properties", function()
		local container

		beforeEach(function()
			container = Instance.new("Folder")
			container.Name = "RefPatchTestContainer"
			container.Parent = game:GetService("Workspace")
		end)

		afterEach(function()
			if container then
				container:Destroy()
				container = nil
			end
		end)

		it("should include Ref property changes in patch", function()
			local instanceMap = InstanceMap.new()
			local model = Instance.new("Model")
			model.Parent = container
			local part = Instance.new("Part")
			part.Parent = model
			model.PrimaryPart = part

			instanceMap:insert("MODEL", model)
			instanceMap:insert("PART", part)

			local changes = {
				[model] = { PrimaryPart = true },
			}

			local patch = createPatchSet(instanceMap, changes)

			expect(#patch.updated).to.equal(1)
			expect(patch.updated[1].changedProperties.PrimaryPart).to.be.ok()
			expect(patch.updated[1].changedProperties.PrimaryPart.Ref).to.equal("PART")
		end)

		it("should skip Ref changes when target not tracked", function()
			local instanceMap = InstanceMap.new()
			local model = Instance.new("Model")
			model.Parent = container
			local part = Instance.new("Part")
			part.Parent = model
			model.PrimaryPart = part

			instanceMap:insert("MODEL", model)
			-- part NOT in instanceMap

			local changes = {
				[model] = { PrimaryPart = true },
			}

			local patch = createPatchSet(instanceMap, changes)

			-- No encodable changes → empty updated
			expect(#patch.updated).to.equal(0)
		end)

		it("should handle removal alongside Ref change on different instance", function()
			local instanceMap = InstanceMap.new()
			local model = Instance.new("Model")
			model.Parent = container
			local part = Instance.new("Part")
			part.Parent = model
			model.PrimaryPart = part

			local toRemove = Instance.new("Part")
			toRemove.Parent = container
			toRemove.Parent = nil -- Simulate deletion

			instanceMap:insert("MODEL", model)
			instanceMap:insert("PART", part)
			instanceMap:insert("REMOVED", toRemove)

			local changes = {
				[model] = { PrimaryPart = true },
				[toRemove] = { Parent = true },
			}

			local patch = createPatchSet(instanceMap, changes)

			expect(#patch.removed).to.equal(1)
			expect(#patch.updated).to.equal(1)
		end)

		it("should pass instanceMap in syncSourceOnly mode", function()
			local instanceMap = InstanceMap.new()
			local model = Instance.new("Model")
			model.Parent = container
			local part = Instance.new("Part")
			part.Parent = model
			model.PrimaryPart = part

			instanceMap:insert("MODEL", model)
			instanceMap:insert("PART", part)

			local changes = {
				[model] = { PrimaryPart = true },
			}

			-- In syncSourceOnly mode, only Source changes go through.
			-- PrimaryPart is NOT Source, so it should be filtered out.
			local patch = createPatchSet(instanceMap, changes, true)

			expect(#patch.updated).to.equal(0)
		end)
	end)
end
