//! Module-level declarations: functions, globals, signatures and symbols,
//! and the queries that need the whole module, such as an instruction's
//! result type.

use std::fmt;

use super::{
    Function,
    Module,
    entities::{
        FuncId,
        GlobalId,
        SigId,
        Table,
    },
    instructions::{
        Align,
        InstData,
        Opcode,
    },
    types::Type,
};
use crate::util::bump::{
    ArenaMap,
    Bump,
};

impl<'ir> Module<'ir> {
    /// The type of the value an instruction defines, or `None` if it defines
    /// none. A call's result comes from its signature; an invalid callee or
    /// signature has none, which the verifier reports separately.
    pub(crate) fn result_type(&self, data: &InstData) -> Option<Type> {
        match *data {
            | InstData::Binary {
                opcode: Opcode::VaCopy,
                ..
            }
            | InstData::Unary {
                opcode: Opcode::VaStart | Opcode::VaEnd,
                ..
            }
            | InstData::Store { .. }
            | InstData::MemoryRange { .. }
            | InstData::Jump { .. }
            | InstData::Brif { .. }
            | InstData::Switch { .. }
            | InstData::Return { .. }
            | InstData::Unreachable => None,
            | InstData::Binary { ty, .. }
            | InstData::Unary { ty, .. }
            | InstData::Select { ty, .. }
            | InstData::Const { ty, .. }
            | InstData::WideConst { ty, .. }
            | InstData::Nullary { ty, .. }
            | InstData::Load { ty, .. } => Some(ty),
            | InstData::IntCompare { .. } | InstData::FloatCompare { .. } => Some(Type::I1),
            | InstData::StackAddr { .. }
            | InstData::GlobalAddr { .. }
            | InstData::FuncAddr { .. } => Some(Type::Ptr),
            | InstData::Call { func, .. } => self
                .functions
                .get(func)
                .and_then(|function| self.signatures.get(function.signature))
                .and_then(|signature| signature.result),
            | InstData::CallIndirect { sig, .. } => self
                .signatures
                .get(sig)
                .and_then(|signature| signature.result),
        }
    }

    /// Interns a call signature, copying its parameters into the module's
    /// arena the first time it is seen.
    pub(crate) fn intern_signature(
        &mut self,
        params: &[Type],
        result: Option<Type>,
        variadic: bool,
    ) -> SigId {
        let signature = Signature {
            params: self.arena.alloc_slice_copy(params),
            result,
            variadic,
        };
        if let Some(&id) = self.signature_ids.get(&signature) {
            return id;
        }
        let id = self.signatures.push(signature);
        _ = self.signature_ids.insert(signature, id);
        id
    }

    /// Declares a function without a body. Panics if the name is taken; the
    /// textual parser checks first and reports the clash.
    pub(crate) fn declare_function(
        &mut self,
        name: &str,
        signature: SigId,
        linkage: Linkage,
    ) -> FuncId {
        let name = self.arena.alloc_str(name);
        let id = self.functions.push(Function {
            name,
            signature,
            linkage,
            body: None,
        });
        self.add_symbol(name, Symbol::Function(id));
        id
    }

    /// Declares a global without an initializer. Panics if the name is
    /// taken.
    pub(crate) fn declare_global(&mut self, name: &str, declaration: GlobalDecl) -> GlobalId {
        let name = self.arena.alloc_str(name);
        let id = self.globals.push(Global {
            name,
            linkage: declaration.linkage,
            size: declaration.size,
            align: declaration.align,
            constant: declaration.constant,
            init: None,
        });
        self.add_symbol(name, Symbol::Global(id));
        id
    }

    /// Gives a global its initializer, copying the bytes and relocations
    /// into the module's arena.
    pub(crate) fn define_global(&mut self, global: GlobalId, init: GlobalInit<'_>) {
        let init = match init {
            | GlobalInit::Zero => GlobalInit::Zero,
            | GlobalInit::Bytes { bytes, relocations } => GlobalInit::Bytes {
                bytes:       self.arena.alloc_slice_copy(bytes),
                relocations: self.arena.alloc_slice_copy(relocations),
            },
        };
        self.globals[global].init = Some(init);
    }

    fn add_symbol(&mut self, name: &'ir str, symbol: Symbol) {
        let previous = self.symbols.insert(name, symbol);
        assert!(previous.is_none(), "the symbol @{name} is declared twice");
    }

    /// Sets the target triple and the LLVM data-layout string.
    pub(crate) fn set_target(&mut self, triple: &str, data_layout: &str) {
        self.triple = self.arena.alloc_str(triple);
        self.data_layout = self.arena.alloc_str(data_layout);
    }
}

