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
    ir::{
        DllStorage,
        Linkage,
        Module,
        Symbol,
        Visibility,
    },
    target::{
        CodeOptions,
        Relocation,
        Target,
    },
    util::bump::Bump,
};

/// Symbols the back end calls or reads on its own (`_tls_index` is the
/// Windows image's TLS index); a module may not define them internally,
/// since those references would bind to its definition.
pub(crate) const RUNTIME_SYMBOLS: [&str; 46] = [
    "memmove",
    "__main",
    "___chkstk_ms",
    "__chkstk",
    "fmod",
    "fmodf",
    "fmodl",
    "fmodf128",
    "__divti3",
    "__udivti3",
    "__modti3",
    "__umodti3",
    "__floattixf",
    "__floattitf",
    "__floatuntixf",
    "__floatuntitf",
    "__fixxfti",
    "__fixtfti",
    "__fixunsxfti",
    "__fixunstfti",
    "__addtf3",
    "__subtf3",
    "__multf3",
    "__divtf3",
    "__eqtf2",
    "__netf2",
    "__lttf2",
    "__letf2",
    "__gttf2",
    "__getf2",
    "__unordtf2",
    "__extendsftf2",
    "__extenddftf2",
    "__extendxftf2",
    "__trunctfsf2",
    "__trunctfdf2",
    "__trunctfxf2",
    "__floatsitf",
    "__floatunsitf",
    "__floatditf",
    "__floatunditf",
    "__fixtfsi",
    "__fixunstfsi",
    "__fixtfdi",
    "__fixunstfdi",
    "_tls_index",
];

/// Whether the module defines `symbol`: a function body, a global's
/// initializer, or an alias, which names a definition.
pub(crate) fn defines(module: &Module<'_>, symbol: Symbol) -> bool {
    match symbol {
        | Symbol::Function(func) => {
            let function = module.function(func);
            function.body.is_some() || function.attributes.alias.is_some()
        },
        | Symbol::Global(global) => {
            let global = module.global(global);
            global.init.is_some() || global.attributes.alias.is_some()
        },
    }
}

/// Whether code on `target` reaches `symbol` through a pointer-sized
/// constant holding its address, as Clang does on COFF: the import address
/// table entry `__imp_name` of a `dllimport` declaration, which the loader
/// fills in, or else a `.refptr` stub for a weak declaration, whose address
/// may be null, and on MinGW for a global the module does not define, which
/// may be a DLL's data. LLD's MinGW auto-import then relocates the stub's
/// 64-bit address when the DLL loads, wherever it lies; a 32-bit
/// RIP-relative reference to the data itself could not reach a DLL loaded
/// more than 2 GiB from the executable. Microsoft's linker does not
/// auto-import, so on MSVC such data needs `__declspec(dllimport)`, and an
/// undefined global is otherwise addressed directly, as Clang addresses it.
/// A hidden or protected global is the module's own, never a DLL's, so on
/// MinGW too it is addressed directly unless it is a weak declaration, as
/// Clang's `shouldAssumeDSOLocal` decides.
/// LLD: "MinGW auto-import"; Microsoft Learn, "PE Format", base relocations,
/// and "dllexport, dllimport".
pub(crate) fn through_stub(module: &Module<'_>, target: Target, symbol: Symbol) -> bool {
    match target {
        | Target::LinuxGnu | Target::LinuxMusl => false,
        | Target::WindowsMsvc => imported(module, symbol) || weak_declaration(module, symbol),
        | Target::WindowsGnu =>
            imported(module, symbol)
                || weak_declaration(module, symbol)
                || matches!(symbol, Symbol::Global(global)
                    if !defines(module, symbol)
                        && !module.global(global).thread_local
                        && module.symbol_attributes(symbol).visibility == Visibility::Default),
    }
}

/// Whether `symbol` is a `dllimport` declaration, which code reaches only
/// through its import address table entry `__imp_name`.
/// MSVC extension: Microsoft Learn, "dllexport, dllimport".
pub(crate) fn imported(module: &Module<'_>, symbol: Symbol) -> bool {
    module.symbol_attributes(symbol).dll == DllStorage::Import && !defines(module, symbol)
}

/// Whether `symbol` is a weak declaration, which may stay undefined. An
/// ordinary symbol's address is then null; ELF thread-local declarations
/// instead resolve to offset zero from the thread base.
/// GNU extension: GCC manual, "Common Function Attributes", weak.
pub(crate) fn weak_declaration(module: &Module<'_>, symbol: Symbol) -> bool {
    let linkage = match symbol {
        | Symbol::Function(func) => module.function(func).linkage,
        | Symbol::Global(global) => module.global(global).linkage,
    };
    linkage == Linkage::Weak && !defines(module, symbol)
}

/// Writes the module's assembly for `target` with the default code
/// options, or refuses it.
pub(crate) fn emit_assembly<'m>(
    module: &'m Module<'_>,
    target: Target,
    out: &mut dyn fmt::Write,
) -> Result<(), NativeError<'m>> {
    emit_assembly_with(module, target, CodeOptions::default(), out)
}

/// Writes the module's assembly for `target` with the relocation and TLS
/// models of `code`, or refuses it. The back end generates code for
/// executables only, which also serves `-fno-pic`; it refuses `-fpic` and
/// `-fPIC`, whose code a shared library must be able to contain.
pub(crate) fn emit_assembly_with<'m>(
    module: &'m Module<'_>,
    target: Target,
    code: CodeOptions,
    out: &mut dyn fmt::Write,
) -> Result<(), NativeError<'m>> {
    if !module.triple().is_empty() && Target::parse(module.triple()) != Some(target) {
        return Err(NativeError::TargetMismatch {
            module:    module.triple(),
            requested: target.triple(),
        });
    }
    if matches!(target, Target::LinuxGnu | Target::LinuxMusl)
        && matches!(code.relocation(target), Relocation::Pic(_))
    {
        return Err(NativeError::SharedLibraryCode);
    }
    support::check_module(module, target)?;
    let conv = CallConv::of(target);
    let stubs = Bump::new();
    let mut emitter = Emitter::new(module, target, out, &stubs);
    emitter.header()?;
    let mut scratch = Bump::new();
    for (id, function) in module.functions() {
        let Some(body) = &function.body else { continue };
        {
            let arena = &scratch;
            let mut func = isel::select(module, function, body, target, code.tls_model, arena)
                .map_err(|construct| Unsupported {
                    function: Some(function.name),
                    construct,
                })?;
            let allocation = regalloc::allocate(&mut func, conv, arena);
            regalloc::rewrite(&mut func, &allocation, arena);
            let frame = Frame::layout(&func, &allocation, conv, arena).map_err(|construct| {
                Unsupported {
                    function: Some(function.name),
                    construct,
                }
            })?;
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
    /// `-fpic`, `-fPIC` or `-shared` asks for code a shared library can
    /// contain.
    #[error(
        "the native back end generates code for executables only; -fpic, -fPIC and -shared need \
         --backend=llvm"
    )]
    SharedLibraryCode,
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
