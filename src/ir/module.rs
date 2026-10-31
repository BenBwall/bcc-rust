//! Module-level declarations: functions, globals, signatures and symbols,
//! and the queries that need the whole module, such as an instruction's
//! result type.

use std::fmt;

use bitflags::bitflags;

use super::{
    Function,
    InlineAsm,
    Module,
    entities::{
        AsmId,
        BlockLabel,
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
use crate::{
    configuration::LibraryBuiltins,
    util::bump::{
        ArenaMap,
        ArenaVec,
        Bump,
    },
};

impl<'ir> Module<'ir> {
    /// The type of the value an instruction defines, or `None` if it defines
    /// none. A call's result comes from its signature; an invalid callee or
    /// signature has none, which the verifier reports separately.
    pub(crate) fn result_type(&self, data: &InstData) -> Option<Type> {
        match *data {
            | InstData::Atomic { op, ty, .. } => op.has_result().then_some(ty),
            | InstData::Binary {
                opcode: Opcode::VaCopy,
                ..
            }
            | InstData::Unary {
                opcode: Opcode::VaStart | Opcode::VaEnd | Opcode::StackRestore | Opcode::SjljLongjmp,
                ..
            }
            | InstData::Nullary {
                opcode: Opcode::Trap | Opcode::DebugTrap,
                ..
            }
            | InstData::Store { .. }
            | InstData::InlineAsm { .. }
            | InstData::MemoryRange { .. }
            | InstData::Jump { .. }
            | InstData::Brif { .. }
            | InstData::Switch { .. }
            | InstData::Return { .. }
            | InstData::IndirectBr { .. }
            | InstData::Unreachable => None,
            | InstData::DynAlloca { .. }
            | InstData::VaArgAggregate { .. }
            | InstData::StackAddr { .. }
            | InstData::GlobalAddr { .. }
            | InstData::FuncAddr { .. }
            | InstData::BlockAddr { .. } => Some(Type::Ptr),
            | InstData::Binary { opcode, .. } if opcode.is_overflow() => Some(Type::I1),
            | InstData::Binary { ty, .. }
            | InstData::Unary { ty, .. }
            | InstData::Select { ty, .. }
            | InstData::ObjectSize { ty, .. }
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
            variadic_fixed: None,
            aggregates: &[],
            extensions: &[],
            result_extension: None,
            noundef: &[],
            result_noundef: false,
            variadic_noundef: false,
            chain: false,
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
            variadic_fixed: signature.variadic_fixed,
            aggregates,
            extensions: arena.alloc_slice_copy(signature.extensions),
            result_extension: signature.result_extension,
            noundef: arena.alloc_slice_copy(signature.noundef),
            result_noundef: signature.result_noundef,
            variadic_noundef: signature.variadic_noundef,
            chain: signature.chain,
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
            attributes: SymbolAttributes::default(),
            align: None,
            returns_twice: false,
            inlining: InlineControl::Default,
            inline_copy_of: None,
        });
        self.add_symbol(name, Symbol::Function(id));
        id
    }

    /// Records that `func` is the private inline copy of the external
    /// function named `external` ([`Function::inline_copy_of`]).
    pub(crate) fn set_inline_copy_of(&mut self, func: FuncId, external: &str) {
        let external = self.arena.alloc_str(external);
        self.functions[func].inline_copy_of = Some(external);
    }

    /// The external function whose inline definition the copy `func`
    /// supplies, as Clang emits a C99 inline definition as that function's
    /// `available_externally` body: an ordinary external declaration of the
    /// same signature that this module neither defines nor aliases, and
    /// that `noinline` does not forbid expanding. A call that keeps naming
    /// such a copy after inlining reaches the external definition. `None`
    /// for anything else, which keeps an `always_inline` copy an ordinary
    /// internal function. C99: §6.7.4p6, pp. 112-113; PDF pp. 124-125.
    pub(crate) fn inline_copy_target(&self, func: FuncId) -> Option<FuncId> {
        let function = &self.functions[func];
        let Some(Symbol::Function(external)) = self.symbol(function.inline_copy_of?) else {
            return None;
        };
        let declaration = &self.functions[external];
        (function.body.is_some()
            && declaration.body.is_none()
            && declaration.attributes.alias.is_none()
            && declaration.inline_copy_of.is_none()
            && declaration.linkage == Linkage::External
            && declaration.inlining != InlineControl::Never
            && declaration.signature == function.signature)
            .then_some(external)
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
            thread_local: declaration.thread_local,
            init: None,
            attributes: SymbolAttributes::default(),
        });
        self.add_symbol(name, Symbol::Global(id));
        id
    }

    /// Records a symbol's alias target, section and retention, copying a
    /// section name into the module's arena.
    pub(crate) fn set_symbol_attributes(
        &mut self,
        symbol: Symbol,
        attributes: SymbolAttributes<'_>,
    ) {
        let attributes = SymbolAttributes {
            alias:      attributes.alias,
            section:    attributes.section.map(|name| &*self.arena.alloc_str(name)),
            used:       attributes.used,
            visibility: attributes.visibility,
            dll:        attributes.dll,
        };
        match symbol {
            | Symbol::Function(func) => self.functions[func].attributes = attributes,
            | Symbol::Global(global) => self.globals[global].attributes = attributes,
        }
    }

    /// The alias target, section and retention of a function or global.
    pub(crate) fn symbol_attributes(&self, symbol: Symbol) -> SymbolAttributes<'ir> {
        match symbol {
            | Symbol::Function(func) => self.functions[func].attributes,
            | Symbol::Global(global) => self.globals[global].attributes,
        }
    }

    /// Registers `func` to run before (`Constructor`) or after (`Destructor`)
    /// the program's entry function.
    pub(crate) fn add_init_function(&mut self, init: InitFunction) {
        self.init_functions.push(init);
    }

    /// Constructors and destructors in registration order.
    pub(crate) fn init_functions(&self) -> &[InitFunction] {
        &self.init_functions
    }

    /// Appends a file-scope assembly block, copying its text into the
    /// module's arena. Back ends emit the blocks in this order.
    /// GNU extension: GCC manual, "Basic Asm", at file scope.
    pub(crate) fn add_module_asm(&mut self, text: &str) {
        let text = self.arena.alloc_str(text);
        self.asm_blocks.push(text);
    }

    /// Adds an inline assembly descriptor, copying it into the module's
    /// arena.
    pub(crate) fn add_inline_asm(&mut self, asm: &InlineAsm<'_>) -> AsmId {
        let asm = asm.copy_into(self.arena);
        self.inline_asm.push(asm)
    }

    /// The descriptor an `asm` instruction names.
    pub(crate) fn inline_asm(&self, asm: AsmId) -> &InlineAsm<'ir> {
        &self.inline_asm[asm]
    }

    /// The descriptor `asm`, if the module has it.
    pub(crate) fn get_inline_asm(&self, asm: AsmId) -> Option<&InlineAsm<'ir>> {
        self.inline_asm.get(asm)
    }

    /// Whether any function has inline assembly: the module has an `asm`
    /// descriptor.
    pub(crate) fn has_inline_asm(&self) -> bool {
        !self.inline_asm.is_empty()
    }

    /// The file-scope assembly blocks in source order.
    pub(crate) fn module_asm(&self) -> &[&'ir str] {
        &self.asm_blocks
    }

    /// Restricts the library functions a back end may treat as builtins,
    /// copying the names into the module's arena.
    pub(crate) fn set_library_builtins(&mut self, builtins: LibraryBuiltins<'_>) {
        let arena = self.arena;
        self.library_builtins = match builtins {
            | LibraryBuiltins::All => LibraryBuiltins::All,
            | LibraryBuiltins::None => LibraryBuiltins::None,
            | LibraryBuiltins::Except(names) => LibraryBuiltins::Except(
                arena.alloc_slice_fill_iter(names.iter().map(|name| &*arena.alloc_str(name))),
            ),
        };
    }

    /// The library functions a back end may treat as builtins.
    pub(crate) const fn library_builtins(&self) -> LibraryBuiltins<'ir> {
        self.library_builtins
    }

    /// Lets an object live at address 0, so a back end must not take a
    /// dereference to prove a pointer non-null.
    pub(crate) const fn set_null_pointer_is_valid(&mut self, valid: bool) {
        self.null_pointer_is_valid = valid;
    }

    /// Whether an object may live at address 0.
    pub(crate) const fn null_pointer_is_valid(&self) -> bool {
        self.null_pointer_is_valid
    }

    /// Asks the back ends to keep every floating operation, and so the
    /// invalid exception an operation on a signaling NaN raises, rather
    /// than fold or simplify one away (`-fsignaling-nans`).
    pub(crate) const fn set_signaling_nans(&mut self, signaling: bool) {
        self.signaling_nans = signaling;
    }

    /// Whether floating operations keep their exceptions.
    pub(crate) const fn signaling_nans(&self) -> bool {
        self.signaling_nans
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
            init_functions: ArenaVec::new_in(arena),
            asm_blocks: ArenaVec::new_in(arena),
            library_builtins: LibraryBuiltins::All,
            null_pointer_is_valid: false,
            signaling_nans: false,
            inline_asm: Table::new_in(arena),
        }
    }

    pub(crate) fn arena(&self) -> &'ir Bump {
        self.arena
    }

    pub(crate) fn triple(&self) -> &'ir str {
        self.triple
    }

    /// The bytes a pointer takes in memory: four on the ILP32 WebAssembly
    /// target, eight on every other. The IR's `ptr` values are opaque;
    /// lowering accesses pointer objects of an ILP32 module as `i32` (see
    /// `lowering/emit.rs`), so only relocations and back ends read this.
    pub(crate) fn pointer_bytes(&self) -> u64 {
        if self.triple.starts_with("wasm32") {
            4
        } else {
            8
        }
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

    /// Whether a call to `func` may return twice: its own
    /// [`Function::returns_twice`] or, for an alias, its target's. A
    /// verified alias names a definition that is not an alias, so one step
    /// reaches the canonical function. Every returns-twice safeguard asks
    /// here rather than reading the flag. C99: §7.13.2.1 paragraph 4,
    /// p. 244; PDF p. 256.
    pub(crate) fn returns_twice(&self, func: FuncId) -> bool {
        let Some(function) = self.functions.get(func) else {
            return false;
        };
        function.returns_twice
            || self
                .functions
                .get(self.function_definition(func))
                .is_some_and(|target| target.returns_twice)
    }

    /// Whether any function body makes a `trampoline`, whose code runs
    /// where its storage is: on the stack.
    /// GNU extension: GCC manual, "Nested Functions".
    pub(crate) fn uses_trampolines(&self) -> bool {
        self.functions.iter().any(|(_, function)| {
            function.body.as_ref().is_some_and(|body| {
                body.blocks().any(|block| {
                    body.block_insts(block)
                        .iter()
                        .any(|&inst| body.inst(inst).opcode() == Opcode::Trampoline)
                })
            })
        })
    }

    /// Whether the address of a function that may return twice escapes into
    /// a value (`func_addr`) or a global's relocation, so any indirect call
    /// in the module may reach it. Back ends and the inliner then treat
    /// every indirect call as returning twice; modules without such an
    /// address keep ordinary indirect calls.
    pub(crate) fn returns_twice_escapes(&self) -> bool {
        let escapes = |func| self.returns_twice(func);
        self.globals.iter().any(|(_, global)| {
            matches!(global.init, Some(GlobalInit::Bytes { relocations, .. })
            if relocations.iter().any(|relocation| matches!(
                relocation.target,
                RelocationTarget::Symbol(Symbol::Function(func)) if escapes(func)
            )))
        }) || self.functions.iter().any(|(_, function)| {
            function.body.as_ref().is_some_and(|body| {
                body.blocks().any(|block| {
                    body.block_insts(block).iter().any(|&inst| {
                        matches!(*body.inst(inst), InstData::FuncAddr { func } if escapes(func))
                    })
                })
            })
        })
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

    /// The function a call to or the address of `func` reaches: the
    /// definition an alias names, else `func` itself. An alias names a
    /// definition that is not itself an alias (`verify_symbol_attributes`),
    /// so one step resolves it.
    /// GNU extension: GCC manual, "Common Function Attributes", alias.
    pub(crate) fn function_definition(&self, func: FuncId) -> FuncId {
        match self.function(func).attributes.alias {
            | Some(Symbol::Function(target)) => target,
            | _ => func,
        }
    }

    /// The global whose storage `global` names: the definition an alias
    /// names, else `global` itself.
    /// GNU extension: GCC manual, "Common Variable Attributes", alias.
    pub(crate) fn global_definition(&self, global: GlobalId) -> GlobalId {
        match self.global(global).attributes.alias {
            | Some(Symbol::Global(target)) => target,
            | _ => global,
        }
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
            .field("asm_blocks", &self.asm_blocks)
            .field("inline_asm", &self.inline_asm)
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
#[expect(
    clippy::struct_excessive_bools,
    reason = "Each flag is an independent fact about the signature's calls."
)]
pub(crate) struct Signature<'ir> {
    pub(crate) params:           &'ir [Type],
    pub(crate) result:           Option<Type>,
    pub(crate) variadic:         bool,
    /// Call-site descriptions may retain types and ABI facts after the
    /// ellipsis. Only the prefix is part of the function type.
    pub(crate) variadic_fixed:   Option<u32>,
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
    /// The last fixed parameter, a `ptr`, is the static chain of a GNU
    /// nested function: the record of the function that defines it. A
    /// target passes it in a register of its own (x86-64: `r10`), where a
    /// trampoline puts it, so it takes no argument register.
    /// GNU extension: GCC manual, "Nested Functions".
    pub(crate) chain:            bool,
}

