# C abstract machine UI

An accessible static debugger for the bcc Rust abstract machine, written in
Rust with [Leptos](https://leptos.dev) and built by [Trunk](https://trunkrs.dev)
into a self-contained static site in `web/dist/`. The page's initial HTML is
rendered at build time (static SSR) and hydrated in the browser, so the source,
panels and help text are visible before the Wasm loads.
The page renders engine snapshots; it does not evaluate C. The engine is the
bcc crate's `abstract-machine` module, linked as a Rust dependency and run in a
Web Worker. There is no hand-written JavaScript: the only scripts are the glue
wasm-bindgen generates and a one-line Worker loader the build writes.

Run all commands from `/home/agent/workspace/bcc-rust/worktrees/abstract-machine-site`
on branch `codex/abstract-machine-site`. Use the repository's pinned Rust 1.99.0
toolchain. Install its Wasm standard library and Trunk once:

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk --locked   # or a prebuilt release from trunkrs.dev
sh web/build.sh
python3 -m http.server 8080 --bind 127.0.0.1 --directory web/dist
```

Open `http://127.0.0.1:8080`. Serve over HTTP; Workers and Wasm should not be
opened with `file://`. For iterative UI work, run `trunk serve` in `web/`
instead.

## Build

`web/build.sh` runs `trunk build --release` in `web/` (niced,
`CARGO_BUILD_JOBS=3` unless set) and removes Trunk's lock file from the output.
`web/` is its own Cargo workspace (`bcc-web`, with its own `Cargo.lock`), and
one crate builds three ways, one per feature:

- `hydrate`: the page (`src/main.rs`), which attaches to the prerendered HTML.
- `worker`: the engine Worker (`src/bin/engine_worker.rs`). Only this build
  links the engine crate in `../`, with default features off, so no native
  compiler or LLVM is built.
- `ssr`: the native `prerender` binary (`src/bin/prerender.rs`).

Trunk compiles the first two to Wasm, runs wasm-bindgen and wasm-opt
(downloading the versions matching `Cargo.lock` on first use), and hashes the
page's asset names. Its `post_build` hook then runs
`prerender` on the staged output. It renders the page's fixed initial state
(the first demo, no engine connected) into `index.html`; moves the Worker's
script and Wasm, which Trunk does not hash, into `engine-<hash>/` named after
their contents; writes the Worker's loader there; and records the loader's URL
in a `<meta name="bcc-engine-worker">` tag that the page reads. That binary is
the only server-side code. It runs once per build and never serves requests.
No npm packages, network assets, or service workers are used at run time.

## Deploy

`web/dist/` is the whole site: upload it as-is to any static host or CDN. There
is no server code or routing. Asset URLs are relative (`public_url = "./"`), so
it works at the origin root or under any path prefix, provided the page is
requested at a URL ending in `/` (or as `index.html`). All files must share an
origin, because browsers only start Workers from the page's origin.

Every file except `index.html` has a content hash in its URL
(`bcc-web-<hash>.js`, `style-<hash>.css`, `engine-<hash>/…`), so a new build
never reuses a URL for different bytes. Suggested caching:

- `index.html`: `Cache-Control: no-cache` (revalidate on every visit, so a
  deploy is picked up immediately).
- Everything else: `Cache-Control: public, max-age=31536000, immutable`.

Upload the hashed files before `index.html`, so a visitor never gets a page
whose assets are not there yet. Old hashed files can be kept for a while for
visitors with the previous page open. A `.wasm` MIME type of
`application/wasm` is preferred but not required: wasm-bindgen falls back to
buffered instantiation.

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
files are limited to 32 KiB. Compilation and execution run in a dedicated
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

The site is a Rust crate. `index.html` is Trunk's shell and `style.css` owns the
responsive layout. In `src/`, `lib.rs` maps the modules. `app.rs` owns
controls, page state and bounded snapshot history; the engine Worker starts
from an effect, so prerendering never touches browser APIs. `source_view.rs`
renders source lines, syntax colouring and the next-operation mark.
`panels.rs` renders the stack, objects, sequencing and diagnostics. Program
text and engine data are always rendered as text nodes, never as HTML.
`snapshot.rs` defines the snapshot types the page presents. `demos.rs` embeds
the C programs in `demos/`.

`engine.rs` is the page's Worker proxy. It waits for the Worker's ready
message, sends `load(source)` and `step()` requests, and terminates the Worker
on a timeout or a Worker error. `protocol.rs` defines those messages, which
cross as plain JavaScript values through `serde_wasm_bindgen`. In the Worker,
`machine.rs` drives an engine `Session` and converts each engine `State` to a
`Snapshot`, mapping UTF-8 byte offsets to UTF-16 editor columns.

The page's Back history is presentation-only; the engine session stays at the
newest recorded state. Forward replay reaches that point before requesting a
new step. No C semantics or computed program values live in the page.

`cargo test --features ssr,worker` in `web/` runs the tests natively. The
`machine.rs` tests run the real engine. They check the tour's result and
changed-source result, calls, pointers, indeterminate values, pending effects,
lifetimes, the diagnostic demos' citations and locations, rejected source and
recovery, the step limit, and UTF-16 columns. The `prerender.rs` tests check
rendering into the body and the hashed Worker directory, and `source_view.rs`
tests syntax colouring. Run these with `RUSTFLAGS=`, because the repository's
`.cargo/config.toml` points host builds at the bcc LLVM linker. Generated
artifacts belong in ignored `web/target/` and `web/dist/`; binary output is
not committed.

## Verification

On 2026-10-25 `sh web/build.sh` built the site (Leptos 0.8.22, Trunk 0.22.0,
wasm-bindgen 0.2.129, Rust 1.99.0). `web/dist/` holds the prerendered
`index.html` (about 15 KB, all 20 demo source lines with syntax colouring), the
hashed page script and Wasm (46 KB and 403 KB), the stylesheet, and
`engine-<hash>/` with the Worker's glue (31 KB), Wasm (147 KB) and loader.
In `web/`, `cargo clippy -- -D warnings` passed for the `hydrate` and `worker`
(wasm32) and `ssr,worker` (native, all targets) builds, `cargo +nightly fmt
--check` passed, and `cargo test --features ssr,worker` passed all 10 tests.
The tour returns `15`; changing its input array to `{4, 6, 8}` returns `21`.

A headless smoke test (outside the repository, using jsdom, with the built
Worker run in a Node thread that emulates a classic Worker scope) loaded the
built `dist/` page and its Leptos bundle. The bundle hydrated the prerendered
markup in place, with one header and 20 source lines rather than a second
copy, and started the Worker from the hashed `<meta>` URL. The test then drove
Step, Back, Run to line, Run, the UB demo, edit-and-load, rejected source, and
a hung engine. The hung engine timed out after 10 seconds, showed the
unavailable notice, and Retry engine started a fresh Worker. A second script
served `dist/` over HTTP under `/cdn/bcc-rust/` and fetched every URL the page
and Worker reference (7, all found), and confirmed every file but `index.html` has a hashed
name. T3 browser preview still fails because the
host lacks the required browser shared libraries, so real browser rendering,
mobile layout, screenshots and assistive-technology behavior remain
unverified. The launch commands above provide a local manual check. Earlier
engine-side Rust test, Clippy and Rustfmt results are recorded in the research
document.
