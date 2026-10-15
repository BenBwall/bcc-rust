-- Coroutines: generators, passing values, errors, wrap, status, yield across pcall.
local function gen(n)
  return coroutine.wrap(function() for i = 1, n do coroutine.yield(i * i) end end)
end
local out = {}
for v in gen(8) do out[#out + 1] = v end
print("gen", table.concat(out, " "))

local co = coroutine.create(function(a, b)
  print("start", a, b)
  local c = coroutine.yield(a + b)
  print("got", c)
  local d, e = coroutine.yield(c * 2)
  print("got2", d, e)
  return "done", 99
end)
print(coroutine.resume(co, 1, 2))
print(coroutine.status(co))
print(coroutine.resume(co, 10))
print(coroutine.resume(co, "x", "y"))
print(coroutine.status(co), coroutine.resume(co))

local bad = coroutine.create(function() error("oops") end)
local ok, msg = coroutine.resume(bad)
print("error", ok, (msg:gsub("^.-:%d+: ", "")), coroutine.status(bad))

local p = coroutine.wrap(function()
  local ok, err = pcall(function() coroutine.yield(1); error("inner", 0) end)
  coroutine.yield(tostring(ok) .. ":" .. err)
  return "end"
end)
print("pcall yield", p(), p(), p())

local function producer()
  return coroutine.create(function() for i = 1, 4 do coroutine.yield("item" .. i) end return nil end)
end
local function filter(src)
  return coroutine.create(function()
    while true do
      local _, v = coroutine.resume(src)
      if v == nil then return nil end
      coroutine.yield(v:upper())
    end
  end)
end
local f = filter(producer())
local items = {}
while true do
  local _, v = coroutine.resume(f)
  if not v then break end
  items[#items + 1] = v
end
print("pipeline", table.concat(items, ","))

print("main", coroutine.isyieldable(), select(2, coroutine.running()))
local wrapped = coroutine.wrap(function() return coroutine.isyieldable() end)
print("inside", wrapped())

local sum = 0
local many = {}
for i = 1, 200 do many[i] = coroutine.wrap(function(x) while true do x = coroutine.yield(x + i) end end) end
for round = 1, 10 do for i = 1, 200 do sum = sum + many[i](round) end end
print("many", sum)
print("close", coroutine.close(coroutine.create(function() end)))
local function nest(n)
  if n == 0 then return 0 end
  return coroutine.wrap(function() return 1 + nest(n - 1) end)()
end
print("nested", nest(50))
