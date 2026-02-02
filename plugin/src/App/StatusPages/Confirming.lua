local Rojo = script:FindFirstAncestor("Rojo")
local Plugin = Rojo.Plugin
local Packages = Rojo.Packages

local Roact = require(Packages.Roact)

local Theme = require(Plugin.App.Theme)
local PatchTree = require(Plugin.PatchTree)
local TextButton = require(Plugin.App.Components.TextButton)
local TextInput = require(Plugin.App.Components.TextInput)
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
		showingAcceptConfirm = false,
		unselectedCount = 0,
		selections = initialSelections,
		filterText = "",
	})

	-- Callback to update individual selection
	self.onSelectionChange = function(nodeId, selection)
		self:setState(function(state)
			local newSelections = table.clone(state.selections)
			newSelections[nodeId] = selection
			return { selections = newSelections }
		end)
	end

	-- Set selection for a node and all its descendants (subtree)
	self.onSubtreeSelectionChange = function(nodeId, selection)
		if not self.props.patchTree then
			return
		end

		local node = self.props.patchTree:getNode(nodeId)
		if not node then
			return
		end

		self:setState(function(state)
			local newSelections = table.clone(state.selections)
			-- Set this node if it has a patchType
			if node.patchType then
				newSelections[nodeId] = selection
			end
			-- Set all descendants
			self.props.patchTree:forEach(function(childNode)
				if childNode.patchType then
					newSelections[childNode.id] = selection
				end
			end, node)
			return { selections = newSelections }
		end)
	end
end

function ConfirmingPage:didUpdate(prevProps)
	-- If patchTree changed, update selections
	if prevProps.patchTree ~= self.props.patchTree and self.props.patchTree then
		-- Get the new items that need default selections
		local newSelections = PatchTree.buildInitialSelections(self.props.patchTree)

		-- Preserve existing selections, but unselect items that changed
		local updatedSelections = {}
		for id, selection in pairs(self.state.selections) do
			-- Preserve selection only if the item wasn't changed
			-- (Changed items must be re-reviewed, so leave them unselected)
			local wasChanged = self.props.changedIds and self.props.changedIds[id]
			if not wasChanged then
				updatedSelections[id] = selection
			end
		end

		-- Merge with any new default selections (new items get nil = unselected)
		for id, selection in pairs(newSelections) do
			if updatedSelections[id] == nil then
				updatedSelections[id] = selection
			end
		end

		self:setState({ selections = updatedSelections })
	end
end

function ConfirmingPage:render()
	-- Check if all items have a selection
	local allSelected = true
	if self.props.patchTree then
		allSelected = PatchTree.allNodesSelected(self.props.patchTree, self.state.selections)
	end

	-- Check if there are any changes at all
	local hasChanges = self.props.patchTree and self.props.patchTree:getCount() > 0

	return Theme.with(function(theme)
		local pageContent = Roact.createFragment({
			Header = e("Frame", {
				Size = UDim2.new(1, 0, 0, 28),
				BackgroundTransparency = 1,
				LayoutOrder = 1,
			}, {
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
					TextYAlignment = Enum.TextYAlignment.Center,
					TextTransparency = self.props.transparency,
					Size = UDim2.new(0.5, -5, 1, 0),
					Position = UDim2.new(0, 0, 0, 0),
					BackgroundTransparency = 1,
				}),

				FilterInput = e(TextInput, {
					text = self.state.filterText,
					placeholder = "Filter",
					enabled = true,
					transparency = self.props.transparency,
					textXAlignment = Enum.TextXAlignment.Left,
					size = UDim2.new(0.25, -5, 1, 0),
					position = UDim2.new(0.75, 5, 0, 0),
					onChanged = function(text)
						self:setState({ filterText = text })
					end,
				}),
			}),

			PatchVisualizer = e(PatchVisualizer, {
				size = UDim2.new(1, 0, 1, -100),
				transparency = self.props.transparency,
				layoutOrder = 2,

				patchTree = self.props.patchTree,
				filterText = self.state.filterText,
				selections = self.state.selections,
				onSelectionChange = self.onSelectionChange,
				onSubtreeSelectionChange = self.onSubtreeSelectionChange,

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
				LayoutOrder = 3,
				BackgroundTransparency = 1,
			}, {
				-- Only show Abort button when there are changes
				Abort = if hasChanges
					then e(TextButton, {
						text = "Abort",
						style = "Bordered",
						transparency = self.props.transparency,
						layoutOrder = 1,
						onClick = self.props.onAbort,
					})
					else nil,

				Accept = e(TextButton, {
					text = if hasChanges then "Accept" else "OK",
					style = "Primary",
					transparency = self.props.transparency,
					layoutOrder = 2,
					onClick = function()
						if not self.props.onConfirm then
							return
						end

						-- Check if all items are selected
						if allSelected then
							-- All items selected, proceed immediately
							self.props.onConfirm(self.state.selections)
						else
							-- Some items unselected, show confirmation popup
							local unselectedCount =
								PatchTree.countUnselected(self.props.patchTree, self.state.selections)
							self:setState({
								showingAcceptConfirm = true,
								unselectedCount = unselectedCount,
							})
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

			-- Accept confirmation popup for unselected items
			AcceptConfirm = e(StudioPluginGui, {
				id = "Rojo_AcceptConfirm",
				title = "Unreviewed Items",
				active = self.state.showingAcceptConfirm,
				isEphemeral = true,

				initDockState = Enum.InitialDockState.Float,
				overridePreviousState = true,
				floatingSize = Vector2.new(400, 150),
				minimumSize = Vector2.new(350, 130),

				zIndexBehavior = Enum.ZIndexBehavior.Sibling,

				onClose = function()
					self:setState({ showingAcceptConfirm = false })
				end,
			}, {
				Content = e("Frame", {
					Size = UDim2.fromScale(1, 1),
					BackgroundColor3 = theme.BackgroundColor,
					BorderSizePixel = 0,
				}, {
					Message = e("TextLabel", {
						Text = string.format(
							"%d item%s not been reviewed.\n\nAccept selected changes and skip the rest?",
							self.state.unselectedCount,
							self.state.unselectedCount == 1 and " has" or "s have"
						),
						FontFace = theme.Font.Main,
						TextSize = theme.TextSize.Body,
						TextColor3 = theme.TextColor,
						TextWrapped = true,
						TextXAlignment = Enum.TextXAlignment.Center,
						Size = UDim2.new(1, -20, 1, -60),
						Position = UDim2.new(0, 10, 0, 10),
						BackgroundTransparency = 1,
					}),

					Buttons = e("Frame", {
						Size = UDim2.new(1, -20, 0, 34),
						Position = UDim2.new(0, 10, 1, -44),
						BackgroundTransparency = 1,
					}, {
						Cancel = e(TextButton, {
							text = "Cancel",
							style = "Bordered",
							transparency = self.props.transparency,
							layoutOrder = 1,
							onClick = function()
								self:setState({ showingAcceptConfirm = false })
							end,
						}),

						AcceptAndSkip = e(TextButton, {
							text = "Accept and Skip Rest",
							style = "Primary",
							transparency = self.props.transparency,
							layoutOrder = 2,
							onClick = function()
								self:setState({ showingAcceptConfirm = false })
								if self.props.onConfirm then
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
