--!strict
--!native
--!optimize 2
--[[
	Shared value equality check used by both the diff and matching modules.

	If matching says "equal" but diff says "different" (or vice versa),
	we get phantom changes from cross-paired instances. Both modules MUST
	use this same function.
]]

local NULL_REF: string = "00000000000000000000000000000000"
local EPSILON: number = 0.0001

local function fuzzyEq(a: number, b: number): boolean
	local diff = math.abs(a - b)
	local maxVal = math.max(math.abs(a), math.abs(b), 1)
	return diff < EPSILON or diff < maxVal * EPSILON
end

local function color3Eq(a: Color3, b: Color3): boolean
	return fuzzyEq(a.R, b.R) and fuzzyEq(a.G, b.G) and fuzzyEq(a.B, b.B)
end

local function vector2Eq(a: Vector2, b: Vector2): boolean
	return fuzzyEq(a.X, b.X) and fuzzyEq(a.Y, b.Y)
end

local function vector3Eq(a: Vector3, b: Vector3): boolean
	return fuzzyEq(a.X, b.X) and fuzzyEq(a.Y, b.Y) and fuzzyEq(a.Z, b.Z)
end

local function cframeEq(a: CFrame, b: CFrame): boolean
	local ax, ay, az, aR00, aR01, aR02, aR10, aR11, aR12, aR20, aR21, aR22 = a:GetComponents()
	local bx, by, bz, bR00, bR01, bR02, bR10, bR11, bR12, bR20, bR21, bR22 = b:GetComponents()
	return fuzzyEq(ax, bx)
		and fuzzyEq(ay, by)
		and fuzzyEq(az, bz)
		and fuzzyEq(aR00, bR00)
		and fuzzyEq(aR01, bR01)
		and fuzzyEq(aR02, bR02)
		and fuzzyEq(aR10, bR10)
		and fuzzyEq(aR11, bR11)
		and fuzzyEq(aR12, bR12)
		and fuzzyEq(aR20, bR20)
		and fuzzyEq(aR21, bR21)
		and fuzzyEq(aR22, bR22)
end

local function trueEquals(a: any, b: any): boolean
	if rawequal(a, b) then
		return true
	end

	if a == nil then
		return type(b) == "table" and rawget(b, "Ref") == NULL_REF
	end
	if b == nil then
		return type(a) == "table" and rawget(a, "Ref") == NULL_REF
	end

	local t = typeof(a)
	local tb = typeof(b)
	if t ~= tb then
		if t == "number" and tb == "EnumItem" then
			return a == (b :: EnumItem).Value
		end
		if t == "EnumItem" and tb == "number" then
			return (a :: EnumItem).Value == b
		end
		return false
	end

	if t == "table" then
		if rawlen(a) ~= rawlen(b) then
			return false
		end
		for k, v in next, a do
			local ov = rawget(b, k)
			if ov == nil or not trueEquals(v, ov) then
				return false
			end
		end
		for k in next, b do
			if rawget(a, k) == nil then
				return false
			end
		end
		return true
	end

	if t == "number" then
		return fuzzyEq(a, b)
	end
	if t == "Color3" then
		return color3Eq(a, b)
	end
	if t == "Vector3" then
		return vector3Eq(a, b)
	end
	if t == "Vector2" then
		return vector2Eq(a, b)
	end
	if t == "CFrame" then
		return cframeEq(a, b)
	end

	if a ~= a and b ~= b then
		return true
	end

	return false
end

return trueEquals
