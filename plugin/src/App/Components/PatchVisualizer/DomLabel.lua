local SelectionService = game:GetService("Selection")

local Rojo = script:FindFirstAncestor("Rojo")
local Plugin = Rojo.Plugin
local Packages = Rojo.Packages

local Roact = require(Packages.Roact)
local Flipper = require(Packages.Flipper)

local Assets = require(Plugin.Assets)
local Theme = require(Plugin.App.Theme)
local bindingUtil = require(Plugin.App.bindingUtil)

local e = Roact.createElement

local ChangeList = require(script.Parent.ChangeList)
local ClassIcon = require(Plugin.App.Components.ClassIcon)

local ChangeTag = Roact.Component:extend("ChangeTag")

function ChangeTag:init()
	self:setState({
		isHovered = false,
	})
end

function ChangeTag:render()
	local props = self.props
	local isHovered = self.state.isHovered

	return Theme.with(function(theme)
		local tagColor = props.color or theme.SubTextColor
		return e("Frame", {
			Size = UDim2.new(0, 0, 0, 16),
			AutomaticSize = Enum.AutomaticSize.X,
			BackgroundColor3 = tagColor,
			BackgroundTransparency = props.transparency:map(function(t)
				return 0.85 + (0.15 * t)
			end),
			LayoutOrder = props.layoutOrder or 1,
			Active = true,
			[Roact.Event.MouseEnter] = function()
				self:setState({ isHovered = true })
			end,
			[Roact.Event.MouseLeave] = function()
				self:setState({ isHovered = false })
			end,
		}, {
			Corner = e("UICorner", {
				CornerRadius = UDim.new(0, 3),
			}),
			Padding = e("UIPadding", {
				PaddingLeft = UDim.new(0, 5),
				PaddingRight = UDim.new(0, 5),
			}),
			Label = e("TextLabel", {
				Text = props.text,
				BackgroundTransparency = 1,
				FontFace = theme.Font.Thin,
				TextSize = theme.TextSize.Small,
				TextColor3 = tagColor,
				TextXAlignment = Enum.TextXAlignment.Center,
				TextTransparency = props.transparency,
				Size = UDim2.new(0, 0, 1, 0),
				AutomaticSize = Enum.AutomaticSize.X,
			}),
			Tooltip = if props.tooltipText and isHovered
				then e("TextLabel", {
					Text = props.tooltipText,
					BackgroundColor3 = theme.BorderedContainer.BackgroundColor,
					BackgroundTransparency = 0.05,
					TextColor3 = theme.TextColor,
					FontFace = theme.Font.Main,
					TextSize = theme.TextSize.Small,
					Size = UDim2.new(0, 0, 0, 20),
					AutomaticSize = Enum.AutomaticSize.X,
					Position = UDim2.new(0.5, 0, 0, -22),
					AnchorPoint = Vector2.new(0.5, 0),
					ZIndex = 100,
				}, {
					Corner = e("UICorner", {
						CornerRadius = UDim.new(0, 3),
					}),
					Padding = e("UIPadding", {
						PaddingLeft = UDim.new(0, 6),
						PaddingRight = UDim.new(0, 6),
					}),
				})
				else nil,
		})
	end)
end

