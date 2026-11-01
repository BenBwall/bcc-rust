# How C runs: the bcc site

An interactive introduction to how C programs run and how compilers translate
them. Visitors pick a lesson (a small C program) and see it three ways:

- **Abstract machine.** Step through the program one evaluation at a time.
  Each step leads with one plain sentence on what happened and why C says
  so; the full explanation waits behind a disclosure. Both come from what
  the engine actually did
  (which subexpression it evaluated, which values it read, converted or
  stored, which objects began or ended their lifetime) and cites the C99
  paragraph that requires it, linked to its page of the standard (N1256).
  The first time a run needs an idea (a sequence point, the integer
  promotions, pass by value), a "Why it matters" note says what it means
  for real programs; the standard's terms in the text show their definition
  on hover or focus and are collected in a glossary. A wired line draws
  each step's value flow (`values[1]` read → `4`; the int `200` converted
  to the char `-56`, stored into `narrow`). The exact source range is
  highlighted. The run so far is recorded as a multitrack tape (Figure 1):
  every object is a lane over the steps, its values written in hexagonal
  bus segments as in a timing diagram, cross-hatched while indeterminate;
  each call is a rule over its objects' lanes, dashed while it waits on a
  call of its own; and what a store or a call took its value from is a
  slanted line between lanes, as in Marey's train graph. A playhead marks
  the step shown, the cells under it hold the objects at that step by call,
  the step's numbered note hangs off it, the current full expression is
  wired up as a schematic whose wires fill with values (Figure 2), and a
  datasheet lists the objects (Table 1).