impl<'ir> Signature<'ir> {
    pub(crate) fn fixed_count(&self) -> usize {
        self.variadic_fixed
            .map_or(self.params.len(), |n| n as usize)
    }

    /// The index of the static-chain parameter ([`Self::chain`]), if any.
    pub(crate) fn chain_param(&self) -> Option<usize> {
        self.chain
            .then(|| self.fixed_count().checked_sub(1))
            .flatten()
    }

    /// Whether passing poison at this argument position is immediate UB.
    /// Verified signatures keep the marked indices sorted, so lookups in
    /// per-parameter loops take logarithmic time rather than a full scan.
    pub(crate) fn param_noundef(&self, index: usize) -> bool {
        if index >= self.fixed_count() && self.variadic_noundef {
            return true;
        }
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

/// A `ptr` parameter that stands for a memory-backed C value.
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
    /// Post-ABI: the address of a caller-owned complex scalar copy, passed
    /// as an ordinary pointer (Win64). Its leaves retain the scalar's value
    /// contract; it has no LLVM byval attribute.
    Indirect(AggregateLayout<'ir>),
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
            | Self::Indirect(layout)
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
            | Self::Indirect(_) => "indirect",
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
            first:   layout.first,
        };
        match self {
            | Self::Aggregate(layout) => ParamAbi::Aggregate(copy(layout)),
            | Self::Result(layout) => ParamAbi::Result(copy(layout)),
            | Self::ByVal(layout) => ParamAbi::ByVal(copy(layout)),
            | Self::Indirect(layout) => ParamAbi::Indirect(copy(layout)),
            | Self::Sret(layout) => ParamAbi::Sret(copy(layout)),
            | Self::Registers(registers) => ParamAbi::Registers(registers),
        }
    }
}