impl<'ir> Module<'ir> {
    /// An empty module with no target.
    pub(crate) fn new(arena: &'ir Bump) -> Self {
        Self {
            arena,
            triple: "",
            data_layout: "",
            functions: Table::new_in(arena),
            globals: Table::new_in(arena),
            signatures: Table::new_in(arena),
            signature_ids: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena),
            symbols: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena),
        }
    }

    pub(crate) fn arena(&self) -> &'ir Bump {
        self.arena
    }

    pub(crate) fn triple(&self) -> &'ir str {
        self.triple
    }

    pub(crate) fn data_layout(&self) -> &'ir str {
        self.data_layout
    }

    pub(crate) fn function(&self, func: FuncId) -> &Function<'ir> {
        &self.functions[func]
    }

    pub(crate) fn function_mut(&mut self, func: FuncId) -> &mut Function<'ir> {
        &mut self.functions[func]
    }

    pub(crate) fn get_function(&self, func: FuncId) -> Option<&Function<'ir>> {
        self.functions.get(func)
    }

    /// Every function with its id, in declaration order.
    pub(crate) fn functions(&self) -> impl DoubleEndedIterator<Item = (FuncId, &Function<'ir>)> {
        self.functions.iter()
    }

    pub(crate) fn global(&self, global: GlobalId) -> &Global<'ir> {
        &self.globals[global]
    }

    pub(crate) fn get_global(&self, global: GlobalId) -> Option<&Global<'ir>> {
        self.globals.get(global)
    }

    /// Every global with its id, in declaration order.
    pub(crate) fn globals(&self) -> impl DoubleEndedIterator<Item = (GlobalId, &Global<'ir>)> {
        self.globals.iter()
    }

    pub(crate) fn signature(&self, sig: SigId) -> Signature<'ir> {
        self.signatures[sig]
    }

    pub(crate) fn get_signature(&self, sig: SigId) -> Option<Signature<'ir>> {
        self.signatures.get(sig).copied()
    }

    /// The signature of a function.
    pub(crate) fn function_signature(&self, func: FuncId) -> Signature<'ir> {
        self.signatures[self.functions[func].signature]
    }

    /// The function or global with this name.
    pub(crate) fn symbol(&self, name: &str) -> Option<Symbol> {
        self.symbols.get(name).copied()
    }

    pub(crate) fn symbol_name(&self, symbol: Symbol) -> &'ir str {
        match symbol {
            | Symbol::Function(func) => self.functions[func].name,
            | Symbol::Global(global) => self.globals[global].name,
        }
    }
}

impl fmt::Debug for Module<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Module")
            .field("triple", &self.triple)
            .field("data_layout", &self.data_layout)
            .field("functions", &self.functions)
            .field("globals", &self.globals)
            .field("signatures", &self.signatures)
            .finish_non_exhaustive()
    }
}

/// A call signature: parameter types, an optional result, and whether extra
/// arguments may follow the fixed ones.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct Signature<'ir> {
    pub(crate) params:   &'ir [Type],
    pub(crate) result:   Option<Type>,
    pub(crate) variadic: bool,
}

/// Whether a symbol is visible outside its module.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Linkage {
    External,
    Internal,
}

impl Linkage {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            | Self::External => "external",
            | Self::Internal => "internal",
        }
    }

    pub(crate) fn from_name(name: &str) -> Option<Self> {
        [Self::External, Self::Internal]
            .into_iter()
            .find(|linkage| linkage.name() == name)
    }
}

/// A function or a global, as a relocation or the textual form names it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Symbol {
    Function(FuncId),
    Global(GlobalId),
}

/// A global object. One without an initializer is declared, not defined.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Global<'ir> {
    pub(crate) name:     &'ir str,
    pub(crate) linkage:  Linkage,
    pub(crate) size:     u64,
    pub(crate) align:    Align,
    pub(crate) constant: bool,
    pub(crate) init:     Option<GlobalInit<'ir>>,
}

/// What [`Module::declare_global`] needs besides the name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct GlobalDecl {
    pub(crate) linkage:  Linkage,
    pub(crate) size:     u64,
    pub(crate) align:    Align,
    pub(crate) constant: bool,
}

/// A global's initial contents.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum GlobalInit<'a> {
    /// Every byte is zero.
    Zero,
    /// The bytes, with address constants patched in by the relocations.
    Bytes {
        bytes:       &'a [u8],
        relocations: &'a [Relocation],
    },
}

/// An address constant in a global's bytes: the eight bytes at `offset`
/// hold the address of `symbol` plus `addend`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Relocation {
    pub(crate) offset: u64,
    pub(crate) symbol: Symbol,
    pub(crate) addend: i64,
}
