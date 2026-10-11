//! BCC translates C source into a syntax tree and semantic information. Each
//! source file is lexed once, preprocessing finishes before parsing starts, and
//! semantic analysis consumes the finished tree. The command-line program
//! selects the input, language mode, target, and output view.
//!
//! Start with `translation_phases` (`translation_phases.rs`) for the phase map,
//! then `pipeline::parse_translation_unit` for the batch driver and
//! `pipeline::analyze_translation_unit` for semantic analysis. Read [`run`]
//! to follow a command-line compilation.
//!
//! Files by role, each with any directory of the same name:
//! - Compiler phases and driver: `pipeline.rs`, `translation_phases.rs`.
//! - Invocation and language choices: `main.rs`, `cli.rs`, `configuration.rs`,
//!   `headers.rs`, `target.rs`.
//! - Values, storage, and reporting: `binary128.rs`, `diagnostics.rs`,
//!   `float_parsing.rs`, `util.rs`.
//! - Measurements and test support: `benchmarking.rs`, `test_support.rs`.
//!
//! C99: translation phases 1-7, §5.1.1.2 paragraph 1, pp. 9-10;
//! PDF pp. 21-22. Code generation and linking are outside this front end.

#![cfg_attr(feature = "portable-simd", feature(portable_simd))]

// Compiler phases and driver
mod pipeline;
pub(crate) mod translation_phases;

// Invocation and language choices
mod cli;
pub(crate) mod configuration;
mod headers;
mod target;

// Values, storage, and reporting
mod binary128;
pub(crate) mod diagnostics;
pub(crate) mod float_parsing;
pub(crate) mod util;

// Measurements and test support
#[cfg(feature = "benchmarking-internals")]
mod benchmarking;
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Test fixtures name files with std paths and strings; the arena rule covers the \
              compiler, not its tests."
)]
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
pub use cli::{
    CompileStep,
    MainError,
    compile_file_measured,
    compile_file_with_arguments_measured,
    run,
};
// Only the benchmarking binary emits coz progress points.
#[cfg(all(unix, feature = "benchmarking-internals"))]
use coz as _;

#[cfg(test)]
#[doc(hidden)]
mod shut_up_clippy_about_unused_dev_dependencies {
    use criterion as _;
    use pretty_assertions as _;
    use proptest as _;
    use rstest as _;
}