/// The facts about a structure or union that calling conventions classify
/// it by: its size and alignment, two properties of its members, and, for
/// one of at most [`AggregateLayout::MAX_SCALAR_BYTES`] bytes, the scalars
/// it is made of. Top-level complex values always retain their two leaves,
/// including 32-byte x87 pairs. The IR has no type graph, so lowering records
/// these from the C type.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct AggregateLayout<'ir> {
    pub(crate) size:    u64,
    pub(crate) align:   Align,
    pub(crate) flags:   AggregateFlags,
    /// Every scalar member and array element, through nested aggregates, by
    /// byte offset; the members of a union overlap. Empty for an aggregate
    /// larger than [`AggregateLayout::MAX_SCALAR_BYTES`], except a top-level
    /// complex scalar retains both component leaves. With `first`, these
    /// describe the first member's scalars.
    pub(crate) scalars: &'ir [AggregateScalar],
    /// For a GNU transparent union argument on System V, the first member,
    /// whose facts decide its registers; `size` and `align` stay the
    /// union's, which travels on the stack when the registers are taken.
    pub(crate) first:   Option<FirstMember>,
}

/// The first member of a System V transparent union argument, which GCC and
/// Clang classify in the union's place (Clang's
/// `useFirstFieldIfTransparentUnion`): its size, the flags and scalars of
/// its [`AggregateLayout`], and, for an integer narrower than `int` or
/// `_Bool`, its pre-ABI extension, which its register keeps as a scalar
/// argument's does.
/// GNU extension: GCC manual, "Common Type Attributes", `transparent_union`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct FirstMember {
    pub(crate) size:      u64,
    pub(crate) extension: Option<Extension>,
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
        /// A top-level complex scalar, rather than a record of two reals.
        const COMPLEX = 1 << 2;
        /// A structure or union whose one non-empty member, through nested
        /// records and one-element arrays, is a scalar as large as it is,
        /// as Clang's `isSingleElementStruct` finds it: the WebAssembly C
        /// ABI passes it as that scalar. Lowering records it for the
        /// targets whose convention asks (`TargetLayout::record_passing`).
        const SINGLE_ELEMENT = 1 << 3;
        /// A top-level GNU vector. Its scalars are not its lanes but the
        /// one or two registers lowering chose for it under the claimed
        /// compiler's convention ([`AggregateLayout::scalars`]); none
        /// means memory.
        const VECTOR = 1 << 4;
        /// A member, a one-lane GNU vector, puts the aggregate in memory
        /// under the claimed compiler's System V classification.
        const MEMORY = 1 << 5;
    }
}

