//! A finished WebAssembly module in memory: what the linker and the code
//! generator produce and what the binary encoder and the text printer
//! write. Every function index counts the imported functions first, as
//! the binary format does.

use super::{
    instr::{
        Instr,
        ValType,
    },
    u32_of,
};
use crate::util::bump::ArenaVec;

/// The module.
pub(crate) struct WasmModule<'a> {
    pub(crate) types:         ArenaVec<'a, FuncType<'a>>,
    /// Imported functions, before every defined function.
    pub(crate) imports:       ArenaVec<'a, Import<'a>>,
    pub(crate) functions:     ArenaVec<'a, Function<'a>>,
    /// The function table's elements from index 1; index 0 stays null, so
    /// a null function pointer traps when called.
    pub(crate) table:         ArenaVec<'a, u32>,
    /// The memory's initial size in 64 KiB pages; it may grow without
    /// limit.
    pub(crate) memory_pages:  u32,
    /// The initial value of global 0, the stack pointer `__stack_pointer`.
    pub(crate) stack_pointer: u32,
    pub(crate) exports:       ArenaVec<'a, Export<'a>>,
    /// The function the start section runs at instantiation.
    pub(crate) start:         Option<u32>,
    /// Active data segments of memory 0.
    pub(crate) data:          ArenaVec<'a, Segment<'a>>,
}

impl WasmModule<'_> {
    /// The name of function `function`, imported or defined.
    pub(crate) fn function_name(&self, function: u32) -> &str {
        let imports = u32_of(self.imports.len());
        if function < imports {
            self.imports[function as usize].name
        } else {
            self.functions[(function - imports) as usize].name
        }
    }
}

/// A function type.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct FuncType<'a> {
    pub(crate) params:  &'a [ValType],
    pub(crate) results: &'a [ValType],
}

/// An imported function.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Import<'a> {
    pub(crate) module: &'a str,
    pub(crate) field:  &'a str,
    /// The C name the module calls it by.
    pub(crate) name:   &'a str,
    pub(crate) ty:     u32,
}

/// A defined function.
pub(crate) struct Function<'a> {
    /// The symbol name, for the name section and the text.
    pub(crate) name:        &'a str,
    pub(crate) ty:          u32,
    /// The locals after the parameters.
    pub(crate) locals:      &'a [ValType],
    /// A name for each parameter and local, in index order.
    pub(crate) local_names: &'a [&'a str],
    pub(crate) code:        &'a [Instr],
    /// The labels `br_table` instructions name.
    pub(crate) labels:      &'a [u32],
}

/// An export.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Export<'a> {
    pub(crate) name:  &'a str,
    pub(crate) kind:  ExportKind,
    pub(crate) index: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ExportKind {
    Function,
    Memory,
}

/// Bytes the module places in memory at `offset` when it is instantiated.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Segment<'a> {
    pub(crate) offset: u32,
    pub(crate) bytes:  &'a [u8],
}
