//! The WebAssembly view's data: a WebAssembly module for the page's C
//! source, stepped one instruction at a time by the interpreter in
//! `web/stepper/` inside the engine Worker.
//!
//! Where modules come from is behind [`WasmBackEnd`]: it turns C source
//! into a [`WasmModule`] (the binary, and optionally which C line each
//! instruction came from). [`Bcc`] compiles any source with bcc's own
//! WebAssembly back end in the compiler Worker (`bcc.rs`), with bcc's
//! `.wat` and source map; [`Precompiled`] serves the bundled lessons, built
//! ahead of time by clang with DWARF line tables (`web/wasm/generate.sh`),
//! so the page can show both compilers' output for a lesson.
//!
//! The page sends the module to the engine Worker (`Command::WasmLoad`),
//! which answers with a [`WasmProgram`]: the listing and what each of its
//! lines shows, and the first [`WasmSnapshot`]. Each later command
//! (`Command::Wasm`) answers with the next snapshot. `worker.rs` builds
//! them from the stepper's state, and `narration.rs` explains each step.

#[cfg(feature = "compiler")]
mod bcc;
mod lessons;
#[cfg(feature = "worker")]
mod narration;
mod snapshot;
#[cfg(feature = "worker")]
pub mod worker;

#[cfg(feature = "compiler")]
pub use bcc::{
    Bcc,
    TARGET,
};
pub use lessons::{
    LESSON_FLAGS,
    LESSON_SECTION,
    Precompiled,
    WasmBackEnd,
};
pub use snapshot::*;
