--!strict
--!native
--!optimize 2
--[[
	Copyright 2025 Joseph Melsha

	Licensed under the Apache License, Version 2.0 (the "License");
	you may not use this file except in compliance with the License.
	You may obtain a copy of the License at

	    http://www.apache.org/licenses/LICENSE-2.0

	Unless required by applicable law or agreed to in writing, software
	distributed under the License is distributed on an "AS IS" BASIS,
	WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
	See the License for the specific language governing permissions and
	limitations under the License.
]]

-- Negative codes mean \u00XX
local StringCodes = {
	[0x00] = -0x3030,
	[0x01] = -0x3130,
	[0x02] = -0x3230,
	[0x03] = -0x3330,
	[0x04] = -0x3430,
	[0x05] = -0x3530,
	[0x06] = -0x3630,
	[0x07] = -0x3730,
	[0x08] =  0x625C, -- \b
	[0x09] =  0x745C, -- \t
	[0x0A] =  0x6E5C, -- \n
	[0x0B] = -0x6230,
	[0x0C] =  0x665C, -- \f
	[0x0D] =  0x725C, -- \r
	[0x0E] = -0x6530,
	[0x0F] = -0x6630,
	[0x10] = -0x3031,
	[0x11] = -0x3131,
	[0x12] = -0x3231,
	[0x13] = -0x3331,
	[0x14] = -0x3431,
	[0x15] = -0x3531,
	[0x16] = -0x3631,
	[0x17] = -0x3731,
	[0x18] = -0x3831,
	[0x19] = -0x3931,
	[0x1A] = -0x6131,
	[0x1B] = -0x6231,
	[0x1C] = -0x6331,
	[0x1D] = -0x6431,
	[0x1E] = -0x6531,
	[0x1F] = -0x6631,
}

local IdentifierChar1 = {
  [36] = true,  -- $
  [95] = true,  -- _
}
local IdentifierChar2 = {
  [36] = true,  -- $
  [95] = true,  -- _
}
-- A–Z
for b=65,90 do
  IdentifierChar1[b] = true
  IdentifierChar2[b] = true
end
-- a–z
for b=97,122 do
  IdentifierChar1[b] = true
  IdentifierChar2[b] = true
end
-- 0–9 (only valid in subsequent positions)
for b=48,57 do
  IdentifierChar2[b] = true
end

type Stream = {
	Buf: buffer,
	Pos: number,
	Cap: number,
	Indent: number,
	Pretty: boolean,
	Encoders: {[string]: (out: Stream, v: any) -> ()},
	Null: any?,
	QuoteChar: number,
	UnquoteIdent: boolean,
}

local function Stream(buf: buffer): Stream
	return {
		Buf = buf,
		Pos = 0,
		Cap = buffer.len(buf),
		Indent = 0,
		Pretty = false,
		Encoders = {},
		Null = nil,
		QuoteChar = 0x22,
		UnquoteIdent = false,
	}
end

local function ToString(out: Stream): string
	return buffer.readstring(out.Buf, 0, out.Pos)
end

local function AllocSize(
	n: number
): number
	return math.max(n, 2 ^ math.ceil(math.log(n, 2)))
end

local function Reserve(
	out: Stream,
	n: number
): number
	local pos = out.Pos
	local newPos = pos + n
	if out.Cap < newPos then
		local newCap = AllocSize(newPos)
		local newBuf = buffer.create(newCap)
		buffer.copy(newBuf, 0, out.Buf, 0, pos)
		out.Buf, out.Cap = newBuf, newCap
	end
	out.Pos = newPos
	return pos
end

local function WriteString(out: Stream, s: string)
	local len = string.len(s)
	local pos = Reserve(out, len)
	buffer.writestring(out.Buf, pos, s, len)
end

local function WriteIndent(out: Stream)
	local len = out.Indent * 2
	local pos = Reserve(out, len)
	local buf = out.Buf
	for i=0,len-1 do
		buffer.writeu16(buf, pos + i * 2, 0x2020)
	end
end

local function IncrementIndent(out: Stream, n: number)
	out.Indent += n
end

local function WriteU8(out: Stream, v: number)
	local pos = Reserve(out, 1)
	buffer.writeu8(out.Buf, pos, v)