impl AggregateFlags {
    /// Each flag with its textual name, in printing order.
    pub(crate) const NAMES: [(Self, &'static str); 6] = [
        (Self::UNALIGNED, "unaligned"),
        (Self::FLEXIBLE, "flexible"),
        (Self::COMPLEX, "complex"),
        (Self::SINGLE_ELEMENT, "single_element"),
        (Self::VECTOR, "vector"),
        (Self::MEMORY, "memory"),
    ];
}

/// One scalar of an aggregate: its byte offset and IR type.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct AggregateScalar {
    pub(crate) offset: u32,
    pub(crate) ty:     Type,
}

/// The registers a memory-backed result comes back in: the first starts
/// at byte 0 and an optional second at its explicit offset (normally 8, or
/// 16 for complex x87). A lone `i128` represents the
/// Win64 scalar extension's `<2 x i64>` result in XMM0; `f128` carries a
/// System V SSE/SSEUP result in one vector register.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct ResultRegisters {
    pub(crate) first:         Type,
    pub(crate) second:        Option<Type>,
    /// Explicit return-area byte offset of the second register.
    pub(crate) second_offset: u32,
    /// The returned value is a complex scalar; meaningful bytes retain
    /// poison rather than aggregate register-packing freeze refinement.
    pub(crate) complex:       bool,
    /// The aggregate's alignment, which its return area has.
    pub(crate) align:         Align,
}

