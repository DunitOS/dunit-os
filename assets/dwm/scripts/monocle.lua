-- Overlap is an intentional layout choice; the compositor owns stacking/focus.
local rectangles = {}
for index = 1, workarea.count do
    rectangles[index] = {x = 0, y = 0, w = workarea.width, h = workarea.height}
end
return {
    rectangles = rectangles,
    animation = {duration_ms = 0, easing = 'linear'},
}