-- Proportional diff bar showing the ratio of removals/additions/property changes
local function DiffBar(props)
	local removed = props.removed or 0
	local added = props.added or 0
	local propChanges = props.propChanges or 0
	local total = removed + added + propChanges
	if total == 0 then
		return nil
	end

	return Theme.with(function(theme)
		local children = {
			Layout = e("UIListLayout", {
				FillDirection = Enum.FillDirection.Horizontal,
				SortOrder = Enum.SortOrder.LayoutOrder,
			}),
			Corner = e("UICorner", {
				CornerRadius = UDim.new(0, 2),
			}),
		}

		if propChanges > 0 then
			children.SegProp = e("Frame", {
				Size = UDim2.fromScale(propChanges / total, 1),
				BackgroundColor3 = theme.Diff.Property,
				BackgroundTransparency = props.transparency,
				BorderSizePixel = 0,
				LayoutOrder = 1,
			})
		end

		if added > 0 then
			local color = if props.isWhitespaceOnly then theme.Diff.WhitespaceOnly else theme.Diff.Add
			children.SegAdd = e("Frame", {
				Size = UDim2.fromScale(added / total, 1),
				BackgroundColor3 = color,
				BackgroundTransparency = props.transparency,
				BorderSizePixel = 0,
				LayoutOrder = 2,
			})
		end

		if removed > 0 then
			local color = if props.isWhitespaceOnly then theme.Diff.WhitespaceOnly else theme.Diff.Remove
			children.SegRemove = e("Frame", {
				Size = UDim2.fromScale(removed / total, 1),
				BackgroundColor3 = color,
				BackgroundTransparency = props.transparency,
				BorderSizePixel = 0,
				LayoutOrder = 3,
			})
		end

		return e("Frame", {
			Size = UDim2.new(0, 40, 0, 8),
			BackgroundTransparency = 1,
			ClipsDescendants = true,
			LayoutOrder = props.layoutOrder or 10,
		}, children)
	end)
end

-- Radio button option for selection (as a component to track hover state)
local SelectionOption = Roact.Component:extend("SelectionOption")

function SelectionOption:init()
	self:setState({
		isHovered = false,
	})
end

function SelectionOption:render()
	local props = self.props
	local isHovered = self.state.isHovered

	return Theme.with(function(theme)
		local isSelected = props.isSelected
		local isParentOnly = props.isParentOnly
		local hasChildren = props.hasChildren

		local bgColor = if isSelected
			then (if props.optionType == "push"
				then Color3.fromHex("335FFF")
				elseif props.optionType == "pull" then Color3.fromHex("E74C3C")
				else Color3.fromHex("7F8C8D"))
			else theme.BorderedContainer.BackgroundColor
		local textColor = if isSelected then Color3.new(1, 1, 1) else theme.TextColor

		local displayText = props.text
		-- Use wider button for "Studio" text (6 chars vs 4 for others)
		local buttonWidth = if props.text == "Studio" then 44 else 36

		-- Transparency logic with hover effect
		local bgTransparency, txtTransparency
		if isParentOnly then
			-- Parent-only: partially transparent, less so when hovering the button
			bgTransparency = props.transparency:map(function(t)
				return if isHovered then 0.2 + (0.8 * t) else 0.5 + (0.5 * t)
			end)
			txtTransparency = props.transparency:map(function(t)
				return if isHovered then 0 + t else 0.3 + (0.7 * t)
			end)
		else
			-- Normal buttons: show hover effect by reducing transparency
			bgTransparency = props.transparency:map(function(t)
				if isSelected then
					return if isHovered then 0 + (1 * t) else 0.1 + (0.9 * t)
				else
					return if isHovered then 0.5 + (0.5 * t) else 0.7 + (0.3 * t)
				end
			end)
			txtTransparency = props.transparency
		end

		return e("TextButton", {
			Size = UDim2.new(0, buttonWidth, 0, 18),
			BackgroundColor3 = bgColor,
			BackgroundTransparency = bgTransparency,
			BorderSizePixel = 0,
			Text = displayText,
			FontFace = if isSelected then theme.Font.Bold else theme.Font.Main,
			TextSize = theme.TextSize.Small,
			TextColor3 = textColor,
			TextTransparency = txtTransparency,
			LayoutOrder = props.layoutOrder,
			ZIndex = 10,
			[Roact.Event.MouseEnter] = function()
				self:setState({ isHovered = true })
				-- Trigger subtree highlight when this button would apply to children
				if (isParentOnly or (isSelected and hasChildren)) and props.onSubtreeHoverStart then
					props.onSubtreeHoverStart()
				end
			end,
			[Roact.Event.MouseLeave] = function()
				self:setState({ isHovered = false })
				if props.onSubtreeHoverEnd then
					props.onSubtreeHoverEnd()
				end
			end,
			[Roact.Event.Activated] = function()
				if isParentOnly or (isSelected and hasChildren) then
					-- Parent-only or already selected with children: click applies to subtree
					if props.onSubtreeClick then
						props.onSubtreeClick()
					end
				else
					-- Normal click: select this node only
					if props.onClick then
						props.onClick()
					end
				end
			end,
		}, {
			Corner = e("UICorner", {
				CornerRadius = UDim.new(0, 3),
			}),
		})
	end)
