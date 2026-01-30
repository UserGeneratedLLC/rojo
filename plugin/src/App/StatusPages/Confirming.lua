local Rojo = script:FindFirstAncestor("Rojo")
local Plugin = Rojo.Plugin
local Packages = Rojo.Packages

local Roact = require(Packages.Roact)

local Settings = require(Plugin.Settings)
local Theme = require(Plugin.App.Theme)
local PatchTree = require(Plugin.PatchTree)
local TextButton = require(Plugin.App.Components.TextButton)
local StudioPluginGui = require(Plugin.App.Components.Studio.StudioPluginGui)
local Tooltip = require(Plugin.App.Components.Tooltip)
local PatchVisualizer = require(Plugin.App.Components.PatchVisualizer)
local StringDiffVisualizer = require(Plugin.App.Components.StringDiffVisualizer)
local TableDiffVisualizer = require(Plugin.App.Components.TableDiffVisualizer)

local e = Roact.createElement

local ConfirmingPage = Roact.Component:extend("ConfirmingPage")

function ConfirmingPage:init()
	self.contentSize, self.setContentSize = Roact.createBinding(0)
	self.containerSize, self.setContainerSize = Roact.createBinding(Vector2.new(0, 0))

	-- Initialize selections from patchTree defaults
	local initialSelections = {}
	if self.props.patchTree then
		initialSelections = PatchTree.buildInitialSelections(self.props.patchTree)
	end

	self:setState({
		showingStringDiff = false,
		currentString = "",
		incomingString = "",
		showingTableDiff = false,
		oldTable = {},
		newTable = {},
		showingRejectConfirm = false,
		selections = initialSelections,
	})

	-- Callback to update individual selection
	self.onSelectionChange = function(nodeId, selection)
		self:setState(function(state)
			local newSelections = table.clone(state.selections)
			newSelections[nodeId] = selection
			return { selections = newSelections }
		end)
	end

	-- Set all selections to a specific value
	self.setAllSelections = function(value)
		if not self.props.patchTree then
			return
		end
		local newSelections = {}
		self.props.patchTree:forEach(function(node)
			if node.patchType then
				newSelections[node.id] = value
			end
		end)
		self:setState({ selections = newSelections })
	end
end

function ConfirmingPage:didUpdate(prevProps)
	-- If patchTree changed, reinitialize selections
	if prevProps.patchTree ~= self.props.patchTree and self.props.patchTree then
		local initialSelections = PatchTree.buildInitialSelections(self.props.patchTree)
		self:setState({ selections = initialSelections })
	end
end

