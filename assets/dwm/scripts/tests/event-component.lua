local dwm = require('dwm')
dwm.on('panel::hover', function(event)
    dwm.animate {window = 0, target = event.target, property = 'opacity',
        from = 0, to = 0.5, duration = 140, easing = 'linear'}
end)
