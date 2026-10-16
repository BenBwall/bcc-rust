//! Module-level declarations: functions, globals, signatures and symbols,
//! and the queries that need the whole module, such as an instruction's
//! result type.

use std::fmt;

use bitflags::bitflags;

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
    verifier::Profile,
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
            | InstData::Nullary {
                opcode: Opcode::Trap,
                ..
            }
            | InstData::Store { .. }
            | InstData::MemoryRange { .. }
            | InstData::Jump { .. }
            | InstData::Brif { .. }
            | InstData::Switch { .. }
            | InstData::Return { .. }
            | InstData::Unreachable => None,
            | InstData::Unary {
                opcode: Opcode::DynAlloca,
                ..
            }
            | InstData::StackAddr { .. }
            | InstData::GlobalAddr { .. }
            | InstData::FuncAddr { .. } => Some(Type::Ptr),
            | InstData::Binary { opcode, .. } if opcode.is_overflow() => Some(Type::I1),
            | InstData::Binary { ty, .. }
            | InstData::Unary { ty, .. }
            | InstData::Select { ty, .. }
            | InstData::Const { ty, .. }
            | InstData::WideConst { ty, .. }
            | InstData::Nullary { ty, .. }
            | InstData::Load { ty, .. } => Some(ty),
            | InstData::IntCompare { .. } | InstData::FloatCompare { .. } => Some(Type::I1),

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

    /// Interns a call signature of scalar and pointer parameters, copying
    /// them into the module's arena the first time it is seen.
    pub(crate) fn intern_signature(
        &mut self,
        params: &[Type],
        result: Option<Type>,
        variadic: bool,
    ) -> SigId {
        self.intern(Signature {
            params,
            result,
            variadic,
            aggregates: &[],
            extensions: &[],
            result_extension: None,
            noundef: &[],
            result_noundef: false,
            variadic_noundef: false,
        })
    }

    /// Interns any call signature, copying its parameters, its aggregate
    /// parameters and their layouts, and its extended parameters into the
    /// module's arena the first time it is seen.
    pub(crate) fn intern(&mut self, signature: Signature<'_>) -> SigId {
        if let Some(&id) = self.signature_ids.get(&signature) {
            return id;
        }
        let arena = self.arena;
        let aggregates =
            arena.alloc_slice_fill_iter(signature.aggregates.iter().map(|param| AggregateParam {
                index: param.index,
                abi:   param.abi.copy_into(arena),
            }));
        let signature = Signature {
            params: arena.alloc_slice_copy(signature.params),
            result: signature.result,
            variadic: signature.variadic,
            aggregates,
            extensions: arena.alloc_slice_copy(signature.extensions),
            result_extension: signature.result_extension,
            noundef: arena.alloc_slice_copy(signature.noundef),
            result_noundef: signature.result_noundef,
            variadic_noundef: signature.variadic_noundef,
        };
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

    /// Records that ABI lowering has run, or, for the textual parser, that
    /// the text says it has. The interpreter lays out `va_list` for the
    /// target only in a post-ABI module, whose `va_arg` is expanded.
    pub(crate) fn set_profile(&mut self, profile: Profile) {
        self.profile = profile;
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
            profile: Profile::PreAbi,
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

    /// Whether ABI lowering has run on the module.
    pub(crate) fn profile(&self) -> Profile {
        self.profile
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

    /// How many signatures the module has interned; they are numbered from
    /// 0.
    pub(crate) fn signature_count(&self) -> usize {
        self.signatures.len()
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
            .field("profile", &self.profile)
            .field("functions", &self.functions)
            .field("globals", &self.globals)
            .field("signatures", &self.signatures)
            .finish_non_exhaustive()
    }
}

/// A call signature: parameter types, an optional result, whether extra
/// arguments may follow the fixed ones, which `ptr` parameters stand for
/// structures and unions, and which integer parameters and result C typed
/// narrower than `int`.
///
/// The IR has no aggregate values, so a structure or union passed or
/// returned by value is a `ptr` parameter that an [`AggregateParam`]
/// describes. Before ABI lowering the description is C's view with the
/// aggregate's layout ([`ParamAbi::Aggregate`], [`ParamAbi::Result`]); ABI
/// lowering replaces it by the target's decision ([`ParamAbi::ByVal`],
/// [`ParamAbi::Sret`], [`ParamAbi::Registers`]) or by plain scalars. In the
/// same way an `i8` or `i16` parameter or result records whether its C type
/// is signed, unsigned or `_Bool` before ABI lowering, and how the target
/// widens it after ([`Extension`]).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct Signature<'ir> {
    pub(crate) params:           &'ir [Type],
    pub(crate) result:           Option<Type>,
    pub(crate) variadic:         bool,
    /// The parameters that stand for an aggregate, in increasing parameter
    /// order; empty when every parameter is a plain scalar or pointer.
    pub(crate) aggregates:       &'ir [AggregateParam<'ir>],
    /// The `i8` and `i16` parameters with an [`Extension`], in increasing
    /// parameter order.
    pub(crate) extensions:       &'ir [ExtendedParam],
    /// The [`Extension`] of an `i8` or `i16` result.
    pub(crate) result_extension: Option<Extension>,
    /// Increasing indices of parameters requiring a fully defined value.
    /// This constrains the pointer itself for aggregate memory parameters,
    /// never the bytes it addresses. Available in both profiles.
    pub(crate) noundef:          &'ir [u32],
    /// Poison in any bit of the returned value is immediate UB. For a
    /// post-ABI register result this includes every byte of each register.
    pub(crate) result_noundef:   bool,
    /// Every argument after the fixed parameters requires a defined value.
    pub(crate) variadic_noundef: bool,
}

