--[[
	Integration tests for one-shot sync mode.

	Tests the full one-shot lifecycle: confirm → apply → echo skip → disconnect.
	Uses ServeSession with MockApiContext and simulated confirm callbacks
	that replicate the App/init.lua pattern.
]]

return function()
	local ServeSession = require(script.Parent.Parent.ServeSession)
	local PatchSet = require(script.Parent.Parent.PatchSet)
	local Settings = require(script.Parent.Parent.Settings)
	local testUtils = require(script.Parent.Parent.testUtils)
	local MockApiContext = testUtils.MockApiContext

	local originalOneShotSync

	beforeEach(function()
		originalOneShotSync = Settings:get("oneShotSync")
		Settings:set("oneShotSync", true)
	end)

	afterEach(function()
		Settings:set("oneShotSync", originalOneShotSync)
	end)

	local function createMessagesPacket(patch, cursor)
		return {
			messageCursor = cursor or 1,
			messages = { patch },
		}
	end

	local function createTestPatch(id, value)
		local patch = PatchSet.newEmpty()
		table.insert(patch.updated, {
			id = id or "test-id",
			changedProperties = {
				Value = { String = value or "changed" },
			},
		})
		return patch
	end

	local function createOneShotSession()
		local mockApi = MockApiContext.new()
		local session = ServeSession.new({
			apiContext = mockApi,
			twoWaySync = true,
		})
		session.__status = ServeSession.Status.Connected
		session.__serverInfo = {
			projectName = "OneShotTest",
			sessionId = "one-shot-session-id",
		}
		return session, mockApi
	end

	describe("echo isolation", function()
		it("should skip all echo patches after initial sync confirmation", function()
			local session = createOneShotSession()
			local applyCount = 0

			function session:__applyPatch()
				applyCount += 1
			end

			local initialSyncConfirmed = false
			session:setConfirmCallback(function()
				if initialSyncConfirmed then
					return "Skip"
				end
				initialSyncConfirmed = true
				return "Accept"
			end)

			session.__confirmingPatch = nil
			session.__isConfirming = false

			-- Initial sync
			session:__onWebSocketMessage(createMessagesPacket(createTestPatch("id-1")))
			task.wait()
			expect(applyCount).to.equal(1)

			-- Echo patches (3 of them)
			for i = 2, 4 do
				session:__onWebSocketMessage(createMessagesPacket(createTestPatch("echo-" .. tostring(i))))
				task.wait()
			end

			-- Still only 1 apply
			expect(applyCount).to.equal(1)

			session:stop()
		end)

		it("should not send any echo patches to the server via write", function()
			local session, mockApi = createOneShotSession()

			function session:__applyPatch() end

			local initialSyncConfirmed = false
			session:setConfirmCallback(function()
				if initialSyncConfirmed then
					return "Skip"
				end
				initialSyncConfirmed = true
				return "Accept"
			end)

			session.__confirmingPatch = nil
			session.__isConfirming = false

			-- Initial sync
			session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))
			task.wait()

			-- Multiple echoes
			for _ = 1, 5 do
				session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))
				task.wait()
			end

			-- No write calls to server
			local writeRequests = mockApi:getRequestsByMethod("write")
			expect(#writeRequests).to.equal(0)

			session:stop()
		end)
	end)

	describe("confirm callback flag timing", function()
		it("should set initialSyncConfirmed before __confirmingPatch is cleared", function()
			local session = createOneShotSession()

			function session:__applyPatch() end

			-- Track the exact order of flag changes
			local flagSetBeforePatchCleared = false
			local initialSyncConfirmed = false

			session:setConfirmCallback(function()
				if initialSyncConfirmed then
					return "Skip"
				end
				-- This simulates what happens in App/init.lua:
				-- initialSyncConfirmed is set BEFORE return, and
				-- __confirmingPatch is cleared AFTER return (in ServeSession)
				initialSyncConfirmed = true
				-- At this point __confirmingPatch is still non-nil (set by __onWebSocketMessage)
				flagSetBeforePatchCleared = (session.__confirmingPatch ~= nil)
				return "Accept"
			end)

			session.__confirmingPatch = nil
			session.__isConfirming = false

			session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))
			task.wait()

			-- The flag was set while __confirmingPatch was still active
			expect(flagSetBeforePatchCleared).to.equal(true)
			-- And now both states are consistent
			expect(initialSyncConfirmed).to.equal(true)
			expect(session.__confirmingPatch).to.equal(nil)

			session:stop()
		end)
	end)

	describe("abort path", function()
		it("should disconnect session on abort with no writes", function()
			local session, mockApi = createOneShotSession()

			function session:__applyPatch() end

			session:setConfirmCallback(function()
				return "Abort"
			end)

			session.__confirmingPatch = nil
			session.__isConfirming = false

			session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))
			task.wait()

			-- Session should be disconnected
			expect(session:getStatus()).to.equal(ServeSession.Status.Disconnected)

			-- No writes to server
			local writeRequests = mockApi:getRequestsByMethod("write")
			expect(#writeRequests).to.equal(0)
		end)
	end)

	describe("ChangeBatcher blocked throughout lifecycle", function()
		it("should block automatic writes at every stage of one-shot sync", function()
			local session, mockApi = createOneShotSession()

			function session:__applyPatch() end

			local writeCount = 0
			mockApi.onWrite = function()
				writeCount += 1
				return true, { success = true }
			end

			local initialSyncConfirmed = false
			session:setConfirmCallback(function()
				if initialSyncConfirmed then
					return "Skip"
				end
				initialSyncConfirmed = true
				return "Accept"
			end)

			session.__confirmingPatch = nil
			session.__isConfirming = false

			-- Add a tracked instance
			local part = Instance.new("Part")
			session.__instanceMap:insert("TEST_PART", part)

			-- Stage 1: Before initial sync
			session.__changeBatcher:add(part, "Name")
			session.__changeBatcher:__cycle(1.0)
			expect(writeCount).to.equal(0)

			-- Stage 2: During initial sync confirmation (batcher paused)
			session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))
			task.wait()

			-- Stage 3: After initial sync, echo patches arrive
			session.__changeBatcher:add(part, "Position")
			session.__changeBatcher:__cycle(1.0)
			expect(writeCount).to.equal(0)

			-- Stage 4: After echo skip
			session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))
			task.wait()
			session.__changeBatcher:add(part, "Size")
			session.__changeBatcher:__cycle(1.0)
			expect(writeCount).to.equal(0)

			part:Destroy()
			session:stop()
		end)
	end)

	describe("post-disconnect safety", function()
		it("should ignore messages after session stop", function()
			local session = createOneShotSession()
			local confirmCallbackCalled = false

			session:setConfirmCallback(function()
				confirmCallbackCalled = true
				return "Accept"
			end)

			session:stop()

			-- Message after disconnect
			session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))
			task.wait()

			expect(confirmCallbackCalled).to.equal(false)
		end)

		it("should ignore messages after one-shot lifecycle completes", function()
			local session = createOneShotSession()
			local applyCount = 0

			function session:__applyPatch()
				applyCount += 1
			end

			local initialSyncConfirmed = false
			session:setConfirmCallback(function()
				if initialSyncConfirmed then
					return "Skip"
				end
				initialSyncConfirmed = true
				return "Accept"
			end)

			session.__confirmingPatch = nil
			session.__isConfirming = false

			-- Complete initial sync
			session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))
			task.wait()
			expect(applyCount).to.equal(1)

			-- Simulate endSession (what App does)
			session:stop()

			-- Any further messages should be ignored
			session:__onWebSocketMessage(createMessagesPacket(createTestPatch()))
			task.wait()

			expect(applyCount).to.equal(1)
		end)
	end)

	describe("merge during confirmation", function()
		it("should merge WebSocket messages into confirming patch during review", function()
			local session = createOneShotSession()

			-- Set up confirmation in progress
			local confirmingPatch = createTestPatch("original-id", "original")
			session.__confirmingPatch = confirmingPatch
			session.__isConfirming = true

			local patchUpdateCount = 0
			session:setPatchUpdateCallback(function()
				patchUpdateCount += 1
			end)

			-- Send 3 messages during review
			for i = 1, 3 do
				session:__onWebSocketMessage(
					createMessagesPacket(createTestPatch("merge-" .. tostring(i), "val-" .. tostring(i)))
				)
			end

			-- All should have been merged (original + 3 new = 4 updates)
			expect(#confirmingPatch.updated).to.equal(4)
			expect(patchUpdateCount).to.equal(3)

			session:stop()
		end)
	end)
end
