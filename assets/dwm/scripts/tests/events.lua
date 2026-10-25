local dwm = require('dwm')
assert(dwm.api_version == 2)
dwm.on('window_minimize', function(event)
    assert(event.name == 'window_minimize' and event.window.id == 7 and event.target == 9)
    dwm.animate { window = event.window.id, target = event.target,
        effect = 'genie', duration_ms = 420, easing = function(t) return t end }
end)
-- Unrelated handlers are registered, but cannot run during this dispatch.
dwm.on('window_focus', function() error('incorrect event dispatch') end)