impl<'ir> Signature<'ir> {
    /// Whether passing poison at this argument position is immediate UB.
    /// Verified signatures keep the marked indices sorted, so lookups in
    /// per-parameter loops take logarithmic time rather than a full scan.
    pub(crate) fn param_noundef(&self, index: usize) -> bool {
        if index >= self.params.len() {
            self.variadic_noundef
        } else {
            self.noundef
                .binary_search_by_key(&index, |&param| param as usize)
                .is_ok()
        }
    }

    /// The extension of parameter `index`, if it has one.
    pub(crate) fn param_extension(&self, index: usize) -> Option<Extension> {
        self.extensions
            .iter()
            .find(|param| param.index as usize == index)
            .map(|param| param.extension)
    }

    /// How parameter `index` stands for an aggregate, if it does.
    pub(crate) fn param_abi(&self, index: usize) -> Option<ParamAbi<'ir>> {
        self.aggregates
            .iter()
            .find(|param| param.index as usize == index)
            .map(|param| param.abi)
    }

    /// How the function returns a structure or union through its parameter
    /// 0, if it does.
    pub(crate) fn result_param(&self) -> Option<ParamAbi<'ir>> {
        self.param_abi(0).filter(|abi| abi.is_result())
    }
}

/// An `i8` or `i16` parameter with an [`Extension`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct ExtendedParam {
    /// The parameter's position in [`Signature::params`].
    pub(crate) index:     u32,
    pub(crate) extension: Extension,
}

/// What an integer parameter or result narrower than `int` was in C, and
/// how the calling convention widens it. Before ABI lowering it is the C
/// type's signedness; after it, whether the caller (for an argument) or the
/// callee (for a result) extends the value to 32 bits, which the other side
/// may rely on. A post-ABI `i8` or `i16` without an extension has upper
/// register bits that nobody may read.
/// C99: §6.2.5 paragraphs 2-6, pp. 33-34; PDF pp. 45-46 (signed, unsigned
/// and `_Bool` types).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Extension {
    /// Pre-ABI: a signed integer type, such as `short`, `signed char`, or
    /// `char` where it is signed.
    Signed,
    /// Pre-ABI: an unsigned integer type other than `_Bool`.
    Unsigned,
    /// Pre-ABI: `_Bool`, whose value is 0 or 1.
    Bool,
    /// Post-ABI: sign-extended to 32 bits (LLVM's `signext`).
    Sign,
    /// Post-ABI: zero-extended to 32 bits (LLVM's `zeroext`).
    Zero,
}

impl Extension {
    /// Every extension, in the order the textual form lists them.
    pub(crate) const ALL: [Self; 5] = [
        Self::Signed,
        Self::Unsigned,
        Self::Bool,
        Self::Sign,
        Self::Zero,
    ];

