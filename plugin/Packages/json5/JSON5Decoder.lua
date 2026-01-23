--!strict
--!native
--!optimize 2

-- json5.lua: JSON5 parser written in pure Lua.
--
-- Copyright (c) 2024 Miku AuahDark
--
-- Permission is hereby granted, free of charge, to any person obtaining a copy of
-- this software and associated documentation files (the "Software"), to deal in
-- the Software without restriction, including without limitation the rights to
-- use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies
-- of the Software, and to permit persons to whom the Software is furnished to do
-- so, subject to the following conditions:
--
-- The above copyright notice and this permission notice shall be included in all
-- copies or substantial portions of the Software.
--
-- THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
-- IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
-- FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
-- AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
-- LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
-- OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
-- SOFTWARE.

-- @see https://github.com/MikuAuahDark/json5.lua


---@type string[]|table<string,boolean>
local whitespaceList = {
	"\225\154\128",
	"\226\128\128",
	"\226\128\129",
	"\226\128\130",
	"\226\128\131",
	"\226\128\132",
	"\226\128\133",
	"\226\128\134",
	"\226\128\135",
	"\226\128\136",
	"\226\128\137",
	"\226\128\138",
	"\226\128\168",
	"\226\128\169",
	"\226\128\175",
	"\226\129\159",
	"\227\128\128",
	"\194\160", -- &nbsp;
	"\r\n", -- CRLF
	"\9", -- Tab
	"\10", -- Newline
	"\11", -- Vert. tab
	"\12", -- Form feed
	"\13", -- CR
	" ",
}

---@type string[]|table<string,boolean>
local newlineCharsList = {
	"\226\128\168",
	"\226\128\169",
	"\r\n", -- CRLF
	"\r", -- CR
	"\n", -- LF
}

local function MAKE_LOOKUP(t: {string}): {[string]: boolean}
  local kv = {}
	for i, v in ipairs(t) do
		kv[v] = true
	end
  return kv
end

local whitespace = MAKE_LOOKUP(whitespaceList)
local newlineChars = MAKE_LOOKUP(newlineCharsList)

local escaper = {
	["0"] = "\0",
	["'"] = "'",
	["\""] = "\"",
	["\\"] = "\\",
	b = "\8",
	f = "\12",
	n = "\n",
	r = "\r",
	t = "\t",
	v = "\11"
}

local function Q(obj: any): string
	return (string.format("%q", obj):gsub("\r", "\\r"):gsub("\n", "\\n"))
end

---@param message string
---@param rowcol {[1]:integer,[2]:integer}
local function formatError(
  message: string,
  rowcol: {number}
): string
	return string.format("%s at line %d col %d", message, rowcol[1], rowcol[2])
end

---@param ws string
---@param rowcol {[1]:integer,[2]:integer}
local function advanceNewline(
  ws: string,
  rowcol: {number}
): boolean
	if newlineChars[ws] then
		rowcol[1] = rowcol[1] + 1
		rowcol[2] = 1
		return true
	end

	return false
end

