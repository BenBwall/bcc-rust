//! The native x86-64 back end: post-ABI bcc IR to AT&T assembly, which the
//! bundled clang assembles and links. Stage 1 of
//! `docs/design/native-backend.md`: integer and pointer code under the
//! System V and Win64 conventions.
//!
//! [`emit_assembly`] first checks that the back end supports everything
//! the module uses (`support.rs`), so a module is compiled whole or refused
//! with a diagnostic, never compiled wrongly. Then, one function at a time
//! in a scratch arena reset between functions, it selects machine
//! instructions over virtual registers (`isel.rs`), allocates registers
//! with linear scan (`regalloc.rs`), lays out the frame (`frame.rs`) and
//! writes the text (`emit.rs`). Globals follow the functions.
//!
//! For `int add(int a, int b) { return a + b; }` on System V, selection
//! produces `mov %v0 ← %rdi; mov %v1 ← %rsi; mov %v2 ← %v0; add %v2, %v1;
//! mov %rax ← %v2; ret`; the allocator's hints put `%v0` and `%v2` in
//! `%rdi` and `%v1` in `%rsi`, the rewrite deletes the copies between equal
//! registers, and the text is `addl %esi, %edi; movq %rdi, %rax; retq`.
//!
//! Read [`emit_assembly`], then `isel.rs`, `regalloc.rs`, `frame.rs` and
//! `emit.rs`.
//!
//! - Contract: `support.rs` (what stage 1 compiles), `conv.rs` (the two
//!   conventions' registers).
//! - Machine IR: `mir.rs`.
//! - Passes: `isel.rs` with `isel/`, `regalloc.rs` with `regalloc/`,
//!   `frame.rs`.
//! - Output: `emit.rs` (text), `driver.rs` (assembling and linking).
//! - `tests.rs` and `tests/`: selection, allocation and end-to-end tests.
//!
//! The back end is not a C translation phase; it implements the target ABIs
//! the files cite.

// Contract
mod conv;
mod support;

// Machine IR
mod mir;

// Passes
mod frame;
mod isel;
mod regalloc;

// Output
mod driver;
mod emit;

use std::fmt;

pub(crate) use driver::assemble;
pub(crate) use support::Unsupported;

use self::{
    conv::CallConv,
    emit::Emitter,
    frame::Frame,
};
use crate::{
    ir::Module,
    target::Target,
    util::bump::Bump,
};

/// Symbols the back end calls on its own; a module may not define them
/// internally, since those calls would bind to its definition.
pub(crate) const RUNTIME_SYMBOLS: [&str; 4] = ["memmove", "__main", "___chkstk_ms", "__chkstk"];

/// Writes the module's assembly for `target`, or refuses it.
pub(crate) fn emit_assembly<'m>(
    module: &'m Module<'_>,
    target: Target,
    out: &mut dyn fmt::Write,
) -> Result<(), NativeError<'m>> {
    if !module.triple().is_empty() && Target::parse(module.triple()) != Some(target) {
        return Err(NativeError::TargetMismatch {
            module:    module.triple(),
            requested: target.triple(),
        });
    }
    support::check_module(module)?;
    let conv = CallConv::of(target);
    let mut emitter = Emitter::new(module, target, out);
    emitter.header()?;
    let mut scratch = Bump::new();
    for (id, function) in module.functions() {
        let Some(body) = &function.body else { continue };
        {
            let arena = &scratch;
            let mut func =
                isel::select(module, function, body, target, arena).map_err(|construct| {
                    Unsupported {
                        function: Some(function.name),
                        construct,
                    }
                })?;
            let allocation = regalloc::allocate(&mut func, conv, arena);
            regalloc::rewrite(&mut func, &allocation, arena);
            let frame = Frame::layout(&func, &allocation, conv, arena);
            emitter.function(id, &func, &frame)?;
        }
        scratch.reset();
    }
    emitter.globals()?;
    Ok(())
}

/// Why the back end produced no assembly.
#[derive(Debug, thiserror::Error)]
pub(crate) enum NativeError<'m> {
    #[error("{0}")]
    Unsupported(Unsupported<'m>),
    #[error("the output refused the assembly text")]
    Format(#[from] fmt::Error),
    #[error("the module targets {module}, but assembly was requested for {requested}")]
    TargetMismatch {
        module:    &'m str,
        requested: &'static str,
    },
}

impl<'m> From<Unsupported<'m>> for NativeError<'m> {
    fn from(unsupported: Unsupported<'m>) -> Self {
        Self::Unsupported(unsupported)
    }
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
