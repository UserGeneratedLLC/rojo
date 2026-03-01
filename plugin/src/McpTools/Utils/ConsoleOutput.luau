--!strict

local LogService = game:GetService("LogService")

local MAX_OUTPUT = 10000

local ConsoleOutput = {
	outputMessage = "",
	onLogMessage = nil :: ((message: string, messageType: Enum.MessageType) -> ())?,
}

function ConsoleOutput.startListener(): RBXScriptConnection
	local conn = LogService.MessageOut:Connect(function(message: string, messageType: Enum.MessageType)
		if #ConsoleOutput.outputMessage + #message <= MAX_OUTPUT then
			if #ConsoleOutput.outputMessage == 0 then
				ConsoleOutput.outputMessage = message
			else
				ConsoleOutput.outputMessage = ConsoleOutput.outputMessage .. "\n" .. message
			end
		end

		local callback = ConsoleOutput.onLogMessage
		if callback then
			callback(message, messageType)
		end
	end)
	return conn
end

return ConsoleOutput
