// Runs the explorer compiled to WebAssembly (src/lib.rs) in Node and checks
// that it compiles C to tokens, a syntax tree, IR, assembly and machine code,
// that the site's lessons compile to WebAssembly modules that run, and that the deepest nesting the default limit admits fits the stack while
// deeper nesting ends with a diagnostic.
//
// Usage, from this directory:
//   cargo build --release --target wasm32-unknown-unknown
//   node run.mjs target/wasm32-unknown-unknown/release/wasm_explorer_smoke.wasm
import { closeSync, mkdtempSync, openSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { WASI } from "node:wasi";
import assert from "node:assert/strict";

const { instance } = await WebAssembly.instantiate(readFileSync(process.argv[2]));
const { memory, smoke_alloc, smoke_compile, smoke_compile_wasm } = instance.exports;

function read(report) {
  const length = new DataView(memory.buffer).getUint32(report, true);
  return new TextDecoder().decode(new Uint8Array(memory.buffer, report + 4, length));
}

function place(source) {
  const bytes = new TextEncoder().encode(source);
  const pointer = smoke_alloc(bytes.length);
  new Uint8Array(memory.buffer, pointer, bytes.length).set(bytes);
  return [pointer, bytes.length];
}

function compile(source, { optimize = false, depth = 0 } = {}) {
  const [pointer, length] = place(source);
  return read(smoke_compile(pointer, length, optimize ? 1 : 0, depth));
}

// The explorer's WebAssembly back end, run in the browser's own engine: the
// module it compiles for WASI runs under Node's WASI, its stdout captured.
async function compileAndRunWasm(source, optimize) {
  const [pointer, length] = place(source);
  const report = read(smoke_compile_wasm(pointer, length, optimize ? 1 : 0));
  assert.doesNotMatch(report, /== diagnostic error/, report.slice(0, 2000));
  const hex = report.slice(report.indexOf("== binary\n") + 10).trim();
  const binary = Uint8Array.from(hex.match(/../g), (pair) => parseInt(pair, 16));
  assert.ok(WebAssembly.validate(binary), "the module validates");
  const directory = mkdtempSync(join(tmpdir(), "bcc-wasm-smoke-"));
  const stdout = openSync(join(directory, "stdout"), "w");
  const wasi = new WASI({ version: "preview1", args: ["program"], env: {}, stdout, returnOnExit: true });
  const { instance: program } = await WebAssembly.instantiate(binary, {
    wasi_snapshot_preview1: wasi.wasiImport,
  });
  const status = wasi.start(program);
  closeSync(stdout);
  return { report, status, stdout: readFileSync(join(directory, "stdout"), "utf8") };
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

const wasmProgram = `#include <stdio.h>
#include <stdlib.h>
struct point { int x, y; };
static struct point add(struct point a, struct point b) { return (struct point){a.x + b.x, a.y + b.y}; }
int main(void) {
    struct point p = add((struct point){1, 2}, (struct point){30, 40});
    char *text = malloc(32);
    snprintf(text, 32, "%d,%d %.3f", p.x, p.y, 2.0 / 3);
    puts(text);
    free(text);
    return p.x + p.y;
}
`;
for (const optimize of [false, true]) {
  const { report, status, stdout } = await compileAndRunWasm(wasmProgram, optimize);
  assert.equal(stdout, "31,42 0.667\n");
  assert.equal(status, 73);
  assert.match(report, /^  \(func \$main /m);
  if (!optimize) assert.doesNotMatch(report, /^== positions 0$/m);
}

// The site's lessons, compiled as its WebAssembly view compiles them
// (wasm32-wasip1, -O0) and run in V8, exit with what main returns, modulo
// 256 as WASI hosts report it. The two with undefined behavior read
// never-written shadow stack memory, which is still zero. Their text names
// functions plainly, as the view shows them.
const lessons = {
  "bounds.c": 0,
  "indeterminate.c": 1,
  "objects.c": 15,
  "overflow.c": 1,
  "pointers.c": 17,
  "promotion.c": 255,
  "recursion.c": 24,
};
for (const [name, expected] of Object.entries(lessons)) {
  const source = readFileSync(new URL(`../../web/demos/${name}`, import.meta.url), "utf8");
  const { report, status, stdout } = await compileAndRunWasm(source, false);
  assert.equal(status, expected, name);
  assert.equal(stdout, "", name);
  assert.match(report, /^  \(func \$_start /m, name);
  assert.doesNotMatch(report, /^== positions 0$/m, name);
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