end

-- Selection radio group component
local function SelectionRadio(props)
	if not props.visible then
		return nil
	end

	local hasChildren = props.hasChildren
	local isParentOnly = props.isParentOnly

	return e("Frame", {
		Size = UDim2.new(0, 0, 0, 18),
		AutomaticSize = Enum.AutomaticSize.X,
		BackgroundTransparency = 1,
		Position = UDim2.new(1, -5, 0, 3),
		AnchorPoint = Vector2.new(1, 0),
		ZIndex = 10,
	}, {
		Layout = e("UIListLayout", {
			FillDirection = Enum.FillDirection.Horizontal,
			HorizontalAlignment = Enum.HorizontalAlignment.Right,
			VerticalAlignment = Enum.VerticalAlignment.Center,
			SortOrder = Enum.SortOrder.LayoutOrder,
			Padding = UDim.new(0, 4),
		}),
		Studio = e(SelectionOption, {
			text = "Studio",
			optionType = "pull",
			isSelected = props.selection == "pull",
			transparency = props.transparency,
			layoutOrder = 1,
			hasChildren = hasChildren,
			isParentOnly = isParentOnly,
			onSubtreeHoverStart = props.onSubtreeHoverStart,
			onSubtreeHoverEnd = props.onSubtreeHoverEnd,
			onClick = function()
				if props.onSelectionChange then
					props.onSelectionChange(props.nodeId, "pull")
				end
			end,
			onSubtreeClick = function()
				if props.onSubtreeSelectionChange then
					props.onSubtreeSelectionChange(props.nodeId, "pull")
				end
			end,
		}),
		Skip = e(SelectionOption, {
			text = "Skip",
			optionType = "ignore",
			isSelected = props.selection == "ignore",
			transparency = props.transparency,
			layoutOrder = 2,
			hasChildren = hasChildren,
			isParentOnly = isParentOnly,
			onSubtreeHoverStart = props.onSubtreeHoverStart,
			onSubtreeHoverEnd = props.onSubtreeHoverEnd,
			onClick = function()
				if props.onSelectionChange then
					props.onSelectionChange(props.nodeId, "ignore")
				end
			end,
			onSubtreeClick = function()
				if props.onSubtreeSelectionChange then
					props.onSubtreeSelectionChange(props.nodeId, "ignore")
				end
			end,
		}),
		Atlas = e(SelectionOption, {
			text = "Atlas",
			optionType = "push",
			isSelected = props.selection == "push",
			transparency = props.transparency,
			layoutOrder = 3,
			hasChildren = hasChildren,
			isParentOnly = isParentOnly,
			onSubtreeHoverStart = props.onSubtreeHoverStart,
			onSubtreeHoverEnd = props.onSubtreeHoverEnd,
			onClick = function()
				if props.onSelectionChange then
					props.onSelectionChange(props.nodeId, "push")
				end
			end,
			onSubtreeClick = function()
				if props.onSubtreeSelectionChange then
					props.onSubtreeSelectionChange(props.nodeId, "push")
				end
			end,
		}),
	})
end

local Expansion = Roact.Component:extend("Expansion")

