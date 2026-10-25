local dwm = require('dwm')
local curves = require('animation')
assert(curves == animation and curves == dwm.animation)
curves.register('custom', function(t) return t end)
dwm.on('window::created', function(win)
    assert(win.id == 7)
    dwm.animate(win, { property = 'opacity', from = 0.25, to = 1,
        duration = 250, easing = 'custom' })
end)
