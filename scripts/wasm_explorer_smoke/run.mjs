// Runs the explorer compiled to WebAssembly (src/lib.rs) in Node and checks
// that it compiles C to tokens, a syntax tree, IR, assembly and machine code,
// and that the deepest nesting the default limit admits fits the stack while
// deeper nesting ends with a diagnostic.
//
// Usage, from this directory:
//   cargo build --release --target wasm32-unknown-unknown
//   node run.mjs target/wasm32-unknown-unknown/release/wasm_explorer_smoke.wasm
import { readFileSync } from "node:fs";
import assert from "node:assert/strict";

const { instance } = await WebAssembly.instantiate(readFileSync(process.argv[2]));
const { memory, smoke_alloc, smoke_compile } = instance.exports;

function compile(source, { optimize = false, depth = 0 } = {}) {
  const bytes = new TextEncoder().encode(source);
  const pointer = smoke_alloc(bytes.length);
  new Uint8Array(memory.buffer, pointer, bytes.length).set(bytes);
  const report = smoke_compile(pointer, bytes.length, optimize ? 1 : 0, depth);
  const length = new DataView(memory.buffer).getUint32(report, true);
  return new TextDecoder().decode(new Uint8Array(memory.buffer, report + 4, length));
}

const program = `#include <stdint.h>
static int32_t square(int32_t x) { return x * x; }
int main(void) {
    long double third = 1.0L / 3;
    int total = 0;
    for (int i = 0; i < 10; ++i)
        total += square(i);
    return total + (int)(third * 3);
}
`;
for (const optimize of [false, true]) {
  const report = compile(program, { optimize });
  assert.doesNotMatch(report, /== diagnostic error/, report);
  for (const section of ["tokens", "syntax-tree", "ir", "ir-abi", "asm"]) {
    assert.match(report, new RegExp(`^== ${section}$`, "m"), section);
  }
  assert.match(report, /^function @main\(\) -> i32 external \{$/m);
  assert.match(report, /^main:$/m);
  assert.match(report, /^== machine-code main$/m);
  assert.match(report, /retq$/m);
  if (optimize) console.log(report.slice(report.indexOf("== ir\n")));
}

const errors = compile("int main(void) { return 1 +; }\n");
assert.match(errors, /== diagnostic error\nerror: expected an expression/);
assert.doesNotMatch(errors, /== ir\n/);

// Nesting deeper than the default limit is a diagnostic, and nesting just
// inside it fits the default 1 MiB stack in every shape that recurses deeply.
const nested = (depth) => `int x = ${"(".repeat(depth)}1${")".repeat(depth)};\n`;
const deep = compile(nested(100000));
assert.match(deep, /== diagnostic error\nerror: .*nest/, deep.slice(0, 400));
for (const shape of [
  `int main(void) { return ${"- ".repeat(490)}1; }\n`,
  `int f(int); int main(void) { return ${"f(".repeat(490)}1${")".repeat(490)}; }\n`,
  `int main(void) { int x = 0; ${"if (x) ".repeat(490)}x++; return x; }\n`,
  `int a[1] = ${"{".repeat(490)}1${"}".repeat(490)};\n`,
  nested(490),
]) {
  const report = compile(shape);
  assert.doesNotMatch(report, /== diagnostic error/, report.slice(0, 400));
  assert.match(report, /^== asm$/m);
}
console.log("wasm explorer smoke test passed");
