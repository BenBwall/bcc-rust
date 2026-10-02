//! BCC C compiler

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
    ParseBenchmarkSummary,
    one_million_input_bytes,
    parse_mix,
    parse_one_million,
    parser_mix_input_bytes,
    parser_mix_input_lines,
    preprocess_one_million,
};
pub use cli::{
    MainError,
    run,
};
