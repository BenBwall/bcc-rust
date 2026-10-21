import { connectEngine } from "./engine.js";
import { demos } from "./demos.js";
import { element, renderDiagnostic, renderSnapshot, renderSource } from "./render.js";

const HISTORY_LIMIT = 2000;
const RUN_LIMIT = 1000;
const MAX_SOURCE_BYTES = 256 * 1024;
let engine;
let source = demos.tour.source;
let filename = demos.tour.name;
let history = [];
let cursor = -1;
let firstStep = 0;
let targetLine;
let busy = false;
let running = false;
let edited = false;

element("source-editor").value = source;
element("source-name").textContent = filename;
renderSnapshot(null, source, targetLine);
bindControls();
await initialize();

async function initialize() {
  busy = true;
  updateControls();
  element("engine-label").textContent = "Connecting to Rust / Wasm";
  try {
    engine?.dispose?.();
    engine = await connectEngine();
    element("engine-notice").hidden = true;
    element("engine-label").textContent = "Rust / Wasm · local execution";
    await loadProgram(source, filename);
  } catch (error) {
    engine = undefined;
    element("engine-notice").hidden = false;
    element("notice-message").textContent = message(error);
    element("engine-label").textContent = "Rust / Wasm unavailable";
    announce("Rust WebAssembly engine unavailable. Build instructions are displayed.");
  } finally {
    busy = false;
    updateControls();
  }
}

function bindControls() {
  element("retry-engine").addEventListener("click", initialize);
  element("step").addEventListener("click", () => action(async () => { await stepForward(); showState(true); }));
  element("back").addEventListener("click", () => { if (busy || running || cursor <= 0 || edited) return; cursor -= 1; showState(true); });
  element("reset").addEventListener("click", () => action(() => loadProgram(source, filename)));
  element("run").addEventListener("click", () => { if (running) { running = false; updateControls(); } else run(); });
  element("run-to-cursor").addEventListener("click", () => run(targetLine));
  element("code-view").addEventListener("click", event => {
    const button = event.target.closest("button[data-line]");
    if (!button) return;
    targetLine = Number(button.dataset.line);
    element("cursor-line").textContent = String(targetLine);
    element("cursor-label").textContent = `Target set to line ${targetLine}.`;
    renderSource(source, current()?.location, targetLine);
    element("code-view").querySelector(`button[data-line="${targetLine}"]`)?.focus({ preventScroll: true });
    updateControls();
  });
  element("edit-source").addEventListener("click", () => setEditing(true));
  element("view-source").addEventListener("click", () => setEditing(false));
  element("source-editor").addEventListener("input", () => {
    edited = element("source-editor").value !== source;
    updateControls();
  });
  element("load-source").addEventListener("click", () => action(async () => {
    await loadProgram(element("source-editor").value, filename);
    setEditing(false);
  }));
  element("demo-select").addEventListener("change", () => {
    const demo = demos[element("demo-select").value];
    if (demo) action(() => stageProgram(demo.source, demo.name));
  });
  element("file-input").addEventListener("change", () => action(async () => {
    const file = element("file-input").files[0];
    if (!file) return;
    if (file.size > MAX_SOURCE_BYTES) throw new Error("Choose a C source file smaller than 256 KiB.");
    await stageProgram(await file.text(), file.name);
    element("file-input").value = "";
  }));
}

async function stageProgram(text, name) {
  if (engine) await loadProgram(text, name);
  else {
    source = text;
    filename = name;
    history = [];
    cursor = -1;
    targetLine = undefined;
    element("source-editor").value = text;
    element("source-name").textContent = name;
    edited = false;
    showState();
  }
  setEditing(false);
}

async function loadProgram(text, name) {
  if (new TextEncoder().encode(text).length > MAX_SOURCE_BYTES) throw new Error("Keep source code below 256 KiB.");
  source = text;
  filename = name;
  history = [];
  cursor = -1;
  firstStep = 0;
  targetLine = undefined;
  edited = false;
  element("source-editor").value = text;
  element("source-name").textContent = name;
  element("cursor-line").textContent = "—";
  element("cursor-label").textContent = "Select a line number to set a target.";
  showState();
  history.push(await engine.load(text, name));
  cursor = 0;
  showState(true);
}

