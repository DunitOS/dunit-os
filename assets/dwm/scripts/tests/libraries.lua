assert(math.random == nil and math.randomseed == nil)
assert(os == nil and io == nil and package == nil and debug == nil)
assert(coroutine == nil and math.sin(0) == 0)
local rectangles = {}
for index = 1, workarea.count do
    local left = math.floor((index - 1) * workarea.width / workarea.count)
    local right = math.floor(index * workarea.width / workarea.count)
    table.insert(rectangles, {x=left, y=0, w=right-left, h=workarea.height})
end
assert(string.format('%d cells', #rectangles) == '3 cells')
return {
    rectangles = rectangles,
    animation = {duration_ms = 140, easing = 'ease_out'},
}
