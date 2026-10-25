local dwm = require('dwm')
local properties = {
    {'opacity', 0.25, 1}, {'x', -100, 200}, {'y', 20, 300},
    {'width', 100, 800}, {'height', 80, 600}, {'scale', 0.5, 2},
    {'translation_x', -30, 40}, {'translation_y', 10, -20},
    {'blur', 0, 12}, {'shadow', 0, 20}, {'corner_radius', 0, 16},
    {'color', 0xff0000, 0xffffffff},
}
dwm.on('window::created', function(win)
    for _, property in ipairs(properties) do
        dwm.animate(win, {property = property[1], from = property[2], to = property[3],
            duration = 200, easing = 'linear'})
    end
end)
