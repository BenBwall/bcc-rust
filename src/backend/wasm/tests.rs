//! Tests of the WebAssembly back end: the stackifier on random graphs
//! (`tests/stackify.rs`), whole programs compiled, validated and run under
//! Node's WASI (`tests/execution.rs`), and the C ABI against the bundled
//! Clang (`tests/abi.rs`, with `tests/single_element.c`).

mod abi;
mod execution;
mod stackify;
