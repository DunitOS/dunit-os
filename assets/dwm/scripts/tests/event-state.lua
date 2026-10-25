local dwm = require('dwm')
local count = 0
dwm.on('config::reloaded', function()
    count = count + 1
    dwm.command('workspace:' .. count)
end)
