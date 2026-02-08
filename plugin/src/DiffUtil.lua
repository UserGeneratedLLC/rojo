--!strict
--!optimize 2
--!native

--[[
	Line-level diff utility using Myers O(ND) diff algorithm.
	Produces git-identical line change counts and unified diff hunks.

	Based on: "An O(ND) Difference Algorithm and Its Variations" (Myers, 1986)
	This is the same algorithm git uses internally for line-level diffs.
	Always computes exact results -- no thresholds or approximations.

	Public API:
	  DiffUtil.countLineDifferences(current, incoming) -> LineDiffResult
	  DiffUtil.diffLines(current, incoming, contextLines?) -> DiffResult
]]

local DiffUtil = {}

local EQUAL = 0
local DELETE = 1
local INSERT = 2

DiffUtil.EditType = table.freeze({
	Equal = EQUAL,
	Delete = DELETE,
	Insert = INSERT,
})

-- Split string into lines on \n, matching git behavior.
-- A trailing \n does NOT produce a phantom empty line.
local function splitLines(str: string): { string }
	if str == "" then
		return {}
	end

	local lines = {}
	local count = 0
	local start = 1

	while true do
		local pos = string.find(str, "\n", start, true)
		if pos then
			count += 1
			lines[count] = string.sub(str, start, pos - 1)
			start = pos + 1
		else
			local last = string.sub(str, start)
			if last ~= "" then
				count += 1
				lines[count] = last
			end
			break
		end
	end

	return lines
end

-- Normalize a line for whitespace comparison (strip trailing whitespace and \r)
local function normalizeLine(line: string): string
	return (string.gsub(string.gsub(line, "\r+$", ""), "[ \t]+$", ""))
end

