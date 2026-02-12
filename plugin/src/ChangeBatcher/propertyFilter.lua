--[[
	Property type blacklist for encoding, matching the server-side
	filter_properties_for_meta behavior in src/web/api.rs.

	Types listed here are skipped during property encoding because they
	cannot be serialized to JSON or are meaningless for syncback.
]]

-- Property data types that cannot be encoded standalone.
-- Matches the Variant::Ref / Variant::UniqueId filter in api.rs.
local UNENCODABLE_DATA_TYPES = {
	Ref = true, -- Instance references; need the InstanceMap, not encodable standalone
	UniqueId = true, -- Internal Roblox identifiers; not meaningful for syncback
}

return UNENCODABLE_DATA_TYPES