    /// Whether this kind exists before ABI lowering rather than after.
    pub(crate) const fn is_pre_abi(self) -> bool {
        matches!(self, Self::Signed | Self::Unsigned | Self::Bool)
    }

    /// The textual form's keyword for this kind.
    pub(crate) const fn keyword(self) -> &'static str {
        match self {
            | Self::Signed => "signed",
            | Self::Unsigned => "unsigned",
            | Self::Bool => "bool",
            | Self::Sign => "signext",
            | Self::Zero => "zeroext",
        }
    }

    /// The kind `keyword` names.
    pub(crate) fn from_keyword(keyword: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|extension| extension.keyword() == keyword)
    }
}

/// A `ptr` parameter that stands for a structure or union.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct AggregateParam<'ir> {
    /// The parameter's position in [`Signature::params`].
    pub(crate) index: u32,
    pub(crate) abi:   ParamAbi<'ir>,
}

/// What an aggregate parameter means. The first two kinds exist only before
/// ABI lowering and the others only after it; the verifier checks which.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum ParamAbi<'ir> {
    /// Pre-ABI: the parameter addresses a copy of a structure or union
    /// argument that the caller made for this call. The callee owns the copy
    /// until it returns and may write it; nothing else addresses it.
    Aggregate(AggregateLayout<'ir>),
    /// Pre-ABI, parameter 0 only: the parameter addresses the object that
    /// receives the structure or union result, which the callee stores
    /// before it returns. The signature has no result. The caller passes a
    /// fresh object that nothing else addresses.
    Result(AggregateLayout<'ir>),
    /// Post-ABI: the aggregate is passed in memory, in the stack's argument
    /// area, and the parameter addresses that copy (LLVM's `byval`). The
    /// layout's alignment is the stack slot's, which ABI lowering may raise
    /// above the aggregate's; the caller's pointer must have it too.
    ByVal(AggregateLayout<'ir>),
    /// Post-ABI, parameter 0 only: the result is returned in memory at the
    /// address the caller passes, and the callee returns that address as
    /// well (LLVM's `sret`).
    Sret(AggregateLayout<'ir>),
    /// Post-ABI, parameter 0 only: the result is returned in registers. The
    /// parameter is a return area that the IR stores the result into, as for
    /// [`ParamAbi::Result`], but it is not passed. A back end reserves the
    /// area in the callee and loads the registers from it at `return`; in
    /// the caller it stores the returned registers to the area the call
    /// passes, which ABI lowering makes at least [`ResultRegisters::bytes`]
    /// long and aligned to [`ResultRegisters::align`].
    Registers(ResultRegisters),
}

impl<'ir> ParamAbi<'ir> {
    /// Whether this kind describes a result rather than an argument.
    pub(crate) const fn is_result(self) -> bool {
        matches!(self, Self::Result(_) | Self::Sret(_) | Self::Registers(_))
    }

    /// Whether this kind exists before ABI lowering rather than after.
    pub(crate) const fn is_pre_abi(self) -> bool {
        matches!(self, Self::Aggregate(_) | Self::Result(_))
    }

    /// The aggregate's layout, for the kinds that carry one.
    pub(crate) const fn layout(self) -> Option<AggregateLayout<'ir>> {
        match self {
            | Self::Aggregate(layout)
            | Self::Result(layout)
            | Self::ByVal(layout)
            | Self::Sret(layout) => Some(layout),
            | Self::Registers(_) => None,
        }
    }

    /// The textual form's keyword for this kind.
    pub(crate) const fn keyword(self) -> &'static str {
        match self {
            | Self::Aggregate(_) => "aggregate",
            | Self::Result(_) => "result",
            | Self::ByVal(_) => "byval",
            | Self::Sret(_) => "sret",
            | Self::Registers(_) => "registers",
        }
    }

    /// This description with its layout's scalars in `arena`.
    fn copy_into(self, arena: &Bump) -> ParamAbi<'_> {
        let copy = |layout: AggregateLayout<'_>| AggregateLayout {
            scalars: arena.alloc_slice_copy(layout.scalars),
            size:    layout.size,
            align:   layout.align,
            flags:   layout.flags,
        };
        match self {
            | Self::Aggregate(layout) => ParamAbi::Aggregate(copy(layout)),
            | Self::Result(layout) => ParamAbi::Result(copy(layout)),
            | Self::ByVal(layout) => ParamAbi::ByVal(copy(layout)),
            | Self::Sret(layout) => ParamAbi::Sret(copy(layout)),
            | Self::Registers(registers) => ParamAbi::Registers(registers),
        }
    }
}

