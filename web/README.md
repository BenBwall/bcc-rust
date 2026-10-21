# C abstract machine UI

A dependency-free, accessible static debugger for the bcc Rust abstract machine.
The UI renders engine snapshots; JavaScript does not evaluate C. When the Wasm
artifact is missing, the page displays setup instructions and disables execution.

Run all commands from `/home/agent/workspace/bcc-rust/worktrees/abstract-machine-site`
on branch `codex/abstract-machine-site`. Use the repository's pinned Rust 1.99.0
toolchain and install its Wasm standard library once if needed:

```sh
rustup target add wasm32-unknown-unknown
sh web/build.sh
node --experimental-default-type=module web/check.mjs
node --experimental-default-type=module web/abi-check.mjs
node --experimental-default-type=module web/worker-check.mjs
node --experimental-default-type=module web/worker-check.mjs --missing
python3 -m http.server 8080 --bind 127.0.0.1 --directory web
```

`web/build.sh` runs this exact build through the mandatory shared scheduler:

```sh
python3 /home/agent/workspace/bcc-rust/triage-2026-10-16/swarm/run.py cargo rustc \
  --no-default-features --features abstract-machine \
  --lib --crate-type cdylib --target wasm32-unknown-unknown \
  --release --message-format json-render-diagnostics
```

It copies the emitted artifact directly from Cargo's JSON output into ignored
`web/pkg/bcc_rust.wasm` while the build scheduler still holds its lock. The
native compiler/LLVM feature is disabled. No wasm-bindgen or npm tooling is
required. For a long build, commit local source first and detach it:

```sh
python3 - <<'PY'
from pathlib import Path
import subprocess
with Path('web/build.log').open('w') as log:
    process = subprocess.Popen(['sh', 'web/build.sh'], stdin=subprocess.DEVNULL,
                               stdout=log, stderr=subprocess.STDOUT,
                               start_new_session=True)
print('Detached build PID:', process.pid)
PY
```

Open `http://127.0.0.1:8080`. Serve over HTTP; ES modules and Wasm should not be
opened with `file://`. No npm packages, network assets, CDNs, or service workers
are used.

## Interface

- Choose a demo, open a local C source file, or edit and load your own source.
- Step advances one engine evaluation. Run yields between evaluations; Pause
  stops before the next one. Each Run pauses after 1,000 evaluations.
- Back displays earlier real engine snapshots. Stepping forward through history
  displays those snapshots before requesting a new Rust step. Up to 2,000 states
  are retained. Reset reloads the current source in Rust.
- Select a source line number and choose **Run to line**. Execution pauses at
  the next source range intersecting the selected line, a terminal state, or the Run budget. A blank
  or nonexecutable line may never be reached.
- Highlighted text is the **next operation** supplied by the engine, or the
  faulting operation on a diagnostic. The event describes the **last completed
  operation**. UTF-8 byte offsets are converted to editor character columns. Object
  cards show identity, storage duration, lifetime, and value. Pointer links jump
  to their target object's card. Indeterminate values are labeled explicitly.
- The engine supplies pending effects and undefined behavior explanations and
  citations. Pending effects describe the next store; the UI explains the
  prototype's atomic-store model. UB is a terminal state; no subsequent
  evaluation is requested until reset.

Programs remain in this page's memory and are not uploaded or persisted. Source
files are limited to 32 KiB. Compilation and execution run in a dedicated module
Worker. A call that takes over 10 seconds (20 seconds for initial loading)
terminates that Worker; **Retry engine** creates a new instance. No browser
cross-origin isolation, server execution API, or native trace replay is used.

## Model limitations

This is an **independent bounded C subset evaluator written in Rust**, not the
full bcc syntax/semantic frontend, LLVM backend, or IR interpreter. It supports
signed 32-bit `int`, `int *`, fixed int arrays, int-returning named functions,
int/int-pointer parameters, block locals, `if`/`else`, `while`, `for`, `return`,
assignments, arithmetic/comparisons, address/dereference/subscript, and standalone
increments. Calls must be a whole initializer, assignment RHS, return expression,
or expression statement. The demos obey this restriction.

