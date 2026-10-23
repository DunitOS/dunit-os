-- Pure layout policy: input is a snapshot of usable work-area dimensions.
local area = workarea
local rectangles = {}
assert(area.count == 0 or area.width >= area.count, 'work area too narrow')
for index = 1, area.count do
    local left = ((index - 1) * area.width) // area.count
    local right = (index * area.width) // area.count
    rectangles[index] = {x = left, y = 0, w = right - left, h = area.height}
end
return {
    rectangles = rectangles,
    animation = {duration_ms = 140, easing = 'ease_out'},
}
