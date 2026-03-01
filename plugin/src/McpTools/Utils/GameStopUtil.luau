--!strict

local StudioTestService = game:GetService("StudioTestService")

local MCP_STOP_PLAY_KEY = "Atlas_MCP_STOP_PLAY"

local GameStopUtil = {}

local _plugin: Plugin? = nil

function GameStopUtil.setPlugin(p: Plugin)
	_plugin = p
end

function GameStopUtil.stopPlay()
	assert(_plugin, "GameStopUtil: plugin not set")
	_plugin:SetSetting(MCP_STOP_PLAY_KEY, true)
end

function GameStopUtil.monitorForStopPlay()
	assert(_plugin, "GameStopUtil: plugin not set")
	_plugin:SetSetting(MCP_STOP_PLAY_KEY, false)
	while true do
		if _plugin:GetSetting(MCP_STOP_PLAY_KEY) then
			_plugin:SetSetting(MCP_STOP_PLAY_KEY, false)
			StudioTestService:EndTest({})
		end
		task.wait(1)
	end
end

return GameStopUtil
