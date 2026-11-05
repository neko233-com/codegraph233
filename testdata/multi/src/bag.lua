local M = {}
local Items = require "src.items"

--[[ Bag: inventory facade.
     The comment must not leak declarations. ]]
function M.Open(playerId)
    local bag = Items.ForPlayer(playerId)
    bag:Refresh()
    return bag
end

function M:Close()
    self:Save()
end

local function clamp(v, lo, hi)
    if v < lo then
        return lo
    end
    return hi
end

return M
