// Runs the explorer compiled to WebAssembly (src/lib.rs) in Node and checks
// that it shows the front end phase by phase (logical lines, preprocessing
// tokens, the preprocessed text and its macro expansions, which tracing
// leaves unchanged) and compiles C to tokens, a syntax tree, IR, assembly
// and machine code, or for WASI to a module that runs under Node's WASI,
// and that the deepest nesting the default limit admits fits the stack while
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

const module = await WebAssembly.compile(readFileSync(process.argv[2]));

/// A fresh instance's compile functions and its linear memory.
async function instantiate() {
  const { memory, smoke_alloc, smoke_compile, smoke_compile_json, smoke_compile_wasm } =
    (await WebAssembly.instantiate(module)).exports;
  const call = (source, run) => {
    const bytes = new TextEncoder().encode(source);
    const pointer = smoke_alloc(bytes.length);
    new Uint8Array(memory.buffer, pointer, bytes.length).set(bytes);
    const report = run(pointer, bytes.length);
    const length = new DataView(memory.buffer).getUint32(report, true);
    return new TextDecoder().decode(new Uint8Array(memory.buffer, report + 4, length));
  };
  const compile = (source, { optimize = false, depth = 0, trace = true } = {}) =>
    call(source, (pointer, length) =>
      smoke_compile(pointer, length, optimize ? 1 : 0, depth, trace ? 1 : 0));
  // Every output but the trace, as JSON, for x86-64 or with `wasm` for
  // wasm32-wasip1.
  const outputs = (source, { trace = true, wasm = false } = {}) =>
    call(source, (pointer, length) =>
      smoke_compile_json(pointer, length, trace ? 1 : 0, wasm ? 1 : 0));
  const compileWasm = (source, optimize) =>
    call(source, (pointer, length) => smoke_compile_wasm(pointer, length, optimize ? 1 : 0));
  return { compile, outputs, compileWasm, memory };
}
const { compile, outputs, compileWasm } = await instantiate();

