-- A heavier mixed workload: sieve, string building, gc, matrix multiply, a stack machine.
local N = 20000
local sieve, primes = {}, 0
for i = 2, N do sieve[i] = true end
for i = 2, N do
  if sieve[i] then
    primes = primes + 1
    for j = i * i, N, i do sieve[j] = false end
  end
end
print("primes", primes)

local buf = {}
for i = 1, 2000 do buf[#buf + 1] = tostring(i * 3) end
local s = table.concat(buf, ",")
print("string", #s, s:sub(1, 30), select(2, s:gsub(",", ",")))

local objs = {}
for i = 1, 3000 do objs[i] = {id = i, name = "n" .. i, tags = {i % 3, i % 5}} end
local before = collectgarbage("count")
for i = 1, 3000, 2 do objs[i] = nil end
collectgarbage("collect")
local live = 0
for i = 1, 3000 do if objs[i] then live = live + objs[i].tags[1] + objs[i].tags[2] end end
print("gc", live, collectgarbage("count") < before)

local n = 30
local function mat(f)
  local m = {}
  for i = 1, n do m[i] = {}; for j = 1, n do m[i][j] = f(i, j) end end
  return m
end
local A, B = mat(function(i, j) return i + j end), mat(function(i, j) return i - j end)
local C = mat(function(i, j) local s = 0; for k = 1, n do s = s + A[i][k] * B[k][j] end; return s end)
print("matrix", C[1][1], C[n][n], C[7][13])

local code = {{"push", 3}, {"push", 4}, {"add"}, {"push", 5}, {"mul"}, {"dup"}, {"add"}}
local stack = {}
for _, ins in ipairs(code) do
  local op = ins[1]
  if op == "push" then stack[#stack + 1] = ins[2]
  elseif op == "add" then local b, a = table.remove(stack), table.remove(stack); stack[#stack + 1] = a + b
  elseif op == "mul" then local b, a = table.remove(stack), table.remove(stack); stack[#stack + 1] = a * b
  elseif op == "dup" then stack[#stack + 1] = stack[#stack] end
end
print("vm", stack[1])
local fibs = setmetatable({}, {__index = function(t, k)
  local v = k < 2 and k or t[k - 1] + t[k - 2]
  rawset(t, k, v)
  return v
end})
print("memo", fibs[80])
print(load("return 1 + 2 * 3")(), load("syntax error here"), #arg)
local env = {x = 5}
print(load("return x * 2", "chunk", "t", env)(), pcall(load, nil))
print(os.time{year = 2000, month = 1, day = 1, hour = 12} ~= nil, os.date("!%Y-%m-%d", 86400 * 365))
print(#string.dump(function() return 1 end) > 0, type(os.getenv("PATH")), type(io.stdout), io.type(io.stdout))
