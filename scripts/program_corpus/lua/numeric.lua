-- Integers, floats, conversions, overflow, bitwise operators, math library.
print(1, 1.0, -0.0, 1/0, -1/0, 3 // 2, 3.0 // 2, -3 // 2, 3 % -2, -3 % 2, 3.5 % -2)
print(math.maxinteger, math.mininteger, math.maxinteger + 1 == math.mininteger, math.mininteger // -1)
print(math.maxinteger * 2, math.mininteger * -1, math.abs(math.mininteger), 7 // 0.0, -7 // 0.0, 0/0 ~= 0/0)
print(pcall(function() return 1 // 0 end))
print(pcall(function() return 1 % 0 end))
print(5 // 2 * 2 + 5 % 2, 2^10, 2^0.5, 10 / 2, 7 / 2, 1e308 * 10, -(0/0) ~= -(0/0))
print(3 | 4, 7 & 2, 5 ~ 3, ~0, 1 << 62, 1 << 63, 1 << 64, -1 >> 1, -1 >> 63, 2^53 | 0)
print(pcall(function() return 1.5 | 1 end))
print("10" + 5, "0x10" + 0, "1e1" * 1, 10 .. 20, math.type(1), math.type(1.0), math.type("1"))
print(math.tointeger(3.0), math.tointeger(3.5), math.tointeger("8"), 3 == 3.0, math.ult(-1, 1))
print(math.floor(-3.5), math.ceil(-3.5), math.floor(3), math.fmod(-7, 3), math.fmod(7, -3), math.sqrt(144))
print(math.max(3, 7.5, -1), math.min(3, 7.5, -1), math.huge, -math.huge, math.pi)
print(string.format("%.17g", 0.1), string.format("%.17g", 1/3), string.format("%.14g", 2^0.5), 100 // 1e-3)
print(string.format("%.3f %.3f %.3f", math.sin(1), math.cos(1), math.atan(1, 2)), math.exp(1), math.log(8, 2), math.log(100, 10))
print(7.0 == 7, math.maxinteger + 0.0 == math.maxinteger, 2^63 == math.maxinteger + 1.0, math.maxinteger // 1 | 0)
print(tostring(3) == "3", tostring(3.0), 1e100, 123456789012345678)
print(0x7fffffffffffffff, 0xffffffffffffffff, 0x1p4, 1e2, .5, 5., 3e-2)
math.randomseed(42)
local r = {}
for i = 1, 5 do r[#r + 1] = math.random(1, 100) end
print("random", table.concat(r, " "), math.random(0) ~= nil)
local acc = 0
for i = 1, 100000 do acc = (acc * 31 + i) % 1000003 end
print("lcg", acc)
local x = 0.0
for i = 1, 1000 do x = x + 1 / i end
print("harmonic", string.format("%.12f", x))
local h = 5381
for c in ("the quick brown fox jumps over the lazy dog"):gmatch(".") do h = ((h << 5) + h + c:byte()) & 0xffffffff end
print("djb2", h)
for _, v in ipairs({1, 255, 65535, 2^31, 2^32, 2^53, 2^63, -1, -2^63}) do io.write(string.format("%d ", math.tointeger(v) or 0)) end
print()
print(string.format("%5.1f|%-8.3e|%G", 2.25, 0.000123456, 1e-10), 8 // 3, -8 // 3, 8 % -3, -8 % 3)
print(#tostring(math.pi), tostring(1e15), tostring(1e16), 2^24, 10 // 3.0, 1e15 + 0.5)
