// Exercise the raw Wasm ABI, including invalid inputs and trace export.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const wasmPath = process.argv[2] || "web/pkg/bcc_rust.wasm";
const bytes = readFileSync(wasmPath);
const module = await WebAssembly.compile(bytes);
assert.deepEqual(WebAssembly.Module.imports(module), [], "module must need no host imports");
const instance = await WebAssembly.instantiate(module, {});
const api = instance.exports;
const expected = ["bcc_abi_version", "bcc_alloc", "bcc_free", "bcc_compile", "bcc_state", "bcc_step", "bcc_back", "bcc_reset", "bcc_trace", "bcc_response_ptr", "bcc_response_len"];
for (const name of expected) assert.equal(typeof api[name], "function", name);
assert.ok(api.memory instanceof WebAssembly.Memory);
assert.equal(api.bcc_abi_version(), 1);
const decoder = new TextDecoder("utf-8", { fatal: true });
const encoder = new TextEncoder();
function response() { return JSON.parse(decoder.decode(new Uint8Array(api.memory.buffer, api.bcc_response_ptr(), api.bcc_response_len()))); }
function operation(name) { assert.equal(api[name](), 0, name); return response(); }
function compile(source) {
  const input = encoder.encode(source);
  const ptr = api.bcc_alloc(input.length);
  assert.ok(ptr);
  assert.ok(new Uint8Array(api.memory.buffer, ptr, input.length).every(byte => byte === 0));
  new Uint8Array(api.memory.buffer, ptr, input.length).set(input);
  try { assert.equal(api.bcc_compile(ptr, input.length), 0); return response(); }
  finally { api.bcc_free(ptr, input.length); }
}
for (const name of ["bcc_state", "bcc_step", "bcc_back", "bcc_reset", "bcc_trace"]) {
  assert.equal(api[name](), 1);
  assert.match(response().error, /compile/);
}
assert.equal(api.bcc_alloc(0), 0);
assert.equal(api.bcc_alloc(1024 * 1024 + 1), 0);
api.bcc_free(0, 0);
assert.equal(api.bcc_compile(0, 1), 1);
assert.equal(api.bcc_compile(0, 1024 * 1024 + 1), 1);
assert.equal(api.bcc_compile(api.memory.buffer.byteLength - 1, 2), 1);
const invalid = api.bcc_alloc(1);
new Uint8Array(api.memory.buffer, invalid, 1)[0] = 255;
assert.equal(api.bcc_compile(invalid, 1), 1);
assert.match(response().error, /UTF-8/);
api.bcc_free(invalid, 1);
assert.equal(api.bcc_compile(0, 0), 0);
assert.equal(response().status, "error");
const initial = compile("int main(void) { int x; x = 7; return x; }");
assert.equal(initial.schema_version, 1);
assert.equal(initial.status, "ready");
assert.deepEqual(operation("bcc_back"), initial);
const first = operation("bcc_step");
assert.equal(first.step, initial.step + 1);
assert.deepEqual(operation("bcc_back"), initial);
assert.deepEqual(operation("bcc_step"), first, "forward replay must be identical");
let state = first;
for (let i = 0; i < 100 && ["ready", "running"].includes(state.status); i++) state = operation("bcc_step");
assert.equal(state.status, "finished");
assert.deepEqual(state.return_value, { kind: "integer", value: 7 });
assert.deepEqual(operation("bcc_step"), state, "terminal state must stay fixed");
const trace = operation("bcc_trace");
assert.ok(JSON.stringify(trace).includes("finished"));
assert.deepEqual(operation("bcc_reset"), initial);
const rejected = compile("#include <stdio.h>\nint main(void) { return 0; }");
assert.equal(rejected.status, "error");
assert.ok(rejected.diagnostics.length);
console.log("PASS: raw Wasm exports, allocation, invalid inputs, UTF-8, trace export, replay and reset");
