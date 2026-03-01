--!strict

local strict = require(script.Parent.strict)

local tools: { [string]: (any) -> string? } = {
	run_code = require(script.RunCode),
	insert_model = require(script.InsertModel),
	get_console_output = require(script.GetConsoleOutput),
	get_studio_mode = require(script.GetStudioMode),
	start_stop_play = require(script.StartStopPlay),
	run_script_in_play_mode = require(script.RunScriptInPlayMode),
}

local McpTools = {}

function McpTools.dispatch(toolType: string, args: any): string?
	local handler = tools[toolType]
	if not handler then
		error("Unknown MCP tool: " .. tostring(toolType))
	end
	return handler(args)
end

return strict("McpTools", {
	dispatch = McpTools.dispatch,
})
