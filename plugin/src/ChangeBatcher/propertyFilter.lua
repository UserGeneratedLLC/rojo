--[[
	Property type blacklist for encoding, matching the server-side
	filter_properties_for_meta behavior in src/web/api.rs.

	Types listed here are skipped during property encoding because they
	cannot be serialized to JSON or are meaningless for syncback.

	Note: Ref properties are NOT blacklisted here. They are handled
	specially in encodePatchUpdate.lua using the InstanceMap to resolve
	Studio Instance references to server-side Ref IDs.
]]

-- Property data types that cannot be encoded standalone.
local UNENCODABLE_DATA_TYPES = {
	UniqueId = true, -- Internal Roblox identifiers; not meaningful for syncback
}

return UNENCODABLE_DATA_TYPES
