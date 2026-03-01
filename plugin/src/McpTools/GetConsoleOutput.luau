--!strict

local ConsoleOutput = require(script.Parent.Utils.ConsoleOutput)

local function handleGetConsoleOutput(): string?
	local result = ConsoleOutput.outputMessage or ""
	return result
end

return handleGetConsoleOutput
