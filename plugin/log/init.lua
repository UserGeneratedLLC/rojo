local Fmt = require(script.Parent.Fmt)

local Level = {
	Error = 0,
	Warning = 1,
	Info = 2,
	Debug = 3,
	Trace = 4,
}

local function getLogLevel()
	return Level.Info
end

local function addTags(tag, message)
	return tag .. message
end

local TRACE_TAG = "[Rojo-Trace] "
local INFO_TAG = "[Rojo-Info] "
local DEBUG_TAG = "[Rojo-Debug] "
local WARN_TAG = "[Rojo-Warn] "

local Log = {}

Log.Level = Level

function Log.setLogLevelThunk(thunk)
	getLogLevel = thunk
end

function Log.trace(template, ...)
	if getLogLevel() >= Level.Trace then
		print(addTags(TRACE_TAG, Fmt.fmt(template, ...)))
	end
end

function Log.info(template, ...)
	if getLogLevel() >= Level.Info then
		print(addTags(INFO_TAG, Fmt.fmt(template, ...)))
	end
end

function Log.debug(template, ...)
	if getLogLevel() >= Level.Debug then
		print(addTags(DEBUG_TAG, Fmt.fmt(template, ...)))
	end
end

function Log.warn(template, ...)
	if getLogLevel() >= Level.Warning then
		warn(addTags(WARN_TAG, Fmt.fmt(template, ...)))
	end
end

function Log.error(template, ...)
	error(Fmt.fmt(template, ...))
end

return Log
