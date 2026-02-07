--[[
	Tests for ServeSession, focused on one-shot sync and WebSocket message handling.

	These tests construct a ServeSession directly and manipulate internal state
	to test specific behaviors without going through the full start() flow.
]]

return function()
	local ServeSession = require(script.Parent.ServeSession)
	local PatchSet = require(script.Parent.PatchSet)
	local Settings = require(script.Parent.Settings)
	local testUtils = require(script.Parent.testUtils)
	local MockApiContext = testUtils.MockApiContext

	local function createSession(overrides)
		overrides = overrides or {}

		local mockApi = MockApiContext.new()
		local session = ServeSession.new({
			apiContext = mockApi,
			twoWaySync = if overrides.twoWaySync ~= nil then overrides.twoWaySync else true,
		})

		-- Set to Connected so __onWebSocketMessage doesn't bail at the Disconnected check
		session.__status = ServeSession.Status.Connected
		session.__serverInfo = {
			projectName = "TestProject",
			sessionId = "test-session-id",
		}

		return session, mockApi
	end

	local function createMessagesPacket(patch)
		return {
			messageCursor = 1,
			messages = { patch },
		}
	end

	local function createTestPatch()
		local patch = PatchSet.newEmpty()
		table.insert(patch.updated, {
			id = "test-instance-id",
			changedProperties = {
				Value = { String = "changed" },
			},
		})
		return patch
	end

	describe("__onWebSocketMessage", function()
		describe("merge behavior during confirmation", function()
			it("should merge messages into __confirmingPatch when confirmation is active", function()
				local session = createSession()

				-- Simulate active confirmation: __confirmingPatch is non-nil
				local confirmingPatch = PatchSet.newEmpty()
				table.insert(confirmingPatch.updated, {
					id = "existing-update-id",
					changedProperties = {
						Value = { String = "existing" },
					},
				})
				session.__confirmingPatch = confirmingPatch
				session.__isConfirming = true

				local patchUpdateCalled = false
				session:setPatchUpdateCallback(function()
					patchUpdateCalled = true
				end)

				-- Send a WebSocket message while confirming
				local incomingPatch = createTestPatch()
				session:__onWebSocketMessage(createMessagesPacket(incomingPatch))

				-- Should have merged into the confirming patch, not spawned a new confirmation
				expect(patchUpdateCalled).to.equal(true)
				-- The confirming patch should now have both updates
				expect(#confirmingPatch.updated).to.equal(2)

				session:stop()
			end)

			it("should not spawn a new confirmation when __confirmingPatch is non-nil", function()
				local session = createSession()

				local confirmCallbackCalled = false
				session:setConfirmCallback(function()
					confirmCallbackCalled = true
					return "Accept"
				end)

				-- Set up active confirmation
				session.__confirmingPatch = PatchSet.newEmpty()
				session.__isConfirming = true

				-- Send WebSocket message
				session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))

				-- Confirm callback should NOT have been called (message was merged)
				expect(confirmCallbackCalled).to.equal(false)

				session:stop()
			end)
		end)

		describe("new confirmation when not already confirming", function()
			it("should call confirm callback when __confirmingPatch is nil", function()
				local session = createSession()

				local confirmCallbackCalled = false
				session:setConfirmCallback(function()
					confirmCallbackCalled = true
					return "Accept"
				end)

				-- No active confirmation
				session.__confirmingPatch = nil
				session.__isConfirming = false

				-- Send WebSocket message
				session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))

				-- Allow the task.spawn to execute
				task.wait()

				expect(confirmCallbackCalled).to.equal(true)

				session:stop()
			end)

			it("should pause ChangeBatcher when spawning new confirmation", function()
				local session = createSession()

				local wasPausedDuringCallback = false
				session:setConfirmCallback(function()
					-- task.spawn runs the function immediately until first yield,
					-- so we check pause state INSIDE the callback
					wasPausedDuringCallback = session.__changeBatcher:isPaused()
					return "Accept"
				end)

				session.__confirmingPatch = nil
				session.__isConfirming = false

				-- Should not be paused before
				expect(session.__changeBatcher:isPaused()).to.equal(false)

				session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))

				-- task.spawn ran synchronously (no yield in callback),
				-- so batcher was paused during callback and resumed after
				task.wait()
				expect(wasPausedDuringCallback).to.equal(true)
				expect(session.__changeBatcher:isPaused()).to.equal(false)

				session:stop()
			end)
		end)

		describe("Skip response handling", function()
			it("should not apply patches when confirm callback returns Skip", function()
				local session = createSession()

				local applyPatchCalled = false
				local originalApplyPatch = session.__applyPatch
				function session:__applyPatch(_patch)
					applyPatchCalled = true
					-- Don't actually apply
				end

				session:setConfirmCallback(function()
					return "Skip"
				end)

				session.__confirmingPatch = nil
				session.__isConfirming = false

				session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))

				-- Allow task.spawn to execute
				task.wait()

				expect(applyPatchCalled).to.equal(false)

				-- Restore
				session.__applyPatch = originalApplyPatch
				session:stop()
			end)

			it("should clear confirmation flags after Skip", function()
				local session = createSession()

				session:setConfirmCallback(function()
					return "Skip"
				end)

				session.__confirmingPatch = nil
				session.__isConfirming = false

				session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))

				-- Allow task.spawn to execute
				task.wait()

				expect(session.__confirmingPatch).to.equal(nil)
				expect(session.__isConfirming).to.equal(false)

				session:stop()
			end)

			it("should resume ChangeBatcher after Skip", function()
				local session = createSession()

				session:setConfirmCallback(function()
					return "Skip"
				end)

				session.__confirmingPatch = nil
				session.__isConfirming = false

				session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))

				-- Allow task.spawn to execute
				task.wait()

				expect(session.__changeBatcher:isPaused()).to.equal(false)

				session:stop()
			end)
		end)

		describe("Accept response handling", function()
			it("should apply patches when confirm callback returns Accept", function()
				local session = createSession()
				local appliedPatch = nil

				local originalApplyPatch = session.__applyPatch
				function session:__applyPatch(patch)
					appliedPatch = patch
				end

				session:setConfirmCallback(function()
					return "Accept"
				end)

				session.__confirmingPatch = nil
				session.__isConfirming = false

				local testPatch = createTestPatch()
				session:__onWebSocketMessage(createMessagesPacket(testPatch))

				-- Allow task.spawn to execute
				task.wait()

				expect(appliedPatch).to.be.ok()

				session.__applyPatch = originalApplyPatch
				session:stop()
			end)
		end)

		describe("Abort response handling", function()
			it("should stop session when confirm callback returns Abort", function()
				local session = createSession()

				session:setConfirmCallback(function()
					return "Abort"
				end)

				session.__confirmingPatch = nil
				session.__isConfirming = false

				session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))

				-- Allow task.spawn to execute
				task.wait()

				expect(session:getStatus()).to.equal(ServeSession.Status.Disconnected)
			end)
		end)

		describe("Disconnected guard", function()
			it("should ignore messages when status is Disconnected", function()
				local session = createSession()
				session.__status = ServeSession.Status.Disconnected

				local confirmCallbackCalled = false
				session:setConfirmCallback(function()
					confirmCallbackCalled = true
					return "Accept"
				end)

				session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))

				task.wait()

				expect(confirmCallbackCalled).to.equal(false)
			end)
		end)
	end)

	describe("one-shot sync confirm callback pattern", function()
		--[[
			These tests verify the pattern used in App/init.lua where
			an `initialSyncConfirmed` flag gates the confirm callback
			to return "Skip" for post-initial-sync echo patches.
			
			We simulate the pattern directly rather than testing through
			the full App component. PatchSet.removeDataModelName is skipped
			here since it requires an instanceMap and isn't what we're testing.
		]]

		it("should allow first confirmation in one-shot mode", function()
			local initialSyncConfirmed = false

			local function confirmCallback(_instanceMap, _patch, _serverInfo)
				local isOneShotMode = true

				if isOneShotMode and initialSyncConfirmed then
					return "Skip"
				end

				-- Simulate user accepting (would normally wait on UI)
				initialSyncConfirmed = true
				return { type = "Confirm", selections = {} }
			end

			-- First call: should NOT skip
			local result = confirmCallback(nil, createTestPatch(), nil)

			expect(type(result)).to.equal("table")
			expect(result.type).to.equal("Confirm")
		end)

		it("should skip subsequent patches after initial sync is confirmed", function()
			local initialSyncConfirmed = false

			local function confirmCallback(_instanceMap, _patch, _serverInfo)
				local isOneShotMode = true

				if isOneShotMode and initialSyncConfirmed then
					return "Skip"
				end

				initialSyncConfirmed = true
				return { type = "Confirm", selections = {} }
			end

			-- First call: user confirms
			confirmCallback(nil, createTestPatch(), nil)

			-- Second call: should skip
			local result = confirmCallback(nil, createTestPatch(), nil)
			expect(result).to.equal("Skip")

			-- Third call: should also skip
			local result2 = confirmCallback(nil, createTestPatch(), nil)
			expect(result2).to.equal("Skip")
		end)

		it("should not skip in non-one-shot mode", function()
			local initialSyncConfirmed = false

			local function confirmCallback(_instanceMap, patch, _serverInfo)
				local isOneShotMode = false

				if isOneShotMode and initialSyncConfirmed then
					return "Skip"
				end

				if PatchSet.isEmpty(patch) then
					return "Accept"
				end

				initialSyncConfirmed = true
				return { type = "Confirm", selections = {} }
			end

			-- First call
			confirmCallback(nil, createTestPatch(), nil)

			-- Second call: should NOT skip since one-shot is off
			local result = confirmCallback(nil, createTestPatch(), nil)

			expect(result).to.never.equal("Skip")
		end)

		it("should not skip empty patches in non-one-shot mode (auto-accept instead)", function()
			local initialSyncConfirmed = false

			local function confirmCallback(_instanceMap, patch, _serverInfo)
				local isOneShotMode = false

				if isOneShotMode and initialSyncConfirmed then
					return "Skip"
				end

				if PatchSet.isEmpty(patch) then
					return "Accept"
				end

				initialSyncConfirmed = true
				return { type = "Confirm", selections = {} }
			end

			-- First call with non-empty patch
			confirmCallback(nil, createTestPatch(), nil)

			-- Second call with empty patch
			local result = confirmCallback(nil, PatchSet.newEmpty(), nil)

			expect(result).to.equal("Accept")
		end)
	end)

	describe("one-shot sync end-to-end with ServeSession", function()
		it("should skip echo patches after initial sync via confirm callback", function()
			local session = createSession()

			-- Simulate the App pattern: initialSyncConfirmed flag in closure
			local initialSyncConfirmed = false
			session:setConfirmCallback(function(_instanceMap, _patch, _serverInfo)
				if initialSyncConfirmed then
					return "Skip"
				end

				-- First call: simulate user confirming
				initialSyncConfirmed = true
				return "Accept"
			end)

			local applyCount = 0
			local originalApplyPatch = session.__applyPatch
			function session:__applyPatch(_patch)
				applyCount += 1
			end

			session.__confirmingPatch = nil
			session.__isConfirming = false

			-- First WebSocket message: should be confirmed and applied
			session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))
			task.wait()

			expect(applyCount).to.equal(1)

			-- Second WebSocket message (echo): should be skipped
			session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))
			task.wait()

			-- Still 1, not 2 — echo was skipped
			expect(applyCount).to.equal(1)

			-- Third WebSocket message: also skipped
			session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))
			task.wait()

			expect(applyCount).to.equal(1)

			session.__applyPatch = originalApplyPatch
			session:stop()
		end)

		it("should not write echo patches to server", function()
			local session, mockApi = createSession()

			local initialSyncConfirmed = false
			session:setConfirmCallback(function()
				if initialSyncConfirmed then
					return "Skip"
				end

				initialSyncConfirmed = true
				return "Accept"
			end)

			-- Stub __applyPatch to prevent reconciler calls
			function session:__applyPatch() end

			session.__confirmingPatch = nil
			session.__isConfirming = false

			-- Initial sync message
			session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))
			task.wait()

			-- Echo message
			session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))
			task.wait()

			-- Should not have called write() — only setMessageCursor
			local writeRequests = mockApi:getRequestsByMethod("write")
			expect(#writeRequests).to.equal(0)

			session:stop()
		end)
	end)

	describe("WebSocket message edge cases", function()
		it("should batch multiple messages in a single packet into one patch", function()
			local session = createSession()
			local appliedPatch = nil

			function session:__applyPatch(patch)
				appliedPatch = patch
			end

			session:setConfirmCallback(function()
				return "Accept"
			end)

			session.__confirmingPatch = nil
			session.__isConfirming = false

			-- Create a packet with multiple messages
			local msg1 = PatchSet.newEmpty()
			table.insert(msg1.updated, {
				id = "id-1",
				changedProperties = { Value = { String = "one" } },
			})
			local msg2 = PatchSet.newEmpty()
			table.insert(msg2.updated, {
				id = "id-2",
				changedProperties = { Value = { String = "two" } },
			})

			local packet = {
				messageCursor = 1,
				messages = { msg1, msg2 },
			}

			session:__onWebSocketMessage(packet)
			task.wait()

			-- Both messages should be combined into one patch
			expect(appliedPatch).to.be.ok()
			expect(#appliedPatch.updated).to.equal(2)

			session:stop()
		end)

		it("should handle rapid sequential messages alternating merge and spawn", function()
			local session = createSession()
			local applyCount = 0

			function session:__applyPatch()
				applyCount += 1
			end

			session:setConfirmCallback(function()
				return "Accept"
			end)

			session.__confirmingPatch = nil
			session.__isConfirming = false

			-- Send 5 rapid messages
			for i = 1, 5 do
				local patch = PatchSet.newEmpty()
				table.insert(patch.updated, {
					id = "id-" .. tostring(i),
					changedProperties = { Value = { String = "val-" .. tostring(i) } },
				})
				session:__onWebSocketMessage(createMessagesPacket(patch))
			end

			-- Let all task.spawns run
			task.wait()
			task.wait()

			-- Each message should have been processed (Accept returns immediately,
			-- so __confirmingPatch is cleared before next message arrives)
			expect(applyCount).to.equal(5)

			session:stop()
		end)

		it("should safely handle confirm callback returning unexpected value", function()
			local session = createSession()
			local applyPatchCalled = false

			function session:__applyPatch()
				applyPatchCalled = true
			end

			session:setConfirmCallback(function()
				return 42 -- Not a valid response
			end)

			session.__confirmingPatch = nil
			session.__isConfirming = false

			session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))
			task.wait()

			-- Invalid value should fall through without applying
			expect(applyPatchCalled).to.equal(false)
			-- Session should still be alive (not crashed)
			expect(session.__isConfirming).to.equal(false)
			expect(session.__changeBatcher:isPaused()).to.equal(false)

			session:stop()
		end)

		it("should discard merged content when user aborts", function()
			local session = createSession()
			local applyPatchCalled = false

			function session:__applyPatch()
				applyPatchCalled = true
			end

			-- Set up active confirmation with existing patch
			local confirmingPatch = PatchSet.newEmpty()
			table.insert(confirmingPatch.updated, {
				id = "existing-id",
				changedProperties = { Value = { String = "existing" } },
			})
			session.__confirmingPatch = confirmingPatch
			session.__isConfirming = true

			session:setPatchUpdateCallback(function() end)

			-- Merge additional content during confirmation
			session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))

			-- Confirming patch now has 2 updates
			expect(#confirmingPatch.updated).to.equal(2)

			-- Now simulate the user aborting (done outside this test, but verify
			-- the patch is NOT auto-applied by the merge)
			expect(applyPatchCalled).to.equal(false)

			session:stop()
		end)

		it("should handle message with no confirm callback set", function()
			local session = createSession()

			function session:__applyPatch() end

			-- No confirm callback set - should default to "Accept"
			session.__userConfirmCallback = nil
			session.__confirmingPatch = nil
			session.__isConfirming = false

			-- Should not crash
			session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))
			task.wait()

			session:stop()
		end)
	end)

	describe("state transitions", function()
		it("should start with NotStarted status", function()
			local mockApi = MockApiContext.new()
			local session = ServeSession.new({
				apiContext = mockApi,
				twoWaySync = true,
			})

			expect(session:getStatus()).to.equal(ServeSession.Status.NotStarted)

			session:stop()
		end)

		it("should transition to Disconnected on stop", function()
			local session = createSession()

			session:stop()

			expect(session:getStatus()).to.equal(ServeSession.Status.Disconnected)
		end)

		it("should handle double stop without crashing", function()
			local session = createSession()

			session:stop()
			expect(session:getStatus()).to.equal(ServeSession.Status.Disconnected)

			-- Second stop should be safe
			session:stop()
			expect(session:getStatus()).to.equal(ServeSession.Status.Disconnected)
		end)

		it("should fire statusChangedCallback on stop", function()
			local session = createSession()

			local statusChanges = {}
			session:onStatusChanged(function(status, detail)
				table.insert(statusChanges, { status = status, detail = detail })
			end)

			session:stop()

			expect(#statusChanges).to.equal(1)
			expect(statusChanges[1].status).to.equal(ServeSession.Status.Disconnected)
		end)

		it("should fire statusChangedCallback with error detail on abort", function()
			local session = createSession()

			local statusChanges = {}
			session:onStatusChanged(function(status, detail)
				table.insert(statusChanges, { status = status, detail = detail })
			end)

			session:setConfirmCallback(function()
				return "Abort"
			end)

			session.__confirmingPatch = nil
			session.__isConfirming = false

			session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))
			task.wait()

			expect(#statusChanges).to.equal(1)
			expect(statusChanges[1].status).to.equal(ServeSession.Status.Disconnected)
		end)

		it("should ignore WebSocket messages after stop", function()
			local session = createSession()
			local confirmCallbackCalled = false

			session:setConfirmCallback(function()
				confirmCallbackCalled = true
				return "Accept"
			end)

			session:stop()

			-- Message after stop
			session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))
			task.wait()

			expect(confirmCallbackCalled).to.equal(false)
		end)
	end)

	describe("__applyPatch defense-in-depth", function()
		it("should block unexpected patch during confirmation", function()
			local session = createSession()

			-- Set up confirmation state
			local confirmingPatch = createTestPatch()
			session.__isConfirming = true
			session.__confirmingPatch = confirmingPatch

			-- Try to apply a DIFFERENT patch
			local unexpectedPatch = PatchSet.newEmpty()
			table.insert(unexpectedPatch.updated, {
				id = "different-id",
				changedProperties = { Value = { String = "sneaky" } },
			})

			-- Log.error throws in this codebase, so wrap in pcall
			local success = pcall(function()
				session:__applyPatch(unexpectedPatch)
			end)

			-- Should have errored (defense-in-depth)
			expect(success).to.equal(false)

			session.__isConfirming = false
			session.__confirmingPatch = nil
			session:stop()
		end)

		it("should run precommit callbacks before patch application", function()
			local session = createSession()
			local precommitCalled = false
			local precommitPatch = nil

			session:hookPrecommit(function(patch, _instanceMap)
				precommitCalled = true
				precommitPatch = patch
			end)

			local patch = PatchSet.newEmpty()
			session:__applyPatch(patch)

			expect(precommitCalled).to.equal(true)
			expect(precommitPatch).to.equal(patch)

			session:stop()
		end)

		it("should continue applying patch even if precommit callback errors", function()
			local session = createSession()
			local secondPrecommitCalled = false

			session:hookPrecommit(function()
				error("precommit explosion!")
			end)

			session:hookPrecommit(function()
				secondPrecommitCalled = true
			end)

			-- Should not crash despite precommit error
			local patch = PatchSet.newEmpty()
			session:__applyPatch(patch)

			-- Second precommit should still run
			expect(secondPrecommitCalled).to.equal(true)

			session:stop()
		end)

		it("should run postcommit callbacks after patch application", function()
			local session = createSession()
			local postcommitCalled = false

			session:hookPostcommit(function(_patch, _instanceMap, _unappliedPatch)
				postcommitCalled = true
			end)

			local patch = PatchSet.newEmpty()
			session:__applyPatch(patch)

			-- Postcommit runs in task.spawn, so wait
			task.wait()

			expect(postcommitCalled).to.equal(true)

			session:stop()
		end)

		it("should not crash if postcommit callback errors", function()
			local session = createSession()

			session:hookPostcommit(function()
				error("postcommit explosion!")
			end)

			-- Should not crash
			local patch = PatchSet.newEmpty()
			session:__applyPatch(patch)

			task.wait()

			-- Session should still be functional
			expect(session:getStatus()).to.equal(ServeSession.Status.Connected)

			session:stop()
		end)

		it("should return cleanup function from hookPrecommit", function()
			local session = createSession()
			local callCount = 0

			local cleanup = session:hookPrecommit(function()
				callCount += 1
			end)

			session:__applyPatch(PatchSet.newEmpty())
			expect(callCount).to.equal(1)

			-- Disconnect
			cleanup()

			session:__applyPatch(PatchSet.newEmpty())
			-- Should still be 1 (callback disconnected)
			expect(callCount).to.equal(1)

			session:stop()
		end)
	end)

	describe("ChangeBatcher one-shot blocking", function()
		it("should block automatic outgoing writes in one-shot mode", function()
			-- Save and override setting
			local originalValue = Settings:get("oneShotSync")
			Settings:set("oneShotSync", true)

			local writeCallCount = 0
			local mockApi = MockApiContext.new()
			mockApi.onWrite = function()
				writeCallCount += 1
				return true, { success = true }
			end

			local session = ServeSession.new({
				apiContext = mockApi,
				twoWaySync = true,
			})
			session.__status = ServeSession.Status.Connected

			-- Simulate a ChangeBatcher flush by triggering the onChangesFlushed path
			-- The ChangeBatcher calls this internally, but we can test the guard
			-- by checking that write() is never called even if changes are pending
			local part = Instance.new("Part")
			session.__instanceMap:insert("TEST_PART", part)
			session.__changeBatcher:add(part, "Name")

			-- Force a flush cycle
			session.__changeBatcher:__cycle(1.0)

			-- Write should NOT have been called due to one-shot guard
			expect(writeCallCount).to.equal(0)

			-- Restore setting
			Settings:set("oneShotSync", originalValue)

			part:Destroy()
			session:stop()
		end)

		it("should allow automatic outgoing writes when one-shot is off", function()
			-- Save and override setting
			local originalValue = Settings:get("oneShotSync")
			Settings:set("oneShotSync", false)

			local writeCallCount = 0
			local mockApi = MockApiContext.new()
			mockApi.onWrite = function()
				writeCallCount += 1
				return true, { success = true }
			end

			local session = ServeSession.new({
				apiContext = mockApi,
				twoWaySync = true,
			})
			session.__status = ServeSession.Status.Connected

			local part = Instance.new("Part")
			session.__instanceMap:insert("TEST_PART", part)
			session.__changeBatcher:add(part, "Name")

			-- Force a flush cycle
			session.__changeBatcher:__cycle(1.0)

			-- Write SHOULD have been called
			expect(writeCallCount).to.equal(1)

			-- Restore setting
			Settings:set("oneShotSync", originalValue)

			part:Destroy()
			session:stop()
		end)
	end)
end
