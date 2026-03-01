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

	local function executeCode(): { any }
		local chunk = loadstring(command) :: any
		local chunkfenv = getfenv(chunk)

		local oldPrint = print
		chunkfenv.print = function(...)
			oldPrint(...)
			addToOutput("[OUTPUT]", ...)
		end

		local oldWarn = warn
		chunkfenv.warn = function(...)
			oldWarn(...)
			addToOutput("[WARNING]", ...)
		end

		local oldError = error
		chunkfenv.error = function(...)
			oldError(...)
			addToOutput("[ERROR]", ...)
		end

		local results = table.pack(chunk())
		if #results > 0 then
			addToOutput("[RETURNED RESULTS]", table.unpack(results))
		end

		return results
	end

	local ok, errorMessage = pcall(executeCode)
	if not ok then
		addToOutput("[UNEXPECTED ERROR]", errorMessage)
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
