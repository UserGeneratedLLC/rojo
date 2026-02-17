return function()
	local InstanceMap = require(script.Parent.Parent.InstanceMap)
	local PatchSet = require(script.Parent.Parent.PatchSet)

	local diff = require(script.Parent.diff)

	local function isEmpty(table)
		return next(table) == nil, "Table was not empty"
	end

	local function size(dict)
		local len = 0

		for _ in dict do
			len = len + 1
		end

		return len
	end

	it("should generate an empty patch for empty instances", function()
		local knownInstances = InstanceMap.new()
		local virtualInstances = {
			ROOT = {
				ClassName = "Folder",
				Name = "Some Name",
				Properties = {},
				Children = {},
			},
		}

		local rootInstance = Instance.new("Folder")
		rootInstance.Name = "Some Name"
		knownInstances:insert("ROOT", rootInstance)

		local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

		assert(ok, tostring(patch))

		assert(isEmpty(patch.removed))
		assert(isEmpty(patch.added))
		assert(isEmpty(patch.updated))
	end)

	it("should generate a patch with a changed name", function()
		local knownInstances = InstanceMap.new()
		local virtualInstances = {
			ROOT = {
				ClassName = "Folder",
				Name = "Some Name",
				Properties = {},
				Children = {},
			},
		}

		local rootInstance = Instance.new("Folder")
		knownInstances:insert("ROOT", rootInstance)

		local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

		assert(ok, tostring(patch))

		assert(isEmpty(patch.removed))
		assert(isEmpty(patch.added))
		expect(#patch.updated).to.equal(1)

		local update = patch.updated[1]
		expect(update.id).to.equal("ROOT")
		expect(update.changedName).to.equal("Some Name")
		assert(isEmpty(update.changedProperties))
	end)

	it("should generate a patch with a changed property", function()
		local knownInstances = InstanceMap.new()
		local virtualInstances = {
			ROOT = {
				ClassName = "StringValue",
				Name = "Value",
				Properties = {
					Value = {
						String = "Hello, world!",
					},
				},
				Children = {},
			},
		}

		local rootInstance = Instance.new("StringValue")
		rootInstance.Value = "Initial Value"
		knownInstances:insert("ROOT", rootInstance)

		local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

		assert(ok, tostring(patch))

		assert(isEmpty(patch.removed))
		assert(isEmpty(patch.added))
		expect(#patch.updated).to.equal(1)

		local update = patch.updated[1]
		expect(update.id).to.equal("ROOT")
		expect(update.changedName).to.equal(nil)
		expect(size(update.changedProperties)).to.equal(1)

		local patchProperty = update.changedProperties["Value"]
		expect(patchProperty).to.be.a("table")
		local ty, value = next(patchProperty)
		expect(ty).to.equal("String")
		expect(value).to.equal("Hello, world!")
	end)

	it("should generate an empty patch if no properties changed", function()
		local knownInstances = InstanceMap.new()
		local virtualInstances = {
			ROOT = {
				ClassName = "StringValue",
				Name = "Value",
				Properties = {
					Value = {
						String = "Hello, world!",
					},
				},
				Children = {},
			},
		}

		local rootInstance = Instance.new("StringValue")
		rootInstance.Value = "Hello, world!"
		knownInstances:insert("ROOT", rootInstance)

		local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

		assert(ok, tostring(patch))
		assert(PatchSet.isEmpty(patch), "expected empty patch")
	end)

	it("should ignore unknown properties", function()
		local knownInstances = InstanceMap.new()
		local virtualInstances = {
			ROOT = {
				ClassName = "Folder",
				Name = "Folder",
				Properties = {
					FAKE_PROPERTY = {
						String = "Hello, world!",
					},
				},
				Children = {},
			},
		}

		local rootInstance = Instance.new("Folder")
		knownInstances:insert("ROOT", rootInstance)

		local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

		assert(ok, tostring(patch))

		assert(isEmpty(patch.removed))
		assert(isEmpty(patch.added))
		assert(isEmpty(patch.updated))
	end)

	it("should ignore unreadable properties", function()
		local knownInstances = InstanceMap.new()
		local virtualInstances = {
			ROOT = {
				ClassName = "Fire",
				Name = "Fire",
				Properties = {
					-- heat_xml is a serialization-only property that is not
					-- exposed to Lua.
					heat_xml = {
						Float32 = 5,
					},
				},
				Children = {},
			},
		}

		local rootInstance = Instance.new("Fire")
		knownInstances:insert("ROOT", rootInstance)

		local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

		assert(ok, tostring(patch))

		assert(isEmpty(patch.removed))
		assert(isEmpty(patch.added))
		assert(isEmpty(patch.updated))
	end)

	it("should generate a patch removing unknown children by default", function()
		local knownInstances = InstanceMap.new()
		local virtualInstances = {
			ROOT = {
				ClassName = "Folder",
				Name = "Folder",
				Properties = {},
				Children = {},
			},
		}

		local rootInstance = Instance.new("Folder")
		local unknownChild = Instance.new("Folder")
		unknownChild.Parent = rootInstance
		knownInstances:insert("ROOT", rootInstance)

		local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

		assert(ok, tostring(patch))

		assert(isEmpty(patch.added))
		assert(isEmpty(patch.updated))
		expect(#patch.removed).to.equal(1)
		expect(patch.removed[1]).to.equal(unknownChild)
	end)

	it("should generate an empty patch if unknown children should be ignored", function()
		local knownInstances = InstanceMap.new()
		local virtualInstances = {
			ROOT = {
				ClassName = "Folder",
				Name = "Folder",
				Properties = {},
				Children = {},
				Metadata = {
					ignoreUnknownInstances = true,
				},
			},
		}

		local rootInstance = Instance.new("Folder")
		local unknownChild = Instance.new("Folder")
		unknownChild.Parent = rootInstance
		knownInstances:insert("ROOT", rootInstance)

		local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

		assert(ok, tostring(patch))

		assert(isEmpty(patch.added))
		assert(isEmpty(patch.updated))
		assert(isEmpty(patch.removed))
	end)

	it("should generate a patch with an added child", function()
		local knownInstances = InstanceMap.new()
		local virtualInstances = {
			ROOT = {
				ClassName = "Folder",
				Name = "Folder",
				Properties = {},
				Children = { "CHILD" },
			},

			CHILD = {
				ClassName = "Folder",
				Name = "Child",
				Properties = {},
				Children = {},
			},
		}

		local rootInstance = Instance.new("Folder")
		knownInstances:insert("ROOT", rootInstance)

		local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

		assert(ok, tostring(patch))

		assert(isEmpty(patch.updated))
		assert(isEmpty(patch.removed))
		expect(size(patch.added)).to.equal(1)
		expect(patch.added["CHILD"]).to.equal(virtualInstances["CHILD"])
	end)

	describe("duplicate-named siblings", function()
		it("should handle real DOM children with duplicate names normally", function()
			local knownInstances = InstanceMap.new()
			local virtualInstances = {
				ROOT = {
					ClassName = "Folder",
					Name = "Root",
					Properties = {},
					Children = {},
				},
			}

			local rootInstance = Instance.new("Folder")
			rootInstance.Name = "Root"

			-- Two children with the same name
			local child1 = Instance.new("Folder")
			child1.Name = "Duplicate"
			child1.Parent = rootInstance

			local child2 = Instance.new("Folder")
			child2.Name = "Duplicate"
			child2.Parent = rootInstance

			knownInstances:insert("ROOT", rootInstance)

			local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

			assert(ok, tostring(patch))

			-- Duplicate-named children in real DOM with no virtual counterpart
			-- are eligible for removal like any other unmapped child.
			-- (Previously they were skipped; now the server handles
			-- duplicates via rbxm container serialization.)
			expect(#patch.removed).to.equal(2)
		end)

		it("should add virtual children with duplicate names", function()
			local knownInstances = InstanceMap.new()
			local virtualInstances = {
				ROOT = {
					ClassName = "Folder",
					Name = "Root",
					Properties = {},
					Children = { "CHILD_A", "CHILD_B" },
				},
				CHILD_A = {
					ClassName = "Folder",
					Name = "Duplicate", -- Same name as CHILD_B
					Properties = {},
					Children = {},
				},
				CHILD_B = {
					ClassName = "Folder",
					Name = "Duplicate", -- Same name as CHILD_A
					Properties = {},
					Children = {},
				},
			}

			local rootInstance = Instance.new("Folder")
			rootInstance.Name = "Root"
			knownInstances:insert("ROOT", rootInstance)

			local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

			assert(ok, tostring(patch))

			-- Duplicate-named virtual children should be added (server
			-- handles them via rbxm container serialization)
			expect(patch.added["CHILD_A"]).to.equal(virtualInstances["CHILD_A"])
			expect(patch.added["CHILD_B"]).to.equal(virtualInstances["CHILD_B"])
		end)
	end)

	describe("Archivable instances", function()
		it("should not remove non-Archivable children", function()
			local knownInstances = InstanceMap.new()
			local virtualInstances = {
				ROOT = {
					ClassName = "Folder",
					Name = "Root",
					Properties = {},
					Children = {},
				},
			}

			local rootInstance = Instance.new("Folder")
			rootInstance.Name = "Root"

			local nonArchivable = Instance.new("Folder")
			nonArchivable.Name = "SessionLock"
			nonArchivable.Archivable = false
			nonArchivable.Parent = rootInstance

			knownInstances:insert("ROOT", rootInstance)

			local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

			assert(ok, tostring(patch))

			-- Non-archivable instance should NOT be in removed list
			assert(isEmpty(patch.removed))
		end)

		it("should still remove archivable unknown children", function()
			local knownInstances = InstanceMap.new()
			local virtualInstances = {
				ROOT = {
					ClassName = "Folder",
					Name = "Root",
					Properties = {},
					Children = {},
				},
			}

			local rootInstance = Instance.new("Folder")
			rootInstance.Name = "Root"

			local archivable = Instance.new("Folder")
			archivable.Name = "ToRemove"
			archivable.Archivable = true
			archivable.Parent = rootInstance

			knownInstances:insert("ROOT", rootInstance)

			local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

			assert(ok, tostring(patch))

			expect(#patch.removed).to.equal(1)
		end)
	end)

	describe("ScrollingFrame CanvasPosition", function()
		it("should ignore CanvasPosition changes on ScrollingFrame", function()
			local knownInstances = InstanceMap.new()
			local virtualInstances = {
				ROOT = {
					ClassName = "ScrollingFrame",
					Name = "Scroller",
					Properties = {
						CanvasPosition = {
							Vector2 = { 100, 200 },
						},
					},
					Children = {},
				},
			}

			local rootInstance = Instance.new("ScrollingFrame")
			rootInstance.Name = "Scroller"
			rootInstance.CanvasPosition = Vector2.new(0, 0)
			knownInstances:insert("ROOT", rootInstance)

			local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

			assert(ok, tostring(patch))

			-- CanvasPosition should be ignored
			assert(isEmpty(patch.updated))
		end)
	end)

	describe("property comparison edge cases", function()
		it("should treat equal StringValues as unchanged", function()
			local knownInstances = InstanceMap.new()
			local virtualInstances = {
				ROOT = {
					ClassName = "StringValue",
					Name = "Test",
					Properties = {
						Value = { String = "Same" },
					},
					Children = {},
				},
			}

			local rootInstance = Instance.new("StringValue")
			rootInstance.Name = "Test"
			rootInstance.Value = "Same"
			knownInstances:insert("ROOT", rootInstance)

			local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

			assert(ok, tostring(patch))
			assert(PatchSet.isEmpty(patch), "expected empty patch for unchanged value")
		end)

		it("should detect changed className", function()
			local knownInstances = InstanceMap.new()
			local virtualInstances = {
				ROOT = {
					ClassName = "Model",
					Name = "Root",
					Properties = {},
					Children = {},
				},
			}

			local rootInstance = Instance.new("Folder")
			rootInstance.Name = "Root"
			knownInstances:insert("ROOT", rootInstance)

			local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

			assert(ok, tostring(patch))

			expect(#patch.updated).to.equal(1)
			expect(patch.updated[1].changedClassName).to.equal("Model")
		end)

		it("should handle multiple property changes on same instance", function()
			local knownInstances = InstanceMap.new()
			local virtualInstances = {
				ROOT = {
					ClassName = "StringValue",
					Name = "NewName",
					Properties = {
						Value = { String = "NewValue" },
					},
					Children = {},
				},
			}

			local rootInstance = Instance.new("StringValue")
			rootInstance.Name = "OldName"
			rootInstance.Value = "OldValue"
			knownInstances:insert("ROOT", rootInstance)

			local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

			assert(ok, tostring(patch))

			expect(#patch.updated).to.equal(1)
			expect(patch.updated[1].changedName).to.equal("NewName")
			expect(size(patch.updated[1].changedProperties)).to.equal(1)
		end)
	end)

	describe("ignored class names", function()
		it("should not remove Camera instances", function()
			local knownInstances = InstanceMap.new()
			local virtualInstances = {
				ROOT = {
					ClassName = "Folder",
					Name = "Root",
					Properties = {},
					Children = {},
				},
			}

			local rootInstance = Instance.new("Folder")
			rootInstance.Name = "Root"

			local camera = Instance.new("Camera")
			camera.Parent = rootInstance

			knownInstances:insert("ROOT", rootInstance)

			local ok, patch = diff(knownInstances, virtualInstances, "ROOT")

			assert(ok, tostring(patch))

			-- Camera is in Config.ignoredClassNames, should not be removed
			assert(isEmpty(patch.removed))
		end)
	end)
end
