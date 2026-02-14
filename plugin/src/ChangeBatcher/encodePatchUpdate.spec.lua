return function()
	local encodePatchUpdate = require(script.Parent.encodePatchUpdate)

	it("should return an update when there are property changes", function()
		local part = Instance.new("Part")
		local properties = {
			CFrame = true,
			Color = true,
		}
		local update = encodePatchUpdate(part, "PART", properties)

		expect(update.id).to.equal("PART")
		expect(update.changedProperties.CFrame).to.be.ok()
		expect(update.changedProperties.Color).to.be.ok()
	end)

	it("should return nil when there are no property changes", function()
		local part = Instance.new("Part")
		local properties = {
			NonExistentProperty = true,
		}
		local update = encodePatchUpdate(part, "PART", properties)

		expect(update).to.equal(nil)
	end)

	it("should set changedName in the update when the instance's Name changes", function()
		local part = Instance.new("Part")
		local properties = {
			Name = true,
		}

		part.Name = "We'reGettingToTheCoolPart"

		local update = encodePatchUpdate(part, "PART", properties)

		expect(update.changedName).to.equal("We'reGettingToTheCoolPart")
	end)

	it("should correctly encode property values", function()
		local part = Instance.new("Part")
		local properties = {
			Position = true,
			Color = true,
		}

		part.Position = Vector3.new(0, 100, 0)
		part.Color = Color3.new(0.8, 0.2, 0.9)

		local update = encodePatchUpdate(part, "PART", properties)
		local position = update.changedProperties.Position
		local color = update.changedProperties.Color

		expect(position.Vector3[1]).to.equal(0)
		expect(position.Vector3[2]).to.equal(100)
		expect(position.Vector3[3]).to.equal(0)

		expect(color.Color3[1]).to.be.near(0.8, 0.01)
		expect(color.Color3[2]).to.be.near(0.2, 0.01)
		expect(color.Color3[3]).to.be.near(0.9, 0.01)
	end)

	-- -------------------------------------------------------------------
	-- Ref property encoding tests
	-- -------------------------------------------------------------------

	local InstanceMap = require(script.Parent.Parent.InstanceMap)

	describe("Ref property encoding", function()
		local container
		local instanceMap

		beforeEach(function()
			container = Instance.new("Folder")
			container.Name = "RefTestContainer"
			container.Parent = game:GetService("Workspace")
			instanceMap = InstanceMap.new()
		end)

		afterEach(function()
			instanceMap:stop()
			if container then
				container:Destroy()
				container = nil
			end
		end)

		it("should encode Ref property to tracked instance", function()
			local model = Instance.new("Model")
			model.Parent = container
			local part = Instance.new("Part")
			part.Parent = model
			model.PrimaryPart = part

			instanceMap:insert("MODEL_ID", model)
			instanceMap:insert("PART_ID", part)

			local update = encodePatchUpdate(model, "MODEL_ID", { PrimaryPart = true }, instanceMap)

			expect(update).to.be.ok()
			expect(update.changedProperties.PrimaryPart).to.be.ok()
			expect(update.changedProperties.PrimaryPart.Ref).to.equal("PART_ID")
		end)

		it("should encode nil Ref as null ref", function()
			local model = Instance.new("Model")
			model.Parent = container
			model.PrimaryPart = nil

			instanceMap:insert("MODEL_ID", model)

			local update = encodePatchUpdate(model, "MODEL_ID", { PrimaryPart = true }, instanceMap)

			expect(update).to.be.ok()
			expect(update.changedProperties.PrimaryPart).to.be.ok()
			expect(update.changedProperties.PrimaryPart.Ref).to.equal("00000000000000000000000000000000")
		end)

		it("should skip Ref when target not in InstanceMap", function()
			local model = Instance.new("Model")
			model.Parent = container
			local part = Instance.new("Part")
			part.Parent = model
			model.PrimaryPart = part

			instanceMap:insert("MODEL_ID", model)
			-- part is NOT inserted into instanceMap

			local update = encodePatchUpdate(model, "MODEL_ID", { PrimaryPart = true }, instanceMap)

			-- PrimaryPart should be skipped (not encoded)
			if update then
				expect(update.changedProperties.PrimaryPart).to.equal(nil)
			end
		end)

		it("should encode Ref alongside non-Ref properties", function()
			local model = Instance.new("Model")
			model.Parent = container
			local part = Instance.new("Part")
			part.Parent = model
			model.PrimaryPart = part
			model.Name = "TestModel"

			instanceMap:insert("MODEL_ID", model)
			instanceMap:insert("PART_ID", part)

			local update = encodePatchUpdate(model, "MODEL_ID", { PrimaryPart = true, Name = true }, instanceMap)

			expect(update).to.be.ok()
			expect(update.changedProperties.PrimaryPart).to.be.ok()
			expect(update.changedProperties.PrimaryPart.Ref).to.equal("PART_ID")
			expect(update.changedName).to.equal("TestModel")
		end)

		it("should return nil when only unresolvable Ref changes", function()
			local model = Instance.new("Model")
			model.Parent = container
			local part = Instance.new("Part")
			part.Parent = model
			model.PrimaryPart = part

			instanceMap:insert("MODEL_ID", model)
			-- part not in map

			local update = encodePatchUpdate(model, "MODEL_ID", { PrimaryPart = true }, instanceMap)

			-- No encodable changes → nil
			expect(update).to.equal(nil)
		end)

		it("should encode ObjectValue.Value Ref", function()
			local objVal = Instance.new("ObjectValue")
			objVal.Parent = container
			local target = Instance.new("Part")
			target.Parent = container
			objVal.Value = target

			instanceMap:insert("OBJVAL_ID", objVal)
			instanceMap:insert("TARGET_ID", target)

			local update = encodePatchUpdate(objVal, "OBJVAL_ID", { Value = true }, instanceMap)

			expect(update).to.be.ok()
			expect(update.changedProperties.Value).to.be.ok()
			expect(update.changedProperties.Value.Ref).to.equal("TARGET_ID")
		end)

		it("should encode self-referencing Ref", function()
			local model = Instance.new("Model")
			model.Parent = container
			local part = Instance.new("Part")
			part.Parent = model
			model.PrimaryPart = part

			instanceMap:insert("MODEL_ID", model)
			-- PrimaryPart points to part, which IS in the map
			instanceMap:insert("PART_ID", part)

			local update = encodePatchUpdate(model, "MODEL_ID", { PrimaryPart = true }, instanceMap)

			expect(update).to.be.ok()
			expect(update.changedProperties.PrimaryPart.Ref).to.equal("PART_ID")
		end)

		it("should warn when no InstanceMap provided for Ref property", function()
			local model = Instance.new("Model")
			model.Parent = container
			local part = Instance.new("Part")
			part.Parent = model
			model.PrimaryPart = part

			-- Pass nil for instanceMap
			local update = encodePatchUpdate(model, "MODEL_ID", { PrimaryPart = true }, nil)

			-- Should not crash; PrimaryPart is skipped
			if update then
				expect(update.changedProperties.PrimaryPart).to.equal(nil)
			end
		end)

		it("should encode multiple Ref properties on same instance", function()
			-- Use a Weld which has Part0 and Part1 ref properties
			local weld = Instance.new("WeldConstraint")
			weld.Parent = container
			local partA = Instance.new("Part")
			partA.Parent = container
			local partB = Instance.new("Part")
			partB.Parent = container
			weld.Part0 = partA
			weld.Part1 = partB

			instanceMap:insert("WELD_ID", weld)
			instanceMap:insert("PART_A", partA)
			instanceMap:insert("PART_B", partB)

			local update = encodePatchUpdate(weld, "WELD_ID", { Part0 = true, Part1 = true }, instanceMap)

			expect(update).to.be.ok()
			expect(update.changedProperties.Part0).to.be.ok()
			expect(update.changedProperties.Part0.Ref).to.equal("PART_A")
			expect(update.changedProperties.Part1).to.be.ok()
			expect(update.changedProperties.Part1.Ref).to.equal("PART_B")
		end)
	end)

	-- -------------------------------------------------------------------
	-- Regression tests for known limitations (audit section 10)
	--
	-- These tests assert the CORRECT behavior. They are expected to FAIL
	-- against the current implementation, proving the bugs are real.
	-- DO NOT fix the implementation to make them pass.
	-- If a test unexpectedly passes, leave it -- it's free coverage.
	-- -------------------------------------------------------------------

	describe("Known limitation: untracked Ref target (10c)", function()
		local container2
		local instanceMap2

		beforeEach(function()
			container2 = Instance.new("Folder")
			container2.Name = "RefRegressionContainer"
			container2.Parent = game:GetService("Workspace")
			instanceMap2 = InstanceMap.new()
		end)

		afterEach(function()
			instanceMap2:stop()
			if container2 then
				container2:Destroy()
				container2 = nil
			end
		end)

		it("should encode Ref to untracked instance (EXPECTED FAIL)", function()
			-- The CORRECT behavior: if a Model's PrimaryPart points to a Part
			-- that exists in Studio but isn't tracked by Atlas, the Ref should
			-- still be encoded somehow (e.g., deferred or queued).
			-- ACTUAL behavior: property is silently dropped with a warning.
			local model = Instance.new("Model")
			model.Parent = container2
			local part = Instance.new("Part")
			part.Parent = model
			model.PrimaryPart = part

			instanceMap2:insert("MODEL_ID", model)
			-- Part is NOT in instanceMap

			local update = encodePatchUpdate(model, "MODEL_ID", { PrimaryPart = true }, instanceMap2)

			-- CORRECT: PrimaryPart should be encoded
			-- ACTUAL: update is nil or PrimaryPart is missing
			expect(update).to.be.ok()
			expect(update.changedProperties.PrimaryPart).to.be.ok()
		end)
	end)

	describe("Known limitation: missing instanceMap arg (10d)", function()
		local container3
		local instanceMap3

		beforeEach(function()
			container3 = Instance.new("Folder")
			container3.Name = "RefMissingMapContainer"
			container3.Parent = game:GetService("Workspace")
			instanceMap3 = InstanceMap.new()
		end)

		afterEach(function()
			instanceMap3:stop()
			if container3 then
				container3:Destroy()
				container3 = nil
			end
		end)

		it("should encode Ref when instanceMap is nil (EXPECTED FAIL)", function()
			-- This simulates ServeSession.lua:738 which calls encodePatchUpdate
			-- WITHOUT the 4th instanceMap argument. The CORRECT behavior is that
			-- Ref properties should be encoded. ACTUAL: they are dropped.
			local model = Instance.new("Model")
			model.Parent = container3
			local part = Instance.new("Part")
			part.Parent = model
			model.PrimaryPart = part

			instanceMap3:insert("MODEL_ID", model)
			instanceMap3:insert("PART_ID", part)

			-- Call WITHOUT instanceMap (4th arg) -- simulates ServeSession.lua:738
			local update = encodePatchUpdate(model, "MODEL_ID", { PrimaryPart = true })

			-- CORRECT: PrimaryPart should be encoded as { Ref = "PART_ID" }
			-- ACTUAL: PrimaryPart is dropped because instanceMap is nil
			expect(update).to.be.ok()
			expect(update.changedProperties.PrimaryPart).to.be.ok()
			expect(update.changedProperties.PrimaryPart.Ref).to.equal("PART_ID")
		end)
	end)
end
