# How C runs: the bcc site

An interactive introduction to how C programs run and how compilers translate
them. Visitors pick a lesson (a small C program) and see it two ways:

- **Abstract machine.** Step through the program one evaluation at a time.
  Each step is explained in plain language from what the engine actually did
  (which subexpression it evaluated, which values it read, converted or
  stored, which objects began or ended their lifetime) and cites the C99
  paragraph that requires it. The exact source range is highlighted, the
  current full expression is drawn as a tree whose values fill in as it is
  evaluated, and memory is drawn as call frames holding objects, with
  pointers as arrows.
- **Compiler pipeline.** What bcc itself produces for the same program:
  tokens, syntax tree, bcc IR before and after ABI lowering, LLVM IR, native
  x86-64 assembly, and machine code with its encoded bytes, each with a short
  explanation. Hovering a source line highlights what it became (line by line
  for tokens and the syntax tree, per function for the IR and machine stages,
  whose output keeps no source locations), and the reverse.

Every variable keeps one colour across the source, the expression tree and the
memory diagram. The page has light and dark themes and works down to tablet
width.

The site is written in Rust with [Leptos](https://leptos.dev) and built by
[Trunk](https://trunkrs.dev) into a static site in `web/dist/`. The page's
initial HTML is rendered at build time (static SSR) and hydrated in the
browser, so the lessons and their source are visible before any script runs.
The engine is the bcc crate's `abstract-machine` module, run in a Web Worker.
Code is shown in [Monaco](https://microsoft.github.io/monaco-editor/), the
editor behind VS Code, with Monarch grammars for C, bcc IR, LLVM IR, AT&T
assembly and the machine-code listing.

## Build

Use the repository's pinned Rust 1.99.0 toolchain, plus Node.js 22 and npm.
Install the Wasm standard library and Trunk once:

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk --locked   # or a prebuilt release from trunkrs.dev
sh web/build.sh
python3 -m http.server 8080 --bind 127.0.0.1 --directory web/dist
```

Open `http://127.0.0.1:8080`. Serve over HTTP; Workers, Wasm and module
scripts do not work from `file://`.

`web/build.sh` first builds the editor bundle in `web/editor/` (`npm ci`, then
`node build.mjs`, which runs esbuild), then runs `trunk build --release` in
`web/` (niced, `CARGO_BUILD_JOBS=3` unless set) and removes Trunk's lock file
from the output. `web/` is its own Cargo workspace (`bcc-web`, with its own
`Cargo.lock`), and one crate builds three ways, one per feature:

- `hydrate`: the page (`src/main.rs`), which attaches to the prerendered HTML.
- `worker`: the engine Worker (`src/bin/engine_worker.rs`). Only this build
  links the engine crate in `../`, with default features off, so no native
  compiler or LLVM is built.
- `ssr`: the native `prerender` binary (`src/bin/prerender.rs`).

Trunk compiles the first two to Wasm, runs wasm-bindgen and wasm-opt
(downloading the versions matching `Cargo.lock` on first use), hashes the
page's asset names, and copies `editor/dist/` (the Monaco bundle) to `editor/`
and `editor/fonts/` (Inter and JetBrains Mono, both SIL OFL, named with their
package version) to `fonts/`. Its `post_build` hook then runs `prerender` on
the staged output. It renders the page's fixed initial state (the first
lesson, no engine connected) into `index.html`; moves the Worker's script and
Wasm into `engine-<hash>/` and the editor bundle into `editor-<hash>/`, each
named after its contents; writes the Worker's loader; and records both URLs in
`<meta>` tags the page reads. Nothing is fetched from a CDN at run time.

### JavaScript

The site's logic is Rust. Two pieces of hand-written JavaScript remain, both
for the editor: `js/bridge.js`, a one-line dynamic `import()` that wasm-bindgen
binds (Rust cannot express one), and the Monaco glue in `editor/src/` (editor
creation, decorations and events behind a small plain-data API, the Monarch
grammars and the themes). `src/editor.rs` holds the wasm-bindgen bindings and
decides everything the editor shows. Monaco loads after hydration; until then,
and without JavaScript, the code views show Rust-highlighted text.

## Deploy

`web/dist/` is the whole site: upload it as-is to any static host or CDN. The
Pages workflow (`.github/workflows/pages.yml`) tests and builds it on pull
requests and deploys it from `main`. There is no server code or routing. Asset
URLs are relative (`public_url = "./"`), so it works at the origin root or
under any path prefix, provided the page is requested at a URL ending in `/`
(or as `index.html`). All files must share an origin, because browsers only
start Workers from the page's origin.

Every file except `index.html` has a content hash or version in its URL, so a
new build never reuses a URL for different bytes. Suggested caching:
`index.html` with `Cache-Control: no-cache`; everything else with
`public, max-age=31536000, immutable`. Upload the hashed files before
`index.html`.

## Interface

- Choose a lesson, open a local C file, or edit the source in place and choose
  **Run edited code**.
- **Step** (`→` or `j`) performs one evaluation; **Back** (`←` or `k`) shows
  the previous state; **Run** (`r`) steps until the program ends and **Pause**
  stops it; **Reset** (`Shift+R`) reloads the program. Shortcuts are ignored
  while typing in the editor. Click a line number to set a run target, then
  **Run to line**.
- The narration card names the rule each step applied, colour coded by kind
  (evaluate, convert, memory, lifetime, control flow, call, stopped), marks
  sequence points, and lists the cited C99 paragraphs with a gloss. Earlier
  steps are listed below it and can be revisited.
- In the source, a solid box marks what the last step evaluated and a dashed
  underline what runs next; undefined behavior is marked in red.
- Back replays retained snapshots (up to 2,000); the engine stays at the newest
  state, and stepping forward replays history before asking it for more.
- An announcement region narrates each step for screen readers.

Programs stay in the page's memory and are not uploaded or persisted. Source
files are limited to 32 KiB. A Worker call that takes over 10 seconds (20
seconds for initial loading) terminates the Worker; **Retry engine** starts a
new one.

## The compiler pipeline view

The bundled lessons carry outputs precomputed by the native bcc build in
`web/pipeline/<lesson>/`. Regenerate them after changing a lesson or the
compiler, from the repository root:

```sh
BCC=path/to/bcc-rust sh web/pipeline/generate.sh           # rewrite
BCC=path/to/bcc-rust sh web/pipeline/generate.sh --check   # compare only
```

The script needs a native `bcc-rust` (built with LLVM) and GNU `objdump`. The
site's tests check that every lesson's outputs were generated from its current
source.

Edited source is compiled live in the Worker through `explorer.rs` once bcc's
compiler builds for wasm32 (`bcc_rust::explorer::compile`). Until the Worker
links it, `Command::Compile` answers with an error and the view explains that
compiler outputs are available for the bundled lessons. LLVM IR is only ever
precomputed: the browser build of bcc has no LLVM.

## Model limitations

The engine is an **independent bounded C subset evaluator written in Rust**,
not bcc's syntax/semantic frontend, LLVM backend or IR interpreter. It supports
signed 32-bit `int` and signed 8-bit plain `char` locals (as on x86-64), `int
*`, fixed int arrays, int-returning named functions, int and int-pointer
parameters, block locals, `if`/`else`, `while`, `for`, `return`, assignments,
arithmetic and comparisons, address-of, dereference, subscripts, and
standalone increments. Calls must be a whole initializer, assignment right-hand
side, return expression, or expression statement.

Indeterminate reads stop with a clearly labeled **policy stop**; the engine
does not claim every indeterminate read is undefined. Signed overflow,
division by zero, out-of-bounds access, null indirection and use after a
lifetime ends stop as detected undefined behavior with a citation.

No preprocessing, globals, library I/O, floating point, casts, structs,
pointers to char, dynamic allocation or function pointers. Unsupported source
is rejected explicitly. Operands are evaluated left to right (a chosen
policy); stores commit atomically; nested side effects are rejected, so
unsequenced-modification checks are not needed. Automatic objects appear when
their declaration is reached, though C99 begins their lifetime on entry to the
block; the narration says so. A completed run does not certify freedom from
undefined behavior.

Engine limits are explicit terminal diagnostics: 2,048 steps, 32 frames, 128
objects, 512 integer cells, 1,024 operands, 64 nesting levels, 8,192 compiled
operations and an 8 MiB trace-retention budget.

## Files

`index.html` is Trunk's shell and `style.css` the design (tokens for both
themes, layout, components, and the classes the editor decorations use). In
`src/`, `lib.rs` maps the modules. `app.rs` owns the page state, controls,
keyboard shortcuts and layout. `panels/` renders the narration, expression
tree, memory diagram and pipeline view. `editor.rs` mounts Monaco, and
`highlight.rs` colours the prerendered fallback. `palette.rs` assigns each
variable its colour. `snapshot.rs` defines what the page presents, `demos.rs`
the lessons, and `pipeline.rs` the compiler stages, their explanations, the
bundled outputs and the source-to-output links. `engine.rs` proxies the
Worker over the messages in `protocol.rs`; in the Worker, `machine.rs` drives
an engine `Session` and turns each state into a snapshot (UTF-8 byte ranges
become UTF-16 editor columns), and `explorer.rs` is the seam to the
in-browser compiler.

## Tests

From `web/`, `RUSTFLAGS= cargo test --features ssr,worker` runs the site's
tests natively (`RUSTFLAGS=` keeps the repository's LLVM linker flags away from
this host build): every lesson runs to its documented outcome with a titled,
cited event at each step; event ranges, expression trees and waiting frames;
diagnostics and recovery; UTF-16 columns; colouring; the outputs' sync with
the lessons and the source-to-output links; and prerendering. The engine's own
tests run from the repository root with `cargo test --lib --no-default-features
--features abstract-machine abstract_machine`. Generated artifacts belong in
ignored `web/target/`, `web/dist/` and `web/editor/{node_modules,dist,fonts}/`.
