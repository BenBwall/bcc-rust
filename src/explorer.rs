//! The explorer runs one C source through every stage of the compiler and
//! returns what each stage produced, as text the command line would print,
//! for the website's interactive tour of a compiler. It needs no LLVM, C
//! toolchain, threads or file system, so it builds for
//! `wasm32-unknown-unknown` with the `explorer` feature alone.
//!
//! [`compile`] first shows the front of the compiler phase by phase: the
//! logical lines that translation phases 1 and 2 leave, the preprocessing
//! tokens of phase 3, and one preprocessing run (phases 4-6) with a trace of
//! what phase 4 did, which gives the `-E` text and the token listing. Then it
//! compiles the source as `--emit` does: parsing (the syntax tree), semantic
//! analysis, lowering to the bcc IR before and after ABI lowering, and the
//! native x86-64 back end's assembly, which the in-process object encoder
//! turns into machine code. A stage that fails yields `None` and
//! diagnostics; the stages after it do not run.
//!
//! For `int main(void) { return 1 + 2; }`, `tokens` lists the eleven
//! tokens, `ir` holds `main`'s blocks (`ret i32 3` once the optimizer
//! folds the sum), `asm` its x86-64 assembly, `machine_code` the bytes
//! of each of its instructions, and `line_map` maps line 1 to `main` in
//! each of them.
//!
//! Read [`compile`], then `compilation.rs`.
//!
//! Files by role:
//! - The interface types: `model.rs`.
//! - The stages: `compilation.rs`; the front end, phase by phase:
//!   `front_end.rs`; the function-level line map: `line_map.rs`.
//! - LLVM IR for prebuilt demonstrations, on a native host: `llvm.rs`.
//! - Agreement with the command line: `tests/integration/explorer.rs`.
//!
//! The explorer is not a C translation phase; it drives phases 1-7
//! (C99 §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22) as the command
//! line does.

// Interface
mod model;

// Stages
mod compilation;
mod front_end;
mod line_map;

// Native-only output
#[cfg(feature = "native")]
mod llvm;

#[cfg(feature = "native")]
pub use llvm::{
    LlvmIrError,
    llvm_ir,
};
pub use model::{
    ExplorerArgument,
    ExplorerConditional,
    ExplorerDefinition,
    ExplorerDiagnostic,
    ExplorerDirective,
    ExplorerExpansion,
    ExplorerExpansionStep,
    ExplorerFile,
    ExplorerFunctionBytes,
    ExplorerInclude,
    ExplorerInstruction,
    ExplorerLineMapping,
    ExplorerLineRange,
    ExplorerLogicalLine,
    ExplorerOptions,
    ExplorerOutput,
    ExplorerOutputLine,
    ExplorerPpToken,
    ExplorerPreprocessing,
    ExplorerSkippedLines,
    ExplorerSourceChange,
};

/// Compiles `source` through every stage the explorer shows. Malformed input
/// yields diagnostics, never a panic. On a native host the compilation runs
/// on the command line's reserved stack, and an internal compiler error
/// becomes an error diagnostic.
#[must_use]
pub fn compile(source: &str, options: &ExplorerOptions) -> ExplorerOutput {
    let size = source.len()
        + options
            .files
            .iter()
            .map(|file| file.contents.len())
            .sum::<usize>();
    if size > MAX_SOURCE_BYTES {
        return compilation::failed(format_args!(
            "the explorer compiles at most {MAX_SOURCE_BYTES} bytes of source; this is {size}"
        ));
    }
    #[cfg(feature = "native")]
    {
        let run = std::panic::AssertUnwindSafe(|| {
            crate::with_compilation_stack(crate::cli::COMPILATION_STACK_BYTES, || {
                compilation::compile(source, options)
            })
        });
        match std::panic::catch_unwind(run) {
            | Ok(Ok(output)) => output,
            | Ok(Err(error)) => compilation::failed(format_args!(
                "cannot reserve the compilation stack: {error}"
            )),
            | Err(_) => compilation::failed(format_args!("internal compiler error")),
        }
    }
    // WebAssembly has one thread on one fixed stack; the smaller default
    // nesting limit keeps deep input a diagnostic there.
    #[cfg(not(feature = "native"))]
    compilation::compile(source, options)
}

/// The most source text, with the in-memory headers, the explorer compiles:
/// [`WASM_MAX_SOURCE_BYTES`] in WebAssembly, and 1 MiB on a native host,
/// whose arenas reserve far more address space.
pub const MAX_SOURCE_BYTES: usize = if cfg!(target_family = "wasm") {
    WASM_MAX_SOURCE_BYTES
} else {
    1 << 20
};

/// The most source text the explorer compiles in WebAssembly. There each
/// arena is one fixed region of linear memory (`util::vm::REGION_BYTES`, 32
/// MiB), and a compilation that fills one ends the module with a trap, so
/// the limit leaves every arena room for ordinary programs. Measured with
/// the release build on programs of one dense shape each (many short
/// declarations, one long line of them, many declarators, a long
/// initializer, a long sum, many small functions, many macro invocations,
/// many locals, one long string, many comments), 64 KiB compiles them all
/// within the page's ten seconds; 96 KiB of initializer elements already
/// fills the parser's arena, and 512 KiB fills one arena or another in
/// every shape but comments. Only a macro expansion that multiplies its
/// input many thousandfold can still fill one below the limit.
pub const WASM_MAX_SOURCE_BYTES: usize = 64 * 1024;

/// The nesting limit the explorer applies without a native host's reserved
/// compilation stack. WebAssembly recursion is bounded twice: by the
/// module's shadow stack (1 MiB unless the module links with
/// `-zstack-size`) and by the engine's own call stack, about 1 MiB in V8.
/// The deepest shapes (parenthesized operands, nested calls, braced
/// initializers) take under a kilobyte of shadow stack per level in a
/// release build and exhaust it near 1100 levels, so 500 leaves a margin
/// of two, which a debug build's larger frames may use up. Overflowing
/// either stack traps, and the instance must not be used again; pass a
/// lower `max_nesting_depth` rather than relying on a larger stack.
pub const WASM_MAX_NESTING_DEPTH: u32 = 500;