/// The facts about a structure or union that calling conventions classify
/// it by: its size and alignment, two properties of its members, and, for
/// one of at most [`AggregateLayout::MAX_SCALAR_BYTES`] bytes, the scalars
/// it is made of. The IR has no type graph, so lowering records these from
/// the C type.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct AggregateLayout<'ir> {
    pub(crate) size:    u64,
    pub(crate) align:   Align,
    pub(crate) flags:   AggregateFlags,
    /// Every scalar member and array element, through nested aggregates, by
    /// byte offset; the members of a union overlap. Empty for an aggregate
    /// larger than [`AggregateLayout::MAX_SCALAR_BYTES`].
    pub(crate) scalars: &'ir [AggregateScalar],
}

impl AggregateLayout<'_> {
    /// The largest aggregate whose scalars are recorded: two eightbytes,
    /// the most any supported convention passes in registers, since the IR
    /// has no vector types.
    pub(crate) const MAX_SCALAR_BYTES: u64 = 16;
}

bitflags! {
    /// Properties of an aggregate's members that change how it is passed.
    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
    pub(crate) struct AggregateFlags: u8 {
        /// A member, at some level of nesting, is not at a multiple of its
        /// type's alignment, as in a packed structure.
        const UNALIGNED = 1 << 0;
        /// The aggregate, or one it contains, ends in a flexible array
        /// member.
        const FLEXIBLE = 1 << 1;
    }
}

impl AggregateFlags {
    /// Each flag with its textual name, in printing order.
    pub(crate) const NAMES: [(Self, &'static str); 2] =
        [(Self::UNALIGNED, "unaligned"), (Self::FLEXIBLE, "flexible")];
}

/// One scalar of an aggregate: its byte offset and IR type.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct AggregateScalar {
    pub(crate) offset: u32,
    pub(crate) ty:     Type,
}

/// The registers a memory-backed result comes back in: the first starts
/// at byte 0 and an optional second at byte 8. A lone `i128` represents the
/// Win64 scalar extension's `<2 x i64>` result in XMM0; `f128` carries a
/// System V SSE/SSEUP result in one vector register.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct ResultRegisters {
    pub(crate) first:  Type,
    pub(crate) second: Option<Type>,
    /// The aggregate's alignment, which its return area has.
    pub(crate) align:  Align,
}

impl ResultRegisters {
    /// Paired results occupy two ABI eightbytes. Natural LLVM structure
    /// layout must place the second field at this same byte offset.
    /// C99: §6.8.6.4p3, p. 139; PDF p. 151; the layout is an IR/ABI contract.
    pub(crate) const SECOND_OFFSET: u32 = 8;

    /// Whether both parts fit eightbytes and their natural layout agrees
    /// with the return area produced by ABI lowering.
    pub(crate) fn valid_layout(self) -> bool {
        self.second.is_none_or(|second| {
            let first_size = self.first.bytes();
            let second_size = second.bytes();
            first_size <= Self::SECOND_OFFSET
                && second_size <= Self::SECOND_OFFSET
                && first_size.div_ceil(second_size) * second_size == Self::SECOND_OFFSET
        })
    }

    /// Each register's byte offset and type in the return area.
    pub(crate) fn parts(self) -> impl Iterator<Item = (u64, Type)> {
        std::iter::once((0, self.first))
            .chain(self.second.map(|ty| (u64::from(Self::SECOND_OFFSET), ty)))
    }

    /// The registers' types in order.
    pub(crate) fn types(self) -> impl Iterator<Item = Type> {
        self.parts().map(|(_, ty)| ty)
    }

    /// The bytes of the return area the registers cover.
    pub(crate) fn bytes(self) -> u64 {
        self.parts()
            .map(|(offset, ty)| offset + u64::from(ty.bytes()))
            .max()
            .unwrap_or(0)
    }

    /// Callee storage also covers the aggregate's trailing padding: C code
    /// may copy the complete object into this area before returning fields.
    pub(crate) fn area_bytes(self) -> u64 {
        self.bytes().max(2 * u64::from(Self::SECOND_OFFSET))
    }
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
