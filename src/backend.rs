//! Back ends: each turns a post-ABI [`crate::ir::Module`] into something that
//! runs. ABI lowering has already made every call-site and parameter decision
//! for the target, so a back end only maps scalar operations, memory and
//! control flow onto its output, and every back end sees the same calling
//! convention.
//!
//! - `llvm.rs` and `llvm/` print LLVM IR text and drive the bundled clang to
//!   turn it into objects and executables.

// LLVM
pub(crate) mod llvm;
