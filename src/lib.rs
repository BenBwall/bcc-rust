//! BCC C compiler
//!
//! It connects translation phases 1-7 (§5.1.1.2, pp. 9-10; PDF pp. 21-22);
//! Declaration semantic analysis follows parsing; full expression/statement
//! semantics and code generation remain later work.
#![cfg_attr(feature = "portable-simd", feature(portable_simd))]

// Only the benchmarking binary emits coz progress points.
#[cfg(all(unix, feature = "benchmarking-internals"))]
use coz as _;

#[cfg(feature = "benchmarking-internals")]
mod benchmarking;
mod binary128;
mod cli;
pub(crate) mod configuration;
pub(crate) mod diagnostics;
pub(crate) mod float_parsing;
mod headers;
mod pipeline;
mod target;
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Test fixtures name files with std paths and strings; the arena rule covers the \
              compiler, not its tests."
)]
mod test_support;
pub(crate) mod translation_phases;
pub(crate) mod util;

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

#[cfg(test)]
#[doc(hidden)]
mod shut_up_clippy_about_unused_dev_dependencies {
    use criterion as _;
    use pretty_assertions as _;
    use proptest as _;
    use rstest as _;
}
