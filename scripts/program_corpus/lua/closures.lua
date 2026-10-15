-- Closures, upvalues, varargs, recursion and goto.
local function counter(start, step)
  local n = start
  return function() n = n + step; return n end, function() return n end
end
local inc, get = counter(10, 3)
for _ = 1, 5 do inc() end
print("counter", get())

local fns = {}
for i = 1, 5 do fns[i] = function() i = i + 100; return i end end
print("loop upvalues", fns[1](), fns[1](), fns[5](), fns[3]())

local function compose(...)
  local fs = {...}
  return function(...)
    local args = table.pack(...)
    for i = #fs, 1, -1 do args = table.pack(fs[i](table.unpack(args, 1, args.n))) end
    return table.unpack(args, 1, args.n)
  end
end
local add1 = function(x) return x + 1 end
local dbl = function(x) return x * 2 end
print("compose", compose(add1, dbl)(20), compose(dbl, add1)(20))

local function fib(n) if n < 2 then return n end return fib(n - 1) + fib(n - 2) end
print("fib", fib(20))

local function tail(n, acc) if n == 0 then return acc end return tail(n - 1, acc + n) end
print("tailcall", tail(100000, 0))

local function varargs(...)
  local n = select("#", ...)
  local last = select(n > 0 and n or 1, ...)
  return n, last, (select(-1, 1, 2, 3))
end
print("varargs", varargs(), varargs(nil, nil), varargs("a", "b", "c"))

local ok, err = pcall(function() local t = nil; return t.x end)
print("pcall", ok, (tostring(err):gsub("^.-:%d+: ", "")))
print("error object", select(2, pcall(error, {code = 7})).code)
print("xpcall", xpcall(function() error("boom", 0) end, function(m) return "handled " .. m end))

do
  local i, out = 1, {}
  ::again::
  out[#out + 1] = i * i
  i = i + 1
  if i <= 6 then goto again end
  print("goto", table.concat(out, ","))
end

local y = (function(f) return (function(x) return x(x) end)(function(x) return f(function(...) return x(x)(...) end) end) end)
local fact = y(function(self) return function(n) if n <= 1 then return 1 end return n * self(n - 1) end end)
print("ycomb", fact(10), fact(20))

local t = setmetatable({}, {__close = function() print("closed") end})
do local c <close> = t; print("in scope") end
local k <const> = 42
print("const", k)
