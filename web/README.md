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
- **Compiler pipeline.** What bcc itself produces for the same program, from
  the first translation phases to machine code: the logical lines after
  line splicing (phases 1-2), the preprocessing tokens (phase 3), the
  preprocessed text with every macro expansion, conditional group and
  `#include` it took (phase 4), tokens (phases 5-7), syntax tree, bcc IR
  before and after ABI lowering, LLVM IR, native x86-64 assembly, and machine
  code with its encoded bytes, each with a short explanation and, for the
  front end, the C99 rules it applies. The real compiler runs in the browser and recompiles as you
  type, at `-O0` or `-O2`, with a note on what the optimizer changed. Its
  errors appear as squiggles in the source and in a diagnostics list, and the
  view says which stage stopped and why. Hovering a source line highlights
  what it became (line by line for the front-end stages, per function for
  the IR and machine stages, whose output keeps no source locations), and
  the reverse.

Every variable keeps one colour across the source, the expression tree and the
memory diagram. The page has light and dark themes and works down to tablet
width.

The site is written in Rust with [Leptos](https://leptos.dev) and built by
[Trunk](https://trunkrs.dev) into a static site in `web/dist/`. The page's
initial HTML is rendered at build time (static SSR) and hydrated in the
browser, so the lessons and their source are visible before any script runs.
The abstract machine is the bcc crate's `abstract-machine` module, run in a
Web Worker. The compiler is bcc's own, built for WebAssembly without LLVM (the
crate's `explorer` feature), run in a second Worker that the page starts only
when it first compiles.
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
- `worker`: the abstract machine's Worker (`src/bin/engine_worker.rs`). It
  links the crate in `../` with only its `abstract-machine` feature.
- `compiler`: the compiler's Worker (`src/bin/compiler_worker.rs`). It links
  the crate in `../` with only its `explorer` feature: the whole compiler
  from preprocessor to machine code, without LLVM, the CLI or threads.
- `ssr`: the native `prerender` binary (`src/bin/prerender.rs`).

Trunk compiles the first three to Wasm, runs wasm-bindgen and wasm-opt
(downloading the versions matching `Cargo.lock` on first use), hashes the
page's asset names, and copies `editor/dist/` (the Monaco bundle) to `editor/`
and `editor/fonts/` (Inter and JetBrains Mono, both SIL OFL, named with their
package version) to `fonts/`. Its `post_build` hook then runs `prerender` on
the staged output. It renders the page's fixed initial state (the first
lesson, no engine connected) into `index.html`; moves each Worker's script and
Wasm into `engine-<hash>/` and `compiler-<hash>/`, and the editor bundle into
`editor-<hash>/`, each named after its contents; writes each Worker's loader;
and records the three URLs in `<meta>` tags the page reads. Nothing is fetched
from a CDN at run time.

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
files are limited to 32 KiB. An abstract machine call that takes over 10
seconds (20 seconds for initial loading) terminates its Worker; **Retry
engine** starts a new one.

## The compiler pipeline view

Every stage except LLVM IR is compiled in the browser by bcc itself:
`bcc_rust::explorer::compile`, the same front end, optimizer and x86-64 back
end as the native build, minus LLVM. `src/explorer.rs` maps its output onto
the page's `Compilation`: the text stages as they are (the syntax tree with
`--syntax-locations`), each function's machine code as an offset/bytes/
instruction listing, diagnostics in the editor's UTF-16 columns, and the
compiler's per-function `line_map`, which links source lines to the IR,
assembly and listing.

The bundled lessons go through the same live path as edited source. Their
outputs used to be precomputed by the native CLI; compiling them live means
the lesson view is exactly what the in-browser compiler does, there is one
code path to test, and the optimization switch, diagnostics and line map work
the same for every program. The cost is loading the compiler Worker before a
lesson's pipeline view first appears. Only LLVM IR, which the browser build
cannot produce, is still precomputed for the lessons, at `-O0` and `-O2`, in
`web/pipeline/<lesson>/`. Edited source shows a note instead. Regenerate the
LLVM IR after changing a lesson or the compiler, from the repository root:

```sh
BCC=path/to/bcc-rust sh web/pipeline/generate.sh           # rewrite
BCC=path/to/bcc-rust sh web/pipeline/generate.sh --check   # compare only
```

The script needs a native `bcc-rust` (built with LLVM). The site's tests check
that each lesson's LLVM IR was generated from its current source, and the main
CI workflow's Tests job runs `generate.sh --check` with the compiler it just
built, so a compiler change that alters it fails CI until it is regenerated.
The crate's `tests/integration/explorer.rs` checks that the explorer's other
stages equal the CLI's for every lesson.

The front end is shown phase by phase, each from the compiler's own output
for that phase (`bcc_rust::explorer::compile` returns all of them; the page
reimplements none):

- **Logical lines (phases 1-2)** come from the lexer's own `splice`: each
  line after trigraph replacement, new-line mapping and line splicing, with
  the physical lines it came from in the gutter (`8–12`), `↩` where a splice
  joined two, and in the source the backslash it deleted. A card lists every
  splice, trigraph, CRLF and UTF-8 character. Trigraphs are replaced in the
  ISO modes (the site compiles as `c17`), not the GNU ones.
