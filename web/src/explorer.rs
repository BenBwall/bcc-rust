//! The Worker's seam to bcc's real compiler, for the pipeline view of edited
//! source. The bundled demos use outputs precomputed by the native compiler
//! (`pipeline.rs`); live compilation needs bcc built for wasm32, which exposes
//! `bcc_rust::explorer::compile(source, options) -> ExplorerOutput`.
//!
//! When that lands, this function maps its output onto [`Compilation`]:
//! tokens, syntax tree, both IR stages and assembly carry over as text, each
//! function's machine-code bytes become the listing format
//! `web/pipeline/generate.sh` writes, and `llvm` stays `None`, because the
//! browser build has no LLVM.

use crate::pipeline::Compilation;

/// Compile `source` with the in-browser compiler.
///
/// # Errors
/// Always, until the Worker links bcc's wasm32 explorer.
pub fn compile(source: &str) -> Result<Compilation, String> {
    let _ = source;
    Err(UNAVAILABLE.to_owned())
}

/// Why edited source has no live pipeline view yet.
pub const UNAVAILABLE: &str = "Live compilation of edited source needs bcc's in-browser compiler, \
                               which is not part of this build yet.";