- **Compiler pipeline.** What bcc itself produces for the same program, from
  the first translation phases to machine code: the logical lines after
  line splicing (phases 1-2), the preprocessing tokens (phase 3), the
  preprocessed text with every macro expansion, conditional group and
  `#include` it took (phase 4), tokens (phases 5-7), syntax tree, semantic
  types (every declaration's scope, type, linkage, storage duration and size,
  and every expression's type), bcc IR before and after ABI lowering, LLVM
  IR, native x86-64 assembly, and machine code with its encoded bytes, each
  with a short explanation and, for the front end, the C99 rules it applies.
  Each stage leads with one sentence on what it takes from the stage before
  and hands to the next, with a button to that next stage. The real compiler runs in the browser and recompiles as you
  type, at `-O0` or `-O2`, with a note on what the optimizer changed. Its
  errors appear as squiggles in the source and in a diagnostics list, and the
  view says which stage stopped and why. Hovering a source line highlights
  what it became (line by line for the front-end stages and for each
  instruction of the IR that kept its source line, which at -O2 is often
  none once the optimizer rebuilds a function; per function for the rest,
  LLVM IR and the machine stages, whose output keeps no source locations),
  and the reverse. Moving the cursor to a line (with the arrow keys, or a
  tap on a touch screen) does the same.

- **WebAssembly.** The program compiled to WebAssembly by bcc, in the
  browser, executed one
  instruction at a time by a WebAssembly interpreter written in Rust. For a
  lesson, the module Clang (wasi-sdk) built ahead of time is a switch away,
  to compare the two compilers' code. The
  C source and the module's text format sit one above the other, with the C
  line running, the instruction just executed and the next one marked; the
  operand stack, locals, call stack, the blocks a branch can reach, globals
  and linear memory (the C shadow stack with each call's frame, the static
  data, the bytes just read or written) show the machine's state, and a
  narration explains each instruction for programmers new to WebAssembly.

The page opens with a map of how to read it, drawn as a train graph: one C
statement, given meaning by the abstract machine and carried by the compiler
down through its forms to machine code, each station a link to the view
that shows it.

The site is drawn as one instrument, after a hardware sequencer's front
panel (a grey device, black screens, coloured encoders, hard-edged keys,
monospace engraving) and the datasheet a run is recorded on (rules, figure
and table captions). Code is always on a black screen. Vermilion means
"now" (the playhead, the step that ran, the key that steps), blue means
data flow. Space Mono sets the device and Newsreader italic the notes'
leading sentences. Every variable keeps one colour, shown as a swatch beside
its name on the tape, in the cells and in the datasheet, and as an underline
in the source.

The machine view stacks the tape, the cells under its playhead, and the note
beside the schematic and the datasheet; where the insight column is under
760 px the schematic and the datasheet go below the note. The tape fits the
recorded run to the screen; the ZOOM encoder scales it from a quarter to four
times that, and a longer run scrolls, the tape moving under a playhead that
stays in the middle of the screen. Calls are grouped by function and depth
on the call stack, so a loop's repeated calls share one set of lanes; a group
with no call at the playhead folds to one row of call bars, and its label
opens it.

The page has light and dark themes (the device in daylight and under low
light; its screens stay black in both) and works down to phone width. At
almost every size the line that runs and the note on what it did are on
screen together: down to 821 px the source stays beside the machine (a sticky
column, sized to fit under the control strip); narrower, the device becomes a
stack of modules with the source sized so the note follows it on the same
screen, then the tape and the cells, and the transport keys docked at the
bottom of the window. Windows 600 px tall or less (a phone held sideways, a
zoomed laptop) let the header scroll away and drop the source's legends and
the encoders.

The site is written in Rust with [Leptos](https://leptos.dev) and built by
[Trunk](https://trunkrs.dev) into a static site in `web/dist/`. The page's
initial HTML is rendered at build time (static SSR) and hydrated in the
browser, so the lessons and their source are visible before any script runs.
The abstract machine is the bcc crate's `abstract-machine` module, run in a
Web Worker, beside the WebAssembly stepper (`web/stepper/`). The compiler is bcc's own, built for WebAssembly without LLVM (the
crate's `explorer` feature), run in a second Worker that the page starts only
when it first compiles.
Code is shown in [Monaco](https://microsoft.github.io/monaco-editor/), the
editor behind VS Code, with Monarch grammars for C, bcc IR, LLVM IR, AT&T
assembly, the machine-code listing and the WebAssembly text format.

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
  links the crate in `../` with only its `abstract-machine` feature, and the
  WebAssembly stepper in `stepper/`, a dependency-free crate of the same
  workspace.
- `compiler`: the compiler's Worker (`src/bin/compiler_worker.rs`). It links
  the crate in `../` with only its `explorer` feature: the whole compiler
  from preprocessor to machine code, without LLVM, the CLI or threads.
- `ssr`: the native `prerender` binary (`src/bin/prerender.rs`).

Trunk compiles the first three to Wasm, runs wasm-bindgen and wasm-opt
(downloading the versions matching `Cargo.lock` on first use), hashes the
page's asset names, and copies `editor/dist/` (the Monaco bundle) to `editor/`
and `editor/fonts/` (Space Mono regular and bold and Newsreader italic, all
SIL OFL, named with their package version) to `fonts/`. Its `post_build` hook then runs `prerender` on
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
decides everything the editor shows. Monaco is the page's largest download
(about 820 KB compressed), so it waits until the abstract machine's engine is
ready and then loads while the browser is idle, or earlier when a key is
pressed or an editor is pressed. Until then, and without JavaScript, the code
views show Rust-highlighted text, which takes the focus and passes it on to
Monaco.

## Deploy

`web/dist/` is the whole site: upload it as-is to any static host or CDN. The
Pages workflow (`.github/workflows/pages.yml`) lints, tests and builds it on
pull requests and deploys it from `main`. There is no server code or routing.
Asset URLs are relative (`public_url = "./"`), so it works at the origin root
or under any path prefix, provided the page is requested at a URL ending in
`/` (or as `index.html`). All files must share an origin, because browsers
only start Workers from the page's origin.

Every file except `index.html` and wasm-bindgen's snippet (`snippets/`) has a
content hash or version in its URL, so a new build never reuses a URL for
different bytes. Suggested caching: those two with `Cache-Control: no-cache`;
everything else with `public, max-age=31536000, immutable`. `_headers`
(copied into `dist/`) states exactly that for hosts that read one, such as
Cloudflare Pages and Netlify; GitHub Pages ignores it. Serve the text files and
the Wasm modules compressed: gzip shrinks the first load from 2.1 MB to about
840 KB. Upload the hashed files before `index.html`.

What a first visit fetches: the page, its stylesheet, the three web fonts, the
page's Wasm (the largest part, 540 KB gzip) and the abstract machine's Worker
(180 KB gzip). The Monaco bundle follows once the engine is ready, the
compiler's Worker (1.4 MB gzip) when a program is first compiled (an edit,
or the pipeline or WebAssembly view), and Monaco's icon font (150 KB) with the
first read-only listing, whose folding arrows use it.

## Interface

- Choose a lesson, open a local C file, or edit the source in place and choose
  **Run edited code**.
- **Step** (`→` or `j`) performs one evaluation; **Back** (`←` or `k`) shows
  the previous state; **Run** (`r`) steps until the program ends and **Pause**
  stops it; **Reset** (`Shift+R`) reloads the program. Shortcuts are ignored
  while typing in the editor. Click a line number to set a run target, then
  **Run to line**.
- The tape is a slider: drag it (or its playhead) to any recorded step;
  with the focus on it, the arrow keys step as everywhere, `Home` and `End`
  go to the first and last recorded step, and `Page Up`/`Page Down` move ten
  steps. The encoders are sliders too, turned by the arrow keys, the mouse
  wheel or a vertical drag: STEP scrubs through the recorded steps, SPEED
  sets Run's pause between steps (20 to 700 ms), ZOOM scales the tape.
- `Tab` moves the keyboard focus through the page, editors included, rather
  than indenting (Monaco's `tabFocusMode`). In the source editor, `Alt+M`
  (`⌥M` on a Mac) switches `Tab` to indenting and back; a note in the
  editor's corner says which while the keyboard has the focus there.
  Monaco's own toggle (`Ctrl+M`) cannot undo the option, and Firefox takes
  `Ctrl+M` to mute the tab.
- The note names the rule each step applied and its kind (evaluate,
  convert, memory, lifetime, control flow, call, stopped), and marks
  sequence points. It is layered: a one-sentence summary (the engine's
  `Event::summary`) and the cited C99 paragraphs as links come first; **How C
  defines this step** opens the full explanation, what each paragraph says,
  and links to the glossary entries of the step's terms, and stays open from
  step to step once opened. A name is narrated by what its operator does
  with the object (read it, store into it, give it its initial value, take
  its address, update it), binary operators and calls say that the operand
  order is the machine's choice (C99 §6.5p3), and undefined behavior is
  titled by its cause. Object names in the text are marked in their
  colour. Earlier steps are listed below it and can be revisited.
- In the source, a solid box marks what the last step evaluated and a dashed
  underline what runs next; undefined behavior is marked in red. The compiler
  pipeline view drops these marks and shows the compiler's instead: the
  selected stage's marks in the source and the lines linked to the output
  line under the pointer or the cursor.
- The links between the source and the pipeline's output or the
  WebAssembly text follow the pointer and the cursor of the editor with the
  keyboard focus, whichever moved last; a focused editor marks its cursor's
  line with a bar in its gutter, and a read-only view the focus enters
  starts at its first line in view. When the cursor rests on a line, a
  polite live region says what it links to ("Source line 13 became line 40
  of the bcc IR."), once, or as soon as the compiler's output arrives. An
  IR function's header and closing brace stand for the whole function, so
  they link from its first and last lines only, not from every line in it.
- Back replays retained snapshots (up to 2,000); the engine stays at the newest
  state, and stepping forward replays history before asking it for more.
- An announcement region narrates each step for screen readers, ending with
  what it did on the tape ("Tape: track n read, 4."). The tape's slider
  names the step at the playhead, the cells and the schematic carry text
  equivalents, and with reduced motion the reels stand still and the tape
  jumps rather than glides.

Programs stay in the page's memory and are not uploaded or persisted. Source
files are limited to 32 KiB. An abstract machine call that takes over 10
seconds (20 seconds for initial loading) terminates its Worker; **Retry
engine** starts a new one.

## The compiler pipeline view

Every stage except LLVM IR is compiled in the browser by bcc itself:
`bcc_rust::explorer::compile`, the same front end, optimizer and x86-64 back
end as the native build, minus LLVM. `src/explorer.rs` maps its output onto
the page's `Compilation`: the text stages as they are (the syntax tree and the
semantic types with `--syntax-locations`), each function's machine code as an
offset/bytes/instruction listing, diagnostics in the editor's UTF-16 columns, and the
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
- **Preprocessor (phase 4)** shows the text phase 4 leaves, from one
  preprocessing run with the preprocessor's opt-in trace on: laid out as
  `bcc-rust -E` lays it out (line markers included), but with each token
  spelled as written and adjacent string literals still apart, since `-E`
  shows them after phase 6 has joined them. The compiler says where each
  output line came from, so each header the source includes is folded into
  one line saying what it held (a switch shows it), its lines link to the
  `#include`, and the gutter numbers each line with the source line it came
  from. A card lists the headers, the macros defined, the other directives
  executed (`#pragma`, `#line`, `#error`, …), every expansion with its result
  and its steps (each replacement with its arguments, replacements made while
  an argument is prescanned, predefined macros such as `__LINE__`, each `#`
  and `##` and its result, nested replacements indented as a tree), and each
  conditional directive with the lines it skipped, saying which arms were
  not evaluated because an earlier group was kept. Places are physical, as
  the editor counts them; where `#line` says otherwise, the card shows that
  too. In the source, invocations are underlined with their expansion on
  hover, skipped lines are hatched, and conditionals carry a kept or
  skipped mark. The browser build has no host headers: `#include` finds only
  bcc's embedded freestanding headers (`<limits.h>`, `<stdint.h>`,
  `<stddef.h>`, `<stdbool.h>`, …), so `<stdio.h>` is reported as not found,
  and the card and the stop explanation point to the WebAssembly view, whose
  target has bcc's own C library. The pipeline does not borrow that
  library's headers: they describe bcc's WASI runtime, not an x86-64 C
  library, and the pipeline's stages are what the native `bcc-rust` prints
  (`tests/integration/explorer.rs` compares them, and the lessons' LLVM IR
  comes from the native compiler).
- **Tokens (phases 5-7)** is the `--tokens` listing of a second, ordinary
  preprocessing run.
- **Syntax tree** and **Semantic types** (phase 7) are `--syntax-tree` and
  `--semantic-types`, both with `--syntax-locations`, so each line that has a
  place in the source links to it: every scope with its declarations (type,
  linkage, storage duration, size and alignment), then each expression's
  type and value category, then the implicit conversions applied to them.

The page receives at most 20,000 phase-3 entries and 2,000 expansions and
says how many more there were; source columns are mapped line by line, so a
large source costs time in proportion to its size.

The lesson "From source to a running program" (`demos/path.c`) follows one
small program through the whole pipeline. It opens at the first stage; each
stage adds a note on what it did to this program at `-O0` (`Demo::path`,
checked against the compiler's output by a test in `src/explorer.rs`), and
the last stage's **Next** opens the abstract machine to run it, with the
focus on the narration's heading; for any other program it starts the path
again. The page's third "how to use" step opens the pipeline at its first
stage: the whole-path lesson when the editor holds a bundled lesson, the
learner's own program (opened or edited) otherwise.

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
- **Bounded.** The page compiles at most 64 KiB of source and says so for a
  larger program without loading the compiler; the explorer itself refuses
  more in WebAssembly (`WASM_MAX_SOURCE_BYTES`; 1 MiB on a native host). In
  the browser each of the compiler's arenas is one fixed 32 MiB region of
  linear memory, and filling one ends the module with a trap. Measured on
  programs of one dense shape each (many short declarations, one long line
  of them, many declarators, a long initializer, a long sum, many small
  functions, many macro invocations, many locals, one long string, many
  comments), 64 KiB compiles them all within the page's ten seconds, while
  96 KiB of initializer elements fills the parser's arena and 512 KiB fills
  one arena or another in every shape but comments. The explorer stops at
  500 levels of nesting with a diagnostic, and its trace of phase 4 stops
  after 50,000 steps or 8 MiB, each argument cut at 160 bytes (the card
  says so). The trace has an arena of its own, a quarter of which it may
  use, and charges every byte to that budget before allocating it, so a
  macro that expands enormously cannot fill an arena through it: tracing
  makes no program fail that compiles without it. A compilation that takes
  over 10 seconds terminates the compiler Worker, as does a trap, which
  leaves the Wasm instance unusable. The view says why: a Rust panic sends
  its message before the trap; a trap without one is a failed allocation, an
  arena that filled ("bcc ran out of memory …"); an overflow of the
  engine's call stack means the program nests too deeply. The next edit
  starts a fresh compiler.
- **Diagnostics.** Errors and warnings are Monaco markers in the source, with
  the message on hover, and a list under the stage with each message's place
  (click it to show the line), the part of the compiler that reported it, and
  the full rendered message. In the WebAssembly view the markers come instead
  from bcc's `wasm32-wasip1` compilation of the editor's text (debounced the
  same way; loading the program reuses it), whose C library headers the
  x86-64 compilation lacks. When an error stops bcc, the stage strip marks
  the stages that did not run, and the stage explains, in beginner's terms,
  whether the front end found a mistake in the program, or the middle or back
  end met a construct bcc does not translate yet.
- **`-O0` / `-O2`.** The switch recompiles with bcc's optimizer. A note says
  what the optimizer does at the selected stage and, by comparing both
  levels, what changed in this program: instruction, block, memory-access and
  stack-slot counts, code size, and calls that were inlined away.

## The WebAssembly view

The view steps a WebAssembly module with `stepper/` (crate
`bcc-wasm-stepper`), an interpreter written for teaching rather than speed.
It decodes and validates a module (the 2.0 core specification: MVP plus
mutable globals, sign extension, non-trapping float-to-int conversion, bulk
memory, multiple values and reference types; no SIMD, threads, tail calls or
exceptions), prints it in the text format one instruction per line, and runs
it one instruction per step, recording what each step popped, pushed, read,
wrote, branched to or called. Traps stop the program with their reason. Its
host provides a WASI preview 1 subset (`fd_write` to standard output and
error, `proc_exit`, arguments, environment, a simulated clock, seeded
randomness), answers `ENOSYS` for the other preview 1 functions, and leaves
other modules' imports unresolved: calling one traps. It is deterministic,
so Back re-executes from the nearest checkpoint (one every 1,024
instructions, thinned as a run grows). Linear memory is copied on write,
so a checkpoint shares unchanged memory, and the checkpoints together keep
at most 64 MiB. The two most recent copies at multiples of 32,768
instructions are kept apart from that budget, so stepping back near the
current instruction re-executes at most 65,536 instructions even in a
program whose memory alone fills the budget. One session runs at most 50
million instructions, one request at most 2 million, and memory may grow to
64 MiB.

Controls: **Step** (`→` or `j`) executes one instruction; **Over** (`o`)
runs a call to its return; **Out** (`u`) runs until the current function
returns; **Next line** (`n`) runs to the next C line, through calls; **Run**
(`r`) goes line by line until the program ends or reaches a breakpoint;
**Back** (`←` or `k`) and **Reset** (`Shift+R`) go back. Click a line
number in the listing to set a breakpoint, or in the C source to set a run
target. Hover either side to find the lines the other made. **Start at
main** (on unless turned off; the browser's local storage keeps that) runs
the code before `main` (the C library's startup and any constructors) when
a program loads or is reset, up to the first instruction of `main`
(`WasmControl::StartAtMain`, at most 100,000 instructions, else it stays at
the start), or to a breakpoint set in that code, and the narration says
what it skipped. Back still steps through the startup. A module whose entry point is
`main` itself, as Clang's lessons are, starts there anyway.

Where modules come from is behind the `WasmBackEnd` trait in `src/wasm/`:
C source in, a `WasmModule` out: the binary, optionally the C line of each
instruction (`WasmLine { offset, line, column }`; a row covers its
instruction and those after it up to the next row's offset, and line 0 is
code with no C line), and optionally the compiler's own text format. There
are two:

- `Bcc` (`src/wasm/bcc.rs`) runs in the compiler Worker:
  `bcc_rust::explorer::compile` for `wasm32-wasip1` at `-O0`, as
  `bcc-rust program.c --target=wasm32-wasip1 -std=c17 -O0` would. Each
  source-map position becomes a line row (bcc records no columns, so the
  column is 0, and a prologue's line is 0; code a macro expands to has the
  invocation's line, as in Clang's line tables, except that a macro invoked
  in an argument that starts on a later line than the outer invocation has
  the outer invocation's line), and bcc's `.wat`, with the line
  of every instruction (`ExplorerWasm::instructions`) and each function's
  range (`functions`), becomes the listing, in place of the stepper's own
  printout. The engine Worker places each step, call frame and block in it
  by module offset. bcc's module is a WASI command: `$_start`, which bcc's
  bundled C library adds, runs the constructors, reads the arguments
  (`args_sizes_get`, `args_get`; up to eight, 256 bytes, in static storage,
  so the heap is untouched), calls `main` and passes its result to
  `proc_exit`, 125 instructions before `main` at `-O0`. When bcc refuses a program for WebAssembly (`setjmp`,
  `_Float16`, …), the view shows its diagnostics.
- `Precompiled` serves the lessons, compiled by clang at `-O0 -g` into
  `web/wasm/`, whose DWARF line tables the stepper reads.

The view steps bcc's module unless the learner switches to Clang's, which
only the lessons have; the browser's local storage keeps that choice for
later visits. A lesson falls back to Clang's module, saying why, when bcc
has none to step: bcc refused the program (its diagnostics are shown), the
program is more than bcc can compile in the browser (it fills one of the
compiler's fixed memory regions or nests too deeply), the browser stopped
the compilation (a timeout or a stopped Worker: **Compile again** retries),
or the stepper cannot load bcc's module (one needing more than its 64 MiB
of memory). Each is tried once, so a failure is shown, not retried in a
loop. After the engine Worker restarts, the module is loaded into the new
one without compiling again. Edited source is compiled by bcc when it runs (**Run
edited code**). The repository's pinned LLVM targets x86-64 only, so
regenerate the Clang modules with a clang that targets wasm32, such as
wasi-sdk's:

```sh
CLANG=path/to/wasi-sdk/bin/clang sh web/wasm/generate.sh           # rewrite
CLANG=path/to/wasi-sdk/bin/clang sh web/wasm/generate.sh --check   # compare only
```

Each module records the flags and the exact source it was built from in a
`bcc.lesson` custom section, and the site's tests compare that with the
lessons, so a stale module fails CI, which has no wasm32 clang.

## Files

`index.html` is Trunk's shell and `style.css` the design (tokens for both
themes, layout, components, and the classes the editor decorations use). In
`src/`, `lib.rs` maps the modules. `app.rs` owns the page state, controls,
keyboard shortcuts and layout. `panels/` renders the note (`narration.rs`),
the tape (`tape.rs`, which also records the run's lanes), the cells under its
playhead, the expression schematic, the objects' datasheet, the encoders
(`knob.rs`) and the pipeline view, with the front-end stages' cards and
source marks in `panels/front_end.rs`. `editor.rs` mounts Monaco, and
`highlight.rs` colours the prerendered fallback. `app/wasm.rs` holds the
WebAssembly view's page state and controls, `panels/wasm.rs` (with
`panels/wasm/`) draws it, and `wasm.rs` (with `wasm/`) defines its data, the
`WasmBackEnd` seam with bcc's back end (`wasm/bcc.rs`, compiler Worker), and, in the engine Worker, the stepper session
(`wasm/worker.rs`) and the narration (`wasm/narration.rs`). The interpreter
itself is `stepper/`, mapped in its `src/lib.rs`. `palette.rs` assigns each
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
the front-end stages, and the optimizer comparison; prerendering; every
lesson stepped through its WebAssembly with a placed, narrated state at each
instruction; and every lesson compiled by bcc for `wasm32-wasip1`, stepped
through bcc's own text to the exit status Clang's module returns, the
conversion of bcc's output, its diagnostics for a program it refuses, and the
choice between the two compilers. `RUSTFLAGS= cargo test -p bcc-wasm-stepper`
runs the interpreter's tests: each instruction family against the
specification's values, control flow, traps, memory bounds, decoding and
validation, the stepping commands, the WASI host, and the lesson modules. The
engine's own
tests run from the repository root with `cargo test --lib --no-default-features
--features abstract-machine abstract_machine`.

The crate enables clippy's pedantic group (`web/Cargo.toml`), and the Pages
workflow's lint job fails on any warning, natively and for each wasm32 build:

```sh
RUSTFLAGS= cargo clippy --workspace --all-targets --features ssr,worker,compiler -- -D warnings
cargo clippy --target wasm32-unknown-unknown --features hydrate -- -D warnings   # and worker, compiler
```

Generated artifacts belong in ignored `web/target/`, `web/dist/` and
`web/editor/{node_modules,dist,fonts}/`.
