//! The WebAssembly back end: post-ABI bcc IR for the `wasm32` targets to a
//! WebAssembly module, in the binary format or as text. It needs no LLVM,
//! so the explorer's in-browser compiler has it too.
//!
//! [`compile`] links the program's modules with, for WASI, the bundled C
//! runtime's (`runtime.rs`, compiled from C by bcc itself), resolving
//! every symbol by name and keeping only what the entry points reach
//! (`link.rs`). Then it translates each function (`codegen.rs`): SSA
//! values become locals, the control-flow graph becomes `block`, `loop`
//! and `br` through the stackifier (`stackify.rs`, Ramsey's algorithm, with
//! a dispatch loop for irreducible graphs), memory accesses become loads
//! and stores of linear memory, and stack slots live in a frame on a
//! shadow stack whose pointer is global 0. The binary encoder
//! (`encode.rs`) writes the module with a `name` section and a
//! `bcc.sourcemap` section mapping code offsets to IR positions; the text
//! printer (`text.rs`) writes the same instructions as `.wat`.
//!
//! For `int add(int a, int b) { return a + b; }` the function becomes
//! `(func $add (param $v0 i32) (param $v1 i32) (result i32) local.get $v0
//! local.get $v1 i32.add local.set $v2 local.get $v2 return)`.
//!
//! Features beyond WebAssembly 1.0: mutable globals (the stack pointer),
//! sign-extension operators, non-trapping float-to-int conversions and
//! bulk memory (`memory.copy`, `memory.fill`), all standardized in
//! WebAssembly 2.0 and supported by every engine Clang targets by default.
//!
//! Refused with a diagnostic: `setjmp`/`longjmp` and other functions that
//! return twice (they need the exception-handling proposal), inline and
//! file-scope assembly, `__builtin_return_address`, the x87 format, and an
//! atomic compare-exchange of an object without a lock-free size.
//!
//! Read [`compile`], then `link.rs`, `codegen.rs` and `stackify.rs`.
//!
//! - Output model: `module.rs`, `instr.rs`.
//! - Linking and the runtime: `link.rs`, `runtime.rs`.
//! - Translation: `codegen.rs` with `codegen/`, `stackify.rs`.
//! - Output: `encode.rs` (binary), `text.rs` (text).
//! - `tests.rs` and `tests/`: the stackifier, validity and execution.
//!
//! The back end is not a C translation phase; it implements the
//! WebAssembly C ABI that `abi/wasm.rs` cites.

// Output model
mod instr;
mod module;

// Linking and the runtime
mod link;
pub(crate) mod runtime;

// Translation
mod codegen;
pub(crate) mod stackify;

// Output
mod encode;
mod text;

pub(crate) use encode::SourceEntry;
pub(crate) use module::WasmModule;

use crate::{
    ir::Module,
    target::Target,
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// A compiled module: its binary encoding, the code offset of each IR
/// position, and each function's code range.
pub(crate) struct Compiled<'a> {
    pub(crate) module:  WasmModule<'a>,
    pub(crate) bytes:   ArenaVec<'a, u8>,
    pub(crate) entries: ArenaVec<'a, SourceEntry>,
    /// Each defined function's name and code range in `bytes`.
    pub(crate) bodies:  ArenaVec<'a, (&'a str, u32, u32)>,
    /// The offset in `bytes` of every instruction of every function, in
    /// code order.
    pub(crate) offsets: ArenaVec<'a, u32>,
}

impl Compiled<'_> {
    /// The module as WebAssembly text.
    pub(crate) fn write_text(&self, out: &mut impl core::fmt::Write) -> core::fmt::Result {
        text::write_module(&self.module, out, &mut Discard)
    }

    /// The module as WebAssembly text, and for each instruction (in the
    /// order of [`Compiled::offsets`]) its 1-based line in the text.
    pub(crate) fn write_text_with_lines(
        &self,
        out: &mut impl core::fmt::Write,
        lines: &mut impl Extend<u32>,
    ) -> core::fmt::Result {
        text::write_module(&self.module, out, lines)
    }
}

/// A sink for lines nobody wants.
struct Discard;

impl Extend<u32> for Discard {
    fn extend<I: IntoIterator<Item = u32>>(&mut self, _: I) {}
}

/// Links and compiles `units` for `target`: the first `program` are the
/// program's, the others the runtime's.
pub(crate) fn compile<'a>(
    units: &'a [&'a Module<'_>],
    program: usize,
    target: Target,
    arena: &'a Bump,
) -> Result<Compiled<'a>, WasmError<'a>> {
    for module in units {
        if !module.module_asm().is_empty() {
            return Err(WasmError::Unsupported {
                function:  None,
                construct: "file-scope assembly",
            });
        }
    }
    let linked = link::link(units, program, target, arena)?;
    let mut functions = ArenaVec::with_capacity_in(linked.order.len(), arena);
    let mut scratch = Bump::new();
    for &defined in &linked.order {
        let function = match defined {
            | link::Defined::Input(unit, func) =>
                codegen::generate(&linked, unit as usize, func, arena, &scratch).map_err(
                    |construct| WasmError::Unsupported {
                        function: Some(linked.symbol_name(unit as usize, func)),
                        construct,
                    },
                )?,
            | link::Defined::Synthetic(kind) => linked.synthetic(kind, arena),
        };
        functions.push(function);
        scratch.reset();
    }
    let mut module = linked.module;
    module.functions = functions;
    let encoded = encode::encode(&module, arena);
    let mut bodies = ArenaVec::with_capacity_in(module.functions.len(), arena);
    for (function, &(start, end)) in module.functions.iter().zip(&encoded.bodies) {
        bodies.push((function.name, start, end));
    }
    Ok(Compiled {
        module,
        bytes: encoded.bytes,
        entries: encoded.entries,
        bodies,
        offsets: encoded.offsets,
    })
}

/// `count` as a WebAssembly index or count, which the binary format makes
/// 32 bits; the linker keeps every table and offset within them.
pub(crate) fn u32_of(count: usize) -> u32 {
    u32::try_from(count).expect("a WebAssembly index fits 32 bits")
}

/// The low 32 bits of `bits`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "Keeping the low bits is the point."
)]
pub(crate) const fn low32(bits: u128) -> u32 {
    bits as u32
}

/// The low 64 bits of `bits`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "Keeping the low bits is the point."
)]
pub(crate) const fn low64(bits: u128) -> u64 {
    bits as u64
}

/// Why no module was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum WasmError<'a> {
    #[error("the WebAssembly back end does not support {construct}{}", in_function(*function))]
    Unsupported {
        function:  Option<&'a str>,
        construct: &'static str,
    },
    #[error("undefined symbol: {0}")]
    Undefined(&'a str),
    #[error("duplicate symbol: {0}")]
    Duplicate(&'a str),
}

/// ` (in function NAME)`, or nothing.
fn in_function(function: Option<&str>) -> impl core::fmt::Display + '_ {
    core::fmt::from_fn(move |f| match function {
        | Some(name) => write!(f, " (in function {name})"),
        | None => Ok(()),
    })
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
