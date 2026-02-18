return function()
	local trueEquals = require(script.Parent.trueEquals)

	local NULL_REF = "00000000000000000000000000000000"

	describe("identity / rawequal", function()
		it("same table reference", function()
			local t = { a = 1 }
			expect(trueEquals(t, t)).to.equal(true)
		end)

		it("same string", function()
			local s = "hello"
			expect(trueEquals(s, s)).to.equal(true)
		end)

		it("same number", function()
			expect(trueEquals(42, 42)).to.equal(true)
		end)

		it("same boolean", function()
			expect(trueEquals(true, true)).to.equal(true)
		end)

		it("same Vector3 reference", function()
			local v = Vector3.new(1, 2, 3)
			expect(trueEquals(v, v)).to.equal(true)
		end)

		it("same Instance reference", function()
			local inst = Instance.new("Folder")
			expect(trueEquals(inst, inst)).to.equal(true)
			inst:Destroy()
		end)
	end)

	describe("nil handling", function()
		it("nil vs nil", function()
			expect(trueEquals(nil, nil)).to.equal(true)
		end)

		it("nil vs non-nil", function()
			expect(trueEquals(nil, 1)).to.equal(false)
		end)

		it("non-nil vs nil", function()
			expect(trueEquals(1, nil)).to.equal(false)
		end)

		it("nil vs null-ref table", function()
			expect(trueEquals(nil, { Ref = NULL_REF })).to.equal(true)
		end)

		it("null-ref table vs nil", function()
			expect(trueEquals({ Ref = NULL_REF }, nil)).to.equal(true)
		end)

		it("nil vs non-null ref", function()
			expect(trueEquals(nil, { Ref = "abc123" })).to.equal(false)
		end)

		it("nil vs null-ref with extra keys still matches", function()
			expect(trueEquals(nil, { Ref = NULL_REF, extra = 1 })).to.equal(true)
		end)

		it("nil vs empty table", function()
			expect(trueEquals(nil, {})).to.equal(false)
		end)

		it("nil vs table with wrong key", function()
			expect(trueEquals(nil, { NotRef = NULL_REF })).to.equal(false)
		end)
	end)

	describe("numbers -- absolute epsilon", function()
		it("0 vs 0", function()
			expect(trueEquals(0, 0)).to.equal(true)
		end)

		it("0 vs 0.00005 within epsilon", function()
			expect(trueEquals(0, 0.00005)).to.equal(true)
		end)

		it("0 vs 0.0001 at boundary (strict less-than)", function()
			expect(trueEquals(0, 0.0001)).to.equal(false)
		end)

		it("0 vs 0.00009999 just under", function()
			expect(trueEquals(0, 0.00009999)).to.equal(true)
		end)

		it("0 vs -0.00005 negative within", function()
			expect(trueEquals(0, -0.00005)).to.equal(true)
		end)

		it("0 vs 0.001 beyond", function()
			expect(trueEquals(0, 0.001)).to.equal(false)
		end)
	end)

	describe("numbers -- relative epsilon", function()
		it("1000 vs 1000.05 within relative", function()
			expect(trueEquals(1000, 1000.05)).to.equal(true)
		end)

		it("1000 vs 1000.5 beyond relative", function()
			expect(trueEquals(1000, 1000.5)).to.equal(false)
		end)

		it("1e6 vs 1e6+50 within relative", function()
			expect(trueEquals(1000000, 1000050)).to.equal(true)
		end)

		it("1e6 vs 1e6+200 beyond relative", function()
			expect(trueEquals(1000000, 1000200)).to.equal(false)
		end)
	end)

	describe("numbers -- negative", function()
		it("-5 vs -5.00005 within", function()
			expect(trueEquals(-5, -5.00005)).to.equal(true)
		end)

		it("-5 vs -5.001 beyond", function()
			expect(trueEquals(-5, -5.001)).to.equal(false)
		end)

		it("-1 vs 1 different", function()
			expect(trueEquals(-1, 1)).to.equal(false)
		end)
	end)

	describe("numbers -- special values", function()
		it("0 vs -0", function()
			expect(trueEquals(0, -0)).to.equal(true)
		end)

		it("huge vs huge", function()
			expect(trueEquals(math.huge, math.huge)).to.equal(true)
		end)

		it("-huge vs -huge", function()
			expect(trueEquals(-math.huge, -math.huge)).to.equal(true)
		end)

		it("huge vs -huge", function()
			expect(trueEquals(math.huge, -math.huge)).to.equal(false)
		end)
	end)

	describe("NaN", function()
		it("NaN vs NaN", function()
			expect(trueEquals(0 / 0, 0 / 0)).to.equal(true)
		end)

		it("NaN vs 0", function()
			expect(trueEquals(0 / 0, 0)).to.equal(false)
		end)

		it("NaN vs string", function()
			expect(trueEquals(0 / 0, "nan")).to.equal(false)
		end)
	end)

	describe("Color3", function()
		it("black vs black", function()
			expect(trueEquals(Color3.new(0, 0, 0), Color3.new(0, 0, 0))).to.equal(true)
		end)

		it("white vs white", function()
			expect(trueEquals(Color3.new(1, 1, 1), Color3.new(1, 1, 1))).to.equal(true)
		end)

		it("same mid-gray", function()
			expect(trueEquals(Color3.new(0.5, 0.5, 0.5), Color3.new(0.5, 0.5, 0.5))).to.equal(true)
		end)

		it("red vs green", function()
			expect(trueEquals(Color3.new(1, 0, 0), Color3.new(0, 1, 0))).to.equal(false)
		end)

		it("fromRGB same", function()
			expect(trueEquals(Color3.fromRGB(128, 128, 128), Color3.fromRGB(128, 128, 128))).to.equal(true)
		end)

		it("fromRGB one int off", function()
			expect(trueEquals(Color3.fromRGB(128, 128, 128), Color3.fromRGB(129, 128, 128))).to.equal(false)
		end)

		it("near-boundary within epsilon", function()
			expect(trueEquals(Color3.new(0.5, 0, 0), Color3.new(0.50005, 0, 0))).to.equal(true)
		end)

		it("near-boundary beyond epsilon", function()
			expect(trueEquals(Color3.new(0.5, 0, 0), Color3.new(0.502, 0, 0))).to.equal(false)
		end)
	end)

	describe("Vector3", function()
		it("zero vs zero", function()
			expect(trueEquals(Vector3.new(0, 0, 0), Vector3.new(0, 0, 0))).to.equal(true)
		end)

		it("same components", function()
			expect(trueEquals(Vector3.new(1, 2, 3), Vector3.new(1, 2, 3))).to.equal(true)
		end)

		it("within epsilon on Z", function()
			expect(trueEquals(Vector3.new(1, 2, 3), Vector3.new(1, 2, 3.00005))).to.equal(true)
		end)

		it("beyond epsilon on Z", function()
			expect(trueEquals(Vector3.new(1, 2, 3), Vector3.new(1, 2, 3.001))).to.equal(false)
		end)

		it("Z differs by 1", function()
			expect(trueEquals(Vector3.new(1, 2, 3), Vector3.new(1, 2, 4))).to.equal(false)
		end)

		it("negative components", function()
			expect(trueEquals(Vector3.new(-100, -200, -300), Vector3.new(-100, -200, -300))).to.equal(true)
		end)

		it("large values within relative epsilon", function()
			expect(trueEquals(Vector3.new(1e6, 1e6, 1e6), Vector3.new(1e6 + 50, 1e6, 1e6))).to.equal(true)
		end)
	end)

	describe("Vector2", function()
		it("zero vs zero", function()
			expect(trueEquals(Vector2.new(0, 0), Vector2.new(0, 0))).to.equal(true)
		end)

		it("within epsilon on Y", function()
			expect(trueEquals(Vector2.new(1, 2), Vector2.new(1, 2.00005))).to.equal(true)
		end)

		it("Y differs by 1", function()
			expect(trueEquals(Vector2.new(1, 2), Vector2.new(1, 3))).to.equal(false)
		end)
	end)

	describe("CFrame", function()
		it("identity vs identity", function()
			expect(trueEquals(CFrame.identity, CFrame.identity)).to.equal(true)
		end)

		it("translation only same", function()
			expect(trueEquals(CFrame.new(1, 2, 3), CFrame.new(1, 2, 3))).to.equal(true)
		end)

		it("translation Z differs", function()
			expect(trueEquals(CFrame.new(1, 2, 3), CFrame.new(1, 2, 4))).to.equal(false)
		end)

		it("same rotation", function()
			expect(trueEquals(CFrame.Angles(0, math.pi / 4, 0), CFrame.Angles(0, math.pi / 4, 0))).to.equal(true)
		end)

		it("different rotation", function()
			expect(trueEquals(CFrame.Angles(0, math.pi / 4, 0), CFrame.Angles(0, math.pi / 2, 0))).to.equal(false)
		end)

		it("tiny translation perturbation within epsilon", function()
			expect(trueEquals(CFrame.new(1, 2, 3), CFrame.new(1, 2, 3.00005))).to.equal(true)
		end)

		it("constructed from components match", function()
			local cf1 = CFrame.new(1, 2, 3) * CFrame.Angles(0.1, 0.2, 0.3)
			local ax, ay, az, r00, r01, r02, r10, r11, r12, r20, r21, r22 = cf1:GetComponents()
			local cf2 = CFrame.new(ax, ay, az, r00, r01, r02, r10, r11, r12, r20, r21, r22)
			expect(trueEquals(cf1, cf2)).to.equal(true)
		end)

		it("one rotation component perturbed beyond epsilon", function()
			local cf1 = CFrame.new(1, 2, 3) * CFrame.Angles(0.1, 0.2, 0.3)
			local ax, ay, az, r00, r01, r02, r10, r11, r12, r20, r21, r22 = cf1:GetComponents()
			local cf2 = CFrame.new(ax, ay, az, r00, r01 + 0.001, r02, r10, r11, r12, r20, r21, r22)
			expect(trueEquals(cf1, cf2)).to.equal(false)
		end)
	end)

	describe("tables -- deep equality", function()
		it("empty vs empty", function()
			expect(trueEquals({}, {})).to.equal(true)
		end)

		it("same array", function()
			expect(trueEquals({ 1, 2, 3 }, { 1, 2, 3 })).to.equal(true)
		end)

		it("different rawlen", function()
			expect(trueEquals({ 1, 2, 3 }, { 1, 2 })).to.equal(false)
		end)

		it("same rawlen different element", function()
			expect(trueEquals({ 1, 2, 3 }, { 1, 2, 4 })).to.equal(false)
		end)

		it("same dict", function()
			expect(trueEquals({ a = 1 }, { a = 1 })).to.equal(true)
		end)

		it("different dict value", function()
			expect(trueEquals({ a = 1 }, { a = 2 })).to.equal(false)
		end)

		it("extra key in b", function()
			expect(trueEquals({ a = 1 }, { a = 1, b = 2 })).to.equal(false)
		end)

		it("extra key in a", function()
			expect(trueEquals({ a = 1, b = 2 }, { a = 1 })).to.equal(false)
		end)

		it("nested tables equal", function()
			expect(trueEquals({ a = { b = 1 } }, { a = { b = 1 } })).to.equal(true)
		end)

		it("nested tables different", function()
			expect(trueEquals({ a = { b = 1 } }, { a = { b = 2 } })).to.equal(false)
		end)

		it("mixed types in array", function()
			expect(trueEquals({ 1, "two", true }, { 1, "two", true })).to.equal(true)
		end)

		it("fuzzy number values in table", function()
			expect(trueEquals({ x = 1.00005 }, { x = 1 })).to.equal(true)
		end)

		it("encoded property table", function()
			expect(trueEquals({ Vector3 = { 1, 2, 3 } }, { Vector3 = { 1, 2, 3 } })).to.equal(true)
		end)

		it("null ref strings rawequal", function()
			expect(trueEquals({ Ref = NULL_REF }, { Ref = NULL_REF })).to.equal(true)
		end)
	end)

	describe("EnumItem cross-type", function()
		it("EnumItem vs matching number", function()
			expect(trueEquals(Enum.SortOrder.LayoutOrder, Enum.SortOrder.LayoutOrder.Value)).to.equal(true)
		end)

		it("number vs matching EnumItem", function()
			expect(trueEquals(Enum.SortOrder.LayoutOrder.Value, Enum.SortOrder.LayoutOrder)).to.equal(true)
		end)

		it("EnumItem vs wrong number", function()
			expect(trueEquals(Enum.SortOrder.LayoutOrder, 999)).to.equal(false)
		end)

		it("same EnumItem rawequal", function()
			expect(trueEquals(Enum.SortOrder.LayoutOrder, Enum.SortOrder.LayoutOrder)).to.equal(true)
		end)

		it("different EnumItem values", function()
			expect(trueEquals(Enum.SortOrder.LayoutOrder, Enum.SortOrder.Name)).to.equal(false)
		end)
	end)

	describe("type mismatches", function()
		it("string vs number", function()
			expect(trueEquals("hello", 42)).to.equal(false)
		end)

		it("boolean vs number", function()
			expect(trueEquals(true, 1)).to.equal(false)
		end)

		it("string '1' vs number 1", function()
			expect(trueEquals("1", 1)).to.equal(false)
		end)

		it("table vs string", function()
			expect(trueEquals({}, "table")).to.equal(false)
		end)

		it("Vector3 vs table", function()
			expect(trueEquals(Vector3.new(1, 2, 3), { 1, 2, 3 })).to.equal(false)
		end)

		it("Color3 vs Vector3", function()
			expect(trueEquals(Color3.new(1, 0, 0), Vector3.new(1, 0, 0))).to.equal(false)
		end)

		it("true vs false", function()
			expect(trueEquals(true, false)).to.equal(false)
		end)

		it("different strings", function()
			expect(trueEquals("abc", "abd")).to.equal(false)
		end)
	end)
end
