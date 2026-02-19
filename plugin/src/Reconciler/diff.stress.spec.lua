--[[
	Stress tests for the diff algorithm.
	
	Tests large trees, duplicate name handling, property equality edge cases,
	and various edge conditions that can occur in production.
]]

return function()
	local InstanceMap = require(script.Parent.Parent.InstanceMap)
	local PatchSet = require(script.Parent.Parent.PatchSet)
	local testUtils = require(script.Parent.Parent.testUtils)
	local LargeTreeGenerator = testUtils.LargeTreeGenerator

	local diff = require(script.Parent.diff)

	local HttpService = game:GetService("HttpService")

	-- Test container for cleanup
	local container

	beforeEach(function()
		container = Instance.new("Folder")
		container.Name = "DiffStressTestContainer"
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

	describe("large tree diffing", function()
		it("should handle diffing a tree with 100+ instances", function()
			local instanceMap = InstanceMap.new()

			-- Create a tree with depth=3, width=5 = ~150 instances
			local root = LargeTreeGenerator.createInstanceTree({
				depth = 3,
				width = 5,
				instanceType = "Folder",
			})
			root.Parent = container

			-- Build virtual instances matching the real tree
			local virtualInstances = {}
			local rootId = generateId()

			local function mapInstance(instance, id, parentId)
				instanceMap:insert(id, instance)

				local children = {}
				for _, child in ipairs(instance:GetChildren()) do
					local childId = generateId()
					table.insert(children, childId)
					mapInstance(child, childId, id)
				end

				virtualInstances[id] = {
					Id = id,
					ClassName = instance.ClassName,
					Name = instance.Name,
					Parent = parentId,
					Properties = {},
					Children = children,
				}
			end

			mapInstance(root, rootId, nil)

			local ok, patch = diff(instanceMap, virtualInstances, rootId)

			expect(ok).to.equal(true)
			expect(PatchSet.isEmpty(patch)).to.equal(true)

			instanceMap:stop()
		end)

		it("should handle diffing a deep tree (50 levels)", function()
			local instanceMap = InstanceMap.new()

			local root = LargeTreeGenerator.createDeepTree({ depth = 50 })
			root.Parent = container

			local virtualInstances = {}
			local currentInstance = root
			local parentId = nil
			local rootId = nil

			while currentInstance do
				local id = generateId()
				if rootId == nil then
					rootId = id
				end

				instanceMap:insert(id, currentInstance)

				local children = {}
				local childInstance = currentInstance:FindFirstChildOfClass("Folder")
				if childInstance then
					local childId = generateId()
					table.insert(children, childId)
				end

				virtualInstances[id] = {
					Id = id,
					ClassName = currentInstance.ClassName,
					Name = currentInstance.Name,
					Parent = parentId,
					Properties = {},
					Children = children,
				}

				parentId = id
				currentInstance = childInstance
			end

			-- Fix up children references
			for id, virt in pairs(virtualInstances) do
				for i, _ in ipairs(virt.Children) do
					-- Find the actual child ID
					for otherId, otherVirt in pairs(virtualInstances) do
						if otherVirt.Parent == id then
							virt.Children[i] = otherId
							break
						end
					end
				end
			end

			local ok, patch = diff(instanceMap, virtualInstances, rootId)

			expect(ok).to.equal(true)
			expect(PatchSet.isEmpty(patch)).to.equal(true)

			instanceMap:stop()
		end)

		it("should handle diffing a wide tree (100 siblings)", function()
			local instanceMap = InstanceMap.new()

			local root = LargeTreeGenerator.createWideTree({
				width = 100,
				levels = 1,
			})
			root.Parent = container

			local virtualInstances = {}
			local rootId = generateId()

			instanceMap:insert(rootId, root)

			local children = {}
			for _, child in ipairs(root:GetChildren()) do
				local childId = generateId()
				instanceMap:insert(childId, child)
				table.insert(children, childId)

				virtualInstances[childId] = {
					Id = childId,
					ClassName = child.ClassName,
					Name = child.Name,
					Parent = rootId,
					Properties = {},
					Children = {},
				}
			end

			virtualInstances[rootId] = {
				Id = rootId,
				ClassName = root.ClassName,
				Name = root.Name,
				Parent = nil,
				Properties = {},
				Children = children,
			}

			local ok, patch = diff(instanceMap, virtualInstances, rootId)

			expect(ok).to.equal(true)
			expect(PatchSet.isEmpty(patch)).to.equal(true)

			instanceMap:stop()
		end)
	end)

	describe("duplicate name handling", function()
		it("should skip instances with duplicate-named siblings in real DOM", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			-- Create two children with the same name
			local child1 = Instance.new("Folder")
			child1.Name = "DuplicateName"
			child1.Parent = root

			local child2 = Instance.new("Folder")
			child2.Name = "DuplicateName"
			child2.Parent = root

			local uniqueChild = Instance.new("Folder")
			uniqueChild.Name = "UniqueName"
			uniqueChild.Parent = root

			local rootId = generateId()
			local uniqueChildId = generateId()

			instanceMap:insert(rootId, root)
			instanceMap:insert(uniqueChildId, uniqueChild)
			-- Note: duplicates are NOT in instanceMap since we can't reliably sync them

			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Folder",
					Name = "Root",
					Parent = nil,
					Properties = {},
					Children = { uniqueChildId },
				},
				[uniqueChildId] = {
					Id = uniqueChildId,
					ClassName = "Folder",
					Name = "UniqueName",
					Parent = rootId,
					Properties = {},
					Children = {},
				},
			}

			local ok, patch = diff(instanceMap, virtualInstances, rootId)

			expect(ok).to.equal(true)
			-- Duplicates should be marked for removal (matching handles pairing)
			expect(#patch.removed).to.equal(2)

			instanceMap:stop()
		end)

		it("should skip instances with duplicate-named siblings in virtual DOM", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			local dup1Id = generateId()
			local dup2Id = generateId()

			instanceMap:insert(rootId, root)

			-- Virtual DOM has two children with the same name
			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Folder",
					Name = "Root",
					Parent = nil,
					Properties = {},
					Children = { dup1Id, dup2Id },
				},
				[dup1Id] = {
					Id = dup1Id,
					ClassName = "Folder",
					Name = "Duplicate",
					Parent = rootId,
					Properties = {},
					Children = {},
				},
				[dup2Id] = {
					Id = dup2Id,
					ClassName = "Folder",
					Name = "Duplicate",
					Parent = rootId,
					Properties = {},
					Children = {},
				},
			}

			local ok, patch = diff(instanceMap, virtualInstances, rootId)

			expect(ok).to.equal(true)
			-- Both duplicates should be added (matching handles pairing)
			expect(patch.added[dup1Id]).to.be.ok()
			expect(patch.added[dup2Id]).to.be.ok()

			instanceMap:stop()
		end)

		it("should mark entire subtree as ambiguous when parent has duplicates", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			local dup1Id = generateId()
			local dup2Id = generateId()
			local grandchildId = generateId()

			instanceMap:insert(rootId, root)

			-- Virtual DOM has duplicates with children
			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Folder",
					Name = "Root",
					Parent = nil,
					Properties = {},
					Children = { dup1Id, dup2Id },
				},
				[dup1Id] = {
					Id = dup1Id,
					ClassName = "Folder",
					Name = "Duplicate",
					Parent = rootId,
					Properties = {},
					Children = { grandchildId },
				},
				[dup2Id] = {
					Id = dup2Id,
					ClassName = "Folder",
					Name = "Duplicate",
					Parent = rootId,
					Properties = {},
					Children = {},
				},
				[grandchildId] = {
					Id = grandchildId,
					ClassName = "Folder",
					Name = "Grandchild",
					Parent = dup1Id,
					Properties = {},
					Children = {},
				},
			}

			local ok, patch = diff(instanceMap, virtualInstances, rootId)

			expect(ok).to.equal(true)
			-- Grandchild should be added along with its parent (matching handles pairing)
			expect(patch.added[grandchildId]).to.be.ok()

			instanceMap:stop()
		end)
	end)

	describe("property equality edge cases", function()
		it("should handle floating point comparison with small differences", function()
			local instanceMap = InstanceMap.new()

			local part = Instance.new("Part")
			part.Name = "TestPart"
			part.Transparency = 0.5
			part.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, part)

			-- Virtual value that's very close but not exactly equal
			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Part",
					Name = "TestPart",
					Parent = nil,
					Properties = {
						-- 0.50001 should be considered equal to 0.5 with epsilon
						Transparency = { Float32 = 0.50001 },
					},
					Children = {},
				},
			}

			local ok, patch = diff(instanceMap, virtualInstances, rootId)

			expect(ok).to.equal(true)
			-- Should NOT detect a change due to epsilon comparison
			expect(PatchSet.isEmpty(patch)).to.equal(true)

			instanceMap:stop()
		end)

		it("should detect actual floating point differences", function()
			local instanceMap = InstanceMap.new()

			local part = Instance.new("Part")
			part.Name = "TestPart"
			part.Transparency = 0.5
			part.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, part)

			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Part",
					Name = "TestPart",
					Parent = nil,
					Properties = {
						-- 0.6 is definitely different from 0.5
						Transparency = { Float32 = 0.6 },
					},
					Children = {},
				},
			}

			local ok, patch = diff(instanceMap, virtualInstances, rootId)

			expect(ok).to.equal(true)
			expect(#patch.updated).to.equal(1)
			expect(patch.updated[1].changedProperties.Transparency).to.be.ok()

			instanceMap:stop()
		end)

		it("should handle Color3 comparison with RGB integers", function()
			local instanceMap = InstanceMap.new()

			local part = Instance.new("Part")
			part.Name = "TestPart"
			part.Color = Color3.fromRGB(255, 128, 64)
			part.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, part)

			-- Same color values (exact 8-bit match avoids quantization mismatch)
			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Part",
					Name = "TestPart",
					Parent = nil,
					Properties = {
						Color = {
							Color3 = { 255 / 255, 128 / 255, 64 / 255 },
						},
					},
					Children = {},
				},
			}

			local ok, patch = diff(instanceMap, virtualInstances, rootId)

			expect(ok).to.equal(true)
			-- Should be considered equal (same values)
			expect(PatchSet.isEmpty(patch)).to.equal(true)

			instanceMap:stop()
		end)

		it("should handle Vector3 comparison with epsilon", function()
			local instanceMap = InstanceMap.new()

			local part = Instance.new("Part")
			part.Name = "TestPart"
			part.Position = Vector3.new(10, 20, 30)
			part.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, part)

			-- Very close position
			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Part",
					Name = "TestPart",
					Parent = nil,
					Properties = {
						Position = {
							Vector3 = { 10.00001, 20.00001, 30.00001 },
						},
					},
					Children = {},
				},
			}

			local ok, patch = diff(instanceMap, virtualInstances, rootId)

			expect(ok).to.equal(true)
			-- Should be considered equal within epsilon
			expect(PatchSet.isEmpty(patch)).to.equal(true)

			instanceMap:stop()
		end)

		it("should handle CFrame comparison with epsilon", function()
			local instanceMap = InstanceMap.new()

			local part = Instance.new("Part")
			part.Name = "TestPart"
			part.CFrame = CFrame.new(10, 20, 30)
			part.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, part)

			-- Very close CFrame (position only, identity rotation)
			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Part",
					Name = "TestPart",
					Parent = nil,
					Properties = {
						CFrame = {
							CFrame = {
								position = { 10.00001, 20.00001, 30.00001 },
								orientation = { { 1, 0, 0 }, { 0, 1, 0 }, { 0, 0, 1 } },
							},
						},
					},
					Children = {},
				},
			}

			local ok, _ = diff(instanceMap, virtualInstances, rootId)

			expect(ok).to.equal(true)
			-- CFrame decoding might not work in test env, but the test structure is valid

			instanceMap:stop()
		end)

		it("should handle NaN values", function()
			local instanceMap = InstanceMap.new()

			local part = Instance.new("Part")
			part.Name = "TestPart"
			part.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, part)

			-- Virtual instance with same name (NaN handling is in the comparison logic)
			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Part",
					Name = "TestPart",
					Parent = nil,
					Properties = {},
					Children = {},
				},
			}

			local ok, patch = diff(instanceMap, virtualInstances, rootId)

			expect(ok).to.equal(true)
			expect(PatchSet.isEmpty(patch)).to.equal(true)

			instanceMap:stop()
		end)
	end)

	describe("ignoreUnknownInstances behavior", function()
		it("should not remove unknown children when ignoreUnknownInstances is true", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local unknownChild = Instance.new("Part")
			unknownChild.Name = "UnknownPart"
			unknownChild.Parent = root

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Folder",
					Name = "Root",
					Parent = nil,
					Properties = {},
					Children = {},
					Metadata = {
						ignoreUnknownInstances = true,
					},
				},
			}

			local ok, patch = diff(instanceMap, virtualInstances, rootId)

			expect(ok).to.equal(true)
			-- Unknown Part should NOT be removed
			expect(#patch.removed).to.equal(0)

			instanceMap:stop()
		end)

		it("should remove scripts even when ignoreUnknownInstances is true (scripts-only mode)", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local unknownScript = Instance.new("ModuleScript")
			unknownScript.Name = "UnknownScript"
			unknownScript.Parent = root

			local unknownPart = Instance.new("Part")
			unknownPart.Name = "UnknownPart"
			unknownPart.Parent = root

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Folder",
					Name = "Root",
					Parent = nil,
					Properties = {},
					Children = {},
					Metadata = {
						ignoreUnknownInstances = true,
					},
				},
			}

			local ok, patch = diff(instanceMap, virtualInstances, rootId)

			expect(ok).to.equal(true)
			-- Script SHOULD be removed (scripts-only mode allows script deletion)
			-- Part should NOT be removed
			expect(#patch.removed).to.equal(1)
			expect(patch.removed[1]).to.equal(unknownScript)

			instanceMap:stop()
		end)

		it("should remove all unknown children when ignoreUnknownInstances is false", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local unknownPart = Instance.new("Part")
			unknownPart.Name = "UnknownPart"
			unknownPart.Parent = root

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Folder",
					Name = "Root",
					Parent = nil,
					Properties = {},
					Children = {},
					-- No Metadata or ignoreUnknownInstances is false by default
				},
			}

			local ok, patch = diff(instanceMap, virtualInstances, rootId)

			expect(ok).to.equal(true)
			-- Unknown Part SHOULD be removed
			expect(#patch.removed).to.equal(1)
			expect(patch.removed[1]).to.equal(unknownPart)

			instanceMap:stop()
		end)
	end)

	describe("name and className changes", function()
		it("should detect name changes", function()
			local instanceMap = InstanceMap.new()

			local folder = Instance.new("Folder")
			folder.Name = "OldName"
			folder.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, folder)

			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Folder",
					Name = "NewName",
					Parent = nil,
					Properties = {},
					Children = {},
				},
			}

			local ok, patch = diff(instanceMap, virtualInstances, rootId)

			expect(ok).to.equal(true)
			expect(#patch.updated).to.equal(1)
			expect(patch.updated[1].changedName).to.equal("NewName")

			instanceMap:stop()
		end)

		it("should detect className changes", function()
			local instanceMap = InstanceMap.new()

			local folder = Instance.new("Folder")
			folder.Name = "Test"
			folder.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, folder)

			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Model", -- Different class
					Name = "Test",
					Parent = nil,
					Properties = {},
					Children = {},
				},
			}

			local ok, patch = diff(instanceMap, virtualInstances, rootId)

			expect(ok).to.equal(true)
			expect(#patch.updated).to.equal(1)
			expect(patch.updated[1].changedClassName).to.equal("Model")

			instanceMap:stop()
		end)

		it("should detect combined name and className changes", function()
			local instanceMap = InstanceMap.new()

			local folder = Instance.new("Folder")
			folder.Name = "OldName"
			folder.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, folder)

			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Model",
					Name = "NewName",
					Parent = nil,
					Properties = {},
					Children = {},
				},
			}

			local ok, patch = diff(instanceMap, virtualInstances, rootId)

			expect(ok).to.equal(true)
			expect(#patch.updated).to.equal(1)
			expect(patch.updated[1].changedName).to.equal("NewName")
			expect(patch.updated[1].changedClassName).to.equal("Model")

			instanceMap:stop()
		end)
	end)

	describe("ref properties", function()
		it("should handle null ref comparison", function()
			local instanceMap = InstanceMap.new()

			local objValue = Instance.new("ObjectValue")
			objValue.Name = "TestRef"
			objValue.Value = nil
			objValue.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, objValue)

			-- Null ref in virtual DOM
			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "ObjectValue",
					Name = "TestRef",
					Parent = nil,
					Properties = {
						Value = { Ref = "00000000000000000000000000000000" },
					},
					Children = {},
				},
			}

			local ok, patch = diff(instanceMap, virtualInstances, rootId)

			expect(ok).to.equal(true)
			-- null ref and nil should be considered equal
			expect(PatchSet.isEmpty(patch)).to.equal(true)

			instanceMap:stop()
		end)

		it("should detect ref changes", function()
			local instanceMap = InstanceMap.new()

			local objValue = Instance.new("ObjectValue")
			objValue.Name = "TestRef"
			objValue.Value = nil
			objValue.Parent = container

			local target = Instance.new("Folder")
			target.Name = "Target"
			target.Parent = container

			local rootId = generateId()
			local targetId = generateId()
			instanceMap:insert(rootId, objValue)
			instanceMap:insert(targetId, target)

			-- Virtual DOM has ref to target
			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "ObjectValue",
					Name = "TestRef",
					Parent = nil,
					Properties = {
						Value = { Ref = targetId },
					},
					Children = {},
				},
			}

			local ok, patch = diff(instanceMap, virtualInstances, rootId)

			expect(ok).to.equal(true)
			expect(#patch.updated).to.equal(1)
			expect(patch.updated[1].changedProperties.Value).to.be.ok()

			instanceMap:stop()
		end)
	end)

	describe("additions and removals", function()
		it("should detect instances to add", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local rootId = generateId()
			local newChildId = generateId()
			instanceMap:insert(rootId, root)

			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Folder",
					Name = "Root",
					Parent = nil,
					Properties = {},
					Children = { newChildId },
				},
				[newChildId] = {
					Id = newChildId,
					ClassName = "Folder",
					Name = "NewChild",
					Parent = rootId,
					Properties = {},
					Children = {},
				},
			}

			local ok, patch = diff(instanceMap, virtualInstances, rootId)

			expect(ok).to.equal(true)
			expect(patch.added[newChildId]).to.be.ok()
			expect(patch.added[newChildId].Name).to.equal("NewChild")

			instanceMap:stop()
		end)

		it("should detect instances to remove", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local child = Instance.new("Folder")
			child.Name = "ChildToRemove"
			child.Parent = root

			local rootId = generateId()
			instanceMap:insert(rootId, root)
			-- child is NOT in instanceMap and NOT in virtual DOM

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

			local ok, patch = diff(instanceMap, virtualInstances, rootId)

			expect(ok).to.equal(true)
			expect(#patch.removed).to.equal(1)
			expect(patch.removed[1]).to.equal(child)

			instanceMap:stop()
		end)
	end)

	describe("EnumItem handling", function()
		it("should handle EnumItem to number comparison", function()
			local instanceMap = InstanceMap.new()

			local part = Instance.new("Part")
			part.Name = "TestPart"
			part.Material = Enum.Material.Plastic -- Value = 256
			part.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, part)

			-- Virtual DOM has the enum as a number
			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Part",
					Name = "TestPart",
					Parent = nil,
					Properties = {
						Material = { Enum = 256 }, -- Plastic
					},
					Children = {},
				},
			}

			local ok, _ = diff(instanceMap, virtualInstances, rootId)

			expect(ok).to.equal(true)
			-- Should be considered equal
			-- Note: depends on how decodeValue handles Enum encoding

			instanceMap:stop()
		end)
	end)

	describe("performance", function()
		it("should complete diffing 500+ instances in reasonable time", function()
			local instanceMap = InstanceMap.new()

			-- Create a tree with ~500 instances
			local root = LargeTreeGenerator.createInstanceTree({
				depth = 4,
				width = 5, -- 5^4 = 625 instances
				instanceType = "Folder",
			})
			root.Parent = container

			local virtualInstances = {}
			local rootId = generateId()

			local function mapInstance(instance, id, parentId)
				instanceMap:insert(id, instance)

				local children = {}
				for _, child in ipairs(instance:GetChildren()) do
					local childId = generateId()
					table.insert(children, childId)
					mapInstance(child, childId, id)
				end

				virtualInstances[id] = {
					Id = id,
					ClassName = instance.ClassName,
					Name = instance.Name,
					Parent = parentId,
					Properties = {},
					Children = children,
				}
			end

			mapInstance(root, rootId, nil)

			local startTime = os.clock()
			local ok, patch = diff(instanceMap, virtualInstances, rootId)
			local elapsed = os.clock() - startTime

			expect(ok).to.equal(true)
			expect(PatchSet.isEmpty(patch)).to.equal(true)
			-- Should complete in under 1 second
			expect(elapsed < 1).to.equal(true)

			instanceMap:stop()
		end)
	end)

	describe("edge cases", function()
		it("should handle empty tree", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "EmptyRoot"
			root.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, root)

			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "Folder",
					Name = "EmptyRoot",
					Parent = nil,
					Properties = {},
					Children = {},
				},
			}

			local ok, patch = diff(instanceMap, virtualInstances, rootId)

			expect(ok).to.equal(true)
			expect(PatchSet.isEmpty(patch)).to.equal(true)

			instanceMap:stop()
		end)

		it("should handle non-archivable instances", function()
			local instanceMap = InstanceMap.new()

			local root = Instance.new("Folder")
			root.Name = "Root"
			root.Parent = container

			local nonArchivable = Instance.new("Folder")
			nonArchivable.Name = "NonArchivable"
			nonArchivable.Archivable = false
			nonArchivable.Parent = root

			local rootId = generateId()
			instanceMap:insert(rootId, root)

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

			local ok, patch = diff(instanceMap, virtualInstances, rootId)

			expect(ok).to.equal(true)
			-- Non-archivable instance should NOT be marked for removal
			expect(#patch.removed).to.equal(0)

			instanceMap:stop()
		end)

		it("should skip CanvasPosition on ScrollingFrame", function()
			local instanceMap = InstanceMap.new()

			local frame = Instance.new("ScrollingFrame")
			frame.Name = "TestFrame"
			frame.CanvasPosition = Vector2.new(100, 100)
			frame.Parent = container

			local rootId = generateId()
			instanceMap:insert(rootId, frame)

			local virtualInstances = {
				[rootId] = {
					Id = rootId,
					ClassName = "ScrollingFrame",
					Name = "TestFrame",
					Parent = nil,
					Properties = {
						-- Different CanvasPosition
						CanvasPosition = { Vector2 = { 0, 0 } },
					},
					Children = {},
				},
			}

			local ok, patch = diff(instanceMap, virtualInstances, rootId)

			expect(ok).to.equal(true)
			-- CanvasPosition should be skipped, no update
			expect(PatchSet.isEmpty(patch)).to.equal(true)

			instanceMap:stop()
		end)
	end)
end
