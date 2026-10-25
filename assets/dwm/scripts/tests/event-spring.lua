local dwm = require('dwm')
animation.register('spring', function(t)
    return 1 - math.exp(-6*t) * math.cos(12*t)
end)
dwm.on('window::created', function(win)
    dwm.animate(win, {property = 'opacity', from = 0, to = 1,
        duration = 250, easing = 'spring'})
end)