function Expansion:render()
	local props = self.props

	if not props.rendered then
		return nil
	end

	return e("Frame", {
		BackgroundTransparency = 1,
		Size = UDim2.new(1, -props.indent, 1, -24),
		Position = UDim2.new(0, props.indent, 0, 24),
	}, {
		ChangeList = e(ChangeList, {
			changes = props.changeList,
			transparency = props.transparency,
			showStringDiff = if props.showStringDiff
				then function(current: string, incoming: string)
					props.showStringDiff(current, incoming, props.instancePath)
				end
				else nil,
			showTableDiff = props.showTableDiff,
		}),
	})
end

local DomLabel = Roact.Component:extend("DomLabel")

function DomLabel:init()
	local initHeight = self.props.elementHeight:getValue()
	self.expanded = initHeight > 24
	self.isMounted = true

	self.motor = Flipper.SingleMotor.new(initHeight)
	self.binding = bindingUtil.fromMotor(self.motor)

	self:setState({
		renderExpansion = self.expanded,
		isHovered = false,
	})
	self.motorStepConnection = self.motor:onStep(function(value)
		if not self.isMounted then
			return
		end

		local renderExpansion = value > 24

		self.props.setElementHeight(value)
		if self.props.updateEvent then
			self.props.updateEvent:Fire()
		end

		self:setState(function(state)
			if state.renderExpansion == renderExpansion then
				return nil
			end

			return {
				renderExpansion = renderExpansion,
			}
		end)
	end)
end

function DomLabel:willUnmount()
	self.isMounted = false
	-- Stop the motor to prevent onStep callbacks after unmount
	self.motor:stop()
	if self.motorStepConnection then
		self.motorStepConnection:disconnect()
		self.motorStepConnection = nil
	end
end

function DomLabel:didUpdate(prevProps)
	-- When parent re-renders it creates new bindings starting at 24, but our motor
	-- may be at expanded height. Sync immediately to prevent VirtualScroller from
	-- positioning elements incorrectly.
	if prevProps.setElementHeight ~= self.props.setElementHeight then
		local currentHeight = self.binding:getValue()
		self.props.setElementHeight(currentHeight)
	end

	if
		prevProps.instance ~= self.props.instance
		or prevProps.patchType ~= self.props.patchType
		or prevProps.name ~= self.props.name
		or prevProps.changeList ~= self.props.changeList
	then
		-- Close the expansion when the domlabel is changed to a different thing
		self.expanded = false
		self.motor:setGoal(Flipper.Spring.new(24, {
			frequency = 5,
			dampingRatio = 1,
		}))
	end
end

