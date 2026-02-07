--[[
	Tests for the confirmation behavior settings.

	These replicate the confirm callback pattern from App/init.lua to verify
	that each confirmationBehavior setting correctly auto-accepts or shows
	the confirmation dialog under the right conditions.
]]

return function()
	local PatchSet = require(script.Parent.Parent.PatchSet)
	local Settings = require(script.Parent.Parent.Settings)

	-- Saved settings for restoration
	local savedSettings = {}

	beforeEach(function()
		savedSettings.oneShotSync = Settings:get("oneShotSync")
		savedSettings.confirmationBehavior = Settings:get("confirmationBehavior")
		savedSettings.largeChangesConfirmationThreshold = Settings:get("largeChangesConfirmationThreshold")
	end)

	afterEach(function()
		Settings:set("oneShotSync", savedSettings.oneShotSync)
		Settings:set("confirmationBehavior", savedSettings.confirmationBehavior)
		Settings:set("largeChangesConfirmationThreshold", savedSettings.largeChangesConfirmationThreshold)
	end)

	local function createTestPatch(instanceCount)
		instanceCount = instanceCount or 1
		local patch = PatchSet.newEmpty()
		for i = 1, instanceCount do
			table.insert(patch.updated, {
				id = "id-" .. tostring(i),
				changedProperties = { Value = { String = "val-" .. tostring(i) } },
			})
		end
		return patch
	end

	--[[
		Build a confirm callback that mirrors the logic in App/init.lua lines 719-830.
		Returns the callback function and a state table to inspect results.
	]]
	local function buildConfirmCallback(options)
		options = options or {}
		local state = {
			dialogShown = false,
			knownProjects = options.knownProjects or {},
			-- Simulate auto-connect playtest
			isAutoConnectPlaytestServer = options.isAutoConnectPlaytestServer or false,
		}

		local initialSyncConfirmed = false

		local function confirmCallback(_instanceMap, patch, serverInfo)
			-- PatchSet.removeDataModelName would be called here with instanceMap,
			-- but we skip it in tests since it's not what we're testing

			local isOneShotMode = Settings:get("oneShotSync")

			if isOneShotMode and initialSyncConfirmed then
				return "Skip"
			end

			if not isOneShotMode then
				if PatchSet.isEmpty(patch) then
					return "Accept"
				end

				if state.isAutoConnectPlaytestServer then
					return "Accept"
				end

				local confirmationBehavior = Settings:get("confirmationBehavior")
				if confirmationBehavior ~= "Always" then
					if confirmationBehavior == "Initial" then
						if state.knownProjects[serverInfo.projectName] then
							return "Accept"
						end
					elseif confirmationBehavior == "Large Changes" then
						local threshold = Settings:get("largeChangesConfirmationThreshold")
						if PatchSet.countInstances(patch) < threshold then
							return "Accept"
						end
					elseif confirmationBehavior == "Unlisted PlaceId" then
						if serverInfo.expectedPlaceIds then
							for _, placeId in serverInfo.expectedPlaceIds do
								if placeId == (options.currentPlaceId or 0) then
									return "Accept"
								end
							end
						end
					elseif confirmationBehavior == "Never" then
						return "Accept"
					end
				end
			end

			-- Would show dialog here
			state.dialogShown = true

			if isOneShotMode then
				initialSyncConfirmed = true
			end

			-- Mark project as known after first dialog
			if serverInfo and serverInfo.projectName then
				state.knownProjects[serverInfo.projectName] = true
			end

			return { type = "Confirm", selections = {} }
		end

		return confirmCallback, state
	end

	local defaultServerInfo = {
		projectName = "TestProject",
		sessionId = "test-id",
	}

	describe("Always", function()
		it("should show dialog for non-empty patch", function()
			Settings:set("oneShotSync", false)
			Settings:set("confirmationBehavior", "Always")

			local callback, state = buildConfirmCallback()
			callback(nil, createTestPatch(), defaultServerInfo)

			expect(state.dialogShown).to.equal(true)
		end)

		it("should auto-accept empty patches even with Always setting", function()
			Settings:set("oneShotSync", false)
			Settings:set("confirmationBehavior", "Always")

			local callback, state = buildConfirmCallback()
			local result = callback(nil, PatchSet.newEmpty(), defaultServerInfo)

			expect(state.dialogShown).to.equal(false)
			expect(result).to.equal("Accept")
		end)
	end)

	describe("Never", function()
		it("should auto-accept all patches without showing dialog", function()
			Settings:set("oneShotSync", false)
			Settings:set("confirmationBehavior", "Never")

			local callback, state = buildConfirmCallback()
			local result = callback(nil, createTestPatch(10), defaultServerInfo)

			expect(state.dialogShown).to.equal(false)
			expect(result).to.equal("Accept")
		end)
	end)

	describe("Initial", function()
		it("should show dialog on first connection to a project", function()
			Settings:set("oneShotSync", false)
			Settings:set("confirmationBehavior", "Initial")

			local callback, state = buildConfirmCallback()
			callback(nil, createTestPatch(), defaultServerInfo)

			expect(state.dialogShown).to.equal(true)
		end)

		it("should auto-accept on subsequent connections to the same project", function()
			Settings:set("oneShotSync", false)
			Settings:set("confirmationBehavior", "Initial")

			local callback, state = buildConfirmCallback()

			-- First connection: shows dialog
			callback(nil, createTestPatch(), defaultServerInfo)
			expect(state.dialogShown).to.equal(true)

			-- Reset dialog state
			state.dialogShown = false

			-- Second connection: auto-accepts
			local result = callback(nil, createTestPatch(), defaultServerInfo)
			expect(state.dialogShown).to.equal(false)
			expect(result).to.equal("Accept")
		end)

		it("should show dialog for a different project", function()
			Settings:set("oneShotSync", false)
			Settings:set("confirmationBehavior", "Initial")

			local callback, state = buildConfirmCallback()

			-- First project
			callback(nil, createTestPatch(), defaultServerInfo)
			state.dialogShown = false

			-- Different project: should show dialog
			callback(nil, createTestPatch(), {
				projectName = "DifferentProject",
				sessionId = "different-id",
			})
			expect(state.dialogShown).to.equal(true)
		end)

		it("should auto-accept already known project", function()
			Settings:set("oneShotSync", false)
			Settings:set("confirmationBehavior", "Initial")

			local callback, state = buildConfirmCallback({
				knownProjects = { TestProject = true },
			})

			local result = callback(nil, createTestPatch(), defaultServerInfo)
			expect(state.dialogShown).to.equal(false)
			expect(result).to.equal("Accept")
		end)
	end)

	describe("Large Changes", function()
		it("should auto-accept small patches below threshold", function()
			Settings:set("oneShotSync", false)
			Settings:set("confirmationBehavior", "Large Changes")
			Settings:set("largeChangesConfirmationThreshold", 5)

			local callback, state = buildConfirmCallback()
			local result = callback(nil, createTestPatch(3), defaultServerInfo)

			expect(state.dialogShown).to.equal(false)
			expect(result).to.equal("Accept")
		end)

		it("should show dialog for large patches at or above threshold", function()
			Settings:set("oneShotSync", false)
			Settings:set("confirmationBehavior", "Large Changes")
			Settings:set("largeChangesConfirmationThreshold", 5)

			local callback, state = buildConfirmCallback()
			callback(nil, createTestPatch(5), defaultServerInfo)

			expect(state.dialogShown).to.equal(true)
		end)

		it("should show dialog for patches above threshold", function()
			Settings:set("oneShotSync", false)
			Settings:set("confirmationBehavior", "Large Changes")
			Settings:set("largeChangesConfirmationThreshold", 5)

			local callback, state = buildConfirmCallback()
			callback(nil, createTestPatch(10), defaultServerInfo)

			expect(state.dialogShown).to.equal(true)
		end)
	end)

	describe("Unlisted PlaceId", function()
		it("should auto-accept when placeId is in the allowed list", function()
			Settings:set("oneShotSync", false)
			Settings:set("confirmationBehavior", "Unlisted PlaceId")

			local callback, state = buildConfirmCallback({ currentPlaceId = 12345 })

			local result = callback(nil, createTestPatch(), {
				projectName = "Test",
				sessionId = "test",
				expectedPlaceIds = { 12345, 67890 },
			})

			expect(state.dialogShown).to.equal(false)
			expect(result).to.equal("Accept")
		end)

		it("should show dialog when placeId is not in the allowed list", function()
			Settings:set("oneShotSync", false)
			Settings:set("confirmationBehavior", "Unlisted PlaceId")

			local callback, state = buildConfirmCallback({ currentPlaceId = 99999 })

			callback(nil, createTestPatch(), {
				projectName = "Test",
				sessionId = "test",
				expectedPlaceIds = { 12345, 67890 },
			})

			expect(state.dialogShown).to.equal(true)
		end)

		it("should show dialog when no expectedPlaceIds are provided", function()
			Settings:set("oneShotSync", false)
			Settings:set("confirmationBehavior", "Unlisted PlaceId")

			local callback, state = buildConfirmCallback({ currentPlaceId = 12345 })

			callback(nil, createTestPatch(), {
				projectName = "Test",
				sessionId = "test",
			})

			expect(state.dialogShown).to.equal(true)
		end)
	end)

	describe("play solo auto-connect", function()
		it("should auto-accept regardless of confirmationBehavior setting", function()
			Settings:set("oneShotSync", false)
			Settings:set("confirmationBehavior", "Always")

			local callback, state = buildConfirmCallback({
				isAutoConnectPlaytestServer = true,
			})

			local result = callback(nil, createTestPatch(20), defaultServerInfo)

			expect(state.dialogShown).to.equal(false)
			expect(result).to.equal("Accept")
		end)
	end)

	describe("one-shot mode overrides", function()
		it("should always show dialog in one-shot mode regardless of confirmationBehavior", function()
			Settings:set("oneShotSync", true)
			Settings:set("confirmationBehavior", "Never")

			local callback, state = buildConfirmCallback()
			callback(nil, createTestPatch(), defaultServerInfo)

			-- Even with "Never" setting, one-shot mode forces dialog
			expect(state.dialogShown).to.equal(true)
		end)

		it("should show dialog for empty patches in one-shot mode", function()
			Settings:set("oneShotSync", true)
			Settings:set("confirmationBehavior", "Never")

			local callback, state = buildConfirmCallback()
			callback(nil, PatchSet.newEmpty(), defaultServerInfo)

			-- One-shot mode shows dialog even for empty patches
			expect(state.dialogShown).to.equal(true)
		end)

		it("should skip after initial sync in one-shot mode", function()
			Settings:set("oneShotSync", true)

			local callback, state = buildConfirmCallback()

			-- First call: dialog shown
			callback(nil, createTestPatch(), defaultServerInfo)
			expect(state.dialogShown).to.equal(true)

			-- Second call: skipped
			local result = callback(nil, createTestPatch(), defaultServerInfo)
			expect(result).to.equal("Skip")
		end)
	end)
end
