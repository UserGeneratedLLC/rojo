--[[
	Test utilities for Rojo plugin stress testing.
	
	This module exports utilities for generating large instance trees,
	creating patches, and mocking the API context.
]]

local LargeTreeGenerator = require(script.LargeTreeGenerator)
local PatchGenerator = require(script.PatchGenerator)
local MockApiContext = require(script.MockApiContext)

return {
	LargeTreeGenerator = LargeTreeGenerator,
	PatchGenerator = PatchGenerator,
	MockApiContext = MockApiContext,
}
