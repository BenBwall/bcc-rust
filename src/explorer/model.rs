//! The explorer's options and outputs. They hold only owned standard types,
//! so they cross a Web Worker boundary or serialize (with the `serde`
//! feature) without borrowing the compiler's arenas.

#![expect(
    clippy::disallowed_types,
    reason = "The explorer's interface hands owned strings and vectors to its caller, outside \
              every arena."
)]

/// What to compile and how.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct ExplorerOptions {
    /// The `--target` triple, `x86_64-unknown-linux-gnu` by default.
    pub target:            String,
    /// The `--std` language mode, `c17` by default.
    pub std:               String,
    /// Run bcc's optimizer and define `__OPTIMIZE__`, as `-O2` does; without
    /// it, the module is lowered as at `-O0`.
    pub optimize:          bool,
    /// The `--max-nesting-depth` limit. `None` picks the CLI's default on a
    /// native host and
    /// [`WASM_MAX_NESTING_DEPTH`](super::WASM_MAX_NESTING_DEPTH)
    /// in WebAssembly, whose stack is far smaller.
    pub max_nesting_depth: Option<u32>,
    /// The `--source-date-epoch` that `__DATE__` and `__TIME__` spell, in
    /// seconds since the Unix epoch. `None` spells the epoch itself, so the
    /// output never depends on the clock.
    pub source_date_epoch: Option<i64>,
    /// Headers the source can include, by the name it includes them by
    /// (`#include "list.h"` finds `list.h`), searched before the embedded
    /// resource headers. The host file system is never read.
    pub files:             Vec<ExplorerFile>,
    /// Ends every syntax-tree line with the `@line:column` of its node, as
    /// `--syntax-locations` does, so a viewer can link the tree to the
    /// source line by line.
    pub syntax_locations:  bool,
}

/// An in-memory header.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerFile {
    /// The name an `#include` spells, such as `list.h`.
    pub name:     String,
    /// The header's text.
    pub contents: String,
}

/// Every stage's output. A stage that failed, or that an earlier failure
/// kept from running, is `None`, and the diagnostics say why.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerOutput {
    /// The compilation's diagnostics in the order the CLI prints them, then
    /// any a back-end stage reported.
    pub diagnostics:  Vec<ExplorerDiagnostic>,
    /// The preprocessed tokens, one per line, as `--tokens` prints them.
    pub tokens:       Option<String>,
    /// The syntax tree, as `--syntax-tree` prints it.
    pub syntax_tree:  Option<String>,
    /// The bcc IR before ABI lowering, as `--emit=ir --backend=native`
    /// prints it.
    pub ir:           Option<String>,
    /// The bcc IR after ABI lowering, as `--emit=ir-abi --backend=native`
    /// prints it.
    pub ir_abi:       Option<String>,
    /// The native back end's assembly, as `--emit=asm --backend=native`
    /// prints it.
    pub asm:          Option<String>,
    /// Each function's machine code, encoded from `asm` as
    /// `--native-object` would encode it. ELF targets only.
    pub machine_code: Option<Vec<ExplorerFunctionBytes>>,
    /// For a `wasm32` target, the WebAssembly back end's module in place
    /// of `asm` and `machine_code`: its binary, its text, and where each IR
    /// position's code lies in the binary.
    pub wasm:         Option<ExplorerWasm>,
    /// Which lines of the outputs each function definition became.
    pub line_map:     Vec<ExplorerLineMapping>,
}

/// A WebAssembly module, as `--emit=wasm` and `--emit=wat` write it. For
/// `wasm32-wasip1` it is a complete WASI command (it exports `_start` and
/// `memory` and imports `wasi_snapshot_preview1`); for
/// `wasm32-unknown-unknown` it exports `memory` and every external function,
/// and imports each function it calls but does not define from `env`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerWasm {
    /// The binary module, ready for `WebAssembly.instantiate`.
    pub binary:       Vec<u8>,
    /// The same module as WebAssembly text.
    pub text:         String,
    /// Every defined function: its index (imports count first), symbol, and
    /// code range in `binary`.
    pub functions:    Vec<ExplorerWasmFunction>,
    /// Where the code of each IR instruction starts in `binary`, in offset
    /// order, and where each function's code starts when that is not an IR
    /// instruction's (a prologue, or a function the linker made); the same
    /// table as the module's `bcc.sourcemap` section. An
    /// engine reports these offsets in stack traces and its debugger's
    /// `Debugger.paused` locations.
    pub positions:    Vec<ExplorerWasmPosition>,
    /// Every instruction's byte offset in `binary` and 1-based line in
    /// `text`, in offset order, for highlighting the instruction an engine
    /// is paused at.
    pub instructions: Vec<ExplorerWasmInstruction>,
}