function DomLabel:render()
	local props = self.props
	local depth = props.depth or 1

	-- Derive subtree highlight state from binding (no re-renders needed)
	local ancestorIds = props.ancestorIds
	local isSubtreeHighlighted = if props.subtreeHighlightNodeId
		then props.subtreeHighlightNodeId:map(function(highlightId)
			if highlightId == nil then
				return false
			end
			return ancestorIds[highlightId] == true
		end)
		else nil

	return Theme.with(function(theme)
		local color = if props.isWarning
			then theme.Diff.Warning
			elseif props.patchType then theme.Diff.Background[props.patchType]
			else theme.TextColor

		local indent = (depth - 1) * 12 + 15

		-- Line guides help indent depth remain readable
		local lineGuides = {}
		for i = 2, depth do
			if props.depthsComplete[i] then
				continue
			end
			if props.isFinalChild and i == depth then
				-- This line stops halfway down to merge with our connector for the right angle
				lineGuides["Line_" .. i] = e("Frame", {
					Size = UDim2.new(0, 2, 0, 15),
					Position = UDim2.new(0, (12 * (i - 1)) + 6, 0, -1),
					BorderSizePixel = 0,
					BackgroundTransparency = props.transparency,
					BackgroundColor3 = theme.BorderedContainer.BorderColor,
				})
			else
				-- All other lines go all the way
				-- with the exception of the final element, which stops halfway down
				lineGuides["Line_" .. i] = e("Frame", {
					Size = UDim2.new(0, 2, 1, if props.isFinalElement then -9 else 2),
					Position = UDim2.new(0, (12 * (i - 1)) + 6, 0, -1),
					BorderSizePixel = 0,
					BackgroundTransparency = props.transparency,
					BackgroundColor3 = theme.BorderedContainer.BorderColor,
				})
			end
		end

		if depth ~= 1 then
			lineGuides["Connector"] = e("Frame", {
				Size = UDim2.new(0, 8, 0, 2),
				Position = UDim2.new(0, 2 + (12 * props.depth), 0, 12),
				AnchorPoint = Vector2.xAxis,
				BorderSizePixel = 0,
				BackgroundTransparency = props.transparency,
				BackgroundColor3 = theme.BorderedContainer.BorderColor,
			})
		end

		return e("Frame", {
			ClipsDescendants = true,
			BackgroundTransparency = if props.elementIndex % 2 == 0 then 0.985 else 1,
			BackgroundColor3 = theme.Diff.Row,
			Size = self.binding:map(function(expand)
				return UDim2.new(1, 0, 0, expand)
			end),
		}, {
			Padding = e("UIPadding", {
				PaddingLeft = UDim.new(0, 10),
				PaddingRight = UDim.new(0, 10),
			}),
			Button = e("Frame", {
				Active = true, -- Required for InputBegan to work on Frames
				BackgroundTransparency = 1,
				Size = UDim2.new(1, 0, 1, 0),
				[Roact.Event.MouseEnter] = function()
					self:setState({ isHovered = true })
				end,
				[Roact.Event.MouseLeave] = function()
					self:setState({ isHovered = false })
				end,
				[Roact.Event.InputBegan] = function(_rbx: Instance, input: InputObject)
					if input.UserInputType ~= Enum.UserInputType.MouseButton1 then
						return
					end

					-- Check for double click
					local now = os.clock()
					local lastClickTime = self.lastClickTime or 0
					self.lastClickTime = now

					if now - lastClickTime < 0.3 then
						-- Double click opens the instance in explorer
						self.lastDoubleClickTime = now
						if props.instance then
							SelectionService:Set({ props.instance })
						end
					else
						-- Single click expands the changes (after a delay to check for double click)
						task.delay(0.3, function()
							-- Guard against unmounted component or stale click
							if not self.isMounted then
								return
							end

							if os.clock() - (self.lastDoubleClickTime or 0) <= 0.35 then
								-- This was a double click, so don't expand
								return
							end

							-- Use self.props to get current props, not stale closure
							local p = self.props

							-- Removed script: show diff of old source vs empty
							if
								p.patchType == "Remove"
								and p.instance
								and p.changeInfo
								and p.changeInfo.linesRemoved
								and p.showStringDiff
							then
								local ok, source = pcall(function()
									return (p.instance :: any).Source
								end)
								if ok and type(source) == "string" then
									p.showStringDiff(source, "", p.instancePath)
									return
								end
							end

							-- Added script: find Source in changeList and show diff of empty vs new source
							if
								p.patchType == "Add"
								and p.changeList
								and p.changeInfo
								and p.changeInfo.linesAdded
								and p.showStringDiff
							then
								for _, entry in p.changeList do
									if entry[1] == "Source" and type(entry[3]) == "string" then
										p.showStringDiff("", tostring(entry[3]), p.instancePath)
										return
									end
								end
							end

							if p.changeList then
								-- If the only change is Source, open the diff viewer directly
								-- changeList[1] is the header, changeList[2+] are entries
								local cl = p.changeList
								if #cl == 2 and cl[2][1] == "Source" and p.showStringDiff then
									p.showStringDiff(tostring(cl[2][2]), tostring(cl[2][3]), p.instancePath)
								else
									self.expanded = not self.expanded
									local goalHeight = 24
										+ (if self.expanded then math.clamp(#p.changeList * 24, 24, 24 * 6) else 0)
									self.motor:setGoal(Flipper.Spring.new(goalHeight, {
										frequency = 5,
										dampingRatio = 1,
									}))
								end
							end
						end)
					end
				end,
			}),
			Expansion = if props.changeList
				then e(Expansion, {
					rendered = self.state.renderExpansion,
					indent = indent,
					transparency = props.transparency,
					changeList = props.changeList,
					showStringDiff = props.showStringDiff,
					showTableDiff = props.showTableDiff,
				})
				else nil,
			DiffIcon = if props.patchType
				then e("ImageLabel", {
					Image = Assets.Images.Diff[props.patchType],
					ImageColor3 = color,
					ImageTransparency = props.transparency,
					BackgroundTransparency = 1,
					Size = UDim2.new(0, 14, 0, 14),
					Position = UDim2.new(0, 0, 0, 12),
					AnchorPoint = Vector2.new(0, 0.5),
				})
				else nil,
			ClassIcon = e(ClassIcon, {
				className = props.className,
				color = color,
				transparency = props.transparency,
				size = UDim2.new(0, 16, 0, 16),
				position = UDim2.new(0, indent + 2, 0, 12),
				anchorPoint = Vector2.new(0, 0.5),
			}),
			InstanceName = e("TextLabel", {
				Text = (if props.isWarning then "⚠ " else "") .. props.name,
				RichText = true,
				BackgroundTransparency = 1,
				FontFace = if props.patchType then theme.Font.Bold else theme.Font.Main,
				TextSize = theme.TextSize.Body,
				TextColor3 = color,
				TextXAlignment = Enum.TextXAlignment.Left,
				TextTransparency = props.transparency,
				TextTruncate = Enum.TextTruncate.AtEnd,
				Size = UDim2.new(1, -indent - 50, 0, 24),
				Position = UDim2.new(0, indent + 22, 0, 0),
			}),
			ChangeInfo = e("Frame", {
				BackgroundTransparency = 1,
				Size = UDim2.new(1, -indent - (if props.patchType and props.onSelectionChange then 210 else 80), 0, 24),
				Position = UDim2.new(1, if props.patchType and props.onSelectionChange then -130 else -2, 0, 0),
				AnchorPoint = Vector2.new(1, 0),
				ClipsDescendants = true,
			}, {
				Layout = e("UIListLayout", {
					FillDirection = Enum.FillDirection.Horizontal,
					HorizontalAlignment = Enum.HorizontalAlignment.Right,
					VerticalAlignment = Enum.VerticalAlignment.Center,
					SortOrder = Enum.SortOrder.LayoutOrder,
					Padding = UDim.new(0, 4),
				}),

				-- Git-style line change tags: -M (red) then +N (green)
				-- Always show both when any line change data exists (even -0 or +0).
				-- Grayed when whitespace-only. Always exact counts.
				RemovedTag = if props.changeInfo
						and (props.changeInfo.linesRemoved ~= nil or props.changeInfo.linesAdded ~= nil)
					then e(ChangeTag, {
						text = "-" .. (props.changeInfo.linesRemoved or 0),
						color = if props.changeInfo.isWhitespaceOnly
								or (props.changeInfo.linesRemoved or 0) == 0
							then theme.Diff.WhitespaceOnly
							else theme.Diff.Remove,
						transparency = props.transparency,
						layoutOrder = 3,
						tooltipText = (props.changeInfo.linesRemoved or 0)
							.. (if (props.changeInfo.linesRemoved or 0) == 1 then " line removed" else " lines removed")
							.. (if props.changeInfo.isWhitespaceOnly then " (whitespace only)" else ""),
					})
					else nil,

				AddedTag = if props.changeInfo
						and (props.changeInfo.linesRemoved ~= nil or props.changeInfo.linesAdded ~= nil)
					then e(ChangeTag, {
						text = "+" .. (props.changeInfo.linesAdded or 0),
						color = if props.changeInfo.isWhitespaceOnly
								or (props.changeInfo.linesAdded or 0) == 0
							then theme.Diff.WhitespaceOnly
							else theme.Diff.Add,
						transparency = props.transparency,
						layoutOrder = 2,
						tooltipText = (props.changeInfo.linesAdded or 0)
							.. (if (props.changeInfo.linesAdded or 0) == 1 then " line added" else " lines added")
							.. (if props.changeInfo.isWhitespaceOnly then " (whitespace only)" else ""),
					})
					else nil,

				-- Property changes: compact "NP" format in yellow
				PropsTag = if props.changeInfo and props.changeInfo.propChanges
					then e(ChangeTag, {
						text = props.changeInfo.propChanges .. "P",
						color = theme.Diff.Property,
						transparency = props.transparency,
						layoutOrder = 1,
						tooltipText = props.changeInfo.propChanges .. (if props.changeInfo.propChanges == 1
							then " property change"
							else " property changes") .. " (excluding Source)",
					})
					else nil,

				-- Proportional diff bar
				Bar = if props.changeInfo
						and (
							(props.changeInfo.linesRemoved and props.changeInfo.linesRemoved > 0)
							or (props.changeInfo.linesAdded and props.changeInfo.linesAdded > 0)
							or (props.changeInfo.propChanges and props.changeInfo.propChanges > 0)
						)
					then e(DiffBar, {
						removed = props.changeInfo.linesRemoved or 0,
						added = props.changeInfo.linesAdded or 0,
						propChanges = props.changeInfo.propChanges or 0,
						isWhitespaceOnly = props.changeInfo.isWhitespaceOnly,
						transparency = props.transparency,
						layoutOrder = 4,
					})
					else nil,

				-- Failed changes
				Failed = if props.changeInfo and props.changeInfo.failed
					then e(ChangeTag, {
						text = "Failed " .. props.changeInfo.failed,
						color = theme.Diff.Warning,
						transparency = props.transparency,
						layoutOrder = 5,
						tooltipText = props.changeInfo.failed .. (if props.changeInfo.failed == 1
							then " change failed to apply"
							else " changes failed to apply"),
					})
					else nil,
			}),
			SubtreeHighlight = if isSubtreeHighlighted
				then e("Frame", {
					Size = UDim2.fromScale(1, 1),
					BackgroundColor3 = theme.Diff.SubtreeHighlight,
					BackgroundTransparency = isSubtreeHighlighted:map(function(highlighted)
						return if highlighted then 0.92 else 1
					end),
					BorderSizePixel = 0,
					ZIndex = 0,
				})
				else nil,
			SelectionRadio = e(SelectionRadio, {
				-- Visible for nodes with patchType, or for parent-only nodes on hover
				visible = (props.patchType ~= nil and props.onSelectionChange ~= nil)
					or (
						props.patchType == nil
						and props.hasChildren
						and self.state.isHovered
						and props.onSubtreeSelectionChange ~= nil
					),
				nodeId = props.nodeId,
				selection = props.selection,
				transparency = props.transparency,
				hasChildren = props.hasChildren,
				-- For parent-only nodes (no patchType), only subtree selection makes sense
				isParentOnly = props.patchType == nil,
				onSelectionChange = props.onSelectionChange,
				onSubtreeSelectionChange = props.onSubtreeSelectionChange,
				onSubtreeHoverStart = function()
					if props.setSubtreeHighlightNodeId then
						props.setSubtreeHighlightNodeId(props.nodeId)
					end
				end,
				onSubtreeHoverEnd = function()
					if props.setSubtreeHighlightNodeId then
						props.setSubtreeHighlightNodeId(nil)
					end
				end,
			}),
			LineGuides = e("Folder", nil, lineGuides),
		})
	end)
end

return DomLabel
