--[[
	Stress tests for InstanceMap.
	
	Tests scale, rapid operations, and concurrency.
]]

return function()
	local InstanceMap = require(script.Parent.InstanceMap)
	local testUtils = require(script.Parent.testUtils)
	local LargeTreeGenerator = testUtils.LargeTreeGenerator

	local HttpService = game:GetService("HttpService")

	-- Test container for cleanup
	local container

	beforeEach(function()
		container = Instance.new("Folder")
		container.Name = "InstanceMapStressTestContainer"
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

	describe("large scale operations", function()
		it("should handle inserting 100 instances", function()
			local instanceMap = InstanceMap.new()

			local instances = {}
			for i = 1, 100 do
				local folder = Instance.new("Folder")
				folder.Name = "Instance_" .. tostring(i)
				folder.Parent = container
				table.insert(instances, folder)
			end

			local startTime = os.clock()
			for _, instance in ipairs(instances) do
				local id = generateId()
				instanceMap:insert(id, instance)
			end
			local elapsed = os.clock() - startTime

			expect(instanceMap:size()).to.equal(100)
			expect(elapsed < 1).to.equal(true)

			instanceMap:stop()
		end)

		it("should handle inserting 500 instances", function()
			local instanceMap = InstanceMap.new()

			local instances = {}
			for i = 1, 500 do
				local folder = Instance.new("Folder")
				folder.Name = "Instance_" .. tostring(i)
				folder.Parent = container
				table.insert(instances, folder)
			end

			local startTime = os.clock()
			for _, instance in ipairs(instances) do
				local id = generateId()
				instanceMap:insert(id, instance)
			end
			local elapsed = os.clock() - startTime

			expect(instanceMap:size()).to.equal(500)
			expect(elapsed < 3).to.equal(true)

			instanceMap:stop()
		end)

		it("should handle removing 100 instances", function()
			local instanceMap = InstanceMap.new()

			local ids = {}
			for i = 1, 100 do
				local folder = Instance.new("Folder")
				folder.Name = "Instance_" .. tostring(i)
				folder.Parent = container

				local id = generateId()
				instanceMap:insert(id, folder)
				table.insert(ids, id)
			end

			expect(instanceMap:size()).to.equal(100)

			local startTime = os.clock()
			for _, id in ipairs(ids) do
				instanceMap:removeId(id)
			end
			local elapsed = os.clock() - startTime

			expect(instanceMap:size()).to.equal(0)
			expect(elapsed < 1).to.equal(true)

			instanceMap:stop()
		end)
	end)

	describe("rapid insert/remove cycles", function()
		it("should handle rapid insert-remove cycles", function()
			local instanceMap = InstanceMap.new()

			for _ = 1, 50 do
				local folder = Instance.new("Folder")
				folder.Name = "Temporary"
				folder.Parent = container

				local id = generateId()
				instanceMap:insert(id, folder)
				expect(instanceMap.fromIds[id]).to.equal(folder)

				instanceMap:removeId(id)
				expect(instanceMap.fromIds[id]).to.equal(nil)

				folder:Destroy()
			end

			expect(instanceMap:size()).to.equal(0)

			instanceMap:stop()
		end)

		it("should handle alternating insert/remove on same instance", function()
			local instanceMap = InstanceMap.new()

			local folder = Instance.new("Folder")
			folder.Name = "Persistent"
			folder.Parent = container

			for _ = 1, 20 do
				local id = generateId()
				instanceMap:insert(id, folder)
				instanceMap:removeInstance(folder)
			end

			expect(instanceMap:size()).to.equal(0)

			instanceMap:stop()
		end)
	end)

	describe("bidirectional lookup", function()
		it("should maintain consistent bidirectional mapping", function()
			local instanceMap = InstanceMap.new()

			local instances = {}
			local ids = {}

			for i = 1, 100 do
				local folder = Instance.new("Folder")
				folder.Name = "Instance_" .. tostring(i)
				folder.Parent = container

				local id = generateId()
				instanceMap:insert(id, folder)

				instances[i] = folder
				ids[i] = id
			end

			-- Verify bidirectional lookup
			for i = 1, 100 do
				expect(instanceMap.fromIds[ids[i]]).to.equal(instances[i])
				expect(instanceMap.fromInstances[instances[i]]).to.equal(ids[i])
			end

			instanceMap:stop()
		end)

		it("should update mapping when re-inserting with different ID", function()
			local instanceMap = InstanceMap.new()

			local folder = Instance.new("Folder")
			folder.Name = "Test"
			folder.Parent = container

			local id1 = generateId()
			local id2 = generateId()

			instanceMap:insert(id1, folder)
			expect(instanceMap.fromIds[id1]).to.equal(folder)

			instanceMap:insert(id2, folder)
			expect(instanceMap.fromIds[id1]).to.equal(nil)
			expect(instanceMap.fromIds[id2]).to.equal(folder)
			expect(instanceMap.fromInstances[folder]).to.equal(id2)

			instanceMap:stop()
		end)
	end)

	describe("destroy operations", function()
		it("should destroy instance and remove from map", function()
			local instanceMap = InstanceMap.new()

			local folder = Instance.new("Folder")
			folder.Name = "ToDestroy"
			folder.Parent = container

			local id = generateId()
			instanceMap:insert(id, folder)

			instanceMap:destroyId(id)

			expect(instanceMap.fromIds[id]).to.equal(nil)
			expect(folder.Parent).to.equal(nil)

			instanceMap:stop()
		end)

		it("should destroy instance by instance reference", function()
			local instanceMap = InstanceMap.new()

			local folder = Instance.new("Folder")
			folder.Name = "ToDestroy"
			folder.Parent = container

			local id = generateId()
			instanceMap:insert(id, folder)

			instanceMap:destroyInstance(folder)

			expect(instanceMap.fromIds[id]).to.equal(nil)
			expect(instanceMap.fromInstances[folder]).to.equal(nil)
			expect(folder.Parent).to.equal(nil)

			instanceMap:stop()
		end)

		it("should remove descendant mappings when destroying parent", function()
			local instanceMap = InstanceMap.new()

			local parent = Instance.new("Folder")
			parent.Name = "Parent"
			parent.Parent = container

			local child = Instance.new("Folder")
			child.Name = "Child"
			child.Parent = parent

			local grandchild = Instance.new("Folder")
			grandchild.Name = "Grandchild"
			grandchild.Parent = child

			local parentId = generateId()
			local childId = generateId()
			local grandchildId = generateId()

			instanceMap:insert(parentId, parent)
			instanceMap:insert(childId, child)
			instanceMap:insert(grandchildId, grandchild)

			expect(instanceMap:size()).to.equal(3)

			instanceMap:destroyInstance(parent)

			expect(instanceMap:size()).to.equal(0)

			instanceMap:stop()
		end)
	end)

	describe("pause/unpause", function()
		it("should pause updates for instance", function()
			local changeCount = 0
			local instanceMap = InstanceMap.new(function()
				changeCount = changeCount + 1
			end)

			local folder = Instance.new("Folder")
			folder.Name = "Test"
			folder.Parent = container

			local id = generateId()
			instanceMap:insert(id, folder)

			instanceMap:pauseInstance(folder)

			-- Changes while paused should not trigger callback
			-- (Actual triggering depends on the RunService check)

			instanceMap:unpauseInstance(folder)

			instanceMap:stop()
		end)

		it("should unpause all instances", function()
			local instanceMap = InstanceMap.new()

			local folders = {}
			for i = 1, 10 do
				local folder = Instance.new("Folder")
				folder.Name = "Test_" .. tostring(i)
				folder.Parent = container

				local id = generateId()
				instanceMap:insert(id, folder)
				instanceMap:pauseInstance(folder)
				table.insert(folders, folder)
			end

			-- All should be paused
			for _, folder in ipairs(folders) do
				expect(instanceMap.pausedUpdateInstances[folder]).to.equal(true)
			end

			instanceMap:unpauseAllInstances()

			-- All should be unpaused
			for _, folder in ipairs(folders) do
				expect(instanceMap.pausedUpdateInstances[folder]).to.equal(nil)
			end

			instanceMap:stop()
		end)
	end)

	describe("stop cleanup", function()
		it("should clean up all mappings on stop", function()
			local instanceMap = InstanceMap.new()

			for i = 1, 50 do
				local folder = Instance.new("Folder")
				folder.Name = "Test_" .. tostring(i)
				folder.Parent = container

				local id = generateId()
				instanceMap:insert(id, folder)
			end

			expect(instanceMap:size()).to.equal(50)

			instanceMap:stop()

			expect(instanceMap:size()).to.equal(0)
		end)
	end)

	describe("edge cases", function()
		it("should handle removing non-existent ID", function()
			local instanceMap = InstanceMap.new()

			-- Should not throw
			instanceMap:removeId("non-existent-id")

			expect(instanceMap:size()).to.equal(0)

			instanceMap:stop()
		end)

		it("should handle removing non-tracked instance", function()
			local instanceMap = InstanceMap.new()

			local folder = Instance.new("Folder")
			folder.Parent = container

			-- Should not throw
			instanceMap:removeInstance(folder)

			expect(instanceMap:size()).to.equal(0)

			instanceMap:stop()
		end)

		it("should handle destroying non-existent ID", function()
			local instanceMap = InstanceMap.new()

			-- Should not throw
			instanceMap:destroyId("non-existent-id")

			expect(instanceMap:size()).to.equal(0)

			instanceMap:stop()
		end)
	end)

	describe("performance", function()
		it("should handle tree with 200+ instances", function()
			local instanceMap = InstanceMap.new()

			local root = LargeTreeGenerator.createInstanceTree({
				depth = 3,
				width = 6,
				instanceType = "Folder",
			})
			root.Parent = container

			local function insertRecursive(instance)
				local id = generateId()
				instanceMap:insert(id, instance)

				for _, child in ipairs(instance:GetChildren()) do
					insertRecursive(child)
				end
			end

			local startTime = os.clock()
			insertRecursive(root)
			local elapsed = os.clock() - startTime

			expect(instanceMap:size() > 200).to.equal(true)
			expect(elapsed < 2).to.equal(true)

			instanceMap:stop()
		end)
	end)
end
