--!strict
--!nolint DeprecatedApi

local DeepCopy = require(script.Parent.Utils.DeepCopy)

type Args = {
	command: string,
}

local function valueToString(value: any): string
	local t = type(value)
	if t == "number" then
		if value ~= value then
			return "NaN"
		elseif value == math.huge then
			return "Infinity"
		elseif value == -math.huge then
			return "-Infinity"
		end
		return tostring(value)
	elseif t == "nil" then
		return "null"
	elseif t == "string" then
		return string.format("%q", value)
	elseif t == "boolean" then
		return tostring(value)
	elseif t == "table" then
		local parts: { string } = {}
		local isArray = #value > 0
		if isArray then
			for _, v in value :: { any } do
				table.insert(parts, valueToString(v))
			end
			return "[" .. table.concat(parts, ", ") .. "]"
		else
			for k, v in value :: { [any]: any } do
				table.insert(parts, valueToString(k) .. ": " .. valueToString(v))
			end
			return "{" .. table.concat(parts, ", ") .. "}"
		end
	end
	return tostring(value)
end

local function runCodeWithOutput(command: string): string
	local output = ""

	local function serializeTable(arg: any): any
		if typeof(arg) == "table" then
			local hasTable = false
			for key, _ in arg do
				if typeof(key) ~= "number" then
					hasTable = true
					break
				end
			end

			local newArg = {}
			for key, value in arg do
				local newKey = serializeTable(key)
				newArg[if hasTable then tostring(newKey) else newKey] = serializeTable(value)
			end
			return newArg
		elseif type(arg) == "userdata" then
			return tostring(arg) or "UNABLE_TO_SERIALIZE"
		end
		return arg
	end

	local function toStrTable(t: { any }): { string }
		local clonedTable = DeepCopy(t)
		local strTable = table.create(#clonedTable)
		for i, arg in clonedTable do
			local serializedArg = serializeTable(arg)
			strTable[i] = valueToString(serializedArg)
		end
		return strTable
	end

	local function addToOutput(header: string, ...)
		local strResults = toStrTable(table.pack(...))
		output ..= header .. " " .. table.concat(strResults, "\t") .. "\n"
	end

	local chunk, syntaxErr = loadstring(command)
	if not chunk then
		addToOutput("[SYNTAX ERROR]", syntaxErr or "Unknown syntax error")
		return output
	end

	local chunkfenv = getfenv(chunk :: any)

	local savedPrint = chunkfenv.print
	local savedWarn = chunkfenv.warn
	local savedError = chunkfenv.error

	local errorCaptured = false

	chunkfenv.print = function(...)
		savedPrint(...)
		addToOutput("[OUTPUT]", ...)
	end

	chunkfenv.warn = function(...)
		savedWarn(...)
		addToOutput("[WARNING]", ...)
	end

	chunkfenv.error = function(message: any, level: number?)
		errorCaptured = true
		addToOutput("[ERROR]", message)
		savedError(message, (level or 1) + 1)
	end

	local ok, resultOrErr = pcall(function()
		return table.pack((chunk :: any)())
	end)

	chunkfenv.print = savedPrint
	chunkfenv.warn = savedWarn
	chunkfenv.error = savedError

	if ok then
		local results = resultOrErr :: { any }
		if #results > 0 then
			addToOutput("[RETURNED RESULTS]", table.unpack(results))
		end
	elseif not errorCaptured then
		addToOutput("[UNEXPECTED ERROR]", resultOrErr)
	end

	return output
end

local ConsoleOutput = require(script.Parent.Utils.ConsoleOutput)

local function handleRunCode(args: Args): string?
	assert(type(args.command) == "string", "Missing command in run_code")

	ConsoleOutput.outputMessage = ""
	return runCodeWithOutput(args.command)
end

return handleRunCode
