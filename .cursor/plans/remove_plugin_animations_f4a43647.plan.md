---
name: Remove Plugin Animations
overview: Replace every `Flipper.Spring.new(...)` with `Flipper.Instant.new(...)` across all plugin components to make all UI transitions instant, and remove the 0.3s double-click delay in DomLabel expand/collapse.
todos:
  - id: domlabel
    content: "DomLabel.lua: Replace 2 Spring goals with Instant, remove task.delay(0.3) wrapper from expand logic"
    status: completed
  - id: page
    content: "Page.lua: Replace Spring goal with Instant"
    status: completed
  - id: touchripple
    content: "TouchRipple.lua: Replace 3 Spring goals with Instant, remove EXPAND_SPRING constant"
    status: completed
  - id: textbutton
    content: "TextButton.lua: Replace 3 Spring goals with Instant, remove SPRING_PROPS constant"
    status: completed
  - id: textinput
    content: "TextInput.lua: Replace 3 Spring goals with Instant, remove SPRING_PROPS constant"
    status: completed
  - id: notification
    content: "Notification.lua: Replace 2 Spring goals with Instant"
    status: completed
  - id: dropdown
    content: "Dropdown.lua: Replace Spring goal with Instant"
    status: completed
  - id: iconbutton
    content: "IconButton.lua: Replace 2 Spring goals with Instant, remove HOVER_SPRING_PROPS constant"
    status: completed
  - id: checkbox
    content: "Checkbox.lua: Replace Spring goal with Instant"
    status: completed
isProject: false
---

# Remove All Animations From Plugin

## Strategy

Every animation in the plugin uses Flipper motors with `Flipper.Spring.new(value, { frequency, dampingRatio })` goals. The Flipper library already has `Flipper.Instant.new(value)` which sets the motor value immediately with no interpolation. The fix is a mechanical replacement of every `Flipper.Spring.new(...)` with `Flipper.Instant.new(...)`.

This preserves the binding/motor infrastructure (no structural refactoring needed) while making every transition instant.

Additionally, the DomLabel expand/collapse has a `task.delay(0.3)` that waits to distinguish single-click from double-click before expanding. This delay should be removed so expansion is immediate on click.

## Files to Change (10 files, ~20 replacements)

### 1. [DomLabel.lua](plugin/src/App/Components/PatchVisualizer/DomLabel.lua) -- Expand/collapse

- Line 436: `Flipper.Spring.new(24, {...})` -> `Flipper.Instant.new(24)`
- Line 602: `Flipper.Spring.new(goalHeight, {...})` -> `Flipper.Instant.new(goalHeight)`
- Lines 545-608: Remove the `task.delay(0.3)` wrapper so expansion logic runs immediately on single click (keep double-click detection, just don't delay the expand)

### 2. [Page.lua](plugin/src/App/Page.lua) -- Page transitions

- Line 65: `Flipper.Spring.new(...)` -> `Flipper.Instant.new(...)`

### 3. [TouchRipple.lua](plugin/src/App/Components/TouchRipple.lua) -- Click ripple effect

- Line 84: `Flipper.Spring.new(1, EXPAND_SPRING)` -> `Flipper.Instant.new(1)`
- Line 85: `Flipper.Spring.new(1, EXPAND_SPRING)` -> `Flipper.Instant.new(1)`
- Line 93: `Flipper.Spring.new(0, {...})` -> `Flipper.Instant.new(0)`
- Remove unused `EXPAND_SPRING` constant

### 4. [TextButton.lua](plugin/src/App/Components/TextButton.lua) -- Hover/enabled

- Line 36: `Flipper.Spring.new(...)` -> `Flipper.Instant.new(...)`
- Line 64: `Flipper.Spring.new(1, SPRING_PROPS)` -> `Flipper.Instant.new(1)`
- Line 70: `Flipper.Spring.new(0, SPRING_PROPS)` -> `Flipper.Instant.new(0)`
- Remove unused `SPRING_PROPS` constant

### 5. [TextInput.lua](plugin/src/App/Components/TextInput.lua) -- Hover/enabled

- Line 34: `Flipper.Spring.new(...)` -> `Flipper.Instant.new(...)`
- Line 101: `Flipper.Spring.new(1, SPRING_PROPS)` -> `Flipper.Instant.new(1)`
- Line 107: `Flipper.Spring.new(0, SPRING_PROPS)` -> `Flipper.Instant.new(0)`
- Remove unused `SPRING_PROPS` constant

### 6. [Notification.lua](plugin/src/App/Components/Notifications/Notification.lua) -- Appear/dismiss

- Line 43: `Flipper.Spring.new(0, {...})` -> `Flipper.Instant.new(0)`
- Line 50: `Flipper.Spring.new(1, {...})` -> `Flipper.Instant.new(1)`

### 7. [Dropdown.lua](plugin/src/App/Components/Dropdown.lua) -- Open/close

- Line 39: `Flipper.Spring.new(...)` -> `Flipper.Instant.new(...)`

### 8. [IconButton.lua](plugin/src/App/Components/IconButton.lua) -- Hover circle

- Line 41: `Flipper.Spring.new(1, HOVER_SPRING_PROPS)` -> `Flipper.Instant.new(1)`
- Line 45: `Flipper.Spring.new(0, HOVER_SPRING_PROPS)` -> `Flipper.Instant.new(0)`
- Remove unused `HOVER_SPRING_PROPS` constant

### 9. [Checkbox.lua](plugin/src/App/Components/Checkbox.lua) -- Active state

- Line 26: `Flipper.Spring.new(...)` -> `Flipper.Instant.new(...)`

## Not Touched

- **Spinner.lua** -- Uses `RenderStepped` rotation, not Flipper springs. This is a loading indicator, not a decorative animation. Keeping it.
- **bindingUtil.lua** -- `mapLerp`, `blendAlpha`, `fromMotor` are utility functions still needed by the instant-value bindings. No changes needed.
- **Flipper package** -- Vendored, not modified.
