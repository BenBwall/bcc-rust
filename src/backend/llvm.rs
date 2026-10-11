//! The LLVM back end: prints a post-ABI module as LLVM IR text for the pinned
//! LLVM 23.1.1, and hands that text to the bundled clang, which verifies it,
//! optionally optimizes it, generates code and links.
//!
//! [`emit_module`] writes the target lines, each global, each function, and
//! last the declarations of the intrinsics the functions used. The mapping is
//! mechanical:
//!
//! - Each block parameter becomes a `phi` at the start of its block, with one
//!   incoming entry per edge into the block. LLVM wants an entry for every
//!   edge, so two edges from one terminator give two identical entries (the IR
//!   verifier makes their arguments equal). Blocks unreachable from the entry
//!   block are not printed, so every printed `phi` has an entry.
//! - Constants (`iconst`, `fconst`, `poison`, `null`) and the addresses of
//!   stack slots, globals and functions are not instructions in LLVM; they are
//!   printed in place at each use. Stack slots become `alloca [N x i8]` at the
//!   top of the entry block.
//! - `ptr_add` is `getelementptr [inbounds] i8`; `nsw`, `nuw` and `exact` map
//!   one to one; `copy` calls `llvm.memcpy`, or `llvm.memmove` when it
//!   `may_overlap`, and `fill` calls `llvm.memset`; the `va_*` operations call
//!   `llvm.va_start`, `llvm.va_copy` and `llvm.va_end`, and `va_arg` is LLVM's
//!   `va_arg`.
//! - A global's contents are `[N x i8]`, or, when it holds address constants, a
//!   packed struct `<{ [k x i8], ptr, ... }>` that alternates byte runs with
//!   the relocated pointers (`getelementptr (i8, ptr @sym, i64 addend)` when
//!   the addend is not zero), so a relocation may sit at any offset.
//! - Values print as `%v3`, blocks as `%block2` and stack slots as `%slot0`,
//!   the numbers of the bcc IR text, so the two texts line up.
//!
//! Access tags are not printed yet: the module has no descriptor table to
//! build `!tbaa` metadata from.
//!
//! The driver helpers in `driver.rs` run clang on a `.ll` file. The
//! comparison arms of `middle-end.md` map onto [`LlvmOptions`] like this:
//!
//! | Arm | Middle end | `opt` | `disable_llvm_passes` |
//! | --- | --- | --- | --- |
//! | A0 | none | `Some(OptLevel::O0)` | `false` |
//! | A1 | bcc pipeline | `Some(OptLevel::O2)` | `true` |
//! | A2 | none | `Some(OptLevel::O2)` | `false` |
//! | A3 | bcc pipeline | `Some(OptLevel::O2)` | `false` |
//!
//! Read [`emit_module`] first, then `function.rs` for bodies and `global.rs`
//! for initializers.
//!
//! - Printing: `function.rs` prints declarations and bodies; `global.rs` prints
//!   globals and their relocations; `syntax.rs` spells types, names and
//!   constants; `target.rs` holds each target's data layout.
//! - Running clang: `driver.rs` finds the toolchain and builds the command.
//! - `tests.rs` and `tests/` hold golden, validity and execution tests.

// Printing
mod function;
mod global;
mod syntax;
mod target;

// Running clang
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "Tooling around the compiler: it reads the environment, builds paths and reads \
              clang's output once per clang run, outside the arena-allocated compile path."
)]
mod driver;

use std::fmt;

use bitflags::bitflags;
pub(crate) use driver::{
    ClangError,
    LlvmOptions,
    OptLevel,
    OutputKind,
    Toolchain,
    compile_ll,
};
pub(crate) use target::data_layout;

use crate::{
    ir::Module,
    target::Target,
    util::bump::Bump,
};

