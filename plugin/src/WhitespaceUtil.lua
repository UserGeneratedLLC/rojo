--[[
	Utility for detecting whitespace-only changes in source code.
	Uses LCS (Longest Common Subsequence) based diff algorithm for accurate
	change detection that properly handles insertions and deletions.
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

-- Computes the length of the Longest Common Subsequence between two line arrays
-- If normalize is true, lines are compared after whitespace normalization
local function ComputeLcsLength(linesA: { string }, linesB: { string }, normalize: boolean): number
	local m, n = #linesA, #linesB
	if m == 0 or n == 0 then
		return 0
	end

	-- Precompute comparison lines (normalized if requested)
	local compareA, compareB = linesA, linesB
	if normalize then
		compareA = table.create(m)
		compareB = table.create(n)
		for i = 1, m do
			compareA[i] = NormalizeLine(linesA[i])
		end
		for j = 1, n do
			compareB[j] = NormalizeLine(linesB[j])
		end
	end

	-- Space-optimized DP for LCS
	-- prev[j+1] = LCS length for linesA[1..i-1] and linesB[1..j]
	-- curr[j+1] = LCS length for linesA[1..i] and linesB[1..j]
	local prev = table.create(n + 1, 0)
	local curr = table.create(n + 1, 0)

	for i = 1, m do
		for j = 1, n do
			if compareA[i] == compareB[j] then
				curr[j + 1] = prev[j] + 1
			else
				curr[j + 1] = math.max(prev[j + 1], curr[j])
			end
		end
		-- Swap rows and reset curr
		prev, curr = curr, prev
		for j = 1, n + 1 do
			curr[j] = 0
		end
	end

	return prev[n + 1]
end

-- Counts lines that differ between two strings using LCS-based diff
-- Returns: totalDiff, whitespaceDiff (lines that only differ in whitespace)
-- Returns 10000 for totalDiff if exceeds 100 (frontend displays as "100+")
-- whitespaceDiff is 10000 only if ALL checked differences were whitespace-only
function WhitespaceUtil.CountLineDifferences(current: string, incoming: string): (number, number)
	if type(current) ~= "string" or type(incoming) ~= "string" then
		return 0, 0
	end

	local currentLines = SplitLines(current)
	local incomingLines = SplitLines(incoming)
	local m, n = #currentLines, #incomingLines

	-- Quick check: if size difference alone exceeds limit
	local minPossibleDiff = math.abs(m - n)
	if minPossibleDiff > 100 then
		return 10000, 0
	end

	-- Compute LCS with exact matching
	-- totalDiff = deletions + insertions = (m - lcs) + (n - lcs)
	local exactLcs = ComputeLcsLength(currentLines, incomingLines, false)
	local totalDiff = (m - exactLcs) + (n - exactLcs)

	if totalDiff > 100 then
		-- Compute normalized LCS to determine if changes are whitespace-only
		local normalizedLcs = ComputeLcsLength(currentLines, incomingLines, true)
		local normalizedDiff = (m - normalizedLcs) + (n - normalizedLcs)
		return 10000, if normalizedDiff == 0 then 10000 else 0
	end

	if totalDiff == 0 then
		return 0, 0
	end

	-- Compute LCS with normalized matching to find whitespace-only changes
	local normalizedLcs = ComputeLcsLength(currentLines, incomingLines, true)
	local normalizedDiff = (m - normalizedLcs) + (n - normalizedLcs)

	-- whitespaceDiff = changes that disappear after normalization
	local whitespaceDiff = totalDiff - normalizedDiff

	return totalDiff, whitespaceDiff
end

return WhitespaceUtil
