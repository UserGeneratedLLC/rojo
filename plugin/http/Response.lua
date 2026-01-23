local HttpService = game:GetService("HttpService")
local JSON5Decoder = require(script.Parent.Parent.json5.JSON5Decoder)

local stringTemplate = [[
Http.Response {
	code: %d
	body: %s
}]]

local Response = {}
Response.__index = Response

function Response:__tostring()
	return stringTemplate:format(self.code, self.body)
end

function Response.fromRobloxResponse(response)
	local self = {
		body = response.Body,
		code = response.StatusCode,
		headers = response.Headers,
	}

	return setmetatable(self, Response)
end

function Response:isSuccess()
	return self.code >= 200 and self.code < 300
end

function Response:json()
	-- Try fast native decode first, fallback to JSON5 for NaN/Infinity
	local success, result = pcall(HttpService.JSONDecode, HttpService, self.body)
	if success then
		return result
	end
	print("[HTTP Response:json()] Native decode failed, using JSON5 for size:", #self.body)
	return JSON5Decoder.Decode(self.body)
end

return Response
