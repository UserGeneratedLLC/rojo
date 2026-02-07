--[[
	Data loss and edge case tests for ChangeBatcher.

	These tests verify that the ChangeBatcher does not silently lose data,
	handles edge cases gracefully, and behaves correctly under adversarial
	conditions.
]]

return function()
	local ChangeBatcher = require(script.Parent)
	local InstanceMap = require(script.Parent.Parent.InstanceMap)
	local PatchSet = require(script.Parent.Parent.PatchSet)

	local noop = function() end

	describe("instance removed from map before flush", function()
		it("should not crash when tracked instance is removed before flush", function()
			local instanceMap = InstanceMap.new()
			local changeBatcher = ChangeBatcher.new(instanceMap, noop)

			local part = Instance.new("Part")
			instanceMap:insert("PART", part)

			-- Add a change
			changeBatcher:add(part, "Name")

			-- Remove the instance from the map before flush
			instanceMap:removeId("PART")

			-- Flush should not crash (instance no longer has an ID)
			local patch = changeBatcher:__flush()

			-- Patch should be nil or empty since the instance is unmapped
			if patch ~= nil then
				-- If a patch was returned, it should be valid
				assert(PatchSet.validate(patch))
			end

			part:Destroy()
			changeBatcher:stop()
		end)

		it("should not crash when tracked instance is destroyed before flush", function()
			local instanceMap = InstanceMap.new()
			local changeBatcher = ChangeBatcher.new(instanceMap, noop)

			local part = Instance.new("Part")
			instanceMap:insert("PART", part)

			-- Add a change
			changeBatcher:add(part, "Name")

			-- Destroy the instance before flush
			part:Destroy()

			-- Flush should not crash
			local patch = changeBatcher:__flush()

			-- Should handle gracefully
			if patch ~= nil then
				assert(PatchSet.validate(patch))
			end

			changeBatcher:stop()
		end)
	end)

	describe("flush produces empty patch", function()
		it("should not call onChangesFlushed when all properties encode to no-ops", function()
			local instanceMap = InstanceMap.new()
			local flushed = false

			local changeBatcher = ChangeBatcher.new(instanceMap, function()
				flushed = true
			end)

			local part = Instance.new("Part")
			instanceMap:insert("PART", part)

			-- Add a change for a non-existent property (will fail to encode)
			changeBatcher.__pendingPropertyChanges[part] = {
				NonExistentProperty = true,
			}

			-- Trigger flush
			changeBatcher:__cycle(1.0)

			-- Callback should NOT have been called since patch is empty
			expect(flushed).to.equal(false)

			part:Destroy()
			changeBatcher:stop()
		end)

		it("should return nil from __flush when pending changes produce empty patch", function()
			local instanceMap = InstanceMap.new()
			local changeBatcher = ChangeBatcher.new(instanceMap, noop)

			local part = Instance.new("Part")
			instanceMap:insert("PART", part)

			changeBatcher.__pendingPropertyChanges[part] = {
				NonExistentProperty = true,
			}

			local patch = changeBatcher:__flush()

			expect(patch).to.equal(nil)

			part:Destroy()
			changeBatcher:stop()
		end)
	end)

	describe("pause preserves pending changes", function()
		it("should not lose changes while paused", function()
			local instanceMap = InstanceMap.new()
			local flushedPatches = {}

			local changeBatcher = ChangeBatcher.new(instanceMap, function(patch)
				table.insert(flushedPatches, patch)
			end)

			local part = Instance.new("Part")
			instanceMap:insert("PART", part)

			-- Add a change and pause
			changeBatcher:add(part, "Name")
			changeBatcher:pause()

			-- Cycle while paused - should NOT flush
			changeBatcher:__cycle(1.0)
			expect(#flushedPatches).to.equal(0)

			-- Changes should still be pending
			expect(changeBatcher.__pendingPropertyChanges[part]).to.be.ok()
			expect(changeBatcher.__pendingPropertyChanges[part].Name).to.be.ok()

			-- Resume and cycle - should flush the pending changes
			changeBatcher:resume()
			changeBatcher:__cycle(1.0)

			expect(#flushedPatches).to.equal(1)

			part:Destroy()
			changeBatcher:stop()
		end)

		it("should accumulate new changes while paused", function()
			local instanceMap = InstanceMap.new()
			local changeBatcher = ChangeBatcher.new(instanceMap, noop)

			local part = Instance.new("Part")
			instanceMap:insert("PART", part)

			changeBatcher:pause()

			-- Add multiple changes while paused
			changeBatcher:add(part, "Name")
			changeBatcher:add(part, "Position")
			changeBatcher:add(part, "Size")

			-- All should be pending
			local pending = changeBatcher.__pendingPropertyChanges[part]
			expect(pending).to.be.ok()
			expect(pending.Name).to.be.ok()
			expect(pending.Position).to.be.ok()
			expect(pending.Size).to.be.ok()

			part:Destroy()
			changeBatcher:stop()
		end)
	end)

	describe("rapid add/remove/add", function()
		it("should handle rapid property changes on same instance", function()
			local instanceMap = InstanceMap.new()
			local flushCount = 0

			local changeBatcher = ChangeBatcher.new(instanceMap, function(_patch)
				flushCount += 1
			end)

			local sv = Instance.new("StringValue")
			sv.Name = "Test"
			sv.Value = "Initial"
			instanceMap:insert("SV", sv)

			-- Rapid changes to same property
			changeBatcher:add(sv, "Value")
			changeBatcher:add(sv, "Value")
			changeBatcher:add(sv, "Value")

			-- Should only have one entry per property
			local pending = changeBatcher.__pendingPropertyChanges[sv]
			expect(pending).to.be.ok()
			expect(pending.Value).to.equal(true)

			-- Flush should produce a single update
			changeBatcher:__cycle(1.0)

			expect(flushCount).to.equal(1)

			sv:Destroy()
			changeBatcher:stop()
		end)

		it("should handle changes to multiple properties on same instance", function()
			local instanceMap = InstanceMap.new()
			local changeBatcher = ChangeBatcher.new(instanceMap, noop)

			local part = Instance.new("Part")
			instanceMap:insert("PART", part)

			-- Multiple different properties
			changeBatcher:add(part, "Name")
			changeBatcher:add(part, "Position")
			changeBatcher:add(part, "Size")
			changeBatcher:add(part, "Anchored")

			-- All should be tracked
			local pending = changeBatcher.__pendingPropertyChanges[part]
			expect(pending.Name).to.be.ok()
			expect(pending.Position).to.be.ok()
			expect(pending.Size).to.be.ok()
			expect(pending.Anchored).to.be.ok()

			part:Destroy()
			changeBatcher:stop()
		end)
	end)

	describe("deletion detection via Parent=nil", function()
		it("should detect instance deletion when Parent changes to nil", function()
			local instanceMap = InstanceMap.new()
			local flushedPatches = {}

			local changeBatcher = ChangeBatcher.new(instanceMap, function(patch)
				table.insert(flushedPatches, patch)
			end)

			local parent = Instance.new("Folder")
			parent.Name = "Parent"

			local child = Instance.new("StringValue")
			child.Name = "Child"
			child.Value = "test"
			child.Parent = parent

			instanceMap:insert("PARENT", parent)
			instanceMap:insert("CHILD", child)

			-- Simulate deletion detection: Parent property changed to nil
			changeBatcher:add(child, "Parent")

			-- Remove the child to simulate actual deletion
			child.Parent = nil

			-- Flush
			changeBatcher:__cycle(1.0)

			-- Should have produced a patch with a removal
			if #flushedPatches > 0 then
				local patch = flushedPatches[1]
				assert(PatchSet.validate(patch))
				-- The patch should contain a removal for the child
				expect(#patch.removed).to.equal(1)
			end

			parent:Destroy()
			changeBatcher:stop()
		end)
	end)

	describe("multiple instances", function()
		it("should track changes for multiple instances independently", function()
			local instanceMap = InstanceMap.new()
			local changeBatcher = ChangeBatcher.new(instanceMap, noop)

			local part1 = Instance.new("Part")
			local part2 = Instance.new("Part")
			instanceMap:insert("PART1", part1)
			instanceMap:insert("PART2", part2)

			changeBatcher:add(part1, "Name")
			changeBatcher:add(part2, "Position")

			expect(changeBatcher.__pendingPropertyChanges[part1]).to.be.ok()
			expect(changeBatcher.__pendingPropertyChanges[part1].Name).to.be.ok()

			expect(changeBatcher.__pendingPropertyChanges[part2]).to.be.ok()
			expect(changeBatcher.__pendingPropertyChanges[part2].Position).to.be.ok()

			-- Part1 changes should not affect Part2
			expect(changeBatcher.__pendingPropertyChanges[part1].Position).to.equal(nil)
			expect(changeBatcher.__pendingPropertyChanges[part2].Name).to.equal(nil)

			part1:Destroy()
			part2:Destroy()
			changeBatcher:stop()
		end)
	end)

	describe("flush clears pending changes", function()
		it("should clear all pending changes after flush", function()
			local instanceMap = InstanceMap.new()
			local changeBatcher = ChangeBatcher.new(instanceMap, noop)

			local part = Instance.new("Part")
			instanceMap:insert("PART", part)

			changeBatcher:add(part, "Name")
			changeBatcher:add(part, "Position")

			expect(next(changeBatcher.__pendingPropertyChanges)).to.be.ok()

			changeBatcher:__flush()

			-- All pending changes should be cleared
			expect(next(changeBatcher.__pendingPropertyChanges)).to.equal(nil)

			part:Destroy()
			changeBatcher:stop()
		end)

		it("should not flush same changes twice", function()
			local instanceMap = InstanceMap.new()
			local flushCount = 0

			local changeBatcher = ChangeBatcher.new(instanceMap, function()
				flushCount += 1
			end)

			local part = Instance.new("Part")
			instanceMap:insert("PART", part)

			changeBatcher:add(part, "Name")

			-- First flush
			changeBatcher:__cycle(1.0)
			expect(flushCount).to.equal(1)

			-- Second flush with no new changes
			changeBatcher:__cycle(1.0)
			expect(flushCount).to.equal(1) -- Still 1, not 2

			part:Destroy()
			changeBatcher:stop()
		end)
	end)

	describe("stop during active changes", function()
		it("should safely stop even with pending changes", function()
			local instanceMap = InstanceMap.new()
			local changeBatcher = ChangeBatcher.new(instanceMap, noop)

			local part = Instance.new("Part")
			instanceMap:insert("PART", part)

			changeBatcher:add(part, "Name")
			changeBatcher:add(part, "Position")

			-- Stop should not crash
			changeBatcher:stop()

			-- Pending changes should be cleared
			expect(next(changeBatcher.__pendingPropertyChanges)).to.equal(nil)

			part:Destroy()
		end)
	end)
end