end

local function WriteU16(out: Stream, v: number)
	local pos = Reserve(out, 2)
	buffer.writeu16(out.Buf, pos, v)
end

local function WriteU32(out: Stream, v: number)
	local pos = Reserve(out, 4)
	buffer.writeu32(out.Buf, pos, v)
end

local function EncodeNull(out: Stream, v: nil)
	WriteU32(out, 0x6C6C756E) -- null
end

local function EncodeBoolean(out: Stream, v: boolean)
	if v then
		WriteU32(out, 0x65757274) -- true
	else
		local pos = Reserve(out, 5)
		local buf = out.Buf
		buffer.writeu32(buf, pos, 0x736C6166) -- fals
		buffer.writeu8(buf, pos + 4, 0x65) -- e
	end
end

local function EncodeNumber(out: Stream, n: number)
	if n ~= n then
		local pos = Reserve(out, 3)
		local buf = out.Buf
		buffer.writeu16(buf, pos, 0x614E) -- Na
		buffer.writeu8(buf, pos + 2, 0x4E)   -- N
	elseif n == math.huge then
		local pos = Reserve(out, 8)
		local buf = out.Buf
		buffer.writeu32(buf, pos, 0x69666E49) -- Infi
		buffer.writeu32(buf, pos + 4, 0x7974696E) -- 'nity
	elseif n == -math.huge then
		local pos = Reserve(out, 9)
		local buf = out.Buf
		buffer.writeu8(buf, pos, 0x2D) -- '-'
		buffer.writeu32(buf, pos + 1, 0x69666E49) -- Infi
		buffer.writeu32(buf, pos + 5, 0x7974696E) -- 'nity
	else
		local s = tostring(n)
		local i = string.find(s, "e%+")
		if i then
			s = string.sub(s, 1, i) .. string.sub(s, i + 2)
		end
		WriteString(out, s)
	end
end

local function IsIdentifierName(
	out: Stream,
	s: string
): boolean
	local len = string.len(s)
	if len == 0 then
		return false
	end
  local first = string.byte(s, 1)
  if not IdentifierChar1[first] then
    return false
  end
	local identifierChar2 = IdentifierChar2
  for i=2,len do
    if not identifierChar2[string.byte(s, i)] then
      return false
    end
  end
	return true
end

local function EncodeString(
	out: Stream,
	s: string,
	unquoteIdent: boolean?
)
	if unquoteIdent and not IsIdentifierName(out, s) then
		unquoteIdent = false
	end

	local len = string.len(s)
	local pos = Reserve(out, 2 + len * 6)
	local buf = out.Buf

	local quoteCode = out.QuoteChar

	if not unquoteIdent then
		buffer.writeu8(buf, pos, quoteCode) -- " or '
		pos += 1
	end

	for i=1,len do
		local b = string.byte(s, i)
		if b > 31 then
			if b ~= quoteCode and b ~= 0x5C then
				buffer.writeu8(buf, pos, b)
				pos += 1
			else
				buffer.writeu16(buf, pos, 0x5C + bit32.lshift(b, 8))  -- \" or \' or \\
				pos += 2
			end
		else
			local code = StringCodes[b]
			if code < 0 then
				code = -code
				buffer.writeu32(buf, pos, 0x3030755C) -- \u00
				pos += 4
			end
			buffer.writeu16(buf, pos, code)
			pos += 2
		end
	end

	if not unquoteIdent then
		buffer.writeu8(buf, pos, quoteCode) -- " or '
		pos += 1
	end

	out.Pos = pos
end

local function EncodeBuffer(out: Stream, data: buffer)
	local len = buffer.len(data)
	local pos = Reserve(out, 2 + len * 6)
	local buf = out.Buf

	local quoteCode = out.QuoteChar

	buffer.writeu8(buf, pos, quoteCode) -- " or '
	pos += 1

	for i=0,len-1 do
		local b = buffer.readu8(data, i)
		if b > 31 then
			if b ~= quoteCode and b ~= 0x5C then
				buffer.writeu8(buf, pos, b)
				pos += 1
			else
				buffer.writeu16(buf, pos, 0x5C + bit32.lshift(b, 8))  -- \" or \' or \\
				pos += 2
			end
		else
			local code = StringCodes[b]
			if code < 0 then
				code = -code
				buffer.writeu32(buf, pos, 0x3030755C) -- \u00
				pos += 4
			end
			buffer.writeu16(buf, pos, code)
			pos += 2
		end
	end

	buffer.writeu8(buf, pos, quoteCode) -- " or '
	pos += 1

	out.Pos = pos
