//! The back ends: consumers of a post-ABI module of the bcc IR.
//!
//! Every back end receives a module after ABI lowering, where each value is a
//! scalar or a pointer and calling-convention decisions are already explicit,
//! so all of them see the same program. A back end either turns the module
//! into an artifact for the target or, like the interpreter, executes it.
//!
//! Files by role:
//! - `llvm.rs` and `llvm/`: print LLVM IR text and drive the bundled clang to
//!   turn it into objects and executables. The module map there shows how the
//!   optimizer comparison arms map onto its options.
//! - `native.rs` and `native/`: bcc's own x86-64 code generator, which writes
//!   assembly for the bundled clang to assemble, or encodes ELF and COFF
//!   objects itself (the default for Windows targets), and links with the
//!   bundled clang (`docs/design/native-backend.md`).
//! - `wasm.rs` and `wasm/`: bcc's own WebAssembly code generator for the
//!   `wasm32` targets, with a linker and a C runtime for WASI, which the
//!   explorer's in-browser compiler shares.
//! - `interpreter.rs` and `interpreter/`: execute a module directly with exact
//!   poison and undefined-behaviour tracking. It is the reference semantics of
//!   the IR and the oracle for differential tests. It also accepts pre-ABI
//!   modules, since it implements `va_arg` itself, and lays out the target's
//!   `va_list` in post-ABI ones.
//! - `difftest.rs` and `difftest/` (tests only): random programs that check the
//!   optimizer and the LLVM back end against the interpreter.
//!
//! The back ends are not C translation phases; where one implements a rule of
//! C, such as a host function from the standard library, the item that does
//! so cites the clause.

// Code generation
#[cfg(feature = "native")]
pub(crate) mod llvm;
pub(crate) mod native;
pub(crate) mod wasm;

// Reference execution
#[cfg(feature = "native")]
pub(crate) mod interpreter;

/// Writes an object a back end made to `path`, or to stdout for `-`, as
/// Clang writes `-c -o -` (GCC refuses it, as its assembler cannot write an
/// object to a pipe; its `-S` and `-E` take `-` for stdout too).
#[cfg(feature = "native")]
pub(crate) fn write_object(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    if path == std::path::Path::new("-") {
        let mut out = std::io::stdout().lock();
        out.write_all(bytes)?;
        return out.flush();
    }
    std::fs::write(path, bytes)
}

// Differential tests
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "The differential tests build programs and reports with std types; the arena rule \
              covers the compiler, not its tests."
)]
mod difftest;
