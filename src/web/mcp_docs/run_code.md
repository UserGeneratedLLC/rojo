Run a Luau command in Roblox Studio and return the printed output.
Can be used to both make changes and retrieve information.

The code is executed via `loadstring` in the Studio command bar context.
Output from `print()`, `warn()`, and `error()` is captured and returned.
Return values from the code chunk are also included in the output.

The code has full Studio API access — you can read/write properties, create/destroy instances, call services, etc.