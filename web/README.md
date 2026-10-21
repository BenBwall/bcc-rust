# C abstract machine UI

A dependency-free, accessible static debugger for the bcc Rust abstract machine.
The UI renders engine snapshots; JavaScript does not evaluate C. When the Wasm
artifact is missing, the page displays setup instructions and disables execution.

Run all commands from `/home/agent/workspace/bcc-rust/worktrees/abstract-machine-site`.
The engine/Wasm integration is being developed in parallel. Build instructions
will be completed here when `docs/research/abstract-machine-wasm-contract.txt`
and its Rust artifact are available. Do not run the native compiler's default
build to obtain a web engine.

Serve the UI now (the unavailable-engine state is functional):

```sh
python3 -m http.server 8080 --bind 127.0.0.1 --directory web
```

Open `http://127.0.0.1:8080`. Serve over HTTP; ES modules and Wasm should not be
opened with `file://`. No npm packages, network assets, CDNs, or service workers
are used. Browser testing is coordinated by the parent task.

## Interface

- Choose a demo, open a local C source file, or edit and load your own source.
- Step advances one engine evaluation. Run yields between evaluations; Pause
  stops before the next one. Each Run pauses after 1,000 evaluations.
- Back displays earlier real engine snapshots. Stepping forward through history
  displays those snapshots before requesting a new Rust step. Up to 2,000 states
  are retained. Reset reloads the current source in Rust.
- Select a source line number and choose **Run to line**. Execution pauses at
  the next matching source range, a terminal state, or the Run budget. A blank
  or nonexecutable line may never be reached.
- Highlighted text is the current source range supplied by the engine. Object
  cards show identity, storage duration, lifetime, and value. Pointer links jump
  to their target object's card. Indeterminate values are labeled explicitly.
- The engine supplies pending effects, sequence-point descriptions, and undefined
  behavior explanations/citations. UB is a terminal state; no subsequent
  evaluation is requested until reset.

Programs remain in this page's memory and are not uploaded or persisted. Source
files are limited to 256 KiB. This is an experimental C subset explorer; supported
syntax, execution order, and UB coverage are determined by the Rust engine.

## Files and integration boundary

`index.html` and `style.css` own the responsive layout. `app.js` owns controls and
bounded snapshot history. `render.js` renders text with DOM APIs and never
interprets program text as HTML. `demos.js` contains only C source programs.
`engine.js` is the sole Rust/Wasm adapter: `connectEngine()` returns an object
with asynchronous `load(source, filename)` and `step()` methods that each return
a normalized snapshot. It currently reports the unavailable integration
explicitly until the live Wasm contract is supplied. Generated artifacts belong
in ignored `web/pkg/`; no binary build output is committed.
