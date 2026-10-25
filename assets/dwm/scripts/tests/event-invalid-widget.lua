local dwm = require('dwm')
dwm.on('widget::updated', function()
    dwm.widget.set(0, {text = 'invalid target'})
end)
