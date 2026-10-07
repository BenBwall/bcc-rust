//! BCC C compiler
//!
//! It connects translation phases 1-7 (§5.1.1.2, pp. 9-10; PDF pp. 21-22);
//! semantic analysis and code generation are not yet in this pipeline.
#![cfg_attr(feature = "portable-simd", feature(portable_simd))]

#[cfg(test)]
#[doc(hidden)]
mod shut_up_clippy_about_unused_dev_dependencies {
    use criterion as _;
    use pretty_assertions as _;
    use proptest as _;
    use rstest as _;
}
// Only the benchmarking binary emits coz progress points.
#[cfg(all(unix, feature = "benchmarking-internals"))]
use coz as _;

#[cfg(feature = "benchmarking-internals")]
mod benchmarking;
mod cli;
pub(crate) mod configuration;
pub(crate) mod diagnostics;
pub(crate) mod float_parsing;
mod pipeline;
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
    parse_source,
    preprocess,
    preprocess_one_million,
    with_prepared_parse,
};
pub use cli::{
    CompileStep,
    MainError,
    compile_file_measured,
    compile_file_with_arguments_measured,
    run,
};
