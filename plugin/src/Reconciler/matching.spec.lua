return function()
	local Matching = require(script.Parent.matching)

	local function makeVirtualInstance(name, className, props, children)
		return {
			Name = name,
			ClassName = className,
			Properties = props or {},
			Children = children or {},
		}
	end

	local function makeStudioFolder(name, parent)
		local inst = Instance.new("Folder")
		inst.Name = name
		if parent then
			inst.Parent = parent
		end
		return inst
	end

	local function makeStudioPart(name, parent, transparency)
		local inst = Instance.new("Part")
		inst.Name = name
		if transparency then
			inst.Transparency = transparency
		end
		if parent then
			inst.Parent = parent
		end
		return inst
	end

	local function makeStudioModel(name, parent)
		local inst = Instance.new("Model")
		inst.Name = name
		if parent then
			inst.Parent = parent
		end
		return inst
	end

	local container
	beforeEach(function()
		container = Instance.new("Folder")
		container.Name = "MatchingTestContainer"
		container.Parent = workspace
	end)

	afterEach(function()
		if container then
			container:Destroy()
		end
	end)

	describe("session cache", function()
		it("cache hit returns same reference", function()
			local session = Matching.newSession()
			local virtualInstances = {
				A = makeVirtualInstance("Child", "Folder", nil, {}),
			}
			local studioChild = makeStudioFolder("Child", container)

			local parentInst = container

			local result1 = Matching.matchChildren(
				session,
				{ "A" },
				{ studioChild },
				virtualInstances,
				"PARENT",
				parentInst
			)
			local result2 = Matching.matchChildren(
				session,
				{ "A" },
				{ studioChild },
				virtualInstances,
				"PARENT",
				parentInst
			)
			expect(result1).to.equal(result2)
		end)

		it("different parents get different cache entries", function()
			local session = Matching.newSession()
			local virtualInstances = {
				A = makeVirtualInstance("X", "Folder", nil, {}),
				B = makeVirtualInstance("Y", "Folder", nil, {}),
			}
			local studioX = makeStudioFolder("X", container)
			local studioY = makeStudioFolder("Y", container)

			local result1 = Matching.matchChildren(
				session,
				{ "A" },
				{ studioX },
				virtualInstances,
				"PARENT_1",
				container
			)
			local result2 = Matching.matchChildren(
				session,
				{ "B" },
				{ studioY },
				virtualInstances,
				"PARENT_2",
				container
			)
			expect(result1).never.to.equal(result2)
			expect(result1.matched[1].virtualId).to.equal("A")
			expect(result2.matched[1].virtualId).to.equal("B")
		end)

		it("separate sessions are independent", function()
			local session1 = Matching.newSession()
			local session2 = Matching.newSession()
			local virtualInstances = {
				A = makeVirtualInstance("Child", "Folder", nil, {}),
			}
			local studioChild = makeStudioFolder("Child", container)

			local result1 = Matching.matchChildren(session1, { "A" }, { studioChild }, virtualInstances, "P", container)
			local result2 = Matching.matchChildren(session2, { "A" }, { studioChild }, virtualInstances, "P", container)
			expect(result1).never.to.equal(result2)
			expect(result1.totalCost).to.equal(result2.totalCost)
		end)
	end)

	describe("totalCost", function()
		it("zero for identical 1:1 match", function()
			local session = Matching.newSession()
			local virtualInstances = {
				A = makeVirtualInstance("Alpha", "Folder", {}, {}),
			}
			local studioAlpha = makeStudioFolder("Alpha", container)

			local result = Matching.matchChildren(
				session,
				{ "A" },
				{ studioAlpha },
				virtualInstances,
				"ROOT",
				container
			)
			expect(#result.matched).to.equal(1)
			expect(result.totalCost).to.equal(0)
		end)

		it("zero for multiple identical 1:1", function()
			local session = Matching.newSession()
			local virtualInstances = {
				A = makeVirtualInstance("Alpha", "Folder", {}, {}),
				B = makeVirtualInstance("Beta", "Folder", {}, {}),
			}
			local studioAlpha = makeStudioFolder("Alpha", container)
			local studioBeta = makeStudioFolder("Beta", container)

			local result = Matching.matchChildren(
				session,
				{ "A", "B" },
				{ studioAlpha, studioBeta },
				virtualInstances,
				"ROOT",
				container
			)
			expect(#result.matched).to.equal(2)
			expect(result.totalCost).to.equal(0)
		end)

		it("nonzero for property diff", function()
			local session = Matching.newSession()
			local virtualInstances = {
				A = makeVirtualInstance("P", "Part", { Transparency = { Float32 = 0.5 } }, {}),
			}
			local studioPart = makeStudioPart("P", container, 0)

			local result = Matching.matchChildren(session, { "A" }, { studioPart }, virtualInstances, "ROOT", container)
			expect(#result.matched).to.equal(1)
			expect(result.totalCost > 0).to.equal(true)
		end)

		it("unmatched virtual adds penalty", function()
			local session = Matching.newSession()
			local virtualInstances = {
				A = makeVirtualInstance("Alpha", "Folder", {}, {}),
				B = makeVirtualInstance("Beta", "Folder", {}, {}),
			}
			local studioAlpha = makeStudioFolder("Alpha", container)

			local result = Matching.matchChildren(
				session,
				{ "A", "B" },
				{ studioAlpha },
				virtualInstances,
				"ROOT",
				container
			)
			expect(#result.matched).to.equal(1)
			expect(#result.unmatchedVirtual).to.equal(1)
			expect(result.totalCost >= 10000).to.equal(true)
		end)

		it("unmatched studio adds penalty", function()
			local session = Matching.newSession()
			local virtualInstances = {
				A = makeVirtualInstance("Alpha", "Folder", {}, {}),
			}
			local studioAlpha = makeStudioFolder("Alpha", container)
			local studioBeta = makeStudioFolder("Beta", container)

			local result = Matching.matchChildren(
				session,
				{ "A" },
				{ studioAlpha, studioBeta },
				virtualInstances,
				"ROOT",
				container
			)
			expect(#result.matched).to.equal(1)
			expect(#result.unmatchedStudio).to.equal(1)
			expect(result.totalCost >= 10000).to.equal(true)
		end)

		it("sums pair costs and unmatched", function()
			local session = Matching.newSession()
			local virtualInstances = {
				A = makeVirtualInstance("Alpha", "Folder", {}, {}),
				B = makeVirtualInstance("Beta", "Folder", {}, {}),
				C = makeVirtualInstance("Gamma", "Folder", {}, {}),
			}
			local studioAlpha = makeStudioFolder("Alpha", container)
			local studioBeta = makeStudioFolder("Beta", container)

			local result = Matching.matchChildren(
				session,
				{ "A", "B", "C" },
				{ studioAlpha, studioBeta },
				virtualInstances,
				"ROOT",
				container
			)
			expect(#result.matched).to.equal(2)
			expect(#result.unmatchedVirtual).to.equal(1)
			expect(result.totalCost).to.equal(10000)
		end)
	end)

	describe("Ref scoring", function()
		it("Ref match pairs correctly in 2x2 ambiguous", function()
			local session = Matching.newSession()
			local virtualInstances = {
				MODEL_A = makeVirtualInstance("M", "Model", {
					PrimaryPart = { Ref = "HANDLE" },
				}, {}),
				MODEL_B = makeVirtualInstance("M", "Model", {
					PrimaryPart = { Ref = "GRIP" },
				}, {}),
				HANDLE = makeVirtualInstance("Handle", "Part", {}, {}),
				GRIP = makeVirtualInstance("Grip", "Part", {}, {}),
			}

			local studioModelX = makeStudioModel("M", container)
			local studioHandleX = makeStudioPart("Handle", studioModelX, nil)
			studioModelX.PrimaryPart = studioHandleX

			local studioModelY = makeStudioModel("M", container)
			local studioGripY = makeStudioPart("Grip", studioModelY, nil)
			studioModelY.PrimaryPart = studioGripY

			local result = Matching.matchChildren(
				session,
				{ "MODEL_A", "MODEL_B" },
				{ studioModelX, studioModelY },
				virtualInstances,
				"ROOT",
				container
			)
			expect(#result.matched).to.equal(2)

			for _, pair in ipairs(result.matched) do
				if pair.virtualId == "MODEL_A" then
					expect(pair.studioInstance.PrimaryPart.Name).to.equal("Handle")
				elseif pair.virtualId == "MODEL_B" then
					expect(pair.studioInstance.PrimaryPart.Name).to.equal("Grip")
				end
			end
		end)

		it("no Ref properties does not affect scoring", function()
			local session = Matching.newSession()
			local virtualInstances = {
				A = makeVirtualInstance("Data", "Folder", {}, {}),
				B = makeVirtualInstance("Data", "Folder", {}, {}),
			}
			local studioA = makeStudioFolder("Data", container)
			local studioB = makeStudioFolder("Data", container)

			local result = Matching.matchChildren(
				session,
				{ "A", "B" },
				{ studioA, studioB },
				virtualInstances,
				"ROOT",
				container
			)
			expect(#result.matched).to.equal(2)
			expect(result.totalCost).to.equal(0)
		end)
	end)

	describe("ambiguous groups", function()
		it("2x2 same name different children", function()
			local session = Matching.newSession()
			local virtualInstances = {
				FOLDER_A = makeVirtualInstance("Data", "Folder", {}, { "CHILD_A" }),
				FOLDER_B = makeVirtualInstance("Data", "Folder", {}, { "CHILD_B" }),
				CHILD_A = makeVirtualInstance("Alpha", "Folder", {}, {}),
				CHILD_B = makeVirtualInstance("Beta", "Folder", {}, {}),
			}

			local studioFolderX = makeStudioFolder("Data", container)
			makeStudioFolder("Alpha", studioFolderX)

			local studioFolderY = makeStudioFolder("Data", container)
			makeStudioFolder("Beta", studioFolderY)

			local result = Matching.matchChildren(
				session,
				{ "FOLDER_A", "FOLDER_B" },
				{ studioFolderX, studioFolderY },
				virtualInstances,
				"ROOT",
				container
			)
			expect(#result.matched).to.equal(2)

			for _, pair in ipairs(result.matched) do
				local studioChildName = pair.studioInstance:GetChildren()[1].Name
				if pair.virtualId == "FOLDER_A" then
					expect(studioChildName).to.equal("Alpha")
				elseif pair.virtualId == "FOLDER_B" then
					expect(studioChildName).to.equal("Beta")
				end
			end
		end)

		it("3x3 same name different children", function()
			local session = Matching.newSession()
			local virtualInstances = {
				F1 = makeVirtualInstance("Data", "Folder", {}, { "C1" }),
				F2 = makeVirtualInstance("Data", "Folder", {}, { "C2" }),
				F3 = makeVirtualInstance("Data", "Folder", {}, { "C3" }),
				C1 = makeVirtualInstance("One", "Folder", {}, {}),
				C2 = makeVirtualInstance("Two", "Folder", {}, {}),
				C3 = makeVirtualInstance("Three", "Folder", {}, {}),
			}

			local s1 = makeStudioFolder("Data", container)
			makeStudioFolder("One", s1)
			local s2 = makeStudioFolder("Data", container)
			makeStudioFolder("Two", s2)
			local s3 = makeStudioFolder("Data", container)
			makeStudioFolder("Three", s3)

			local result = Matching.matchChildren(
				session,
				{ "F1", "F2", "F3" },
				{ s1, s2, s3 },
				virtualInstances,
				"ROOT",
				container
			)
			expect(#result.matched).to.equal(3)

			for _, pair in ipairs(result.matched) do
				local studioChildName = pair.studioInstance:GetChildren()[1].Name
				if pair.virtualId == "F1" then
					expect(studioChildName).to.equal("One")
				elseif pair.virtualId == "F2" then
					expect(studioChildName).to.equal("Two")
				elseif pair.virtualId == "F3" then
					expect(studioChildName).to.equal("Three")
				end
			end
		end)

		it("identical instances match all with zero cost", function()
			local session = Matching.newSession()
			local virtualInstances = {
				A = makeVirtualInstance("Data", "Folder", {}, {}),
				B = makeVirtualInstance("Data", "Folder", {}, {}),
			}
			local s1 = makeStudioFolder("Data", container)
			local s2 = makeStudioFolder("Data", container)

			local result = Matching.matchChildren(
				session,
				{ "A", "B" },
				{ s1, s2 },
				virtualInstances,
				"ROOT",
				container
			)
			expect(#result.matched).to.equal(2)
			expect(result.totalCost).to.equal(0)
		end)

		it("asymmetric 3v vs 2s", function()
			local session = Matching.newSession()
			local virtualInstances = {
				A = makeVirtualInstance("Data", "Folder", {}, {}),
				B = makeVirtualInstance("Data", "Folder", {}, {}),
				C = makeVirtualInstance("Data", "Folder", {}, {}),
			}
			local s1 = makeStudioFolder("Data", container)
			local s2 = makeStudioFolder("Data", container)

			local result = Matching.matchChildren(
				session,
				{ "A", "B", "C" },
				{ s1, s2 },
				virtualInstances,
				"ROOT",
				container
			)
			expect(#result.matched).to.equal(2)
			expect(#result.unmatchedVirtual).to.equal(1)
		end)
	end)

	describe("recursive matching", function()
		it("nested ambiguous resolved by grandchildren", function()
			local session = Matching.newSession()
			local virtualInstances = {
				OUTER_A = makeVirtualInstance("Data", "Folder", {}, { "INNER_A1", "INNER_A2" }),
				OUTER_B = makeVirtualInstance("Data", "Folder", {}, { "INNER_B1", "INNER_B2" }),
				INNER_A1 = makeVirtualInstance("P", "Part", { Transparency = { Float32 = 0.1 } }, {}),
				INNER_A2 = makeVirtualInstance("P", "Part", { Transparency = { Float32 = 0.2 } }, {}),
				INNER_B1 = makeVirtualInstance("P", "Part", { Transparency = { Float32 = 0.8 } }, {}),
				INNER_B2 = makeVirtualInstance("P", "Part", { Transparency = { Float32 = 0.9 } }, {}),
			}

			local studioOuterX = makeStudioFolder("Data", container)
			makeStudioPart("P", studioOuterX, 0.1)
			makeStudioPart("P", studioOuterX, 0.2)

			local studioOuterY = makeStudioFolder("Data", container)
			makeStudioPart("P", studioOuterY, 0.8)
			makeStudioPart("P", studioOuterY, 0.9)

			local result = Matching.matchChildren(
				session,
				{ "OUTER_A", "OUTER_B" },
				{ studioOuterY, studioOuterX },
				virtualInstances,
				"ROOT",
				container
			)
			expect(#result.matched).to.equal(2)

			for _, pair in ipairs(result.matched) do
				local kids = pair.studioInstance:GetChildren()
				if pair.virtualId == "OUTER_A" then
					expect(kids[1].Transparency < 0.5).to.equal(true)
				elseif pair.virtualId == "OUTER_B" then
					expect(kids[1].Transparency > 0.5).to.equal(true)
				end
			end
		end)
	end)

	describe("edge cases", function()
		it("empty both sides", function()
			local session = Matching.newSession()
			local result = Matching.matchChildren(session, {}, {}, {}, "ROOT", container)
			expect(#result.matched).to.equal(0)
			expect(#result.unmatchedVirtual).to.equal(0)
			expect(#result.unmatchedStudio).to.equal(0)
			expect(result.totalCost).to.equal(0)
		end)

		it("all virtual unmatched", function()
			local session = Matching.newSession()
			local virtualInstances = {
				A = makeVirtualInstance("Alpha", "Folder", {}, {}),
				B = makeVirtualInstance("Beta", "Folder", {}, {}),
			}
			local result = Matching.matchChildren(session, { "A", "B" }, {}, virtualInstances, "ROOT", container)
			expect(#result.matched).to.equal(0)
			expect(#result.unmatchedVirtual).to.equal(2)
			expect(result.totalCost).to.equal(20000)
		end)

		it("all studio unmatched", function()
			local session = Matching.newSession()
			local s1 = makeStudioFolder("Alpha", container)
			local s2 = makeStudioFolder("Beta", container)
			local result = Matching.matchChildren(session, {}, { s1, s2 }, {}, "ROOT", container)
			expect(#result.matched).to.equal(0)
			expect(#result.unmatchedStudio).to.equal(2)
			expect(result.totalCost).to.equal(20000)
		end)

		it("class name discrimination", function()
			local session = Matching.newSession()
			local virtualInstances = {
				A = makeVirtualInstance("Foo", "Folder", {}, {}),
				B = makeVirtualInstance("Foo", "Part", {}, {}),
			}
			local studioFolder = makeStudioFolder("Foo", container)
			local studioPart = makeStudioPart("Foo", container, nil)

			local result = Matching.matchChildren(
				session,
				{ "A", "B" },
				{ studioPart, studioFolder },
				virtualInstances,
				"ROOT",
				container
			)
			expect(#result.matched).to.equal(2)

			for _, pair in ipairs(result.matched) do
				if pair.virtualId == "A" then
					expect(pair.studioInstance.ClassName).to.equal("Folder")
				elseif pair.virtualId == "B" then
					expect(pair.studioInstance.ClassName).to.equal("Part")
				end
			end
		end)

		it("single child each side", function()
			local session = Matching.newSession()
			local virtualInstances = {
				A = makeVirtualInstance("Only", "Folder", {}, {}),
			}
			local studioOnly = makeStudioFolder("Only", container)

			local result = Matching.matchChildren(session, { "A" }, { studioOnly }, virtualInstances, "ROOT", container)
			expect(#result.matched).to.equal(1)
			expect(result.matched[1].virtualId).to.equal("A")
			expect(result.matched[1].studioInstance).to.equal(studioOnly)
			expect(result.totalCost).to.equal(0)
		end)
	end)

	describe("parity with Rust fixture", function()
		it("4 Parts with different Transparency values in reversed order", function()
			local session = Matching.newSession()
			local transparencies = { 0, 0.3, 0.6, 0.9 }

			local virtualInstances = {}
			local virtualChildren = {}
			for i, t in ipairs(transparencies) do
				local id = "V" .. tostring(i)
				virtualInstances[id] = makeVirtualInstance("Line", "Part", {
					Transparency = { Float32 = t },
				}, {})
				table.insert(virtualChildren, id)
			end

			local studioChildren = {}
			for i = #transparencies, 1, -1 do
				local inst = Instance.new("Part")
				inst.Name = "Line"
				inst.Transparency = transparencies[i]
				inst.Parent = container
				table.insert(studioChildren, inst)
			end

			local result =
				Matching.matchChildren(session, virtualChildren, studioChildren, virtualInstances, "ROOT", container)

			expect(#result.matched).to.equal(4)
			expect(#result.unmatchedVirtual).to.equal(0)
			expect(#result.unmatchedStudio).to.equal(0)

			for _, pair in ipairs(result.matched) do
				local vInst = virtualInstances[pair.virtualId]
				local vT = vInst.Properties.Transparency.Float32
				local sT = pair.studioInstance.Transparency
				expect(math.abs(vT - sT) < 0.001).to.equal(true)
			end
		end)
	end)
end
