if not plugin then
	return
end

local RunService = game:GetService("RunService")

local Rojo = script:FindFirstAncestor("Rojo")
local Packages = Rojo.Packages

local Log = require(Packages.Log)
local Roact = require(Packages.Roact)

local Settings = require(script.Settings)
local Config = require(script.Config)
local App = require(script.App)

local ConsoleOutput = require(script.McpTools.Utils.ConsoleOutput)
local GameStopUtil = require(script.McpTools.Utils.GameStopUtil)

GameStopUtil.setPlugin(plugin)

Log.setLogLevelThunk(function()
	return Log.Level[Settings:get("logLevel")] or Log.Level.Info
end)

if RunService:IsRunning() and RunService:IsServer() then
	task.spawn(GameStopUtil.monitorForStopPlay)
end

local consoleOutputConn = ConsoleOutput.startListener()

local app = Roact.createElement(App, {
	plugin = plugin,
})
local tree = Roact.mount(app, game:GetService("CoreGui"), "Atlas UI")

plugin.Unloading:Connect(function()
	Roact.unmount(tree)
	consoleOutputConn:Disconnect()
end)

if Config.isDevBuild then
	local TestEZ = require(script.Parent.TestEZ)

	require(script.runTests)(TestEZ)
end
