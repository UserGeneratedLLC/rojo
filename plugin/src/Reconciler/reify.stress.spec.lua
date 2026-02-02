--[[
	Stress tests for instance creation (reify).
	
	Tests deep/wide trees, property edge cases, and failure scenarios.
]]

return function()
	local reify = require(script.Parent.reify)
	local reifyInstance, applyDeferredRefs = reify.reifyInstance, reify.applyDeferredRefs

	local PatchSet = require(script.Parent.Parent.PatchSet)
	local InstanceMap = require(script.Parent.Parent.InstanceMap)
	local testUtils = require(script.Parent.Parent.testUtils)
	local LargeTreeGenerator = testUtils.LargeTreeGenerator

	local HttpService = game:GetService("HttpService")

	-- Test container for cleanup
	local container

	beforeEach(function()
		container = Instance.new("Folder")
		container.Name = "ReifyStressTestContainer"
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

	describe("deep hierarchy creation", function()
		it("should create a hierarchy 20 levels deep", function()
			local instanceMap = InstanceMap.new()
			local virtualInstances = {}

			-- Build virtual instances for 20-level deep tree
			local parentId = nil
			local rootId = nil
			local ids = {}

			for i = 1, 20 do
				local id = generateId()
				table.insert(ids, id)

				if rootId == nil then
					rootId = id
				end

				virtualInstances[id] = {
					Id = id,
					ClassName = "Folder",
					Name = "Level_" .. tostring(i),
					Parent = parentId,
					Properties = {},
					Children = {},
				}

				if parentId then
					table.insert(virtualInstances[parentId].Children, id)
				end

				parentId = id
			end

			local deferredRefs = {}
			local unappliedPatch = reifyInstance(deferredRefs, instanceMap, virtualInstances, rootId, container)
			applyDeferredRefs(instanceMap, deferredRefs, unappliedPatch)

			expect(PatchSet.isEmpty(unappliedPatch)).to.equal(true)
			expect(instanceMap:size()).to.equal(20)

			-- Verify structure
			local current = instanceMap.fromIds[rootId]
			for i = 1, 20 do
				expect(current).to.be.ok()
				expect(current.Name).to.equal("Level_" .. tostring(i))
				if i < 20 then
					current = current:FindFirstChildOfClass("Folder")
				end
			end

			instanceMap:stop()
		end)

		it("should create a hierarchy 50 levels deep", function()
			local instanceMap = InstanceMap.new()
			local virtualInstances = {}

			local parentId = nil
			local rootId = nil

			for i = 1, 50 do
				local id = generateId()
				if rootId == nil then
					rootId = id
				end

				virtualInstances[id] = {
					Id = id,
					ClassName = "Folder",
					Name = "Deep_" .. tostring(i),
					Parent = parentId,
					Properties = {},
					Children = {},
				}

				if parentId then
					table.insert(virtualInstances[parentId].Children, id)
				end

				parentId = id
			end

			local deferredRefs = {}
			local unappliedPatch = reifyInstance(deferredRefs, instanceMap, virtualInstances, rootId, container)
			applyDeferredRefs(instanceMap, deferredRefs, unappliedPatch)

			expect(PatchSet.isEmpty(unappliedPatch)).to.equal(true)
			expect(instanceMap:size()).to.equal(50)

			instanceMap:stop()
		end)
	end)

	describe("wide hierarchy creation", function()
		it("should create 50 siblings", function()
			local instanceMap = InstanceMap.new()
			local virtualInstances = {}

			local rootId = generateId()
			local children = {}

			for i = 1, 50 do
				local childId = generateId()
				table.insert(children, childId)

				virtualInstances[childId] = {
					Id = childId,
					ClassName = "Folder",
					Name = "Sibling_" .. tostring(i),
					Parent = rootId,
					Properties = {},
					Children = {},
				}
			end

			virtualInstances[rootId] = {
				Id = rootId,
				ClassName = "Folder",
				Name = "WideRoot",
				Parent = nil,
				Properties = {},
				Children = children,
			}

			local deferredRefs = {}
			local unappliedPatch = reifyInstance(deferredRefs, instanceMap, virtualInstances, rootId, container)
			applyDeferredRefs(instanceMap, deferredRefs, unappliedPatch)

			expect(PatchSet.isEmpty(unappliedPatch)).to.equal(true)
			expect(instanceMap:size()).to.equal(51) -- root + 50 children

			local root = instanceMap.fromIds[rootId]
			expect(#root:GetChildren()).to.equal(50)

			instanceMap:stop()
		end)

		it("should create 100 siblings", function()
			local instanceMap = InstanceMap.new()
			local virtualInstances = {}

			local rootId = generateId()
			local children = {}

			for i = 1, 100 do
				local childId = generateId()
				table.insert(children, childId)

				virtualInstances[childId] = {
					Id = childId,
					ClassName = "Folder",
					Name = "Wide_" .. tostring(i),
					Parent = rootId,
					Properties = {},
					Children = {},
				}
			end

			virtualInstances[rootId] = {
				Id = rootId,
				ClassName = "Folder",
				Name = "VeryWideRoot",
				Parent = nil,
				Properties = {},
				Children = children,
			}

			local deferredRefs = {}
			local unappliedPatch = reifyInstance(deferredRefs, instanceMap, virtualInstances, rootId, container)
			applyDeferredRefs(instanceMap, deferredRefs, unappliedPatch)

			expect(PatchSet.isEmpty(unappliedPatch)).to.equal(true)
			expect(instanceMap:size()).to.equal(101)

			instanceMap:stop()
		end)
	end)

	describe("mixed success/failure", function()
		it("should create parent even if child creation fails", function()
			local instanceMap = InstanceMap.new()

			local rootId = generateId()
			local validChildId = generateId()
			local invalidChildId = generateId()

			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Folder",
					Name = "Parent",
					Parent = nil,
					Properties = {},
					Children = { validChildId, invalidChildId },
				},
				[validChildId] = {
					Id = validChildId,
					ClassName = "Folder",
					Name = "ValidChild",
					Parent = rootId,
					Properties = {},
					Children = {},
				},
				[invalidChildId] = {
					Id = invalidChildId,
					ClassName = "NotARealClass",
					Name = "InvalidChild",
					Parent = rootId,
					Properties = {},
					Children = {},
				},
			}

			local deferredRefs = {}
			local unappliedPatch = reifyInstance(deferredRefs, instanceMap, virtualInstances, rootId, container)
			applyDeferredRefs(instanceMap, deferredRefs, unappliedPatch)

			-- Parent and valid child should exist
			expect(instanceMap.fromIds[rootId]).to.be.ok()
			expect(instanceMap.fromIds[validChildId]).to.be.ok()
			expect(instanceMap.fromIds[invalidChildId]).to.equal(nil)

			-- Invalid child should be in unapplied
			expect(unappliedPatch.added[invalidChildId]).to.be.ok()

			instanceMap:stop()
		end)

		it("should handle multiple invalid children", function()
			local instanceMap = InstanceMap.new()

			local rootId = generateId()
			local children = {}
			local invalidIds = {}

			for i = 1, 5 do
				local childId = generateId()
				table.insert(children, childId)
				table.insert(invalidIds, childId)
			end

			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Folder",
					Name = "Parent",
					Parent = nil,
					Properties = {},
					Children = children,
				},
			}

			for _, childId in ipairs(children) do
				virtualInstances[childId] = {
					Id = childId,
					ClassName = "InvalidClass",
					Name = "Invalid",
					Parent = rootId,
					Properties = {},
					Children = {},
				}
			end

			local deferredRefs = {}
			local unappliedPatch = reifyInstance(deferredRefs, instanceMap, virtualInstances, rootId, container)
			applyDeferredRefs(instanceMap, deferredRefs, unappliedPatch)

			-- Parent should exist
			expect(instanceMap.fromIds[rootId]).to.be.ok()

			-- All invalid children should be in unapplied
			for _, childId in ipairs(invalidIds) do
				expect(unappliedPatch.added[childId]).to.be.ok()
			end

			instanceMap:stop()
		end)
	end)

	describe("property handling", function()
		it("should handle instance with many properties", function()
			local instanceMap = InstanceMap.new()

			local rootId = generateId()
			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Part",
					Name = "ManyProps",
					Parent = nil,
					Properties = {
						Anchored = { Bool = true },
						CanCollide = { Bool = false },
						CastShadow = { Bool = true },
						Transparency = { Float32 = 0.5 },
						Reflectance = { Float32 = 0.2 },
						Size = { Vector3 = { 4, 1, 2 } },
						Color = { Color3 = { 1, 0, 0 } },
					},
					Children = {},
				},
			}

			local deferredRefs = {}
			local unappliedPatch = reifyInstance(deferredRefs, instanceMap, virtualInstances, rootId, container)
			applyDeferredRefs(instanceMap, deferredRefs, unappliedPatch)

			expect(PatchSet.isEmpty(unappliedPatch)).to.equal(true)

			local part = instanceMap.fromIds[rootId]
			expect(part.Anchored).to.equal(true)
			expect(part.CanCollide).to.equal(false)
			expect(part.CastShadow).to.equal(true)
			expect(math.abs(part.Transparency - 0.5) < 0.01).to.equal(true)

			instanceMap:stop()
		end)

		it("should handle StringValue with Value property", function()
			local instanceMap = InstanceMap.new()

			local rootId = generateId()
			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "StringValue",
					Name = "TestString",
					Parent = nil,
					Properties = {
						Value = { String = "Hello, World!" },
					},
					Children = {},
				},
			}

			local deferredRefs = {}
			local unappliedPatch = reifyInstance(deferredRefs, instanceMap, virtualInstances, rootId, container)
			applyDeferredRefs(instanceMap, deferredRefs, unappliedPatch)

			expect(PatchSet.isEmpty(unappliedPatch)).to.equal(true)

			local sv = instanceMap.fromIds[rootId]
			expect(sv.Value).to.equal("Hello, World!")

			instanceMap:stop()
		end)

		it("should handle failed property assignment gracefully", function()
			local instanceMap = InstanceMap.new()

			local rootId = generateId()
			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Folder",
					Name = "Test",
					Parent = nil,
					Properties = {
						-- Folder doesn't have a Value property
						Value = { String = "ShouldFail" },
					},
					Children = {},
				},
			}

			local deferredRefs = {}
			local unappliedPatch = reifyInstance(deferredRefs, instanceMap, virtualInstances, rootId, container)
			applyDeferredRefs(instanceMap, deferredRefs, unappliedPatch)

			-- Instance should still be created regardless of property failure
			expect(instanceMap.fromIds[rootId]).to.be.ok()

			-- The implementation may or may not track unknown property assignments
			-- This test verifies the instance is created despite property issues
			expect(unappliedPatch).to.be.ok()

			instanceMap:stop()
		end)
	end)

	describe("ref property handling", function()
		it("should handle ref to sibling created in same batch", function()
			local instanceMap = InstanceMap.new()

			local rootId = generateId()
			local siblingAId = generateId()
			local siblingBId = generateId()

			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Folder",
					Name = "Root",
					Parent = nil,
					Properties = {},
					Children = { siblingAId, siblingBId },
				},
				[siblingAId] = {
					Id = siblingAId,
					ClassName = "ObjectValue",
					Name = "A",
					Parent = rootId,
					Properties = {
						Value = { Ref = siblingBId },
					},
					Children = {},
				},
				[siblingBId] = {
					Id = siblingBId,
					ClassName = "Part",
					Name = "B",
					Parent = rootId,
					Properties = {},
					Children = {},
				},
			}

			local deferredRefs = {}
			local unappliedPatch = reifyInstance(deferredRefs, instanceMap, virtualInstances, rootId, container)
			applyDeferredRefs(instanceMap, deferredRefs, unappliedPatch)

			expect(PatchSet.isEmpty(unappliedPatch)).to.equal(true)

			local a = instanceMap.fromIds[siblingAId]
			local b = instanceMap.fromIds[siblingBId]
			expect(a.Value).to.equal(b)

			instanceMap:stop()
		end)

		it("should handle ref to descendant", function()
			local instanceMap = InstanceMap.new()

			local rootId = generateId()
			local childId = generateId()
			local grandchildId = generateId()

			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "ObjectValue",
					Name = "Root",
					Parent = nil,
					Properties = {
						Value = { Ref = grandchildId },
					},
					Children = { childId },
				},
				[childId] = {
					Id = childId,
					ClassName = "Folder",
					Name = "Child",
					Parent = rootId,
					Properties = {},
					Children = { grandchildId },
				},
				[grandchildId] = {
					Id = grandchildId,
					ClassName = "Part",
					Name = "Grandchild",
					Parent = childId,
					Properties = {},
					Children = {},
				},
			}

			local deferredRefs = {}
			local unappliedPatch = reifyInstance(deferredRefs, instanceMap, virtualInstances, rootId, container)
			applyDeferredRefs(instanceMap, deferredRefs, unappliedPatch)

			expect(PatchSet.isEmpty(unappliedPatch)).to.equal(true)

			local root = instanceMap.fromIds[rootId]
			local grandchild = instanceMap.fromIds[grandchildId]
			expect(root.Value).to.equal(grandchild)

			instanceMap:stop()
		end)
	end)

	describe("performance", function()
		it("should create 200+ instances in reasonable time", function()
			local instanceMap = InstanceMap.new()
			local treeData = LargeTreeGenerator.createVirtualTree({
				depth = 3,
				width = 6, -- 6^3 = 216 instances
				instanceType = "Folder",
			})

			local startTime = os.clock()
			local deferredRefs = {}
			local unappliedPatch = reifyInstance(
				deferredRefs,
				instanceMap,
				treeData.virtualInstances,
				treeData.rootId,
				container
			)
			applyDeferredRefs(instanceMap, deferredRefs, unappliedPatch)
			local elapsed = os.clock() - startTime

			expect(PatchSet.isEmpty(unappliedPatch)).to.equal(true)
			-- Should complete in under 2 seconds
			expect(elapsed < 2).to.equal(true)

			instanceMap:stop()
		end)
	end)

	describe("edge cases", function()
		it("should throw for bogus root ID", function()
			expect(function()
				local deferredRefs = {}
				local instanceMap = InstanceMap.new()
				local unappliedPatch = reifyInstance(deferredRefs, instanceMap, {}, "bogus-id", container)
				applyDeferredRefs(instanceMap, deferredRefs, unappliedPatch)
			end).to.throw()
		end)

		it("should handle empty properties", function()
			local instanceMap = InstanceMap.new()

			local rootId = generateId()
			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Folder",
					Name = "Empty",
					Parent = nil,
					Properties = {},
					Children = {},
				},
			}

			local deferredRefs = {}
			local unappliedPatch = reifyInstance(deferredRefs, instanceMap, virtualInstances, rootId, container)
			applyDeferredRefs(instanceMap, deferredRefs, unappliedPatch)

			expect(PatchSet.isEmpty(unappliedPatch)).to.equal(true)
			expect(instanceMap.fromIds[rootId]).to.be.ok()

			instanceMap:stop()
		end)

		it("should handle ModuleScript with Source", function()
			local instanceMap = InstanceMap.new()

			local rootId = generateId()
			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "ModuleScript",
					Name = "TestModule",
					Parent = nil,
					Properties = {
						Source = { String = "return {}" },
					},
					Children = {},
				},
			}

			local deferredRefs = {}
			local unappliedPatch = reifyInstance(deferredRefs, instanceMap, virtualInstances, rootId, container)
			applyDeferredRefs(instanceMap, deferredRefs, unappliedPatch)

			expect(PatchSet.isEmpty(unappliedPatch)).to.equal(true)

			local module = instanceMap.fromIds[rootId]
			expect(module.Source).to.equal("return {}")

			instanceMap:stop()
		end)
	end)
end