Indeterminate reads trigger a clearly labeled conservative **policy stop**;
the prototype does not claim every indeterminate read is necessarily UB. The
array-bounds demo demonstrates a separate detected UB stop with a C99 citation.

No preprocessing, globals, library I/O, floating point, casts, structs, dynamic
allocation, or function pointers are supported. Unsupported source is rejected
explicitly. Left-to-right operand evaluation is a chosen execution policy.
Stores commit atomically, and pending effects describe the next store. Deferred
writes and full C99 sequencing/conflict checks are not implemented; nested side
effects are rejected. A completed run does not certify freedom from UB.

Engine limits are explicit terminal diagnostics: 2,048 steps, 32 frames,
128 total objects, 512 integer cells, 1,024 operands, 64 parser nesting levels,
8,192 compiled operations, and a conservative 8 MiB trace-retention budget.
The UI distinguishes resource limits and unsupported syntax from detected UB.
Pointer offsets count int elements, not bytes or host addresses. Automatic
lifetimes begin when declarations execute; C99 fixed objects instead begin
lifetime at block entry and initialize when the declaration is reached. Ended
objects remain visible as lifetime tombstones. `main`'s return value appears at completion.

## Files and integration boundary

`index.html` and `style.css` own the responsive layout. `app.js` owns controls and
bounded snapshot history. `render.js` renders text with DOM APIs and never
interprets program text as HTML. `demos.js` contains only C source programs.
`engine.js` exposes `connectEngine()` with asynchronous `load(source)` and
`step()` methods returning normalized snapshots. `engine-worker.js` owns the
Wasm instance, and `wasm-session.js` validates raw ABI v1 and engine JSON schema
v1. The ABI has no imports and exchanges UTF-8 through `bcc_alloc`, `bcc_compile`,
`bcc_step`, and response-pointer/length exports. Input allocations are freed once;
response bytes are decoded before another call. Every call refreshes its memory
view. ABI/schema mismatches stop with a visible error.

The UI's Back history is presentation-only; the Rust instance stays at the newest
recorded state. Forward replay reaches that point before requesting a new Rust
step. The adapter also exposes Rust-native `back`, `reset`, and `state` to the
integration check. No C semantics or computed program values live in JavaScript.

`check.mjs` executes the real Wasm artifact with the UI adapter and checks the
demo's result, changed-source result, function calls, arrays/pointers, lifetimes,
pending effects, UB citations, rejection, and Rust-native reverse replay/reset.
`abi-check.mjs` checks raw export/allocation contracts, rejected input bytes,
trace export and reverse replay. `worker-check.mjs` also exercises the actual Worker/proxy modules through a
Node messaging/fetch shim, including compile/step, reload, cancellation and
missing-artifact errors. It does not simulate C values or render the page.
Generated artifacts belong in ignored `web/pkg/`; binary output is not committed.

## Verification

The documented `sh web/build.sh` path and the real-artifact `check.mjs` passed
on 2026-10-21. The tour returns `15`; changing its input array to `{4, 6, 8}`
returns `21`. Checks cover actual calls, pointer writes into arrays, indeterminate
storage, ended lifetimes, pending stores, separate policy/UB stops with citations,
Rust reverse replay/reset, rejected source, UTF-8 columns, and runtime limits.
The local generated artifact is `web/pkg/bcc_rust.wasm` (about 1.3 MiB with the
repository's release debug information).

JavaScript and shell syntax checks, HTML ID/ARIA/asset checks, local HTTP asset
checks, and `git diff --check` also passed. T3 browser preview and HTML preview both failed because the host lacks the
required browser shared libraries. Browser rendering, mobile layout, screenshots
and assistive-technology behavior remain unverified. See the research document
for the exact prerequisite error. The launch commands above provide a local
manual check. Rust test, Clippy and Rustfmt results are recorded there as well.
