// Runs the explorer compiled to WebAssembly (src/lib.rs) in Node and checks
// that it shows the front end phase by phase (logical lines, preprocessing
// tokens, the preprocessed text and its macro expansions) and compiles C to
// tokens, a syntax tree, IR, assembly and machine code,
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

const macros = compile(`#define SQUARE(x) ((x) * (x))
#define GLUE(a, b) a ## b
int GLUE(count, er) = SQUARE(1 + \\
2); /* a comment */
`);
assert.doesNotMatch(macros, /== diagnostic error/, macros);
assert.match(macros, /^== logical-lines 3$/m, macros);
assert.match(macros, /^3-4: splice$/m, macros);
assert.match(macros, /^== pp-tokens \d+$/m, macros);
assert.match(macros, /^GLUE\(count, er\) -> counter$/m, macros);
assert.match(macros, /^SQUARE\(1 \+ 2\) -> \(\(1 \+ 2\) \* \(1 \+ 2\)\)$/m, macros);
assert.match(macros, /^== preprocessed\n# 1 "<input>"\n[^]*^int counter = \(\(1 \+/m, macros);

// Programs up to the WebAssembly limit compile without filling an arena,
// in each dense shape, and a larger one is refused with a diagnostic rather
// than a trap. Each fixed region is 32 MiB; these sizes are what fits.
const LIMIT = 64 * 1024;
const fill = (unit, head = "", tail = "\n") =>
  head + unit.repeat(Math.floor((LIMIT - 64 - head.length - tail.length) / unit.length)) + tail;
for (const [name, source] of Object.entries({
  declarations: fill("int x;\n", "", ""),
  line: fill("int x;"),
  initializer: fill("0,", "int x[] = {", "};\n"),
  sum: fill(" + 1", "int y = 0", ";\n"),
  macros: (() => {
    let text = "#define X(a) ((a) + 1)\n";
    for (let i = 0; text.length < LIMIT - 64; i++) text += `int v${i} = X(${i});\n`;
    return text.slice(0, text.lastIndexOf("\n", LIMIT - 64) + 1);
  })(),
})) {
  const started = Date.now();
  const report = compile(source);
  assert.doesNotMatch(report, /== diagnostic error/, `${name}: ${report.slice(0, 400)}`);
  assert.match(report, /^== asm$/m, name);
  assert.ok(Date.now() - started < 10000, `${name} took ${Date.now() - started} ms`);
}
const tooLarge = compile("int x;\n".repeat(LIMIT / 7 + 1));
assert.match(tooLarge, /== diagnostic error\nerror: the explorer compiles at most 65536 bytes of source/, tooLarge.slice(0, 400));
assert.doesNotMatch(tooLarge, /== tokens/);

// A macro expansion that multiplies its input stops the phase-4 trace
// rather than filling the preprocessing arena.
const blowUp = compile(`#define A 0,0,0,
#define B A A A A A A A A A A
#define C B B B B B B B B B B
#define D C C C C C C C C C C
#define E D D D D D D D D D D
int a[] = {E 0};
`);
assert.doesNotMatch(blowUp, /== diagnostic error/, blowUp.slice(0, 400));
assert.match(blowUp, /^== trace truncated$/m);
assert.match(blowUp, /^== asm$/m);

const errors = compile("int main(void) { return 1 +; }\n");
assert.match(errors, /== diagnostic error\nerror: expected an expression/);
assert.doesNotMatch(errors, /== ir\n/);

// Nesting deeper than the default limit is a diagnostic, and nesting just
// inside it fits the default 1 MiB stack in every shape that recurses deeply.
const nested = (depth) => `int x = ${"(".repeat(depth)}1${")".repeat(depth)};\n`;
// Within the size limit, far past the nesting one.
const deep = compile(nested(20000));
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
