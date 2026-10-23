Official Lua 5.5.1 source, unmodified.
https://www.lua.org/ftp/lua-5.5.1.tar.gz
SHA256: 1c4b4068d67061f2a2231ad2b5422e77acea1487ea9890f6320af614f4373dce
MIT license: src/lua.h copyright/license block. Core, auxiliary, base, math, string and table libraries are built for the compatibility probe and the separate DWM policy runner. Integration uses existing process/PTY and gui-v1 mechanisms. No upstream source modifications.

The same unmodified release also builds /system/bin/lua and /system/bin/luac with Dunit cross-TCC; the full upstream library set is enabled there. See docs/architecture/lua-system.md and config/lua.h.
