return function()
	local DiffUtil = require(script.Parent.DiffUtil)

	describe("countLineDifferences", function()
		it("should return zeros for identical strings", function()
			local result = DiffUtil.countLineDifferences("hello\nworld\n", "hello\nworld\n")
			expect(result.added).to.equal(0)
			expect(result.removed).to.equal(0)
			expect(result.isWhitespaceOnly).to.equal(false)
		end)

		it("should return zeros for empty strings", function()
			local result = DiffUtil.countLineDifferences("", "")
			expect(result.added).to.equal(0)
			expect(result.removed).to.equal(0)
			expect(result.isWhitespaceOnly).to.equal(false)
		end)

		it("should handle nil/non-string inputs gracefully", function()
			local result = DiffUtil.countLineDifferences(nil, "hello")
			expect(result.added).to.equal(0)
			expect(result.removed).to.equal(0)

			result = DiffUtil.countLineDifferences("hello", 42)
			expect(result.added).to.equal(0)
			expect(result.removed).to.equal(0)
		end)

		it("should count pure additions (empty to non-empty)", function()
			local result = DiffUtil.countLineDifferences("", "line1\nline2\nline3\n")
			expect(result.added).to.equal(3)
			expect(result.removed).to.equal(0)
			expect(result.isWhitespaceOnly).to.equal(false)
		end)

		it("should count pure removals (non-empty to empty)", function()
			local result = DiffUtil.countLineDifferences("line1\nline2\nline3\n", "")
			expect(result.added).to.equal(0)
			expect(result.removed).to.equal(3)
			expect(result.isWhitespaceOnly).to.equal(false)
		end)

		it("should count a single modified line as 1 removal + 1 addition", function()
			-- git diff: old line removed, new line added
			local result = DiffUtil.countLineDifferences("hello\n", "world\n")
			expect(result.added).to.equal(1)
			expect(result.removed).to.equal(1)
			expect(result.isWhitespaceOnly).to.equal(false)
		end)

		it("should count multiple modified lines correctly", function()
			local old = "line1\nline2\nline3\n"
			local new = "changed1\nchanged2\nline3\n"
			-- Lines 1 and 2 changed: 2 removals + 2 additions
			local result = DiffUtil.countLineDifferences(old, new)
			expect(result.added).to.equal(2)
			expect(result.removed).to.equal(2)
		end)

		it("should handle mixed additions and removals", function()
			local old = "alpha\nbeta\ngamma\n"
			local new = "alpha\ndelta\ngamma\nepsilon\n"
			-- beta -> delta (1 remove + 1 add), + epsilon (1 add)
			local result = DiffUtil.countLineDifferences(old, new)
			expect(result.added).to.equal(2)
			expect(result.removed).to.equal(1)
		end)

		it("should detect whitespace-only changes (trailing spaces)", function()
			local old = "hello\nworld\n"
			local new = "hello  \nworld  \n"
			local result = DiffUtil.countLineDifferences(old, new)
			expect(result.added).to.equal(2)
			expect(result.removed).to.equal(2)
			expect(result.isWhitespaceOnly).to.equal(true)
		end)

		it("should detect whitespace-only changes (trailing tabs)", function()
			local old = "hello\n"
			local new = "hello\t\n"
			local result = DiffUtil.countLineDifferences(old, new)
			expect(result.isWhitespaceOnly).to.equal(true)
		end)

		it("should detect whitespace-only changes (trailing \\r)", function()
			local old = "hello\n"
			local new = "hello\r\n"
			local result = DiffUtil.countLineDifferences(old, new)
			expect(result.isWhitespaceOnly).to.equal(true)
		end)

		it("should NOT mark as whitespace-only when content changes", function()
			local old = "hello\nworld\n"
			local new = "hello  \nchanged\n"
			local result = DiffUtil.countLineDifferences(old, new)
			expect(result.isWhitespaceOnly).to.equal(false)
		end)

		it("should not produce phantom diffs from trailing newlines", function()
			-- "hello\n" and "hello" should diff the same as identical
			local result = DiffUtil.countLineDifferences("hello\n", "hello")
			expect(result.added).to.equal(0)
			expect(result.removed).to.equal(0)
		end)

		it("should handle single line strings", function()
			local result = DiffUtil.countLineDifferences("old", "new")
			expect(result.added).to.equal(1)
			expect(result.removed).to.equal(1)
		end)

		it("should compute exact counts for large diffs (no overflow)", function()
			local lines = {}
			for i = 1, 150 do
				lines[i] = "line" .. i
			end
			local big = table.concat(lines, "\n") .. "\n"
			local result = DiffUtil.countLineDifferences("", big)
			expect(result.added).to.equal(150)
			expect(result.removed).to.equal(0)
		end)

		it("should handle adding lines at the end", function()
			local old = "A\nB\n"
			local new = "A\nB\nC\nD\n"
			local result = DiffUtil.countLineDifferences(old, new)
			expect(result.added).to.equal(2)
			expect(result.removed).to.equal(0)
		end)

		it("should handle removing lines from the beginning", function()
			local old = "A\nB\nC\nD\n"
			local new = "C\nD\n"
			local result = DiffUtil.countLineDifferences(old, new)
			expect(result.added).to.equal(0)
			expect(result.removed).to.equal(2)
		end)

		it("should handle interleaved changes", function()
			local old = "A\nB\nC\nD\nE\n"
			local new = "A\nX\nC\nY\nE\n"
			-- B -> X, D -> Y: 2 removals + 2 additions
			local result = DiffUtil.countLineDifferences(old, new)
			expect(result.added).to.equal(2)
			expect(result.removed).to.equal(2)
		end)
	end)

	describe("diffLines", function()
		it("should return empty hunks for identical strings", function()
			local result = DiffUtil.diffLines("hello\nworld\n", "hello\nworld\n")
			expect(result.added).to.equal(0)
			expect(result.removed).to.equal(0)
			expect(#result.hunks).to.equal(0)
		end)

		it("should return correct counts matching countLineDifferences", function()
			local old = "alpha\nbeta\ngamma\n"
			local new = "alpha\ndelta\ngamma\nepsilon\n"

			local counts = DiffUtil.countLineDifferences(old, new)
			local full = DiffUtil.diffLines(old, new)

			expect(full.added).to.equal(counts.added)
			expect(full.removed).to.equal(counts.removed)
			expect(full.isWhitespaceOnly).to.equal(counts.isWhitespaceOnly)
		end)

		it("should produce a single hunk for a simple change", function()
			local old = "hello\n"
			local new = "world\n"
			local result = DiffUtil.diffLines(old, new)
			expect(#result.hunks).to.equal(1)

			local hunk = result.hunks[1]
			expect(hunk.oldStart).to.equal(1)
			expect(hunk.oldCount).to.equal(1)
			expect(hunk.newStart).to.equal(1)
			expect(hunk.newCount).to.equal(1)
		end)

		it("should include context lines around changes", function()
			local old = "A\nB\nC\nD\nE\nF\nG\n"
			local new = "A\nB\nC\nX\nE\nF\nG\n"
			-- D -> X in the middle, with 3 lines context on each side
			local result = DiffUtil.diffLines(old, new, 3)
			expect(#result.hunks).to.equal(1)

			local hunk = result.hunks[1]
			-- Should include context: A,B,C (before) + change + E,F,G (after) = all 7 lines
			expect(hunk.oldStart).to.equal(1)
			expect(hunk.newStart).to.equal(1)
		end)

		it("should merge adjacent hunks when their contexts overlap", function()
			-- Changes at lines 2 and 6 with 3 context = gap of 4 (< 2*3=6), should merge
			local old = "A\nB\nC\nD\nE\nF\nG\n"
			local new = "A\nX\nC\nD\nE\nY\nG\n"
			local result = DiffUtil.diffLines(old, new, 3)
			-- B->X and F->Y are only 4 lines apart, contexts overlap -> 1 hunk
			expect(#result.hunks).to.equal(1)
		end)

		it("should produce separate hunks when changes are far apart", function()
			-- Build a file with changes separated by more than 2*contextLines
			local lines = {}
			for i = 1, 20 do
				lines[i] = "line" .. i
			end
			local old = table.concat(lines, "\n") .. "\n"

			lines[1] = "changed1"
			lines[20] = "changed20"
			local new = table.concat(lines, "\n") .. "\n"

			local result = DiffUtil.diffLines(old, new, 3)
			-- Changes at line 1 and 20, gap of 19 > 2*3=6 -> separate hunks
			expect(#result.hunks).to.equal(2)
		end)

		it("should have correct hunk line types", function()
			local old = "A\nB\nC\n"
			local new = "A\nX\nC\n"
			local result = DiffUtil.diffLines(old, new, 1)
			expect(#result.hunks).to.equal(1)

			local hunk = result.hunks[1]
			-- Should have: context A, remove B, add X, context C
			local types = {}
			for _, line in hunk.lines do
				table.insert(types, line.type)
			end

			-- Find the remove and add
			local hasRemove = false
			local hasAdd = false
			local hasContext = false
			for _, t in types do
				if t == "remove" then
					hasRemove = true
				end
				if t == "add" then
					hasAdd = true
				end
				if t == "context" then
					hasContext = true
				end
			end

			expect(hasRemove).to.equal(true)
			expect(hasAdd).to.equal(true)
			expect(hasContext).to.equal(true)
		end)

		it("should handle pure additions in hunks", function()
			local old = "A\nB\n"
			local new = "A\nB\nC\nD\n"
			local result = DiffUtil.diffLines(old, new, 3)
			expect(result.added).to.equal(2)
			expect(result.removed).to.equal(0)
			expect(#result.hunks).to.equal(1)
		end)

		it("should handle pure removals in hunks", function()
			local old = "A\nB\nC\nD\n"
			local new = "A\nB\n"
			local result = DiffUtil.diffLines(old, new, 3)
			expect(result.added).to.equal(0)
			expect(result.removed).to.equal(2)
			expect(#result.hunks).to.equal(1)
		end)

		it("should compute exact hunks for large diffs (no overflow)", function()
			local lines = {}
			for i = 1, 150 do
				lines[i] = "line" .. i
			end
			local big = table.concat(lines, "\n") .. "\n"
			local result = DiffUtil.diffLines("", big)
			expect(result.added).to.equal(150)
			expect(result.removed).to.equal(0)
			expect(#result.hunks).to.equal(1)
		end)

		it("should use custom context lines", function()
			local old = "A\nB\nC\nD\nE\n"
			local new = "A\nB\nX\nD\nE\n"
			-- C -> X with 1 context line
			local result = DiffUtil.diffLines(old, new, 1)
			expect(#result.hunks).to.equal(1)
			local hunk = result.hunks[1]
			-- context B, remove C, add X, context D = 4 lines
			local lineCount = #hunk.lines
			expect(lineCount).to.equal(4)
		end)
	end)

	describe("EditType", function()
		it("should expose frozen edit type constants", function()
			expect(DiffUtil.EditType.Equal).to.equal(0)
			expect(DiffUtil.EditType.Delete).to.equal(1)
			expect(DiffUtil.EditType.Insert).to.equal(2)
		end)
	end)
end