function ConfirmingPage:render()
	-- Check if all items have a selection
	local allSelected = true
	if self.props.patchTree then
		allSelected = PatchTree.allNodesSelected(self.props.patchTree, self.state.selections)
	end

	return Theme.with(function(theme)
		local pageContent = Roact.createFragment({
			Title = e("TextLabel", {
				Text = string.format(
					"Sync changes for project '%s':",
					self.props.confirmData.serverInfo.projectName or "UNKNOWN"
				),
				FontFace = theme.Font.Thin,
				LineHeight = 1.2,
				TextSize = theme.TextSize.Body,
				TextColor3 = theme.TextColor,
				TextXAlignment = Enum.TextXAlignment.Left,
				TextTransparency = self.props.transparency,
				Size = UDim2.new(1, 0, 0, theme.TextSize.Large + 2),
				BackgroundTransparency = 1,
			}),

			PatchVisualizer = e(PatchVisualizer, {
				size = UDim2.new(1, 0, 1, -100),
				transparency = self.props.transparency,
				layoutOrder = 3,

				patchTree = self.props.patchTree,
				selections = self.state.selections,
				onSelectionChange = self.onSelectionChange,

				showStringDiff = function(currentString: string, incomingString: string)
					self:setState({
						showingStringDiff = true,
						currentString = currentString,
						incomingString = incomingString,
					})
				end,
				showTableDiff = function(oldTable: { [any]: any? }, newTable: { [any]: any? })
					self:setState({
						showingTableDiff = true,
						oldTable = oldTable,
						newTable = newTable,
					})
				end,
			}),

			Buttons = e("Frame", {
				Size = UDim2.new(1, 0, 0, 34),
				LayoutOrder = 4,
				BackgroundTransparency = 1,
			}, {
				Abort = e(TextButton, {
					text = "Abort",
					style = "Bordered",
					transparency = self.props.transparency,
					layoutOrder = 1,
					onClick = self.props.onAbort,
				}),

				PullAll = if Settings:get("twoWaySync")
					then e(TextButton, {
						text = "Pull All",
						style = "Danger",
						transparency = self.props.transparency,
						layoutOrder = 2,
						onClick = function()
							self.setAllSelections("pull")
						end,
					})
					else nil,

				SkipAll = e(TextButton, {
					text = "Skip All",
					style = "Neutral",
					transparency = self.props.transparency,
					layoutOrder = 3,
					onClick = function()
						self.setAllSelections("ignore")
					end,
				}),

				PushAll = e(TextButton, {
					text = "Push All",
					style = "Success",
					transparency = self.props.transparency,
					layoutOrder = 4,
					onClick = function()
						self.setAllSelections("push")
					end,
				}),

				Commit = e(TextButton, {
					text = "Commit",
					style = "Primary",
					transparency = self.props.transparency,
					layoutOrder = 5,
					enabled = allSelected,
					onClick = function()
						if allSelected and self.props.onConfirm then
							self.props.onConfirm(self.state.selections)
						end
					end,
				}),

				Layout = e("UIListLayout", {
					HorizontalAlignment = Enum.HorizontalAlignment.Right,
					FillDirection = Enum.FillDirection.Horizontal,
					SortOrder = Enum.SortOrder.LayoutOrder,
					Padding = UDim.new(0, 10),
				}),
			}),

			Padding = e("UIPadding", {
				PaddingLeft = UDim.new(0, 8),
				PaddingRight = UDim.new(0, 8),
			}),

			Layout = e("UIListLayout", {
				HorizontalAlignment = Enum.HorizontalAlignment.Center,
				VerticalAlignment = Enum.VerticalAlignment.Center,
				FillDirection = Enum.FillDirection.Vertical,
				SortOrder = Enum.SortOrder.LayoutOrder,
				Padding = UDim.new(0, 10),
			}),

			StringDiff = e(StudioPluginGui, {
				id = "Rojo_ConfirmingStringDiff",
				title = "String diff",
				active = self.state.showingStringDiff,
				isEphemeral = true,

				initDockState = Enum.InitialDockState.Float,
				overridePreviousState = true,
				floatingSize = Vector2.new(500, 350),
				minimumSize = Vector2.new(400, 250),

				zIndexBehavior = Enum.ZIndexBehavior.Sibling,

				onClose = function()
					self:setState({
						showingStringDiff = false,
					})
				end,
			}, {
				TooltipsProvider = e(Tooltip.Provider, nil, {
					Tooltips = e(Tooltip.Container, nil),
					Content = e("Frame", {
						Size = UDim2.fromScale(1, 1),
						BackgroundTransparency = 1,
					}, {
						e(StringDiffVisualizer, {
							size = UDim2.new(1, -10, 1, -10),
							position = UDim2.new(0, 5, 0, 5),
							anchorPoint = Vector2.new(0, 0),
							transparency = self.props.transparency,

							currentString = self.state.currentString,
							incomingString = self.state.incomingString,
						}),
					}),
				}),
			}),

			TableDiff = e(StudioPluginGui, {
				id = "Rojo_ConfirmingTableDiff",
				title = "Table diff",
				active = self.state.showingTableDiff,
				isEphemeral = true,

				initDockState = Enum.InitialDockState.Float,
				overridePreviousState = true,
				floatingSize = Vector2.new(500, 350),
				minimumSize = Vector2.new(400, 250),

				zIndexBehavior = Enum.ZIndexBehavior.Sibling,

				onClose = function()
					self:setState({
						showingTableDiff = false,
					})
				end,
			}, {
				TooltipsProvider = e(Tooltip.Provider, nil, {
					Tooltips = e(Tooltip.Container, nil),
					Content = e("Frame", {
						Size = UDim2.fromScale(1, 1),
						BackgroundTransparency = 1,
					}, {
						e(TableDiffVisualizer, {
							size = UDim2.new(1, -10, 1, -10),
							position = UDim2.new(0, 5, 0, 5),
							anchorPoint = Vector2.new(0, 0),
							transparency = self.props.transparency,

							oldTable = self.state.oldTable,
							newTable = self.state.newTable,
						}),
					}),
				}),
			}),
		})

		if self.props.createPopup then
			return e(StudioPluginGui, {
				id = "Rojo_DiffSync",
				title = string.format(
					"Confirm sync for project '%s':",
					self.props.confirmData.serverInfo.projectName or "UNKNOWN"
				),
				active = true,
				isEphemeral = true,

				initDockState = Enum.InitialDockState.Float,
				overridePreviousState = false,
				floatingSize = Vector2.new(500, 350),
				minimumSize = Vector2.new(400, 250),

				zIndexBehavior = Enum.ZIndexBehavior.Sibling,

				onClose = self.props.onAbort,
			}, {
				Tooltips = e(Tooltip.Container, nil),
				Content = e("Frame", {
					Size = UDim2.fromScale(1, 1),
					BackgroundTransparency = 1,
				}, pageContent),
			})
		end

		return pageContent
	end)
end

return ConfirmingPage