// The explorer's WebAssembly back end, run in the browser's own engine: the
// module it compiles for WASI runs under Node's WASI, its stdout captured.
async function compileAndRunWasm(source, optimize) {
  const report = compileWasm(source, optimize);
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

// Tracing never makes a program fail that compiles without it: deeply
// nested arguments cost the trace a bounded spelling each. 410 levels is
// the most the preprocessor itself takes here: from about 420 its own
// argument capture, which copies what is left of each nested argument,
// fills the expansion arena, with or without the trace.
for (const depth of [400, 410]) {
  const nestedCalls = compile(`#define F(x) x\nint x=${"F(".repeat(depth)}1${")".repeat(depth)};\n`);
  assert.doesNotMatch(nestedCalls, /== diagnostic error/, nestedCalls.slice(0, 400));
  assert.match(nestedCalls, /^int x= *1 *;$/m, `${depth}`);
  assert.match(nestedCalls, /^== asm$/m, `${depth}`);
}

// Tracing never changes what is compiled. Every byte the trace keeps is
// charged to one budget in an arena of its own, so running out stops the
// trace, never the compilation: each shape below, which spends a different
// part of that budget, compiles with the trace on exactly as with it off:
// every output but the trace is the same, header names included.
const repeat = (count, text, separator = "+") => Array(count).fill(text).join(separator);
const parameters = (count) => Array.from({ length: count }, (_, i) => `a${i}`).join(",");
// F replaced fan³ times inside one invocation in the source, each time
// spelling `argument` for each of its parameters.
const tower = (argument, fan, arity = 1) => {
  const call = repeat(arity, "y", ",");
  return `#define F(${parameters(arity)}) 0\n#define H(y) ${repeat(fan, `F(${call})`)}\n` +
    `#define H2(y) ${repeat(fan, "H(y)")}\n#define H3(y) ${repeat(fan, "H2(y)")}\n` +
    `int x = H3(${argument});\n`;
};
// E0 is `unit`, and each E(n+1) is ten E(n).
const tens = (unit) =>
  `#define E0 ${unit}\n` + [1, 2, 3, 4].map((n) => `#define E${n} ${repeat(10, `E${n - 1}`, " ")}\n`).join("");
const lines = (head, line) => {
  let text = head;
  for (let i = 0; text.length < LIMIT - 1024; i++) text += line(i);
  return text;
};
for (const [name, source, truncated, expansions] of [
  ["a trivial program", "int main(void) { return 0; }\n", false],
  ["arguments cut by every invocation", tower(`"${"a".repeat(4000)}"`, 25), true],
  ["arguments that spend the byte budget", tower(`"${"a".repeat(400)}"`, 25, 4), true],
  ["one token longer than every cut", tower(`"${"a".repeat(50000)}"`, 10), false],
  ["empty arguments", lines(
    `#define F(${parameters(1000)}) 1\n#define E ${",".repeat(999)}\n#define APPLY(m, x) m(x)\n` +
      `#define G ${repeat(10, "APPLY(F,E)")}\n#define G2 ${repeat(10, "G")}\n`,
    (i) => `int v${i} = G2;\n`).slice(0, 6500), true],
  ["many arguments", `#define F(${parameters(2000)}) a0+a1999\nint x = F(${repeat(2000, "1", ",")});\n`, false],
  ["arguments around the cut", lines("#define F(x) sizeof x\n",
    (i) => `int n${i} = F("${"a".repeat(150 + (i % 20))}");\n`), false],
  // An empty replacement is one event, so after 30,000 events from one
  // expansion the trace stops as one of these begins.
  ["a stop as an expansion begins", `${tens("0,")}int a[] = {E4 0};\n#define Z\n${"Z\n".repeat(25000)}`, true,
    /^E4 -> 0,[^]*^partial: Z -> $(?![^]*^Z -> )/m],
  ["a stop inside an expansion", `${tens("0,0,0,")}int a[] = {E4 0};\n`, true, /^partial: E4 -> 0,/m],
  ["stringified arguments", `#define S(x) #x\n#define T(x) S(x)\n#define U(x) T(x) T(x) T(x)\n` +
    `const char *s[] = {U(${"b ".repeat(20000)})};\n`, false],
  ["pasted arguments", `#define CAT(a,b) a##b\n#define C2(a,b) CAT(a,b)\n` +
    `int C2(${"a".repeat(20000)},${"b".repeat(20000)});\n`, false],
  ["included headers", `${"#include <stdint.h>\n".repeat(500)}int x;\n`, false],
  ["a header not found", `#include "missing.h"\nint x;\n`, false],
  ["a header included after the trace stops",
    `#include <stddef.h>\n${tens("0,0,0,")}int a[] = {E4 0};\n#include <stdint.h>\nint32_t z;\n`, true],
  ["nested calls", `#define F(x) x\nint x=${"F(".repeat(410)}1${")".repeat(410)};\n`, false],
]) {
  const started = Date.now();
  const traced = compile(source);
  const plain = compile(source, { trace: false });
  assert.doesNotMatch(plain, /^== expansions$/m, name);
  assert.equal(outputs(source), outputs(source, { trace: false }), name);
  assert.match(traced, expansions ?? /^== expansions$/m, name);
  assert.equal(/^== trace truncated$/m.test(traced), truncated, name);
  // Only the expansion the trace stopped in, or as it began, is partial.
  assert.ok((traced.match(/^partial: /gm) ?? []).length <= 1, name);
  assert.ok(Date.now() - started < 20000, `${name} took ${Date.now() - started} ms`);
}

// For wasm32-wasip1 too: the preprocessor lesson's module, binary, text
// and source-map positions, is the same with the trace on and off.
{
  const lesson = readFileSync(new URL("../../web/demos/macros.c", import.meta.url), "utf8");
  const traced = outputs(lesson, { wasm: true });
  assert.equal(traced, outputs(lesson, { wasm: true, trace: false }));
  const { wasm, pp_tokens: tokens } = JSON.parse(traced);
  assert.ok(wasm.binary.length > 0 && wasm.text.includes("(func $main"), "wasm module");
  assert.ok(wasm.positions.some((position) => position.line !== null), "source map");
  assert.ok(tokens.some((token) => token.header_name), "header names");
}

// Tracing a trivial program costs the explorer no more linear memory than
// one region: released regions are kept whole for the next stage rather
// than split by small allocations.
const footprint = async (trace) => {
  const { compile, memory } = await instantiate();
  compile("int main(void) { return 0; }\n", { trace });
  return memory.buffer.byteLength;
};
const [plainBytes, tracedBytes] = [await footprint(false), await footprint(true)];
assert.ok(tracedBytes - plainBytes < 32 * 2 ** 20, `${plainBytes} -> ${tracedBytes}`);

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