end

local function EncodeAny(out: Stream, v: any)
	if v == out.Null then
		v = nil
	end
	out.Encoders[typeof(v)](out, v)
end

local function EncodeArray(out: Stream, o: {any})
	local pretty = out.Pretty
	WriteU8(out, 0x5B) -- [
	if pretty then
		IncrementIndent(out, 1)
	end
	for i, v in ipairs(o) do
		if i > 1 then
			WriteU8(out, 0x2C) -- ,
		end
		if pretty then
			WriteU8(out, 0x0A) -- \n
			WriteIndent(out)
		end
		EncodeAny(out, v)
	end
	if pretty then
		IncrementIndent(out, -1)
		if #o > 0 then
			WriteU8(out, 0x0A) -- \n
			WriteIndent(out)
		end
	end
	WriteU8(out, 0x5D) -- ]
end

-- IDEA: encode keys as packed JSON, add to tuples, sort, encode 
local function EncodeMap(out: Stream, o: {[string]: any})
	local keys = {}
	for k, _ in pairs(o) do
		assert(type(k) == "string")
		table.insert(keys, k)
	end
	table.sort(keys)
	local pretty = out.Pretty
	local unquoteIdent = out.UnquoteIdent
	WriteU8(out, 0x7B) -- {
	if pretty then
		IncrementIndent(out, 1)
	end
	for i, k in ipairs(keys) do
		if i > 1 then
			WriteU8(out, 0x2C) -- ,
		end
		if pretty then
			WriteU8(out, 0x0A) -- \n
			WriteIndent(out)
		end
		EncodeString(out, k, unquoteIdent)
		if pretty then
			WriteU16(out, 0x203A) -- ': '
		else
			WriteU8(out, 0x3A) -- :
		end
		EncodeAny(out, o[k])
	end
	if pretty then
		IncrementIndent(out, -1)
		if #keys > 0 then
			WriteU8(out, 0x0A) -- \n
			WriteIndent(out)
		end
	end
	WriteU8(out, 0x7D) -- }
end

local function EncodeTable(out: Stream, o: any)
	-- assert(getmetatable(o) == nil)
	if #o > 0 or next(o) == nil then
		EncodeArray(out, o)
	else
		EncodeMap(out, o)
	end
end

local function EncodeVector2(out: Stream, v: Vector2)
	EncodeArray(out, { v.X, v.Y })
end

local function EncodeVector3(out: Stream, v: Vector3)
	EncodeArray(out, { v.X, v.Y, v.Z })
end

local function EncodeVector2int16(out: Stream, v: Vector2int16)
	EncodeArray(out, { v.X, v.Y })
end

local function EncodeVector3int16(out: Stream, v: Vector3int16)
	EncodeArray(out, { v.X, v.Y, v.Z })
end

local function EncodeRegion3(out: Stream, v: Region3)
	local min = v.CFrame * (v.Size * -0.5)
	local max = v.CFrame * (v.Size *  0.5)
	EncodeArray(out, { min.X, min.Y, min.Z, max.X, max.Y, max.Z })
end

local function EncodeRegion3int16(out: Stream, v: Region3int16)
	EncodeArray(out, { v.Min.X, v.Min.Y, v.Min.Z, v.Max.X, v.Max.Y, v.Max.Z })
end

local function EncodeUDim(out: Stream, v: UDim)
	EncodeArray(out, { v.Scale, v.Offset })
end

local function EncodeUDim2(out: Stream, v: UDim2)
	EncodeArray(out, { v.X.Scale, v.X.Offset, v.Y.Scale, v.Y.Offset })
end

local function EncodeCFrame(out: Stream, v: CFrame)
	EncodeArray(out, { v:GetComponents() })
end