- **Preprocessing tokens (phase 3)** are the entries the lexer saved for the
  main file, read as phase 4 reads them: one per line with its kind, its
  spelling, and whether it starts a line, follows white space, was a comment
  (now one space), spans a splice or is read by `#include` as a header name.
  bcc's lexer forms no header-name tokens; the `#include` handler reads the
  operand's tokens back as one name, and the listing says which. Hovering an
  entry boxes its exact range in the source.
- **Preprocessor (phase 4)** shows what `bcc-rust -E` prints, from one
  preprocessing run with the preprocessor's opt-in trace on (the same run
  gives the Tokens stage's listing). Each header the source includes is
  folded into one line saying what it held, with a switch to show it; the
  gutter numbers each line with the source line it came from. A card lists
  the headers, the macros defined, every expansion with its result and its
  steps (each replacement with its arguments, each `#` and `##` and its
  result, nested replacements indented), and each conditional directive with
  the lines it skipped. In the source, invocations are underlined with their
  expansion on hover, skipped lines are hatched, and conditionals carry a
  kept or skipped mark. The browser build has no host headers: `#include`
  finds only bcc's embedded freestanding headers (`<limits.h>`,
  `<stdint.h>`, `<stddef.h>`, `<stdbool.h>`, …), so `<stdio.h>` is reported
  as not found.

The lesson "Before the compiler: the preprocessor" (`demos/macros.c`)
exercises these stages and opens at the preprocessor. The abstract machine
has no preprocessor, so it rejects that lesson as outside its subset.

How live compilation runs:

- **Off the UI thread, in its own Worker.** The compiler Worker's Wasm is many
  times the abstract machine's, so the page starts it only when it first
  compiles: when the pipeline view opens, or when the source is edited (so
  errors show as squiggles in the abstract machine view too). A lesson in the
  abstract machine view never loads it. Keeping the compiler apart also means
  that stopping it never loses the abstract machine's session.
- **Debounced.** Typing compiles 400 ms after the last keystroke. One
  compilation runs at a time: the selected level first, then the other level
  for the optimizer comparison. A result for text the editor no longer holds
  is stale and dropped. While the next result is on its way, the view keeps
  the previous one, so typing does not make it flicker. The last eight
  compilations are kept, so switching level or going back to a lesson is
  instant.
- **Bounded.** The explorer refuses more than 1 MiB of source and stops at
  500 levels of nesting with a diagnostic. A compilation that takes over 10
  seconds terminates the compiler Worker, as does a trap (an exhausted
  stack or memory region leaves the Wasm instance unusable). The view says the
  compiler stopped, and the next edit starts a fresh one.
- **Diagnostics.** Errors and warnings are Monaco markers in the source, with
  the message on hover, and a list under the stage with each message's place
  (click it to show the line), the part of the compiler that reported it, and
  the full rendered message. When an error stops bcc, the stage strip marks
  the stages that did not run, and the stage explains, in beginner's terms,
  whether the front end found a mistake in the program, or the middle or back
  end met a construct bcc does not translate yet.
- **`-O0` / `-O2`.** The switch recompiles with bcc's optimizer. A note says
  what the optimizer does at the selected stage and, by comparing both
  levels, what changed in this program: instruction, block, memory-access and
  stack-slot counts, code size, and calls that were inlined away.

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
tree, memory diagram and pipeline view, with the front-end stages' cards and
source marks in `panels/front_end.rs`. `editor.rs` mounts Monaco, and
`highlight.rs` colours the prerendered fallback. `palette.rs` assigns each
variable its colour. `snapshot.rs` defines what the page presents, `demos.rs`
the lessons, and `pipeline.rs` the compiler stages, their explanations and
citations, the source-to-output links, why a compilation stopped, the
optimizer comparison and the bundled LLVM IR; `pipeline/front_end.rs` holds
the front-end phases' data, the folded preprocessed text and its line
links. `engine.rs` proxies both Workers over the messages in
`protocol.rs`. In the abstract machine's Worker, `machine.rs` drives an engine
`Session` and turns each state into a snapshot (UTF-8 byte ranges become
UTF-16 editor columns); in the compiler's, `explorer.rs` runs bcc and turns
its output into a `Compilation`.

## Tests

From `web/`, `RUSTFLAGS= cargo test --features ssr,worker,compiler` runs the
site's tests natively (`RUSTFLAGS=` keeps the repository's LLVM linker flags
away from this host build): every lesson runs to its documented outcome with
a titled, cited event at each step; event ranges, expression trees and
waiting frames; diagnostics and recovery; UTF-16 columns; colouring; every
lesson compiling live at both levels; placed compiler diagnostics and the
stop explanation; the machine-code listing; the LLVM IR's sync with the
lessons; the source-to-output links, the folded headers and gutter labels of
the front-end stages, and the optimizer comparison; and prerendering. The engine's own
tests run from the repository root with `cargo test --lib --no-default-features
--features abstract-machine abstract_machine`. Generated artifacts belong in
ignored `web/target/`, `web/dist/` and `web/editor/{node_modules,dist,fonts}/`.
