-- Tables, sorting, metatables, and the table library.
local t = {}
for i = 1, 20 do t[i] = (i * 7919) % 101 end
table.sort(t)
print("sorted", table.concat(t, " "))
table.sort(t, function(a, b) return a > b end)
print("rsorted", table.concat(t, " ", 1, 8))

local words = {"pear", "apple", "fig", "banana", "kiwi", "cherry", "date"}
table.sort(words, function(a, b) if #a ~= #b then return #a < #b end return a < b end)
print("words", table.concat(words, ","))

local q = {}
for i = 1, 10 do table.insert(q, i) end
table.insert(q, 1, 0)
print("remove", table.remove(q), table.remove(q, 1), #q)
print("move", table.concat(table.move({1, 2, 3, 4, 5}, 2, 4, 1), ","))
print("unpack", table.unpack({1, 2, 3}, 2))
print("pack", table.pack(1, nil, 3).n)

local map = {}
for i = 1, 100 do map["k" .. i] = i * i end
local sum, count = 0, 0
for k, v in pairs(map) do sum = sum + v; count = count + 1 end
print("hash", sum, count, map.k10, map.k100, map.missing)
for i = 1, 100, 2 do map["k" .. i] = nil end
count = 0
for _ in pairs(map) do count = count + 1 end
print("after delete", count)

local mt = {}
mt.__index = function(_, k) return k .. "?" end
mt.__newindex = function(t, k, v) rawset(t, k, v * 2) end
mt.__call = function(self, a, b) return a + b end
mt.__len = function() return 99 end
mt.__tostring = function() return "<obj>" end
mt.__concat = function(a, b) return "cat" end
mt.__eq = function() return true end
mt.__lt = function() return true end
mt.__add = function(a, b) return 1234 end
local o = setmetatable({}, mt)
o.x = 21
print("meta", o.x, o.y, o(2, 3), #o, tostring(o), o .. 1, o + 1, o < o, o == setmetatable({}, mt))

local Animal = {}; Animal.__index = Animal
function Animal.new(name, sound) return setmetatable({name = name, sound = sound}, Animal) end
function Animal:speak() return self.name .. " says " .. self.sound end
local Dog = setmetatable({}, {__index = Animal}); Dog.__index = Dog
function Dog.new(name) local d = Animal.new(name, "woof"); return setmetatable(d, Dog) end
function Dog:fetch() return self.name .. " fetches" end
local d = Dog.new("Rex")
print("oop", d:speak(), d:fetch())

local big = {}
for i = 1, 5000 do big[#big + 1] = i end
for i = 1, 4990 do big[#big] = nil end
print("shrink", #big, next({}), rawlen(big), rawequal(big, big))
local weak = setmetatable({}, {__mode = "k"})
weak[{}] = 1
collectgarbage()
print("weak", next(weak))
print("nested", #{{1, 2}, {3, 4, 5}, n = 2}, ({10, 20, 30})[2])
local s = 0
for i, v in ipairs({5, 6, nil, 8}) do s = s + i * v end
print("ipairs", s)