local function EncodeColor3(out: Stream, v: Color3)
	EncodeArray(out, {
		math.round(v.R*255),
		math.round(v.G*255),
		math.round(v.B*255),
	})
end

local function EncodeNumberRange(out: Stream, v: NumberRange)
	EncodeArray(out, { v.Min, v.Max })
end

local function EncodeRect(out: Stream, v: Rect)
	EncodeArray(out, { v.Min.X, v.Min.Y, v.Max.X, v.Max.Y })
end

local function EncodeEnumItem(out: Stream, v: EnumItem)
	EncodeNumber(out, v.Value)
end

-- Encoders
local Encoders = {}
Encoders["nil"] = EncodeNull
Encoders["boolean"] = EncodeBoolean
Encoders["number"] = EncodeNumber
Encoders["string"] = EncodeString
Encoders["buffer"] = EncodeBuffer
Encoders["table"] = EncodeTable

-- Roblox Extensions
local EncodersExt = table.clone(Encoders)
EncodersExt["Vector2"] = EncodeVector2
EncodersExt["Vector3"] = EncodeVector3
EncodersExt["Vector2int16"] = EncodeVector2int16
EncodersExt["Vector3int16"] = EncodeVector3int16
EncodersExt["Region3"] = EncodeRegion3
EncodersExt["Region3int16"] = EncodeRegion3int16
EncodersExt["UDim"] = EncodeUDim
EncodersExt["UDim2"] = EncodeUDim2
EncodersExt["CFrame"] = EncodeCFrame
EncodersExt["Color3"] = EncodeColor3
EncodersExt["NumberRange"] = EncodeNumberRange
EncodersExt["Rect"] = EncodeRect
EncodersExt["EnumItem"] = EncodeEnumItem

local sharedBuffer = buffer.create(0x1000) -- 4k

local compactStream = Stream(sharedBuffer)
compactStream.Encoders = Encoders
local function Compact(v: any, nullValue: any?): string
	local stream = compactStream
	stream.Pos = 0
	stream.Null = nullValue
	EncodeAny(stream, v)
	return ToString(stream)
end

local prettyStream = Stream(sharedBuffer)
prettyStream.Encoders = Encoders
prettyStream.Pretty = true
local function Pretty(v: any, nullValue: any?): string
	local stream = prettyStream
	stream.Pos = 0
	stream.Indent = 0
	stream.Null = nullValue
	EncodeAny(stream, v)
	return ToString(stream)
end

local compactExtStream = Stream(sharedBuffer)
compactExtStream.Encoders = EncodersExt
local function CompactExt(v: any, nullValue: any?): string
	local stream = compactExtStream
	stream.Pos = 0
	stream.Null = nullValue
	EncodeAny(stream, v)
	return ToString(stream)
end

local prettyExtStream = Stream(sharedBuffer)
prettyExtStream.Encoders = EncodersExt
prettyExtStream.Pretty = true
local function PrettyExt(v: any, nullValue: any?): string
	local stream = prettyExtStream
	stream.Pos = 0
	stream.Indent = 0
	stream.Null = nullValue
	EncodeAny(stream, v)
	return ToString(stream)
end

-- JSON5
local compact5Stream = Stream(sharedBuffer)
compact5Stream.Encoders = Encoders
compact5Stream.UnquoteIdent = true
compact5Stream.QuoteChar = 0x27
local function Compact5(v: any, nullValue: any?): string
	local stream = compact5Stream
	stream.Pos = 0
	stream.Null = nullValue
	EncodeAny(stream, v)
	return ToString(stream)
end

local pretty5Stream = Stream(sharedBuffer)
pretty5Stream.Encoders = Encoders
pretty5Stream.Pretty = true
pretty5Stream.UnquoteIdent = true
pretty5Stream.QuoteChar = 0x27
local function Pretty5(v: any, nullValue: any?): string
	local stream = pretty5Stream
	stream.Pos = 0
	stream.Indent = 0
	stream.Null = nullValue
	EncodeAny(stream, v)
	return ToString(stream)
end


local module = {
	Compact = Compact,
	Pretty = Pretty,
	CompactExt = CompactExt,
	PrettyExt = PrettyExt,

	Compact5 = Compact5,
	Pretty5 = Pretty5,
}

return table.freeze(module)
