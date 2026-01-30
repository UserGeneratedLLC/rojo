return function()
	local ChangeBatcher = require(script.Parent)
	local InstanceMap = require(script.Parent.Parent.InstanceMap)
	local PatchSet = require(script.Parent.Parent.PatchSet)

	local noop = function() end

	describe("new", function()
		it("should create a new ChangeBatcher", function()
			local instanceMap = InstanceMap.new()
			local changeBatcher = ChangeBatcher.new(instanceMap, noop)

			expect(changeBatcher.__pendingPropertyChanges).to.be.a("table")
			expect(next(changeBatcher.__pendingPropertyChanges)).to.equal(nil)
			expect(changeBatcher.__onChangesFlushed).to.equal(noop)
			expect(changeBatcher.__instanceMap).to.equal(instanceMap)
			expect(typeof(changeBatcher.__renderSteppedConnection)).to.equal("RBXScriptConnection")
		end)
	end)

	describe("stop", function()
		it("should disconnect the RenderStepped connection", function()
			local changeBatcher = ChangeBatcher.new(InstanceMap.new(), noop)

			changeBatcher:stop()

			expect(changeBatcher.__renderSteppedConnection.Connected).to.equal(false)
		end)
	end)

	describe("add", function()
		it("should add property changes to be considered for the current batch", function()
			local instanceMap = InstanceMap.new()
			local changeBatcher = ChangeBatcher.new(instanceMap, noop)
			local part = Instance.new("Part")

			instanceMap:insert("PART", part)
			changeBatcher:add(part, "Name")

			local properties = changeBatcher.__pendingPropertyChanges[part]

			expect(properties).to.be.a("table")
			expect(properties.Name).to.be.ok()

			changeBatcher:add(part, "Position")
			expect(properties.Position).to.be.ok()
		end)
	end)

	describe("__cycle", function()
		it("should immediately unpause any paused instances after each cycle", function()
			local instanceMap = InstanceMap.new()
			local changeBatcher = ChangeBatcher.new(instanceMap, noop)
			local part = Instance.new("Part")

			instanceMap.pausedUpdateInstances[part] = true

			changeBatcher:__cycle(0)

			expect(instanceMap.pausedUpdateInstances[part]).to.equal(nil)
		end)
	end)

	describe("__flush", function()
		it("should return nil when there are no changes to process", function()
			local changeBatcher = ChangeBatcher.new(InstanceMap.new(), noop)
			expect(changeBatcher:__flush()).to.equal(nil)
		end)

		it("should return a patch when there are changes to process and the resulting patch is non-empty", function()
			local instanceMap = InstanceMap.new()
			local changeBatcher = ChangeBatcher.new(instanceMap, noop)
			local part = Instance.new("Part")

			instanceMap:insert("PART", part)

			changeBatcher.__pendingPropertyChanges[part] = {
				Position = true,
				Name = true,
			}

			local patch = changeBatcher:__flush()

			assert(PatchSet.validate(patch))
		end)

		it("should return nil when there are changes to process and the resulting patch is empty", function()
			local instanceMap = InstanceMap.new()
			local changeBatcher = ChangeBatcher.new(instanceMap, noop)
			local part = Instance.new("Part")

			instanceMap:insert("PART", part)

			changeBatcher.__pendingPropertyChanges[part] = {
				NonExistentProperty = true,
			}

			expect(changeBatcher:__flush()).to.equal(nil)
		end)
	end)

	describe("pause/resume", function()
		it("should start unpaused by default", function()
			local changeBatcher = ChangeBatcher.new(InstanceMap.new(), noop)
			expect(changeBatcher:isPaused()).to.equal(false)
			changeBatcher:stop()
		end)

		it("should be pausable", function()
			local changeBatcher = ChangeBatcher.new(InstanceMap.new(), noop)
			changeBatcher:pause()
			expect(changeBatcher:isPaused()).to.equal(true)
			changeBatcher:stop()
		end)

		it("should be resumable after pause", function()
			local changeBatcher = ChangeBatcher.new(InstanceMap.new(), noop)
			changeBatcher:pause()
			expect(changeBatcher:isPaused()).to.equal(true)
			changeBatcher:resume()
			expect(changeBatcher:isPaused()).to.equal(false)
			changeBatcher:stop()
		end)

		it("should not flush when paused", function()
			local flushed = false
			local instanceMap = InstanceMap.new()
			local changeBatcher = ChangeBatcher.new(instanceMap, function()
				flushed = true
			end)

			local part = Instance.new("Part")
			instanceMap:insert("PART", part)
			changeBatcher.__pendingPropertyChanges[part] = { Name = true }

			changeBatcher:pause()

			-- Call __cycle with a large dt to trigger flush (normally 0.2s threshold)
			changeBatcher:__cycle(1.0)

			-- Should not have flushed because we're paused
			expect(flushed).to.equal(false)

			changeBatcher:stop()
			part:Destroy()
		end)

		it("should flush after resume", function()
			local flushed = false
			local instanceMap = InstanceMap.new()
			local changeBatcher = ChangeBatcher.new(instanceMap, function()
				flushed = true
			end)

			local part = Instance.new("Part")
			instanceMap:insert("PART", part)
			changeBatcher.__pendingPropertyChanges[part] = { Name = true }

			changeBatcher:pause()
			changeBatcher:__cycle(1.0)
			expect(flushed).to.equal(false)

			changeBatcher:resume()
			changeBatcher:__cycle(1.0)
			expect(flushed).to.equal(true)

			changeBatcher:stop()
			part:Destroy()
		end)
	end)
end
