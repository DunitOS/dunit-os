local dwm = require('dwm')
dwm.on('window::created', function(win)
    dwm.animate(win, {property = 'width', from = 0, to = 800,
        duration = 200, easing = 'linear'})
end)
