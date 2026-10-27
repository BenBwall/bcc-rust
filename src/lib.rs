//! BCC translates C source into a syntax tree and semantic information. Each
//! source file is lexed once, preprocessing finishes before parsing starts, and
//! semantic analysis consumes the finished tree. On request, the middle end
//! lowers the analyzed unit to the bcc IR, optimizes it and lowers its calls
//! for the target's ABI, and a back end prints it, interprets it, or builds it
//! through the bundled clang. The
//! command-line program selects the input, language mode, target, and output.
//!
//! Start with `translation_phases` (`translation_phases.rs`) for the phase map,
//! then `pipeline::parse_translation_unit` for the batch driver,
//! `pipeline::analyze_translation_unit` for semantic analysis and
//! `pipeline::lower_translation_unit` for the middle end. Read `run` to
//! follow a command-line compilation.
//!
//! The `native` feature (the default) builds all of it, with LLVM, the
//! bundled clang and the command line. The `explorer` feature alone builds
//! the pure-Rust compiler, from the preprocessor through the native back
//! end's assembly and machine code, and the `explorer` interface over it;
//! that build needs no C toolchain and targets `wasm32-unknown-unknown`.
//!
//! Files by role, each with any directory of the same name:
//! - Compiler phases and driver: `pipeline.rs`, `translation_phases.rs`.
//! - Every stage's output for the website's compiler tour: `explorer.rs`.
//! - Middle end and back ends: `lowering.rs`, `ir.rs`, `optimizer.rs`,
//!   `abi.rs`, `backend.rs`.
//! - Invocation and language choices: `main.rs`, `cli.rs`, `configuration.rs`,
//!   `headers.rs`, `target.rs`.
//! - Values, storage, and reporting: `binary128.rs`, `diagnostics.rs`,
//!   `float_parsing.rs`, `util.rs`, `x87.rs`.
//! - Measurements and test support: `benchmarking.rs`, `test_support.rs`.
//!
//! C99: translation phases 1-7, §5.1.1.2 paragraph 1, pp. 9-10;
//! PDF pp. 21-22. Translation phase 8 (linking) is left to the bundled
//! clang.

#![cfg_attr(feature = "portable-simd", feature(portable_simd))]
#![cfg_attr(
    all(feature = "explorer", not(feature = "native")),
    expect(
        dead_code,
        unused_imports,
        reason = "Without `native` the command line, LLVM and the interpreter are absent, and so \
                  are the callers of much of the compiler's internal interface."
    )
)]

// Browser execution engine, linked by the site in web/
#[cfg(feature = "abstract-machine")]
pub mod abstract_machine;

// Compiler phases and driver
#[cfg(feature = "explorer")]
pub mod explorer;
#[cfg(feature = "explorer")]
mod pipeline;
#[cfg(feature = "explorer")]
pub(crate) mod translation_phases;

// Middle end
#[cfg(feature = "explorer")]
mod abi;
#[cfg_attr(
    all(not(test), feature = "native"),
    expect(
        dead_code,
        unused_imports,
        reason = "The driver uses the interpreter's entry point and the LLVM printer and driver; \
                  the re-exports for hosts, traps and tools serve their tests."
    )
)]
#[cfg(feature = "explorer")]
mod backend;
#[cfg_attr(
    all(not(test), feature = "native"),
    expect(
        dead_code,
        unused_imports,
        reason = "Only tests parse the textual IR or verify whole modules; the driver builds, \
                  prints and optimizes modules."
    )
)]
#[cfg(feature = "explorer")]
pub(crate) mod ir;
#[cfg(feature = "explorer")]
mod lowering;
#[cfg_attr(
    all(not(test), feature = "native"),
    expect(
        dead_code,
        unused_imports,
        reason = "The driver runs the pipeline; the per-pass counts and times and some re-exports \
                  serve the optimizer's tests."
    )
)]
#[cfg(feature = "explorer")]
mod optimizer;

// Isolated semantic experiment; never part of the compiler's default build.
#[cfg(all(test, feature = "native"))]
mod c_ir_study;

#[expect(
    dead_code,
    unused_imports,
    reason = "OS snapshots and faults also serve standalone stack study executables."
)]
#[cfg(feature = "native")]
mod compilation_stack;

// Invocation and language choices
#[cfg(feature = "explorer")]
mod builtins;
#[cfg(feature = "native")]
mod cli;
#[cfg(feature = "explorer")]
pub(crate) mod configuration;
#[cfg(feature = "explorer")]
mod headers;
#[cfg(feature = "explorer")]
mod target;

// Values, storage, and reporting
#[cfg(feature = "explorer")]
mod binary128;
#[cfg(feature = "explorer")]
pub(crate) mod diagnostics;
#[cfg(feature = "explorer")]
pub(crate) mod float_parsing;
#[cfg(feature = "explorer")]
pub(crate) mod util;
#[cfg(feature = "explorer")]
mod x87;

// Measurements and test support
#[cfg(feature = "benchmarking-internals")]
mod benchmarking;
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Test fixtures name files with std paths and strings; the arena rule covers the \
 \n                  compiler, not its tests."
)]
#[cfg(feature = "native")]
mod test_support;

#[cfg(feature = "benchmarking-internals")]
pub use benchmarking::{
    ArenaUsage,
    BenchmarkInput,
    ParseBenchmarkSummary,
    PreparedParse,
    arena_usage,
    lex,
    parse,
    parse_file,
    parse_file_with_library,
    parse_msvc_source,
    parse_source,
    preprocess,
    preprocess_one_million,
    sema,
    sema_source,
    with_prepared_parse,
};
#[cfg(feature = "native")]
pub use cli::{
    CompileStep,
    MainError,
    compile_file_measured,
    compile_file_with_arguments_measured,
    run,
};
// Only the `coz_preprocessing` example emits coz progress points.
#[cfg(all(unix, feature = "benchmarking-internals"))]
use coz as _;

/// Runs a complete compilation callback on a demand-paged native stack.
/// Create arenas and context inside the callback and keep all phases that
/// borrow them there. Only sendable results may leave the joined worker.
/// The parser's recursive descent runs on this stack; its nesting limit
/// (`--max-nesting-depth`) keeps deep input from exhausting the reservation,
/// which would be fatal rather than a diagnostic.
///
/// # Errors
/// Returns an OS error if the stack reservation or worker cannot be created.
/// Linux and Windows reserve a guarded demand-paged mapping; other platforms
/// ask the standard thread builder for a stack of the requested size.
///
/// # Panics
/// Resumes a callback panic on the caller after joining and freeing the stack.
#[cfg(feature = "native")]
pub fn with_compilation_stack<R: Send>(
    bytes: usize,
    compile: impl FnOnce() -> R + Send,
) -> std::io::Result<R> {
    compilation_stack::run(bytes, |_| compile())
}

#[cfg(all(
    feature = "benchmarking-internals",
    feature = "recursive-lowering-study"
))]
pub use benchmarking::{
    LoweringMeasurement,
    measure_lowering_source,
};
#[cfg(test)]
#[doc(hidden)]
mod shut_up_clippy_about_unused_dev_dependencies {
    use criterion as _;
    use pretty_assertions as _;
    use proptest as _;
    use rstest as _;
}
