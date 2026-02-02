local Rojo = script:FindFirstAncestor("Rojo")
local Plugin = Rojo.Plugin
local Packages = Rojo.Packages

local Roact = require(Packages.Roact)

local PatchTree = require(Plugin.PatchTree)
local PatchSet = require(Plugin.PatchSet)

local Theme = require(Plugin.App.Theme)
local VirtualScroller = require(Plugin.App.Components.VirtualScroller)
local BorderedContainer = require(Plugin.App.Components.BorderedContainer)

local e = Roact.createElement

local DomLabel = require(script.DomLabel)

-- Case-insensitive substring match with wildcard support
-- * = one or more characters (.+ in regex)
-- e.g., "player" matches "PlayerDataSync", "MyPlayer", etc.
-- e.g., "player*sync" matches "PlayerDataSync" but not "PlayerSync"
local function matchesFilter(name: string, filter: string): boolean
	if filter == "" then
		return true
	end

	-- Escape Lua pattern special chars except *
	local escaped = filter:gsub("([%.%+%-%?%^%$%(%)%[%]%%])", "%%%1")
	-- Convert * to .+ (one or more characters)
	local pattern = escaped:gsub("%*", ".+")

	return string.match(name:lower(), pattern:lower()) ~= nil
end

-- Check if node or any of its descendants match the filter
local function nodeOrDescendantMatches(node, filter: string): boolean
	if matchesFilter(node.name or "", filter) then
		return true
	end

	if node.children then
		for _, child in node.children do
			if type(child) == "table" and nodeOrDescendantMatches(child, filter) then
				return true
			end
		end
	end

	return false
end

local PatchVisualizer = Roact.Component:extend("PatchVisualizer")

function PatchVisualizer:init()
	self.contentSize, self.setContentSize = Roact.createBinding(Vector2.new(0, 0))

	self.updateEvent = Instance.new("BindableEvent")
end

function PatchVisualizer:willUnmount()
	self.updateEvent:Destroy()
end

function PatchVisualizer:shouldUpdate(nextProps)
	if self.props.patchTree ~= nextProps.patchTree then
		return true
	end

	if self.props.selections ~= nextProps.selections then
		return true
	end

	if self.props.filterText ~= nextProps.filterText then
		return true
	end

	local currentPatch, nextPatch = self.props.patch, nextProps.patch
	if currentPatch ~= nil or nextPatch ~= nil then
		return not PatchSet.isEqual(currentPatch, nextPatch)
	end

	return false
end

function PatchVisualizer:render()
	local patchTree = self.props.patchTree
	if patchTree == nil and self.props.patch ~= nil then
		patchTree = PatchTree.build(
			self.props.patch,
			self.props.instanceMap,
			self.props.changeListHeaders or { "Property", "Current", "Incoming" }
		)
		if self.props.unappliedPatch then
			patchTree =
				PatchTree.updateMetadata(patchTree, self.props.patch, self.props.instanceMap, self.props.unappliedPatch)
		end
	end

	-- Recursively draw tree
	local scrollElements, elementHeights, elementIndex = {}, {}, 0
	local filterText = self.props.filterText or ""

	if patchTree then
		-- First pass: collect visible nodes in proper tree order (depth-first, alphabetical)
		local visibleNodes = {}

		local function collectVisibleNodes(node, depth)
			local shouldShow = filterText == "" or nodeOrDescendantMatches(node, filterText)

			if shouldShow then
				table.insert(visibleNodes, { node = node, depth = depth })

				-- Process children in alphabetical order (depth-first)
				if node.children then
					local sortedChildren = {}
					for _, child in node.children do
						if type(child) == "table" then
							table.insert(sortedChildren, child)
						end
					end
					table.sort(sortedChildren, function(a, b)
						return (a.name or "") < (b.name or "")
					end)

					for _, child in ipairs(sortedChildren) do
						collectVisibleNodes(child, depth + 1)
					end
				end
			end
		end

		-- Collect from root children in alphabetical order
		local rootChildren = {}
		for _, child in patchTree.ROOT.children do
			if type(child) == "table" then
				table.insert(rootChildren, child)
			end
		end
		table.sort(rootChildren, function(a, b)
			return (a.name or "") < (b.name or "")
		end)

		for _, child in ipairs(rootChildren) do
			collectVisibleNodes(child, 1)
		end

		local elementTotal = #visibleNodes
		local depthsComplete = {}

		local function drawNode(node, depth, nodeIndex)
			elementIndex += 1

			-- Check if this is the final visible child among siblings
			-- Look ahead to see if there's another node at the same depth with the same parent
			local isFinalChild = true
			for i = nodeIndex + 1, #visibleNodes do
				local nextEntry = visibleNodes[i]
				if nextEntry.depth < depth then
					-- Went back up the tree, we're done checking
					break
				end
				if nextEntry.depth == depth and nextEntry.node.parentId == node.parentId then
					-- Found a sibling at the same depth
					isFinalChild = false
					break
				end
			end

			local elementHeight, setElementHeight = Roact.createBinding(24)
			elementHeights[elementIndex] = elementHeight
			scrollElements[elementIndex] = e(DomLabel, {
				transparency = self.props.transparency,
				showStringDiff = self.props.showStringDiff,
				showTableDiff = self.props.showTableDiff,
				updateEvent = self.updateEvent,
				elementHeight = elementHeight,
				setElementHeight = setElementHeight,
				elementIndex = elementIndex,
				isFinalElement = elementIndex == elementTotal,
				depth = depth,
				depthsComplete = table.clone(depthsComplete),
				hasChildren = (node.children ~= nil and next(node.children) ~= nil),
				isFinalChild = isFinalChild,
				patchType = node.patchType,
				className = node.className,
				isWarning = node.isWarning,
				instance = node.instance,
				name = node.name,
				changeInfo = node.changeInfo,
				changeList = node.changeList,
				-- Selection props
				nodeId = node.id,
				selection = self.props.selections and self.props.selections[node.id],
				onSelectionChange = self.props.onSelectionChange,
				onSubtreeSelectionChange = self.props.onSubtreeSelectionChange,
			})

			if isFinalChild then
				depthsComplete[depth] = true
			end
		end

		-- Draw all visible nodes in order
		for i, entry in ipairs(visibleNodes) do
			local depth = entry.depth
			depthsComplete[depth] = false
			for j = depth + 1, #depthsComplete do
				depthsComplete[j] = nil
			end

			drawNode(entry.node, depth, i)
		end
	end

	return Theme.with(function(theme)
		return e(BorderedContainer, {
			transparency = self.props.transparency,
			size = self.props.size,
			position = self.props.position,
			anchorPoint = self.props.anchorPoint,
			layoutOrder = self.props.layoutOrder,
		}, {
			CleanMerge = e("TextLabel", {
				Visible = #scrollElements == 0,
				Text = if filterText ~= ""
					then "No items match the filter."
					else "No changes to sync, project is up to date.",
				FontFace = theme.Font.Main,
				TextSize = theme.TextSize.Medium,
				TextColor3 = theme.TextColor,
				TextWrapped = true,
				Size = UDim2.new(1, 0, 1, 0),
				BackgroundTransparency = 1,
			}),

			VirtualScroller = e(VirtualScroller, {
				size = UDim2.new(1, 0, 1, -2),
				position = UDim2.new(0, 0, 0, 2),
				transparency = self.props.transparency,
				count = #scrollElements,
				updateEvent = self.updateEvent.Event,
				render = function(i)
					return scrollElements[i]
				end,
				getHeightBinding = function(i)
					return elementHeights[i]
				end,
			}),
		})
	end)
end

return PatchVisualizer
