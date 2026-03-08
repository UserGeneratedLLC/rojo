--!strict
--!native
--!optimize 2
--[[
	Minimum-cost bipartite matching via the Hungarian (Kuhn-Munkres) algorithm.

	Given a 1-indexed cost matrix cost[i][j] for `rows` row-agents and `cols`
	column-agents, returns an array of {row, col} pairs representing the
	optimal assignment that minimizes total cost. Handles rectangular matrices
	by padding to square with `padCost`.

	Complexity: O(N^3) where N = max(rows, cols).
]]

local function minCostAssignment(cost: { { number } }, rows: number, cols: number, padCost: number): { { number } }
	if rows == 0 or cols == 0 then
		return {}
	end

	local n = math.max(rows, cols)
	local S = n + 1
	local INF = math.huge

	local c: { { number } } = table.create(n)
	for i = 1, n do
		local row = table.create(n, padCost)
		for j = 1, n do
			if i <= rows and j <= cols then
				row[j] = cost[i][j]
			end
		end
		c[i] = row
	end

	local u = table.create(S, 0)
	local v = table.create(S, 0)
	local p = table.create(S, 0)
	local way = table.create(S, 0)

	for i = 1, n do
		p[S] = i
		local j0 = S
		local minV = table.create(S, INF)
		local used = table.create(S, false)

		while true do
			used[j0] = true
			local i0 = p[j0]
			local delta = INF
			local j1 = S

			for j = 1, n do
				if used[j] then
					continue
				end
				local cur = c[i0][j] - u[i0] - v[j]
				if cur < minV[j] then
					minV[j] = cur
					way[j] = j0
				end
				if minV[j] < delta then
					delta = minV[j]
					j1 = j
				end
			end

			u[p[S]] += delta
			v[S] -= delta
			for j = 1, n do
				if used[j] then
					u[p[j]] += delta
					v[j] -= delta
				else
					minV[j] -= delta
				end
			end

			j0 = j1
			if p[j0] == 0 then
				break
			end
		end

		while true do
			local j1 = way[j0]
			p[j0] = p[j1]
			j0 = j1
			if j0 == S then
				break
			end
		end
	end

	local result: { { number } } = {}
	for j = 1, n do
		if p[j] ~= 0 and p[j] <= rows and j <= cols then
			table.insert(result, { p[j], j })
		end
	end
	return result
end

return minCostAssignment
