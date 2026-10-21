// Integration checks against the actual Rust Wasm artifact; no mock engine.
// Run from the repository root after sh web/build.sh.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createWasmSession } from "./wasm-session.js";
import { demos } from "./demos.js";

const artifact = process.argv[2] || "web/pkg/bcc_rust.wasm";
const { instance, module } = await WebAssembly.instantiate(await readFile(artifact), {});
assert.deepEqual(WebAssembly.Module.imports(module), [], "Engine must not require host imports");
const session = createWasmSession(instance);
let states = execute(demos.tour.source);
let finished = states.at(-1);
assert.equal(finished.status, "finished", JSON.stringify(finished.diagnostic));
assert.equal(finished.returnValue.text, "15", "Demo must execute its loop and pointer writes");
assert(states.some(state => state.stack.length === 2), "Demo must actually call bump");
assert(states.some(state => state.objects.some(object => object.value.kind === "indeterminate")), "Uninitialized storage must be visible");
assert(states.some(state => state.objects.some(object => object.value.kind === "pointer" && object.value.target != null)), "Pointers must carry object identity");
assert(states.some(state => state.effects.length > 0), "Engine must report pending stores");
assert(finished.objects.some(object => object.value.kind === "array" && object.value.elements.map(value => value.text).join(",") === "3,5,7"), "Pointer aliases must update the original array");
assert(finished.objects.some(object => !object.alive), "Ended lifetimes must be retained");
assert.deepEqual(session.back(), states.at(-2), "Rust Back must restore the previous complete observation");
assert.deepEqual(session.step(), finished, "Rust forward replay must reproduce the terminal state");
assert.deepEqual(session.reset(), states[0], "Rust Reset must restore the initial observation");

states = execute(demos.tour.source.replace("{2, 4, 6}", "{4, 6, 8}"));
assert.equal(states.at(-1).returnValue.text, "21", "Changing source must change actual execution");
for (const key of ["indeterminate", "bounds"]) {
  states = execute(demos[key].source);
  finished = states.at(-1);
  assert.equal(finished.status, key === "indeterminate" ? "error" : "undefined_behavior", `${key}: ${JSON.stringify(finished)}`);
  assert.equal(finished.diagnostic.kind, key === "indeterminate" ? "indeterminate" : "undefined_behavior");
  assert(finished.diagnostic?.citation?.includes("6."), "Diagnostic stop must cite the standard");
  assert(finished.location?.start.line > 0, "Stop must identify the faulting source range");
  assert.deepEqual(session.step(), finished, "Stepping a terminal state must not execute further");
  assert.equal(session.back().status, "running", "Back must leave the diagnostic stop");
}

const rejected = session.load("#include <stdio.h>\nint main(void) { return 0; }\n");
assert.equal(rejected.status, "error", "Unsupported source must be rejected explicitly");
assert(rejected.diagnostic?.message);
for (const source of [
  "int main(void) { int a[2] = {3, 7}; int *p = a; *p++; return a[0]; }",
  "int main(void) { return later(); } int later(void) { return 7; }",
]) {
  assert.equal(session.load(source).status, "error", "Unsupported postfix nesting and undeclared calls must be rejected");
}
assert.equal(session.load("int main(void) { return 42; }\n").status, "ready", "A new program must recover after rejected source");
assert.equal(execute("int main(void) { return 42; }\n").at(-1).returnValue.text, "42");
const unicodeSource = "/* π 🧭 */ int main(void) { return 42; }\n";
const unicodeState = session.load(unicodeSource);
assert.deepEqual(unicodeState.location.start, { line: 1, column: unicodeSource.indexOf("{") + 1 }, "UTF-8 byte ranges must map to editor character columns");
assert.throws(() => session.load(" ".repeat(32769)), /32 KiB/, "Browser source limit must match the engine");
assert.deepEqual(session.state(), unicodeState, "Rejected oversized input must preserve the source map and engine state");
const limited = execute("int main(void) { while (1) {} }\n").at(-1);
assert.equal(limited.status, "limit", "A nonterminating loop must stop with a resource limit");
assert.equal(limited.diagnostic.kind, "limit", "Resource exhaustion must not be labeled UB");
console.log("PASS: real Wasm demo, changed source, objects, calls, pointers, effects, diagnostic stops, replay, reset, rejection, UTF-8 ranges, and resource limits");

function execute(source) {
  const result = [session.load(source)];
  while (["ready", "running"].includes(result.at(-1).status)) {
    assert(result.length < 2200, "Engine must terminate or report its step limit");
    result.push(session.step());
  }
  return result;
}