-- Normalize an array of lines for whitespace comparison
local function normalizeLines(lines: { string }): { string }
	local result = table.create(#lines)
	for i = 1, #lines do
		result[i] = normalizeLine(lines[i])
	end
	return result
end

--[[
	Myers algorithm: compute edit distance only (no edit script).
	Returns D (number of edits). Always runs to completion.
	O(ND) time, O(N) space.
]]
local function myersEditDistance(compareA: { string }, compareB: { string }): number
	local m, n = #compareA, #compareB
	if m == 0 then
		return n
	end
	if n == 0 then
		return m
	end

	local maxD = m + n
	local offset = maxD + 1
	local v = table.create(2 * maxD + 2, 0)
	v[offset + 1] = 0

	for d = 0, maxD do
		for k = -d, d, 2 do
			local x
			if k == -d or (k ~= d and v[offset + k - 1] < v[offset + k + 1]) then
				x = v[offset + k + 1]
			else
				x = v[offset + k - 1] + 1
			end

			local y = x - k

			-- Follow diagonal (equal lines)
			while x < m and y < n and compareA[x + 1] == compareB[y + 1] do
				x += 1
				y += 1
			end

			v[offset + k] = x

			if x >= m and y >= n then
				return d
			end
		end
	end

	-- Unreachable for valid inputs, but satisfies the type checker
	return maxD
end

--[[
	Myers algorithm: compute full edit script with backtracking.
	Returns array of {editType, content}. Always runs to completion.
	O(ND) time, O(D*N) space (stores V array for each step for backtracking).

	linesA/linesB: original lines (used for output content)
	compareA/compareB: lines used for comparison (may be normalized)
]]
local function myersEditScript(
	linesA: { string },
	linesB: { string },
	compareA: { string }?,
	compareB: { string }?
): { { number } }
	local cmpA = compareA or linesA
	local cmpB = compareB or linesB
	local m, n = #linesA, #linesB

	-- Trivial cases
	if m == 0 and n == 0 then
		return {}
	end
	if m == 0 then
		local result = table.create(n)
		for i = 1, n do
			result[i] = { INSERT, linesB[i] }
		end
		return result
	end
	if n == 0 then
		local result = table.create(m)
		for i = 1, m do
			result[i] = { DELETE, linesA[i] }
		end
		return result
	end

	local maxD = m + n
	local offset = maxD + 1
	local v = table.create(2 * maxD + 2, 0)
	v[offset + 1] = 0

	local vHistory = {}
	local foundD: number? = nil

	-- Forward pass: find shortest edit path, storing V at each step
	for d = 0, maxD do
		for k = -d, d, 2 do
			local x
			if k == -d or (k ~= d and v[offset + k - 1] < v[offset + k + 1]) then
				x = v[offset + k + 1]
			else
				x = v[offset + k - 1] + 1
			end

			local y = x - k

			while x < m and y < n and cmpA[x + 1] == cmpB[y + 1] do
				x += 1
				y += 1
			end

			v[offset + k] = x

			if x >= m and y >= n then
				foundD = d
				break
			end
		end

		-- Save V AFTER computing step d (needed for backtracking)
		vHistory[d + 1] = table.clone(v)

		if foundD then
			break
		end
	end

	-- foundD is always set for valid inputs (worst case D = m + n)
	assert(foundD, "Myers algorithm failed to find edit path")

	-- Backtrack to recover the edit script (built in reverse, then flipped)
	local edits = {}
	local editCount = 0
	local x, y = m, n

	for d = foundD, 1, -1 do
		local k = x - y
		-- V from step d-1 is stored at index d (1-indexed history)
		local prevV = vHistory[d]

		-- Determine which predecessor we came from
		local prevK
		if k == -d or (k ~= d and prevV[offset + k - 1] < prevV[offset + k + 1]) then
			prevK = k + 1 -- came from insert (down move)
		else
			prevK = k - 1 -- came from delete (right move)
		end

		local prevX = prevV[offset + prevK]
		local prevY = prevX - prevK

		-- After the edit, we landed at:
		local endX, endY
		if k < prevK then
			-- Insert: y increased by 1
			endX, endY = prevX, prevY + 1
		else
			-- Delete: x increased by 1
			endX, endY = prevX + 1, prevY
		end

		-- Diagonal moves (equal lines) from edit endpoint to current (x, y)
		while x > endX and y > endY do
			x -= 1
			y -= 1
			editCount += 1
			edits[editCount] = { EQUAL, linesA[x + 1] }
		end

		-- The edit itself
		if k < prevK then
			-- Insert
			y -= 1
			editCount += 1
			edits[editCount] = { INSERT, linesB[y + 1] }
		else
			-- Delete
			x -= 1
			editCount += 1
			edits[editCount] = { DELETE, linesA[x + 1] }
		end
	end

	-- Remaining diagonals from (0, 0) to current position
	while x > 0 and y > 0 do
		x -= 1
		y -= 1
		editCount += 1
		edits[editCount] = { EQUAL, linesA[x + 1] }
	end

	-- Reverse (we built it backwards)
	local result = table.create(editCount)
	for i = 1, editCount do
		result[i] = edits[editCount - i + 1]
	end

	return result
end

--[[
	Group an edit script into unified diff hunks with context lines.
	Each hunk contains contiguous changes with surrounding context,
	matching git's unified diff format.
]]
local function buildHunks(editScript: { { any } }, contextLines: number): { any }
	if #editScript == 0 then
		return {}
	end

	-- Find indices of changed entries (non-Equal)
	local changeIndices = {}
	local changeCount = 0
	for i, edit in editScript do
		if edit[1] ~= EQUAL then
			changeCount += 1
			changeIndices[changeCount] = i
		end
	end

	if changeCount == 0 then
		return {}
	end

	-- Group changes whose contexts would overlap (gap <= 2 * contextLines)
	local groups = {}
	local groupCount = 0
	local groupStart = changeIndices[1]
	local groupEnd = changeIndices[1]

	for i = 2, changeCount do
		local idx = changeIndices[i]
		if idx - groupEnd <= 2 * contextLines then
			-- Close enough to merge into current group
			groupEnd = idx
		else
			-- Start a new group
			groupCount += 1
			groups[groupCount] = { groupStart, groupEnd }
			groupStart = idx
			groupEnd = idx
		end
	end
	groupCount += 1
	groups[groupCount] = { groupStart, groupEnd }

	-- Build hunks from groups
	local hunks = {}
	for gi = 1, groupCount do
		local group = groups[gi]
		local hunkStart = math.max(1, group[1] - contextLines)
		local hunkEnd = math.min(#editScript, group[2] + contextLines)

		-- Count old/new lines before hunk to get starting line numbers
		local oldLine, newLine = 0, 0
		for i = 1, hunkStart - 1 do
			local editType = editScript[i][1]
			if editType == EQUAL then
				oldLine += 1
				newLine += 1
			elseif editType == DELETE then
				oldLine += 1
			else
				newLine += 1
			end
		end

		local oldStart = oldLine + 1
		local newStart = newLine + 1
		local oldCount, newCount = 0, 0
		local lines = {}
		local lineCount = 0

		for i = hunkStart, hunkEnd do
			local edit = editScript[i]
			local editType = edit[1]
			local lineType

			if editType == EQUAL then
				lineType = "context"
				oldCount += 1
				newCount += 1
			elseif editType == DELETE then
				lineType = "remove"
				oldCount += 1
			else
				lineType = "add"
				newCount += 1
			end

			lineCount += 1
			lines[lineCount] = { type = lineType, content = edit[2] }
		end

		hunks[gi] = {
			oldStart = oldStart,
			oldCount = oldCount,
			newStart = newStart,
			newCount = newCount,
			lines = lines,
		}
	end

	return hunks
end

--[[
	Count line differences between two strings (lightweight, just counts).
	Returns a structured result matching git diff --stat behavior:
	  added = lines present in incoming but not current
	  removed = lines present in current but not incoming
	  isWhitespaceOnly = true when ALL changes are whitespace-only

	A modified line counts as 1 removal + 1 addition (matching git).
	Always computes exact results with no thresholds.
]]
function DiffUtil.countLineDifferences(
	current: string,
	incoming: string
): { added: number, removed: number, isWhitespaceOnly: boolean }
	if type(current) ~= "string" or type(incoming) ~= "string" then
		return { added = 0, removed = 0, isWhitespaceOnly = false }
	end

	-- Fast path for identical strings
	if current == incoming then
		return { added = 0, removed = 0, isWhitespaceOnly = false }
	end

	local currentLines = splitLines(current)
	local incomingLines = splitLines(incoming)
	local m, n = #currentLines, #incomingLines

	-- Compute exact edit distance using Myers
	local d = myersEditDistance(currentLines, incomingLines)

	if d == 0 then
		return { added = 0, removed = 0, isWhitespaceOnly = false }
	end

	-- Derive added/removed from edit distance and line counts
	-- D = removed + added, and removed - added = m - n
	local removed = (d + m - n) // 2
	local added = (d - m + n) // 2

	-- Check whitespace-only: run normalized comparison
	local normD = myersEditDistance(normalizeLines(currentLines), normalizeLines(incomingLines))
	local isWhitespaceOnly = normD == 0

	return {
		added = added,
		removed = removed,
		isWhitespaceOnly = isWhitespaceOnly,
	}
end

--[[
	Full line diff with hunks (for merge dialogue and detailed views).
	Returns counts, whitespace info, and unified diff hunks.
	contextLines defaults to 3, matching git's default.
	Always computes exact results with no thresholds.
]]
function DiffUtil.diffLines(
	current: string,
	incoming: string,
	contextLines: number?
): {
	added: number,
	removed: number,
	isWhitespaceOnly: boolean,
	hunks: { any },
}
	contextLines = contextLines or 3

	if type(current) ~= "string" or type(incoming) ~= "string" then
		return { added = 0, removed = 0, isWhitespaceOnly = false, hunks = {} }
	end

	if current == incoming then
		return { added = 0, removed = 0, isWhitespaceOnly = false, hunks = {} }
	end

	local currentLines = splitLines(current)
	local incomingLines = splitLines(incoming)

	-- Compute full edit script
	local editScript = myersEditScript(currentLines, incomingLines)

	-- Count edits from the script
	local added, removed = 0, 0
	for _, edit in editScript do
		if edit[1] == DELETE then
			removed += 1
		elseif edit[1] == INSERT then
			added += 1
		end
	end

	-- Check whitespace-only
	local isWhitespaceOnly = false
	if added > 0 or removed > 0 then
		local normD = myersEditDistance(normalizeLines(currentLines), normalizeLines(incomingLines))
		isWhitespaceOnly = normD == 0
	end

	-- Build unified diff hunks
	local hunks = buildHunks(editScript, contextLines)

	return {
		added = added,
		removed = removed,
		isWhitespaceOnly = isWhitespaceOnly,
		hunks = hunks,
	}
end

return DiffUtil
