-- Event policies run outside the compositor. The native engine owns frames.
local dwm = require('dwm')
assert(dwm.api_version == 2)
local function minimize(event)
    dwm.animate {
        window = event.window.id,
        target = event.target,
        effect = 'genie',
        duration_ms = 420,
        easing = function(t) return 1 - (1 - t)^3 end,
    }
end
dwm.on('window_minimize', minimize)
dwm.on('window_restore', minimize)
dwm.on('window_open', function(event)
    dwm.animate {
        window = event.window.id, effect = 'scale', duration_ms = 180,
        easing = 'ease_out',
    }
    dwm.animate {window=0, target=dwm.targets.notifications, property='opacity',
        from=0.4, to=1, duration=180, easing='ease_out'}
end)

-- Curves are sampled once. The renderer never executes this function.
dwm.animation.register('spring', function(t)
    return 1 - math.exp(-7*t) * math.cos(12*t)
end)

dwm.on('panel::hover', function(event)
    dwm.animate {window=0, target=event.target, property='opacity',
        from=0.75, to=1, duration=180, easing='ease_out'}
end)

local updates = 0
dwm.on('widget::updated', function(event)
    if event.component == 'activity' then
        updates = updates + 1
        dwm.widget.set(event.target, {text='Lua ' .. updates})
    end
end)
dwm.on('widget::clicked', function(event)
    if event.component == 'activity' then dwm.command('launch:gui_calc') end
end)

-- These targets affect native scene layers, including their input coordinates.
dwm.on('launcher::opened', function()
    dwm.animate {window=0, target=dwm.targets.launcher, property='opacity',
        from=0, to=1, duration=180, easing='ease_out'}
end)
dwm.on('launcher::closed', function()
    dwm.animate {window=0, target=dwm.targets.launcher, property='opacity',
        from=1, to=0, duration=180, easing='ease_out'}
end)
dwm.on('workspace::changed', function()
    dwm.animate {window=0, target=dwm.targets.workspace, property='translation_x',
        from=24, to=0, duration=180, easing='ease_out'}
end)
