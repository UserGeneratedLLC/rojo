--[[
	Utility for detecting whitespace-only changes in source code.
]]

local WhitespaceUtil = {}

local function SplitLines(str: string): { string }
	local lines = {}
	for line in string.gmatch(str .. "\n", "([^\n]*)\n") do
		table.insert(lines, line)
	end
	return lines
end

local function NormalizeLine(line: string): string
	return string.gsub(string.gsub(line, "\r+$", ""), "[ \t]+$", "")
end

-- Counts lines that differ between two strings (capped at 100)
-- Returns: totalDiff, whitespaceDiff (lines that only differ in whitespace)
-- Returns 10000, 10000 if exceeds 100 (frontend displays as "100+")
function WhitespaceUtil.CountLineDifferences(current: string, incoming: string): (number, number)
	if type(current) ~= "string" or type(incoming) ~= "string" then
		return 0, 0
	end

	local currentLines = SplitLines(current)
	local incomingLines = SplitLines(incoming)
	local maxLines = math.max(#currentLines, #incomingLines)

	local totalDiff, whitespaceDiff = 0, 0
	for i = 1, maxLines do
		local currentLine = currentLines[i] or ""
		local incomingLine = incomingLines[i] or ""

		if currentLine ~= incomingLine then
			totalDiff += 1
			if NormalizeLine(currentLine) == NormalizeLine(incomingLine) then
				whitespaceDiff += 1
			end
			if totalDiff > 100 then
				return 10000, 10000
			end
		end
	end

	return totalDiff, whitespaceDiff
end

return WhitespaceUtil
