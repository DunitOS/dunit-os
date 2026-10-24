-- Verify the complete stored ZIP bytes through native Dunit file I/O.
-- Decompression/rendering is deliberately deferred; ZIP entries stay unchanged.
local base = '/assets/icons/chameleon/'
local catalog = assert(io.open(base .. 'catalog.tsv', 'rb'))
local crc_table = {}
for i = 0, 255 do
    local value = i
    for _ = 1, 8 do
        value = ((value & 1) ~= 0) and ((value >> 1) ~ 0xedb88320) or (value >> 1)
    end
    crc_table[i] = value
end
local packs, entries, bytes = 0, 0, 0
for line in catalog:lines() do
    local name, count, size, sha, checksum = line:match('^([^\t]+)\t(%d+)\t(%d+)\t(%x+)\t(%x+)$')
    assert(name and name:match('^[%w%-]+%.zip$') and #sha == 64, 'invalid icon catalog')
    count, size = assert(tonumber(count)), assert(tonumber(size))
    local file = assert(io.open(base .. name, 'rb'))
    assert(file:read(4) == 'PK\003\004', 'invalid ZIP local header: ' .. name)
    assert(file:seek('end', -22) == size - 22, 'icon ZIP length mismatch: ' .. name)
    local footer = assert(file:read(22))
    assert(footer:sub(1,4) == 'PK\005\006', 'ZIP end record absent: ' .. name)
    assert(string.unpack('<I2', footer, 5) == 0 and string.unpack('<I2', footer, 7) == 0, 'split ZIP unsupported')
    assert(string.unpack('<I2', footer, 9) == count and string.unpack('<I2', footer, 11) == count, 'ZIP entry count mismatch: ' .. name)
    assert(string.unpack('<I2', footer, 21) == 0, 'unexpected ZIP comment')
    local directory_size = string.unpack('<I4', footer, 13)
    local directory = string.unpack('<I4', footer, 17)
    assert(directory + directory_size == size - 22, 'ZIP directory bounds mismatch')
    assert(file:seek('set', directory) == directory and file:read(4) == 'PK\001\002', 'ZIP directory missing')
    assert(file:seek('set', 0) == 0)
    local total, crc = 0, 0xffffffff
    while true do
        local chunk = file:read(32768)
        if not chunk then break end
        for i = 1, #chunk do
            crc = (crc >> 8) ~ crc_table[(crc ~ chunk:byte(i)) & 255]
        end
        total = total + #chunk
    end
    assert(file:close())
    assert(total == size and ((crc ~ 0xffffffff) & 0xffffffff) == tonumber(checksum, 16), 'icon archive checksum mismatch: ' .. name)
    packs, entries, bytes = packs + 1, entries + count, bytes + total
    print(('[ICON-PACKS] %s entries=%d bytes=%d CRC32 OK'):format(name, count, total))
end
assert(catalog:close())
assert(packs > 0, 'empty icon catalog')
print(('[ICON-PACKS] verified packs=%d entries=%d bytes=%d'):format(packs, entries, bytes))
