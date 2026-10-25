local dwm = require('dwm')
dwm.on('window_minimize', function(event)
    dwm.animate { window = event.window.id, effect = 'genie', duration_ms = 420,
        easing = function(t) if t > 0.5 then return 0/0 end return t end }
end)
