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
//! - `interpreter.rs` and `interpreter/`: execute a module directly with exact
//!   poison and undefined-behaviour tracking. It is the reference semantics of
//!   the IR and the oracle for differential tests. It also accepts pre-ABI
//!   modules, since it implements `va_arg` itself.
//! - `difftest.rs` and `difftest/` (tests only): random programs that check the
//!   optimizer and the LLVM back end against the interpreter.
//!
//! The back ends are not C translation phases; where one implements a rule of
//! C, such as a host function from the standard library, the item that does
//! so cites the clause.

// Code generation
pub(crate) mod llvm;

// Reference execution
pub(crate) mod interpreter;

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