---@param text string
---@param rowcol {[1]:integer,[2]:integer}
local function getNewline(
  text: string,
  rowcol: {number}
): string?
	for _, nl in ipairs(newlineCharsList) do
		if text:sub(1, #nl) == nl and advanceNewline(nl, rowcol) then
			return nl
		end
	end

	return nil
end

---@param text string
---@param rowcol {[1]:integer,[2]:integer}
local function getWhitespace(
  text: string,
  rowcol: {number}
): string?
	if #text == 0 then
		return nil
	end

	for _, w in ipairs(whitespaceList) do
		if text:sub(1, #w) == w then
			rowcol[2] = rowcol[2] + #w
			advanceNewline(w, rowcol)
			return w
		end
	end

	return nil
end

---@param text string
---@param rowcol {[1]:integer,[2]:integer}
local function stripWhitespace(
  text: string,
  rowcol: {number}
): string
	-- This is quite expensive, O(n^2)
	while true do
		local ws = getWhitespace(text, rowcol)

		if ws == nil then
			break
		end

		text = text:sub(#ws + 1)
	end

	return text
end

---@param text string
---@param rowcol {[1]:integer,[2]:integer}
local function stripInlineComments(
  text: string,
  rowcol: {number}
): string
	-- Find newline
	while true do
		if #text == 0 then
			-- EOF
			break
		end

		local nl = getNewline(text, rowcol)
		if nl then
			text = text:sub(#nl + 1)
			break
		end

		text = text:sub(2)
		rowcol[2] = rowcol[2] + 1
	end

	return text
end

---@param text string
---@param rowcol {[1]:integer,[2]:integer}
---@return string
local function stripBlockComments(
  text: string,
  rowcol: {number}
): string
	-- This is bit expensive but it's necessary for proper row column support
	while #text > 0 do
		-- Found block comment close tag
		if text:sub(1, 2) == "*/" then
			rowcol[2] = rowcol[2] + 2
			return text:sub(3)
		end

		local nl = getNewline(text, rowcol)
		if nl then
			text = text:sub(#nl + 1)
		else
			rowcol[2] = rowcol[2] + 1
			text = text:sub(2)
		end
	end

	error(formatError("missing multiline comment close tag", rowcol))
	return ""
end

---Copied and modified slightly from rxi/json
---@param n integer
local function codepointToutf8(
  n: number
): (string?, string?)
	-- http://scripts.sil.org/cms/scripts/page.php?site_id=nrsi&id=iws-appendixa
	if n <= 0x7f then
		return string.char(n)
	elseif n <= 0x7ff then
		return string.char(math.floor(n / 64) + 192, n % 64 + 128)
	elseif n <= 0xffff then
		return string.char(math.floor(n / 4096) + 224,
      math.floor(n % 4096 / 64) + 128, n % 64 + 128)
	elseif n <= 0x10ffff then
		return string.char(
      math.floor(n / 262144) + 240,
      math.floor(n % 262144 / 4096) + 128,
      math.floor(n % 4096 / 64) + 128,
      n % 64 + 128
    )
	end

	return nil, string.format("invalid unicode codepoint '%x'", n)
end

---@param text string includes the "\u"
---@param rowcol {[1]:integer,[2]:integer}
local function parseUnicodeImpl(
  text: string,
  rowcol: {number}
): (number, string)
	local hexcode = text:match("^\\u(%x%x%x%x)")
	if not hexcode then
		error(formatError("invalid unicode hex escape sequence", rowcol))
	end

	local utf16code = tonumber(hexcode, 16)
	if not utf16code then
		error(formatError("invalid unicode hex escape sequence", rowcol))
	end

	rowcol[2] = rowcol[2] + 6
	return utf16code, text:sub(7)
end

---@param low integer
---@param high integer
local function getSurrogatePair(low: number, high: number): number
	return (high - 0xd800) * 0x400 + (low - 0xdc00) + 0x10000
end

---@param text string includes the "\u"
---@param rowcol {[1]:integer,[2]:integer}
local function parseUnicode(
  text: string,
  rowcol: {number}
): (string, string)
	local row, col = rowcol[1], rowcol[2] -- for codepointToUTF8
	local num
	num, text = parseUnicodeImpl(text, rowcol)

	-- Is it surrogate pair?
	if num >= 0xd800 and num < 0xdc00 then
		-- High surrogate pair. Need low surrogate pair.
		local lownum
		lownum, text = parseUnicodeImpl(text, rowcol)
		if lownum and lownum >= 0xdc00 and lownum <= 0xdfff then
			num = getSurrogatePair(lownum, num)
		end
		-- TODO(low): Should we error in case of invalid pairs?
	end

	local utf8text, errmsg = codepointToutf8(num)
	if not utf8text then
		---@cast errmsg -nil
		error(formatError(assert(errmsg), {row, col}))
	end

	---@cast utf8text -nil
	return utf8text, text
end

---@param text string
---@param stop fun(text:string):boolean
---@param identifierMode boolean
---@param rowcol {[1]:integer,[2]:integer}
local function parseStringImpl(
  text: string,
  stop: (string) -> boolean,
  identifierMode: boolean,
  rowcol: {number}
): (string, string)
	local result = {}

	while true do
		if stop(text) then
			break
		end

		local char = text:sub(1, 1)

		-- Escape sequence?
		if char == "\\" then
			local what = text:sub(2, 2)
			rowcol[2] = rowcol[2] + 1

			if escaper[what] then
				rowcol[2] = rowcol[2] + 1

				if identifierMode then
					error(formatError("escape sequence not allowed", rowcol))
				end

				result[#result+1] = escaper[what]
				text = text:sub(3)
			elseif what == "u" then
				-- Unicode escape
				local unicode
				unicode, text = parseUnicode(text, rowcol)
				result[#result+1] = unicode
			elseif what == "x" then
				if identifierMode then
					error(formatError("hex escape sequence not allowed", rowcol))
				end

				rowcol[2] = rowcol[2] + 2

				local hexstr = text:sub(2, 3)
				local hexnum = tonumber(hexstr, 16)
				if not hexnum then
					error(formatError("invalid hex escape sequence", rowcol))
				end

				result[#result+1] = string.char(hexnum)
				rowcol[2] = rowcol[2] + 2
				text = text:sub(5) -- "\xHH"
			else
				if identifierMode then
					error(formatError("invalid escape sequence", rowcol))
				end

				local nl = getNewline(text:sub(2), rowcol)
				local ignore = 2
				if nl then
					-- JSON5 allows string spanning multiple lines by escaping newline
					ignore = #nl + 2
				end

				-- Ignore
				text = text:sub(ignore)
			end
		elseif char:byte(1, 1) < 32 then
			-- Accept control characters (e.g. from json5 pretty-print)
			result[#result+1] = char
			text = text:sub(2)
			if char == "\n" then
				rowcol[1] = rowcol[1] + 1
				rowcol[2] = 1
			else
				rowcol[2] = rowcol[2] + 1
			end
		else
			result[#result+1] = char
			text = text:sub(2)
			rowcol[2] = rowcol[2] + 1
		end
	end

	return table.concat(result), text
end

---@param text string including the delimiter
---@param rowcol {[1]:integer,[2]:integer}
local function parseString(
  text: string,
  rowcol: {number}
): (string, string)
	local stop, value = text:sub(1, 1), nil

	---@param txt string
	---@return boolean
	local function stopCriterion(txt)
		return txt:sub(1, 1) == stop
	end

	value, text = parseStringImpl(text:sub(2), stopCriterion, false, rowcol)
	rowcol[2] = rowcol[2] + 1
	return value, text:sub(2)
end

local NaN = 0/0

---@param text string
---@param rowcol {[1]:integer,[2]:integer}
local function parseNumber(
  text: string,
  rowcol: {number}
): (number, string)
	local sign = 1
	local signchar = text:sub(1, 1)
	local row, col = rowcol[1], rowcol[2] -- for tonumber parsing

	if signchar == "+" then
		sign = 1
		text = text:sub(2)
		rowcol[2] = rowcol[2] + 1
	elseif signchar == "-" then
		sign = -1
		text = text:sub(2)
		rowcol[2] = rowcol[2] + 1
	end

	if text:sub(1, 3) == "NaN" then
		rowcol[2] = rowcol[2] + 3
		return NaN, text:sub(4)
	end

	local infText = text:find("Infinity", 1, true)
	if infText == 1 then
		rowcol[2] = rowcol[2] + 8
		return math.huge * sign, text:sub(9)
	end

	-- TODO(low): Bring our own number parsing for Lua 5.1?
	local potentialNum = 0
	local lookText = text

	while true do
		if getWhitespace(lookText, rowcol) then
			break
		end

		local next = lookText:sub(1, 1)
		if next == "" or next == "," or next == "]" or next == "}" then
			break
		end

		potentialNum = potentialNum + 1
		lookText = lookText:sub(2)
		rowcol[2] = rowcol[2] + 1
	end

	local numval = text:sub(1, potentialNum)
	local num: number? = nil

	if numval:sub(1, 1) == "0" and numval:sub(2):find("^%d+$") then
		-- Octal is not allowed
		num = nil
	else
		-- Parse normal
		num = tonumber(numval)
	end
	if num == nil then
		error(formatError("invalid number sequence "..Q(numval), {row, col}))
	end

	rowcol[2] = rowcol[2] + potentialNum
	---@cast num -nil
	return num * sign, text:sub(potentialNum + 1)
end

---@param text string
---@param rowcol {[1]:integer,[2]:integer}
---@param nullval any
local function parseNull(
  text: string,
  rowcol: {number},
  nullval: any?
): (any?, string)
	if text:sub(1, 4) ~= "null" then
		error(formatError("invalid null literal", rowcol))
	end

	rowcol[2] = rowcol[2] + 1
	return nullval, text:sub(5)
end

---@param text string
---@param rowcol {[1]:integer,[2]:integer}
local function parseBoolean(
  text: string,
  rowcol: {number}
): boolean
	if text:sub(1, 4) == "true" then
		rowcol[2] = rowcol[2] + 4
		return true, text:sub(5)
	elseif text:sub(1, 5) == "false" then
		rowcol[2] = rowcol[2] + 5
		return false, text:sub(6)
	else
		error(formatError("invalid boolean literal", rowcol))
	end
end

---@param text string
---@param rowcol {[1]:integer,[2]:integer}
local function stripComments(
  text: string,
  rowcol: {number}
): string
	local s = text:sub(1, 2)
	if s == "//" then
		rowcol[2] = rowcol[2] + 2
		return stripInlineComments(text:sub(3), rowcol)
	elseif s == "/*" then
		rowcol[2] = rowcol[2] + 2
		return stripBlockComments(text:sub(3), rowcol)
	end

	return text
end

---@param text string
---@param rowcol {[1]:integer,[2]:integer}
local function stripWhitespaceAndComments(
  text: string,
  rowcol: {number}
): string
	while true do
		local txt = stripWhitespace(text, rowcol)
		txt = stripComments(txt, rowcol)

		if txt == text then
			break
		end

		text = txt
	end

	return text
end

---@alias json5.Value nil|number|boolean|string|json5.Array|json5.Object
---@alias json5.Array json5.Value[]
---@alias json5.Object table<string, json5.Value>

local parseValue

---@param text string
---@param rowcol {[1]:integer,[2]:integer}
---@param nullval any
---@return json5.Array,string
local function parseArray(
  text: string,
  rowcol: {number},
  nullval: any?
): ({any}, string)
	text = text:sub(2)
	rowcol[2] = rowcol[2] + 1

	local result = {}

	while true do
		text = stripWhitespaceAndComments(text, rowcol)

		if text:sub(1, 1) == "]" then
			-- Finish
			rowcol[2] = rowcol[2] + 1
			text = text:sub(2)
			break
		end

		local value
		value, text = parseValue(text, rowcol, nullval)
		text = stripWhitespaceAndComments(text, rowcol)

		-- Insert
		result[#result+1] = value

		-- Continue or finish?
		local lastOrNext = text:sub(1, 1)
		text = text:sub(2)

		if lastOrNext == "]" then
			-- Finish
			rowcol[2] = rowcol[2] + 1
			break
		elseif lastOrNext ~= "," then
			error(formatError("expected comma got "..Q(lastOrNext), rowcol))
		end

		rowcol[2] = rowcol[2] + 1
	end

	return result, text
end

---@param identifier string
local function testIdentifier(
  identifier: string
): boolean
	local firstID = identifier:byte(1, 1)
	if firstID >= 48 and firstID <= 57 then
		return false
	end

	for i = 1, #identifier do
		local char = identifier:byte(i, i)
		if char < 36 then
			return false
		end

		if char >= 37 and char <= 47 then
			return false
		end

		if char >= 58 and char <= 64 then
			return false
		end

		if char >= 91 and char <= 94 then
			return false
		end

		if char == 96 then
			return false
		end

		if char >= 123 and char <= 128 then
			return false
		end
	end

	return true
end

---@param text string
local function stopIdentifier(
  text: string
): boolean
	return text:sub(1, 1) == ":" or getWhitespace(text, {0, 0}) ~= nil
end

---@param text string
---@param rowcol {[1]:integer,[2]:integer}
local function parseIdentifier(
  text: string,
  rowcol: {number}
): (string, string)
	local first = text:sub(1, 1)
	local identifier

	if first == "'" or first == "\"" then
		-- Quoted identifier
		identifier, text = parseString(text, rowcol)
	else
		local row, col = rowcol[1], rowcol[2]
		-- Unquoted identifier
		identifier, text = parseStringImpl(text, stopIdentifier, true, rowcol)

		-- Test identifier validity
		if not testIdentifier(identifier) then
			error(formatError("invalid identifier "..Q(identifier), {row, col}))
		end
	end

	return identifier, text
end

---@param text string
---@param rowcol {[1]:integer,[2]:integer}
---@param nullval any
---@return json5.Object,string
local function parseObject(
  text: string,
  rowcol: {number},
  nullval: any?
): ({[string]: any}, string)
	rowcol[2] = rowcol[2] + 1
	text = text:sub(2)

	local result = {}

	while true do
		text = stripWhitespaceAndComments(text, rowcol)

		if text:sub(1, 1) == "}" then
			-- Finish
			rowcol[2] = rowcol[2] + 1
			text = text:sub(2)
			break
		end

		-- Identifier
		local identifier
		identifier, text = parseIdentifier(text, rowcol)
		text = stripWhitespaceAndComments(text, rowcol)

		-- Colon
		if text:sub(1, 1) ~= ":" then
			error(formatError(
        "expected colon after identifier, got ".. Q(text:sub(1, 1)),
        rowcol)
      )
		end

		rowcol[2] = rowcol[2] + 1
		text = text:sub(2)

		-- Value
		text = stripWhitespaceAndComments(text, rowcol)

		local value
		value, text = parseValue(text, rowcol, nullval)

		text = stripWhitespaceAndComments(text, rowcol)

		-- Insert
		result[identifier] = value

		-- Continue or finish?
		local lastOrNext = text:sub(1, 1)
		text = text:sub(2)

		if lastOrNext == "}" then
			-- Finish
			rowcol[2] = rowcol[2] + 1
			break
		elseif lastOrNext ~= "," then
			error(formatError("expected comma got "..Q(lastOrNext), rowcol))
		end

		rowcol[2] = rowcol[2] + 1
	end

	return result, text
end

---@param text string
---@param rowcol {[1]:integer,[2]:integer}
local function catchEOF(text: string, rowcol: {number})
	error(formatError("unexpected eof", rowcol))
end

local valueTest = {
	["-"] = parseNumber,
	["+"] = parseNumber,
	["."] = parseNumber,
	["0"] = parseNumber,
	["1"] = parseNumber,
	["2"] = parseNumber,
	["3"] = parseNumber,
	["4"] = parseNumber,
	["5"] = parseNumber,
	["6"] = parseNumber,
	["7"] = parseNumber,
	["8"] = parseNumber,
	["9"] = parseNumber,
	["N"] = parseNumber, -- for NaN
	["I"] = parseNumber, -- for Infinity
	["n"] = parseNull,
	["t"] = parseBoolean,
	["f"] = parseBoolean,
	["'"] = parseString,
	["\""] = parseString,
	["["] = parseArray,
	["{"] = parseObject,
	[""] = catchEOF,
}

---@param text string
---@param rowcol {[1]:integer,[2]:integer}
---@param nullval any
---@return json5.Value,string
parseValue = function(
  text: string,
  rowcol: {number},
  nullval: any?
): any
	text = stripWhitespaceAndComments(text, rowcol)

	local first = text:sub(1, 1)
	local func = valueTest[first]

	if not func then
		error(formatError("invalid value literal "..Q(first), rowcol))
	end

	return func(text, rowcol, nullval)
end



local module = {}

---A value that denote "null" value in JSON, if the user need to preserve keys.
---@type any
module.Null = newproxy(false)

---@class json5.opts
---@field public null any Null value substitute.
--   Default is `nil` which means "null" will not preserve key with "null" value
--   and potentially leave holes in an array. To preserve "null",
--   use `json5.null` or any other value.

---Decode JSON5 string to Lua value.
---@param text string JSON5 string.
---@param nullValue Optional null value, see module.Null
---@return json5.Value
function module.Decode(
  text: string,
  nullValue: any?
): any
	local rowcol = {1, 1}
	text = stripWhitespaceAndComments(text, rowcol)

	local value
	value, text = parseValue(text, rowcol, nullValue)
	text = stripWhitespaceAndComments(text, rowcol)

	if #text > 0 then
		error(formatError("trailing garbage", rowcol))
	end

	return value
end

return table.freeze(module)
