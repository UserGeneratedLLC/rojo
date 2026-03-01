--!strict

local StudioModeState = require(script.Parent.Utils.StudioModeState)

local function handleGetStudioMode(): string?
	return StudioModeState.studioMode
end

return handleGetStudioMode
