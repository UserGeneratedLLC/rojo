local ReplicatedStorage = game:GetService("ReplicatedStorage")

local Packages = ReplicatedStorage:WaitForChild("Packages", 10)
local TestEZ = require(Packages:WaitForChild("TestEZ", 10))

local Rojo = ReplicatedStorage:WaitForChild("Rojo", 10)

local Settings = require(Rojo.Plugin.Settings)
Settings:set("logLevel", "Trace")
Settings:set("typecheckingEnabled", true)

require(Rojo.Plugin.runTests)(TestEZ)