impl ResultRegisters {
    /// Ordinary paired results occupy two ABI eightbytes. Complex x87 uses
    /// offset 16 instead. Natural LLVM structure layout must agree.
    /// C99: §6.8.6.4p3, p. 139; PDF p. 151; the layout is an IR/ABI contract.
    pub(crate) const SECOND_OFFSET: u32 = 8;

    /// Whether the parts' natural layout agrees with the return area produced
    /// by ABI lowering, including the special complex x87 pair.
    pub(crate) fn valid_layout(self) -> bool {
        if self.first == Type::F80 && self.second == Some(Type::F80) {
            return self.second_offset == 16 && self.align.bytes() >= 16;
        }
        // A 32-byte vector result in two vector registers.
        if matches!(
            (self.first, self.second),
            (Type::F128, Some(Type::F128)) | (Type::I128, Some(Type::I128))
        ) {
            return self.second_offset == 16 && self.align.bytes() >= 16;
        }
        if self.second_offset != Self::SECOND_OFFSET {
            return false;
        }
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
            .chain(self.second.map(|ty| (u64::from(self.second_offset), ty)))
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
        self.bytes().max(2 * u64::from(self.second_offset))
    }
}

/// Whether a symbol is visible outside its module. A `Weak` definition
/// yields to a strong definition of the same name at link time, so its body
/// may not be the one that runs; a `Weak` declaration (no body, initializer
/// or alias target) may stay undefined. Its address is then null for an
/// ordinary object or function; unresolved ELF TLS uses offset zero from
/// the thread base instead.
/// GNU extension: GCC manual, "Common Function Attributes", weak.
/// <https://gcc.gnu.org/onlinedocs/gcc/Common-Function-Attributes.html>
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Linkage {
    External,
    Internal,
    Weak,
    /// A definition that other units may repeat with the same meaning, of
    /// which the linker keeps one: Microsoft `selectany` data, and inline
    /// functions that a Microsoft `extern` declaration or `dllexport` keeps.
    /// LLVM `weak_odr` in a COMDAT of the symbol's own name, as Clang emits
    /// them. Only a definition has it.
    SelectAny,
    /// Like [`Self::SelectAny`], but a unit that does not reference the
    /// definition may also drop it: the Microsoft external inline functions
    /// that a unit emits only because it reaches them. LLVM `linkonce_odr`
    /// in a COMDAT of the symbol's own name, as Clang emits them. Only a
    /// definition has it.
    LinkOnce,
    /// A zero-filled object the linker merges with the other units'
    /// definitions of its name, keeping an initialized one if there is one
    /// and else the largest: a tentative definition under `-fcommon`, an
    /// ELF `SHN_COMMON` or COFF communal symbol, LLVM `common`. Only a
    /// global definition without a section or alias has it.
    /// Extension: C99 §J.5.11, p. 514; PDF p. 526 (multiple external
    /// definitions).
    Common,
}