async function stepForward() {
  if (cursor < history.length - 1) cursor += 1;
  else {
    history.push(await engine.step());
    cursor += 1;
    if (history.length > HISTORY_LIMIT) { history.shift(); cursor -= 1; firstStep += 1; }
  }
}

async function run(line) {
  if (busy || running || !canStep()) return;
  running = true;
  updateControls();
  let steps = 0;
  try {
    // Yield between real Rust steps so Pause stays available and large loops do
    // not lock the page. The budget also bounds accidental infinite loops.
    while (running && canStep() && steps < RUN_LIMIT) {
      await stepForward();
      steps += 1;
      showState();
      if (line && current()?.location?.start.line === line) break;
      await new Promise(resolve => setTimeout(resolve, 25));
    }
    if (steps === RUN_LIMIT && canStep()) announce(`Paused after ${RUN_LIMIT} evaluations. Run again to continue.`);
    else if (line && current()?.location?.start.line === line) announce(`Reached line ${line}.`);
    else announceState();
  } catch (error) { showError(error); }
  finally { running = false; updateControls(); revealCurrentLine(); }
}

async function action(operation) {
  if (busy || running) return;
  busy = true;
  updateControls();
  try { await operation(); }
  catch (error) { showError(error); }
  finally { busy = false; updateControls(); }
}

function showState(reveal = false) {
  renderSnapshot(current(), source, targetLine);
  updateControls();
  if (reveal) { revealCurrentLine(); announceState(); }
}

function updateControls() {
  const locked = busy || running;
  element("step").disabled = locked || !canStep();
  element("back").disabled = locked || edited || cursor <= 0;
  element("back").title = firstStep ? `Back through the most recent ${HISTORY_LIMIT} snapshots` : "Back one engine snapshot";
  element("reset").disabled = locked || !engine;
  element("run").disabled = busy || (!running && !canStep());
  element("run-label").textContent = running ? "Pause" : "Run";
  element("run").title = running ? "Pause execution" : "Run until completion or undefined behavior";
  element("run").setAttribute("aria-pressed", String(running));
  element("run-to-cursor").disabled = locked || !canStep() || !targetLine;
  element("load-source").disabled = locked || !engine;
  element("demo-select").disabled = locked;
  element("file-input").disabled = locked;
  element("source-editor").disabled = locked;
  element("retry-engine").disabled = busy;
  element("editor-help").textContent = edited ? "Source changed. Load program to execute your edits." : "Load edits to restart execution.";
}

function setEditing(editing) {
  element("code-view").hidden = editing;
  element("editor-view").hidden = !editing;
  element("view-source").setAttribute("aria-pressed", String(!editing));
  element("edit-source").setAttribute("aria-pressed", String(editing));
  if (editing) element("source-editor").focus();
}

function canStep() { return !!engine && !!current() && !edited && !["finished", "undefined_behavior", "error"].includes(current().status); }
function current() { return history[cursor]; }
function announce(text) { element("announcement").textContent = text; }
function announceState() { const snapshot = current(); if (snapshot) announce(`Step ${snapshot.step}. ${snapshot.status.replaceAll("_", " ")}. ${snapshot.event || ""}${snapshot.diagnostic ? ` ${snapshot.diagnostic.message}` : ""}`); }
function revealCurrentLine() { const line = current()?.location?.start.line; if (line) element("code-view").querySelector(`.code-line[data-line="${line}"]`)?.scrollIntoView({ block: "nearest", inline: "nearest" }); }
function showError(error) { renderDiagnostic({ title: "Unable to continue", message: message(error), citation: "Execution controls preserve the last available engine state." }); announce(message(error)); }
function message(error) { return error instanceof Error ? error.message : String(error); }