/// Writes `module` as an LLVM IR module for `target`.
///
/// The module must pass the IR verifier. If it names a target, that must be
/// `target`; the printed triple and data layout always come from `target`.
pub(crate) fn emit_module<'m>(
    module: &'m Module<'_>,
    target: Target,
    out: &mut dyn fmt::Write,
) -> Result<(), EmitError<'m>> {
    if !module.triple().is_empty() && Target::parse(module.triple()) != Some(target) {
        return Err(EmitError::TargetMismatch {
            module:    module.triple(),
            requested: target.triple(),
        });
    }
    let mut emitter = Emitter {
        module,
        out,
        intrinsics: Intrinsics::empty(),
    };
    writeln!(
        emitter.out,
        "target datalayout = \"{}\"",
        data_layout(target)
    )?;
    writeln!(emitter.out, "target triple = \"{}\"", target.triple())?;
    let mut scratch = Bump::new();
    if module.globals().next().is_some() {
        emitter.out.write_char('\n')?;
        for (global, _) in module.globals() {
            scratch.reset();
            emitter.global(global, &scratch)?;
        }
    }
    for (func, _) in module.functions() {
        emitter.out.write_char('\n')?;
        scratch.reset();
        emitter.function(func, &scratch)?;
    }
    emitter.intrinsic_declarations()?;
    Ok(())
}

/// The printer's state: the module, the output, and the intrinsics used so
/// far, whose declarations follow the functions.
struct Emitter<'a, 'm, 'ir> {
    module:     &'m Module<'ir>,
    out:        &'a mut dyn fmt::Write,
    intrinsics: Intrinsics,
}

bitflags! {
    /// The LLVM intrinsics a module calls.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    struct Intrinsics: u8 {
        const MEMCPY = 1 << 0;
        const MEMMOVE = 1 << 1;
        const MEMSET = 1 << 2;
        const VA_START = 1 << 3;
        const VA_COPY = 1 << 4;
        const VA_END = 1 << 5;
    }
}

impl Intrinsics {
    /// Each intrinsic with its declaration, in printing order.
    const DECLARATIONS: [(Self, &'static str); 6] = [
        (
            Self::MEMCPY,
            "declare void @llvm.memcpy.p0.p0.i64(ptr, ptr, i64, i1 immarg)",
        ),
        (
            Self::MEMMOVE,
            "declare void @llvm.memmove.p0.p0.i64(ptr, ptr, i64, i1 immarg)",
        ),
        (
            Self::MEMSET,
            "declare void @llvm.memset.p0.i64(ptr, i8, i64, i1 immarg)",
        ),
        (Self::VA_START, "declare void @llvm.va_start.p0(ptr)"),
        (Self::VA_COPY, "declare void @llvm.va_copy.p0(ptr, ptr)"),
        (Self::VA_END, "declare void @llvm.va_end.p0(ptr)"),
    ];
}

impl Emitter<'_, '_, '_> {
    /// Declares every intrinsic the module used, after a blank line.
    fn intrinsic_declarations(&mut self) -> fmt::Result {
        if self.intrinsics.is_empty() {
            return Ok(());
        }
        self.out.write_char('\n')?;
        for (intrinsic, declaration) in Intrinsics::DECLARATIONS {
            if self.intrinsics.contains(intrinsic) {
                writeln!(self.out, "{declaration}")?;
            }
        }
        Ok(())
    }
}

/// Why a module could not be printed as LLVM IR.
#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]
pub(crate) enum EmitError<'m> {
    #[error("the output refused the LLVM IR text")]
    Format(#[from] fmt::Error),
    #[error("the module targets {module}, but LLVM IR was requested for {requested}")]
    TargetMismatch {
        module:    &'m str,
        requested: &'static str,
    },
    /// Two relocations of a global share bytes; the verifier only checks
    /// that each fits.
    #[error("@{global}: the relocations at offsets {first} and {second} overlap")]
    OverlappingRelocations {
        global: &'m str,
        first:  u64,
        second: u64,
    },
}

// Tests
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod tests;
