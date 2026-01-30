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

local function ChangeTag(props)
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
		})
	end)
end

-- Radio button option for selection
local function SelectionOption(props)
	return Theme.with(function(theme)
		local isSelected = props.isSelected
		local bgColor = if isSelected
			then (if props.optionType == "push"
				then Color3.fromHex("27AE60")
				elseif props.optionType == "pull" then Color3.fromHex("E74C3C")
				else Color3.fromHex("7F8C8D"))
			else theme.BorderedContainer.BackgroundColor
		local textColor = if isSelected then Color3.new(1, 1, 1) else theme.TextColor

		return e("TextButton", {
			Size = UDim2.new(0, 36, 0, 18),
			BackgroundColor3 = bgColor,
			BackgroundTransparency = props.transparency:map(function(t)
				return if isSelected then 0.1 + (0.9 * t) else 0.7 + (0.3 * t)
			end),
			BorderSizePixel = 0,
			Text = props.text,
			FontFace = if isSelected then theme.Font.Bold else theme.Font.Main,
			TextSize = theme.TextSize.Small,
			TextColor3 = textColor,
			TextTransparency = props.transparency,
			LayoutOrder = props.layoutOrder,
			ZIndex = 10,
			[Roact.Event.Activated] = function()
				if props.onClick then
					props.onClick()
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

	return e("Frame", {
		Size = UDim2.new(0, 118, 0, 18),
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
		Pull = e(SelectionOption, {
			text = "Pull",
			optionType = "pull",
			isSelected = props.selection == "pull",
			transparency = props.transparency,
			layoutOrder = 1,
			onClick = function()
				if props.onSelectionChange then
					props.onSelectionChange(props.nodeId, "pull")
				end
			end,
		}),
		Skip = e(SelectionOption, {
			text = "Skip",
			optionType = "ignore",
			isSelected = props.selection == "ignore",
			transparency = props.transparency,
			layoutOrder = 2,
			onClick = function()
				if props.onSelectionChange then
					props.onSelectionChange(props.nodeId, "ignore")
				end
			end,
		}),
		Push = e(SelectionOption, {
			text = "Push",
			optionType = "push",
			isSelected = props.selection == "push",
			transparency = props.transparency,
			layoutOrder = 3,
			onClick = function()
				if props.onSelectionChange then
					props.onSelectionChange(props.nodeId, "push")
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
			showStringDiff = props.showStringDiff,
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
			Button = e("TextButton", {
				BackgroundTransparency = 1,
				Text = "",
				Size = UDim2.new(1, 0, 1, 0),
				[Roact.Event.Activated] = function(_rbx: Instance, _input: InputObject, clickCount: number)
					if clickCount == 1 then
						-- Double click opens the instance in explorer
						self.lastDoubleClickTime = os.clock()
						if props.instance then
							SelectionService:Set({ props.instance })
						end
					elseif clickCount == 0 then
						-- Single click expands the changes
						task.wait(0.25)
						if os.clock() - (self.lastDoubleClickTime or 0) <= 0.25 then
							-- This is a double click, so don't expand
							return
						end

						if props.changeList then
							self.expanded = not self.expanded
							local goalHeight = 24
								+ (if self.expanded then math.clamp(#props.changeList * 24, 24, 24 * 6) else 0)
							self.motor:setGoal(Flipper.Spring.new(goalHeight, {
								frequency = 5,
								dampingRatio = 1,
							}))
						end
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
				LinesTag = if props.changeInfo and props.changeInfo.lineChanges
					then e(ChangeTag, {
						text = (if props.changeInfo.isWhitespaceOnly then "Whitespace " else "Lines ")
							.. (if props.changeInfo.lineChanges >= 10000 then "100+" else props.changeInfo.lineChanges),
						color = if props.changeInfo.isWhitespaceOnly
							then theme.Diff.WhitespaceOnly
							else theme.Diff.Changes,
						transparency = props.transparency,
						layoutOrder = 1,
					})
					else nil,
				PropsTag = if props.changeInfo and props.changeInfo.propChanges
					then e(ChangeTag, {
						text = "Props " .. props.changeInfo.propChanges,
						color = theme.Diff.Changes or theme.Diff.Background.Edit,
						transparency = props.transparency,
						layoutOrder = 2,
					})
					else nil,
				Failed = if props.changeInfo and props.changeInfo.failed
					then e(ChangeTag, {
						text = "Failed " .. props.changeInfo.failed,
						color = theme.Diff.Warning,
						transparency = props.transparency,
						layoutOrder = 3,
					})
					else nil,
			}),
			SelectionRadio = e(SelectionRadio, {
				visible = props.patchType ~= nil and props.onSelectionChange ~= nil,
				nodeId = props.nodeId,
				selection = props.selection,
				transparency = props.transparency,
				onSelectionChange = props.onSelectionChange,
			}),
			LineGuides = e("Folder", nil, lineGuides),
		})
	end)
end

return DomLabel