impl Linkage {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            | Self::External => "external",
            | Self::Internal => "internal",
            | Self::Weak => "weak",
            | Self::SelectAny => "selectany",
            | Self::LinkOnce => "linkonce",
            | Self::Common => "common",
        }
    }

    pub(crate) fn from_name(name: &str) -> Option<Self> {
        [
            Self::External,
            Self::Internal,
            Self::Weak,
            Self::SelectAny,
            Self::LinkOnce,
            Self::Common,
        ]
        .into_iter()
        .find(|linkage| linkage.name() == name)
    }

    /// Whether the definition sits in a COMDAT of its own name, so that the
    /// linker keeps one of the identical definitions other units repeat.
    pub(crate) const fn comdat(self) -> bool {
        matches!(self, Self::SelectAny | Self::LinkOnce)
    }
}

/// Link-time requests on a function or global besides its linkage.
/// GNU extension: GCC manual, "Common Function Attributes" and "Common
/// Variable Attributes": alias, section and used.
/// <https://gcc.gnu.org/onlinedocs/gcc/Common-Function-Attributes.html>
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct SymbolAttributes<'ir> {
    /// This symbol is another name for that definition of the same kind
    /// (LLVM's `alias`). An alias has no body or initializer of its own.
    pub(crate) alias:      Option<Symbol>,
    /// The object-file section that holds the definition.
    pub(crate) section:    Option<&'ir str>,
    /// The definition is kept even when nothing references it (LLVM's
    /// `llvm.used`).
    pub(crate) used:       bool,
    /// ELF symbol visibility; `Default` unless requested.
    pub(crate) visibility: Visibility,
    /// Windows DLL storage: a definition another image may import, or a
    /// declaration whose address this image loads from its import table.
    pub(crate) dll:        DllStorage,
}

/// How a COFF symbol crosses a DLL boundary. An `Export` definition is
/// listed in the image's export table (a `/EXPORT` linker directive); an
/// `Import` declaration is reached through the pointer `__imp_name` that the
/// loader fills in, as Clang emits `dllexport` and `dllimport`.
/// MSVC extension: Microsoft Learn, "dllexport, dllimport".
/// <https://learn.microsoft.com/en-us/cpp/cpp/dllexport-dllimport>
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub(crate) enum DllStorage {
    #[default]
    Default,
    Import,
    Export,
}

impl DllStorage {
    /// The textual form's keyword, absent for the default.
    pub(crate) const fn keyword(self) -> Option<&'static str> {
        match self {
            | Self::Default => None,
            | Self::Import => Some("dllimport"),
            | Self::Export => Some("dllexport"),
        }
    }
}

/// What a function's inline-control attributes request of the inliner.
/// `Never` is GNU `noinline` or `noipa`; `Always` is `always_inline`, which
/// the inliner honors at every direct call whose callee it can copy.
/// GNU extension: GCC manual, "Common Function Attributes", noinline, noipa,
/// `always_inline`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum InlineControl {
    #[default]
    Default,
    Never,
    Always,
}

impl InlineControl {
    /// The textual form's keyword, absent for the default.
    pub(crate) const fn keyword(self) -> Option<&'static str> {
        match self {
            | Self::Default => None,
            | Self::Never => Some("noinline"),
            | Self::Always => Some("alwaysinline"),
        }
    }
}