/// One instruction of a WebAssembly module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerWasmInstruction {
    /// The byte offset in the module.
    pub offset:    u32,
    /// The line in [`ExplorerWasm::text`].
    pub text_line: u32,
}

/// A defined function of a WebAssembly module.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerWasmFunction {
    /// The function index, imports first.
    pub index:  u32,
    /// The symbol name, as the name section records it.
    pub name:   String,
    /// The byte offset of the function's first instruction (after its
    /// locals) in the module.
    pub start:  u32,
    /// The byte offset just past its final `end`.
    pub end:    u32,
    /// The function's physical source lines, when it comes from the input.
    pub source: Option<ExplorerLineRange>,
}

/// The start of the code of one IR instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerWasmPosition {
    /// The byte offset in the module.
    pub offset:   u32,
    /// The function index, imports first.
    pub function: u32,
    /// The IR block (`blockN` in the function in `ir_abi`), or `None` for
    /// code before the function's first IR instruction.
    pub block:    Option<u32>,
    /// The instruction's position in the block, or `None` for the block's
    /// terminator (its branch or return) and for code before the first IR
    /// instruction.
    pub inst:     Option<u32>,
    /// The source line, when the IR records it.
    pub line:     Option<u32>,
}

/// One diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerDiagnostic {
    /// `"error"`, `"warning"` or `"note"`. A note explains why an output is
    /// missing without the program being wrong, such as machine code for a
    /// Windows target.
    pub severity:   String,
    /// Which part of the compiler reported it: `"front-end"` (preprocessing,
    /// parsing and semantic analysis), `"middle-end"` (lowering to the IR)
    /// or `"back-end"` (assembly and machine code).
    pub stage:      String,
    /// The headline, such as `expected ';' after expression`.
    pub message:    String,
    /// The file of the range the diagnostic is about: `<input>` for the
    /// source, or a header's name. `None` with the other location fields
    /// when the diagnostic has no place. This and the lines and columns
    /// are physical: `#line` changes only the place `rendered` names.
    pub file:       Option<String>,
    /// The 1-based line where the range starts.
    pub line:       Option<u32>,
    /// The 1-based column, in characters, where the range starts.
    pub column:     Option<u32>,
    /// The 1-based line of the range's end, in `file`.
    pub end_line:   Option<u32>,
    /// The 1-based column, in characters, just past the range's end.
    pub end_column: Option<u32>,
    /// The diagnostic as the CLI renders it, with its source excerpt,
    /// notes and help, without color.
    pub rendered:   String,
}

/// A function's machine code.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerFunctionBytes {
    /// The symbol name.
    pub name:         String,
    /// The function's bytes, from its symbol's address to its end.
    pub bytes:        Vec<u8>,
    /// Each instruction in address order, as `objdump -d` would list it.
    pub instructions: Vec<ExplorerInstruction>,
}

/// One machine instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerInstruction {
    /// The offset from the function's first byte.
    pub offset: u32,
    /// The instruction's encoding.
    pub bytes:  Vec<u8>,
    /// The assembly line it encodes, as `asm` spells it.
    pub asm:    String,
}

/// The lines of each output that one function definition produced. The IR
/// keeps no source locations, so the mapping is per function: a function's
/// source lines map to its whole body in each output.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerLineMapping {
    /// The function's name in C. Each output names it by the symbol it is
    /// emitted as, which an `__asm__` label can change.
    pub function: String,
    /// The definition's physical lines in the source, which `#line` does
    /// not change.
    pub source:   ExplorerLineRange,
    /// Its lines in [`ExplorerOutput::ir`], from its header to its closing
    /// brace.
    pub ir:       Option<ExplorerLineRange>,
    /// Its lines in [`ExplorerOutput::ir_abi`].
    pub ir_abi:   Option<ExplorerLineRange>,
    /// Its lines in [`ExplorerOutput::asm`], from its label to the
    /// directive that ends it.
    pub asm:      Option<ExplorerLineRange>,
    /// Its lines in the WebAssembly text ([`ExplorerWasm::text`]), from
    /// `(func $name` to its closing parenthesis.
    pub wasm:     Option<ExplorerLineRange>,
}

/// 1-based lines, both inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExplorerLineRange {
    /// The first line.
    pub start: u32,
    /// The last line.
    pub end:   u32,
}

impl Default for ExplorerOptions {
    fn default() -> Self {
        Self {
            target:            "x86_64-unknown-linux-gnu".into(),
            std:               "c17".into(),
            optimize:          false,
            max_nesting_depth: None,
            source_date_epoch: None,
            files:             Vec::new(),
            syntax_locations:  false,
        }
    }
}
