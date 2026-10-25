local dwm=require('dwm')
dwm.on('config::reloaded',function() error('reload probe regression') end)