/// Whether every back end and the textual form can carry `name` as an
/// object-file section: it is nonempty and has no quote, newline or NUL.
/// Semantic analysis refuses other names; the verifier rejects them.
/// GNU extension: GCC manual, "Common Function Attributes", section.
pub(crate) fn valid_section_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['"', '\n', '\r', '\0'])
}

/// ELF symbol visibility. COFF ignores it.
/// GNU extension: GCC manual, "Common Function Attributes", visibility.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub(crate) enum Visibility {
    #[default]
    Default,
    Hidden,
    Protected,
}

impl Visibility {
    /// The textual form's keyword, absent for the default.
    pub(crate) const fn keyword(self) -> Option<&'static str> {
        match self {
            | Self::Default => None,
            | Self::Hidden => Some("hidden"),
            | Self::Protected => Some("protected"),
        }
    }
}

/// A function registered to run before or after the program's entry.
/// Constructors run in increasing priority and registration order;
/// destructors in decreasing priority and reverse registration order.
/// GNU extension: GCC manual, "Common Function Attributes",
/// constructor/destructor.
/// <https://gcc.gnu.org/onlinedocs/gcc/Common-Function-Attributes.html>
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct InitFunction {
    pub(crate) kind:     InitKind,
    pub(crate) priority: u16,
    pub(crate) func:     FuncId,
}

/// Whether an [`InitFunction`] runs before or after the entry function.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum InitKind {
    Constructor,
    Destructor,
}

impl InitKind {
    /// The textual form's keyword.
    pub(crate) const fn keyword(self) -> &'static str {
        match self {
            | Self::Constructor => "constructor",
            | Self::Destructor => "destructor",
        }
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
    pub(crate) name:         &'ir str,
    pub(crate) linkage:      Linkage,
    pub(crate) size:         u64,
    pub(crate) align:        Align,
    pub(crate) constant:     bool,
    /// Every thread has its own copy, initialized from `init` when the
    /// thread starts. `global_addr` yields the evaluating thread's copy, so
    /// the address is not a link-time constant and no relocation names it.
    /// C11: §6.2.4 paragraph 4, p. 38; PDF p. 56.
    pub(crate) thread_local: bool,
    pub(crate) init:         Option<GlobalInit<'ir>>,
    /// Alias target, section and retention requests.
    pub(crate) attributes:   SymbolAttributes<'ir>,
}

/// What [`Module::declare_global`] needs besides the name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct GlobalDecl {
    pub(crate) linkage:      Linkage,
    pub(crate) size:         u64,
    pub(crate) align:        Align,
    pub(crate) constant:     bool,
    pub(crate) thread_local: bool,
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

/// An address constant in a global's bytes: the [`Relocation::size`] bytes
/// at `offset` hold the value of `target` plus `addend`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Relocation {
    pub(crate) offset: u64,
    pub(crate) target: RelocationTarget,
    pub(crate) addend: i64,
}

impl Relocation {
    /// How many bytes of the global the relocation writes, where an address
    /// takes `pointer` bytes ([`Module::pointer_bytes`]).
    pub(crate) const fn size(&self, pointer: u64) -> u64 {
        match self.target {
            | RelocationTarget::LabelDifference { width, .. } => width.bytes() as u64,
            | RelocationTarget::Symbol(_) | RelocationTarget::Label { .. } => pointer,
        }
    }
}

/// What a relocation's bytes hold.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RelocationTarget {
    /// The address of a function or global: a pointer, eight bytes or four
    /// on WebAssembly.
    Symbol(Symbol),
    /// The address of the block `label` names in `func`: a pointer. A
    /// static table of label addresses for computed `goto`.
    /// GNU extension: GCC "Labels as Values".
    Label { func: FuncId, label: BlockLabel },
    /// The address of `label` minus the address of `base`, both in `func`,
    /// truncated to the integer type `width` (`i8` through `i64`). GCC and
    /// Clang accept such differences as constants in static initializers;
    /// the value is known only once code is laid out.
    /// GNU extension: GCC "Labels as Values".
    LabelDifference {
        func:  FuncId,
        label: BlockLabel,
        base:  BlockLabel,
        width: Type,
    },
}
