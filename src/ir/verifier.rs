//! Checks the invariants every pass and back end relies on.
//!
//! The verifier runs after lowering, after each pass in debug and test
//! builds, and on every parsed test input. It never panics on broken IR: it
//! records each problem as a [`VerifierError`] and keeps going where it
//! safely can. A function is checked in five steps: its block structure, the
//! records of where each value is defined, its operand-pool encodings (in
//! `pools.rs`), each instruction's types, flags and edges, and finally that
//! every use is dominated by its definition.

// Operand-pool validation before any unchecked body query.
mod pools;

use std::fmt;

use super::{
    Body,
    FuncId,
    GlobalId,
    Module,
    cfg::ControlFlowGraph,
    dominators::DominatorTree,
    entities::{
        Block,
        Entity,
        Inst,
        Value,
    },
    function::ValueDef,
    instructions::{
        BlockCall,
        InstData,
        InstFlags,
        Opcode,
    },
    module::{
        DllStorage,
        Extension,
        GlobalInit,
        Linkage,
        ParamAbi,
        RelocationTarget,
        Signature,
        Symbol,
        Visibility,
    },
    types::Type,
};
use crate::util::bump::{
    ArenaMap,
    ArenaSet,
    ArenaVec,
    Bump,
};

/// Verifies every global and function of `module`, returning the errors in
/// `scratch`. An empty vector means the module is well formed for `profile`.
pub(crate) fn verify_module<'m, 's>(
    module: &'m Module<'_>,
    profile: Profile,
    scratch: &'s Bump,
) -> ArenaVec<'s, VerifierError<'m>> {
    let mut errors = ArenaVec::new_in(scratch);
    for (global, _) in module.globals() {
        verify_global(module, global, &mut errors);
    }
    for (func, _) in module.functions() {
        verify_function_into(module, func, profile, scratch, &mut errors);
    }
    verify_init_functions(module, &mut errors);
    errors
}

/// An alias names an existing definition of its own kind that is not itself
/// an alias, and has no body or initializer of its own; a section name is
/// nonempty and free of quotes, newlines and NUL, so every back end and the
/// textual form can carry it; an internal symbol has default visibility, as
/// LLVM requires of local linkage. An alias is thread-local exactly when its
/// target is: it names the same storage, one copy per thread or one shared.
/// GNU extension: GCC manual, "Common Function Attributes", alias/section.
/// <https://gcc.gnu.org/onlinedocs/gcc/Common-Function-Attributes.html>
fn verify_symbol_attributes(
    module: &Module<'_>,
    symbol: Symbol,
    mut error: impl FnMut(VerifierErrorKind),
) {
    let attributes = module.symbol_attributes(symbol);
    let linkage = match symbol {
        | Symbol::Function(func) => module.function(func).linkage,
        | Symbol::Global(global) => module.global(global).linkage,
    };
    if linkage == Linkage::Internal && attributes.visibility != Visibility::Default {
        error(VerifierErrorKind::SymbolAttribute(
            "an internal symbol has default visibility",
        ));
    }
    let (defined, thread_local) = match symbol {
        | Symbol::Function(func) => (module.function(func).body.is_some(), false),
        | Symbol::Global(global) => {
            let global = module.global(global);
            (global.init.is_some(), global.thread_local)
        },
    };
    if linkage.comdat() && !defined {
        error(VerifierErrorKind::SymbolAttribute(
            "a selectany or linkonce symbol is a function or global definition",
        ));
    }
    // Microsoft Learn, "dllexport, dllimport": an image exports what it
    // defines and imports what another image defines; neither crosses the
    // boundary for thread-local data.
    match attributes.dll {
        | DllStorage::Import
            if defined || attributes.alias.is_some() || linkage == Linkage::Internal =>
            error(VerifierErrorKind::SymbolAttribute(
                "a dllimport symbol is an external declaration",
            )),
        | DllStorage::Export
            if !(defined || attributes.alias.is_some()) || linkage == Linkage::Internal =>
            error(VerifierErrorKind::SymbolAttribute(
                "a dllexport symbol is an external definition",
            )),
        | DllStorage::Import | DllStorage::Export if thread_local => error(
            VerifierErrorKind::SymbolAttribute("a thread-local symbol has no DLL storage"),
        ),
        | DllStorage::Default | DllStorage::Import | DllStorage::Export => {},
    }
    if let Some(section) = attributes.section
        && !super::valid_section_name(section)
    {
        error(VerifierErrorKind::SymbolAttribute(
            "a section name is nonempty and has no quote, newline or NUL",
        ));
    }
    let Some(target) = attributes.alias else {
        return;
    };
    let exists = match target {
        | Symbol::Function(func) => module.get_function(func).is_some(),
        | Symbol::Global(global) => module.get_global(global).is_some(),
    };
    if !exists {
        error(VerifierErrorKind::SymbolAttribute(
            "an alias target does not exist",
        ));
        return;
    }
    let defined = |symbol| match symbol {
        | Symbol::Function(func) => module.function(func).body.is_some(),
        | Symbol::Global(global) => module.global(global).init.is_some(),
    };
    if matches!(
        (symbol, target),
        (Symbol::Function(_), Symbol::Global(_)) | (Symbol::Global(_), Symbol::Function(_))
    ) {
        error(VerifierErrorKind::SymbolAttribute(
            "an alias and its target are both functions or both globals",
        ));
    }
    if defined(symbol) {
        error(VerifierErrorKind::SymbolAttribute(
            "an alias has no body or initializer of its own",
        ));
    }
    if !defined(target) || module.symbol_attributes(target).alias.is_some() {
        error(VerifierErrorKind::SymbolAttribute(
            "an alias target is a definition that is not itself an alias",
        ));
    }
    let thread_local = |symbol| match symbol {
        | Symbol::Function(_) => false,
        | Symbol::Global(global) => module.global(global).thread_local,
    };
    if thread_local(symbol) != thread_local(target) {
        error(VerifierErrorKind::SymbolAttribute(
            "an alias is thread-local exactly when its target is",
        ));
    }
}

/// Constructors and destructors are existing functions that take no
/// parameters and are defined in the module, directly or through an alias.
/// GNU extension: GCC manual, "Common Function Attributes",
/// constructor/destructor.
/// <https://gcc.gnu.org/onlinedocs/gcc/Common-Function-Attributes.html>
fn verify_init_functions<'m>(module: &'m Module<'_>, errors: &mut ArenaVec<'_, VerifierError<'m>>) {
    for init in module.init_functions() {
        let Some(function) = module.get_function(init.func) else {
            errors.push(VerifierError {
                item:     "",
                location: Location::Item,
                kind:     VerifierErrorKind::InvalidReference(
                    EntityKind::Function,
                    init.func.as_u32(),
                ),
            });
            continue;
        };
        let defined = function.body.is_some() || function.attributes.alias.is_some();
        let no_params = module
            .get_signature(function.signature)
            .is_some_and(|signature| signature.params.is_empty() && !signature.variadic);
        if !defined || !no_params {
            errors.push(VerifierError {
                item:     function.name,
                location: Location::Item,
                kind:     VerifierErrorKind::SymbolAttribute(
                    "a constructor or destructor is a defined function without parameters",
                ),
            });
        }
    }
}

/// Verifies one function, returning the errors in `scratch`.
pub(crate) fn verify_function<'m, 's>(
    module: &'m Module<'_>,
    func: FuncId,
    profile: Profile,
    scratch: &'s Bump,
) -> ArenaVec<'s, VerifierError<'m>> {
    let mut errors = ArenaVec::new_in(scratch);
    verify_function_into(module, func, profile, scratch, &mut errors);
    errors
}

fn verify_function_into<'m, 's>(
    module: &'m Module<'_>,
    func: FuncId,
    profile: Profile,
    scratch: &'s Bump,
    errors: &mut ArenaVec<'s, VerifierError<'m>>,
) {
    let function = module.function(func);
    if function.name.contains('\0') {
        errors.push(VerifierError {
            item:     function.name,
            location: Location::Item,
            kind:     VerifierErrorKind::NulSymbolName,
        });
    }
    verify_symbol_attributes(module, Symbol::Function(func), |kind| {
        errors.push(VerifierError {
            item: function.name,
            location: Location::Item,
            kind,
        });
    });
    let Some(signature) = module.get_signature(function.signature) else {
        errors.push(VerifierError {
            item:     function.name,
            location: Location::Item,
            kind:     VerifierErrorKind::InvalidReference(
                EntityKind::Signature,
                function.signature.as_u32(),
            ),
        });
        return;
    };
    verify_signature(signature, profile, |kind| {
        errors.push(VerifierError {
            item: function.name,
            location: Location::Item,
            kind,
        });
    });
    if signature.variadic_fixed.is_some() {
        errors.push(VerifierError {
            item:     function.name,
            location: Location::Item,
            kind:     VerifierErrorKind::VariadicDescription,
        });
    }
    let Some(body) = &function.body else {
        return;
    };
    let mut positions = ArenaVec::with_capacity_in(body.inst_count(), scratch);
    positions.resize(body.inst_count(), None);
    let mut verifier = Verifier {
        module,
        body,
        name: function.name,
        signature,
        profile,
        scratch,
        errors,
        positions,
    };
    if !verifier.structure() {
        return;
    }
    verifier.definitions();
    verifier.labels();
    if !verifier.pools() {
        return;
    }
    for block in body.blocks() {
        for &inst in body.block_insts(block) {
            verifier.instruction(inst);
        }
    }
    verifier.dominance();
}

/// Checks how a signature's parameters stand for aggregates and which are
/// extended, calling `report` for each one wrong: a description must name a
/// `ptr` parameter, in increasing order; a result must be parameter 0 of a
/// signature without a result; its kind must belong to the profile; and its
/// scalars must lie within the aggregate. An extension must name an `i8` or
/// `i16` parameter or result, in increasing order, with a kind of the
/// profile.
fn verify_signature(
    signature: Signature<'_>,
    profile: Profile,
    mut report: impl FnMut(VerifierErrorKind),
) {
    verify_extensions(signature, profile, &mut report);
    if signature
        .variadic_fixed
        .is_some_and(|n| !signature.variadic || n as usize >= signature.params.len())
    {
        report(VerifierErrorKind::VariadicDescription);
    }
    // A noundef check must not scan every aggregate for each marked index.
    // Preserve the existing diagnostics for malformed, unordered attributes;
    // verified signatures use one order check followed by binary searches.
    let aggregates_ordered = signature
        .aggregates
        .windows(2)
        .all(|pair| pair[0].index < pair[1].index);
    let param_abi = |index: u32| {
        if aggregates_ordered {
            signature
                .aggregates
                .binary_search_by_key(&index, |param| param.index)
                .ok()
                .map(|position| signature.aggregates[position].abi)
        } else {
            signature.param_abi(index as usize)
        }
    };
    let mut previous = None;
    for &index in signature.noundef {
        if previous.is_some_and(|previous| index <= previous)
            || index as usize >= signature.params.len()
            || matches!(param_abi(index), Some(ParamAbi::Registers(_)))
        {
            report(VerifierErrorKind::Noundef(
                "parameter indices must be ordered, unique and passed",
            ));
        }
        previous = Some(index);
    }
    if signature.variadic_noundef && !signature.variadic {
        report(VerifierErrorKind::Noundef(
            "variadic noundef requires a variadic signature",
        ));
    }
    if signature.result_noundef
        && signature.result.is_none()
        && !matches!(signature.result_param(), Some(ParamAbi::Registers(_)))
    {
        report(VerifierErrorKind::Noundef(
            "return noundef requires a returned value",
        ));
    }
    let mut previous = None;
    for param in signature.aggregates {
        let index = param.index;
        let problem = if previous.is_some_and(|previous| index <= previous) {
            Some(ParamAbiProblem::Order)
        } else if signature.params.get(index as usize) != Some(&Type::Ptr) {
            Some(ParamAbiProblem::NotPointer)
        } else if param.abi.is_pre_abi() != (profile == Profile::PreAbi) {
            Some(ParamAbiProblem::Profile)
        } else if param.abi.is_result() && (index != 0 || signature.result.is_some()) {
            Some(ParamAbiProblem::Result)
        } else if let Some(layout) = param.abi.layout()
            && layout
                .scalars
                .iter()
                .any(|scalar| u64::from(scalar.offset) + u64::from(scalar.ty.bytes()) > layout.size)
        {
            Some(ParamAbiProblem::ScalarOutside)
        } else if let Some(layout) = param.abi.layout()
            && let Some(first) = layout.first
            && let Some(problem) = first_member_problem(param.abi, layout, first)
        {
            Some(problem)
        } else if let Some(layout) = param.abi.layout()
            && layout.flags.contains(super::AggregateFlags::COMPLEX)
            && (layout.flags != super::AggregateFlags::COMPLEX
                || !matches!(layout.scalars, [a, b] if a.offset == 0 && a.ty == b.ty && a.ty.is_float() && u64::from(b.offset) * 2 == layout.size && u64::from(b.offset) == if a.ty == Type::F80 { 16 } else { u64::from(a.ty.bytes()) }))
        {
            Some(ParamAbiProblem::ComplexLayout)
        } else if let ParamAbi::Registers(registers) = param.abi
            && registers
                .types()
                .any(|ty| matches!(ty, Type::I1 | Type::Ptr))
        {
            Some(ParamAbiProblem::RegisterType)
        } else if let ParamAbi::Registers(registers) = param.abi
            && !registers.valid_layout()
        {
            Some(ParamAbiProblem::RegisterLayout)
        } else {
            None
        };
        if let Some(problem) = problem {
            report(VerifierErrorKind::ParamAbi { index, problem });
        }
        previous = Some(index);
    }
}

/// Checks the extensions of `signature`'s parameters and result for
/// [`verify_signature`].
fn verify_extensions(
    signature: Signature<'_>,
    profile: Profile,
    report: &mut impl FnMut(VerifierErrorKind),
) {
    let check = |ty: Option<Type>, extension: Extension| {
        if !matches!(ty, Some(Type::I8 | Type::I16)) {
            Some(ExtensionProblem::NotNarrowInteger)
        } else if extension.is_pre_abi() != (profile == Profile::PreAbi) {
            Some(ExtensionProblem::Profile)
        } else {
            None
        }
    };
    let mut previous = None;
    for param in signature.extensions {
        let index = param.index;
        let problem = if previous.is_some_and(|previous| index <= previous) {
            Some(ExtensionProblem::Order)
        } else {
            check(
                signature.params.get(index as usize).copied(),
                param.extension,
            )
        };
        if let Some(problem) = problem {
            report(VerifierErrorKind::Extension {
                param: Some(index),
                problem,
            });
        }
        previous = Some(index);
    }
    if let Some(extension) = signature.result_extension
        && let Some(problem) = check(signature.result, extension)
    {
        report(VerifierErrorKind::Extension {
            param: None,
            problem,
        });
    }
}

/// Which stage of lowering a module is in, and so which constraints apply.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Profile {
    /// Before ABI lowering: `va_arg` is an operation, and a `ptr`
    /// parameter may stand for a structure or union passed or returned by
    /// value ([`ParamAbi::Aggregate`], [`ParamAbi::Result`]).
    PreAbi,
    /// After ABI lowering: `va_arg` has been expanded for the target, and
    /// aggregate parameters say how the target passes them
    /// ([`ParamAbi::ByVal`], [`ParamAbi::Sret`], [`ParamAbi::Registers`]).
    /// In both profiles every value is a scalar or a pointer, since
    /// aggregates are never values.
    PostAbi,
}

impl Profile {
    /// Whether `opcode` may appear in this profile.
    pub(crate) const fn allows(self, opcode: Opcode) -> bool {
        !matches!(
            (self, opcode),
            (Self::PostAbi, Opcode::VaArg | Opcode::VaArgAggregate)
        )
    }
}

struct Verifier<'a, 'm, 'ir, 's> {
    module:    &'m Module<'ir>,
    body:      &'m Body<'ir>,
    name:      &'m str,
    signature: Signature<'ir>,
    profile:   Profile,
    scratch:   &'s Bump,
    errors:    &'a mut ArenaVec<'s, VerifierError<'m>>,
    /// Where each placed instruction is: its block and index there.
    positions: ArenaVec<'s, Option<(Block, usize)>>,
}

impl Verifier<'_, '_, '_, '_> {
    /// Checks that the body has an entry block matching the signature, that
    /// each block is a run of non-terminators ended by one terminator, and
    /// that each instruction appears once. Returns whether the later checks
    /// can rely on the block lists.
    fn structure(&mut self) -> bool {
        let Some(entry) = self.body.entry_block() else {
            self.error(Location::Item, VerifierErrorKind::NoBlocks);
            return false;
        };
        let params = self.body.block_params(entry);
        if params.len() == self.signature.params.len() {
            for (index, (&param, &expected)) in params.iter().zip(self.signature.params).enumerate()
            {
                if param.index() < self.body.value_count()
                    && self.body.value_type(param) != expected
                {
                    self.error(
                        Location::Block(entry),
                        VerifierErrorKind::EntryParameterType {
                            index,
                            expected,
                            found: self.body.value_type(param),
                        },
                    );
                }
            }
        } else {
            self.error(
                Location::Block(entry),
                VerifierErrorKind::EntryParameterCount {
                    expected: self.signature.params.len(),
                    found:    params.len(),
                },
            );
        }
        let mut sound = true;
        for block in self.body.blocks() {
            let insts = self.body.block_insts(block);
            if insts.is_empty() {
                self.error(Location::Block(block), VerifierErrorKind::EmptyBlock);
                continue;
            }
            for (index, &inst) in insts.iter().enumerate() {
                let Some(position) = self.positions.get_mut(inst.index()) else {
                    self.error(
                        Location::Block(block),
                        VerifierErrorKind::InvalidReference(EntityKind::Inst, inst.as_u32()),
                    );
                    sound = false;
                    continue;
                };
                if position.is_some() {
                    self.error(Location::Inst(inst), VerifierErrorKind::InstructionReused);
                    sound = false;
                    continue;
                }
                *position = Some((block, index));
                let terminator = self.body.inst(inst).is_terminator();
                if terminator && index + 1 != insts.len() {
                    self.error(Location::Inst(inst), VerifierErrorKind::TerminatorNotLast);
                } else if !terminator && index + 1 == insts.len() {
                    self.error(Location::Block(block), VerifierErrorKind::MissingTerminator);
                }
            }
        }
        sound
    }

    /// Checks that the value table and the definitions agree: a result's
    /// value names its instruction and a parameter's value its block and
    /// position, in both directions, with the type the instruction defines.
    fn definitions(&mut self) {
        let body = self.body;
        for index in 0..body.value_count() {
            let value = Value::new(index);
            let consistent = match body.value_def(value) {
                | ValueDef::Result(inst) =>
                    inst.index() < body.inst_count() && body.inst_result(inst) == Some(value),
                | ValueDef::Param(block, position) =>
                    block.index() < body.block_count()
                        && body.block_params(block).get(position as usize) == Some(&value),
            };
            if !consistent {
                self.error(Location::Item, VerifierErrorKind::ValueDefinition(value));
            }
        }
        for block in body.blocks() {
            for (position, &param) in body.block_params(block).iter().enumerate() {
                if param.index() >= body.value_count() {
                    self.error(
                        Location::Block(block),
                        VerifierErrorKind::InvalidReference(EntityKind::Value, param.as_u32()),
                    );
                } else if !matches!(body.value_def(param), ValueDef::Param(def_block, def_position)
                    if def_block == block && def_position as usize == position)
                {
                    self.error(
                        Location::Block(block),
                        VerifierErrorKind::ValueDefinition(param),
                    );
                }
            }
            for &inst in body.block_insts(block) {
                let result = body.inst_result(inst);
                if let Some(value) = result
                    && value.index() >= body.value_count()
                {
                    self.error(
                        Location::Inst(inst),
                        VerifierErrorKind::InvalidReference(EntityKind::Value, value.as_u32()),
                    );
                    continue;
                }
                if let Some(value) = result
                    && body.value_def(value) != ValueDef::Result(inst)
                {
                    self.error(
                        Location::Inst(inst),
                        VerifierErrorKind::ValueDefinition(value),
                    );
                }
                let expected = self.module.result_type(body.inst(inst));
                let found = result.map(|value| body.value_type(value));
                if expected != found {
                    self.error(
                        Location::Inst(inst),
                        VerifierErrorKind::ResultType { expected, found },
                    );
                }
            }
        }
    }

    /// Checks that each label names a distinct block other than the entry,
    /// which has no address (LLVM's `blockaddress` rule).
    fn labels(&mut self) {
        let body = self.body;
        let mut seen: ArenaSet<'_, Block> =
            ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, self.scratch);
        for (_, block) in body.labels() {
            if block.index() >= body.block_count() {
                self.error(
                    Location::Item,
                    VerifierErrorKind::InvalidReference(EntityKind::Block, block.as_u32()),
                );
            } else if Some(block) == body.entry_block() {
                self.error(Location::Block(block), VerifierErrorKind::LabeledEntry);
            } else if !seen.insert(block) {
                self.error(Location::Block(block), VerifierErrorKind::DuplicateLabel);
            }
        }
    }

    /// Checks one instruction's flags, profile, types and edges.
    fn instruction(&mut self, inst: Inst) {
        let data = *self.body.inst(inst);
        let opcode = data.opcode();
        let illegal = data.flags().difference(opcode.allowed_flags());
        if !illegal.is_empty() {
            self.error(
                Location::Inst(inst),
                VerifierErrorKind::IllegalFlags {
                    opcode,
                    flags: illegal,
                },
            );
        }
        if !self.profile.allows(opcode) {
            self.error(
                Location::Inst(inst),
                VerifierErrorKind::NotAllowedInProfile {
                    opcode,
                    profile: self.profile,
                },
            );
        }
        let mut checker = InstChecker {
            verifier: self,
            inst,
            opcode,
        };
        match data {
            | InstData::Binary { ty, args, .. } => {
                let operands = if opcode.is_int_binary() || opcode.is_overflow() {
                    checker.require(ty.is_int(), ty);
                    [ty, ty]
                } else if opcode.is_float_binary() {
                    checker.require(ty.is_float(), ty);
                    [ty, ty]
                } else if opcode == Opcode::PtrAdd {
                    checker.require(ty == Type::Ptr, ty);
                    [Type::Ptr, Type::I64]
                } else {
                    checker.require(opcode == Opcode::VaCopy, ty);
                    [Type::Ptr, Type::Ptr]
                };
                checker.operands(&args, &operands);
            },
            | InstData::DynAlloca { ty, size, .. } => {
                checker.require(ty == Type::I64, ty);
                checker.operand(0, size, Type::I64);
            },
            | InstData::VaArgAggregate { sig, list } => {
                checker.operand(0, list, Type::Ptr);
                if let Some(signature) = checker.verifier.module.get_signature(sig) {
                    verify_signature(signature, checker.verifier.profile, |kind| {
                        checker.report(kind);
                    });
                    checker.require(
                        signature.params == [Type::Ptr]
                            && !signature.variadic
                            && signature.result.is_none()
                            && matches!(signature.param_abi(0), Some(ParamAbi::Aggregate(_))),
                        Type::Ptr,
                    );
                } else {
                    checker.report(VerifierErrorKind::InvalidReference(
                        EntityKind::Signature,
                        sig.as_u32(),
                    ));
                }
            },
            | InstData::Unary { ty, arg, .. } => match opcode {
                | _ if opcode.is_bit_unary() => {
                    checker.require(
                        ty.is_int()
                            && (opcode != Opcode::Bswap
                                || matches!(ty, Type::I16 | Type::I32 | Type::I64 | Type::I128)),
                        ty,
                    );
                    checker.operand(0, arg, ty);
                },
                | Opcode::Fneg => {
                    checker.require(ty.is_float(), ty);
                    checker.operand(0, arg, ty);
                },
                | Opcode::Freeze => checker.operand(0, arg, ty),
                | Opcode::VaArg | Opcode::VaStart | Opcode::VaEnd | Opcode::StackRestore =>
                    checker.operand(0, arg, Type::Ptr),
                | _ if opcode.is_conversion() => {
                    if let Some(from) = checker.value_type(arg)
                        && !conversion_is_legal(opcode, from, ty)
                    {
                        checker.report(VerifierErrorKind::Conversion {
                            opcode,
                            from,
                            to: ty,
                        });
                    }
                },
                | _ => checker.require(false, ty),
            },
            | InstData::IntCompare { ty, args, .. } => {
                checker.require(ty.is_int() || ty == Type::Ptr, ty);
                checker.operands(&args, &[ty, ty]);
            },
            | InstData::FloatCompare { ty, args, .. } => {
                checker.require(ty.is_float(), ty);
                checker.operands(&args, &[ty, ty]);
            },
            | InstData::Select { ty, args } => checker.operands(&args, &[Type::I1, ty, ty]),
            | InstData::Const { ty, .. } | InstData::WideConst { ty, .. } => {
                checker.require(
                    match opcode {
                        | Opcode::Iconst => ty.is_int(),
                        | Opcode::Fconst => ty.is_float(),
                        | _ => false,
                    },
                    ty,
                );
                match checker.verifier.body.const_bits(&data) {
                    | None =>
                        if let InstData::WideConst { constant, .. } = data {
                            checker.report(VerifierErrorKind::InvalidReference(
                                EntityKind::Constant,
                                constant.as_u32(),
                            ));
                        },
                    | Some(bits) if bits & !ty.mask() != 0 =>
                        checker.report(VerifierErrorKind::ConstantOutOfRange(ty)),
                    | Some(_) => {},
                }
            },
            | InstData::Nullary { ty, .. } => checker.require(
                matches!(opcode, Opcode::Trap | Opcode::DebugTrap)
                    || (opcode == Opcode::ReadTimestamp && ty == Type::I64)
                    || (matches!(
                        opcode,
                        Opcode::StackSave | Opcode::FrameAddress | Opcode::ReturnAddress
                    ) && ty == Type::Ptr)
                    || opcode == Opcode::Poison
                    || (opcode == Opcode::Null && ty == Type::Ptr),
                ty,
            ),
            | InstData::StackAddr { slot } => {
                if slot.index() >= checker.verifier.body.stack_slot_count() {
                    checker.report(VerifierErrorKind::InvalidReference(
                        EntityKind::StackSlot,
                        slot.as_u32(),
                    ));
                }
            },
            | InstData::GlobalAddr { global } => {
                if checker.verifier.module.get_global(global).is_none() {
                    checker.report(VerifierErrorKind::InvalidReference(
                        EntityKind::Global,
                        global.as_u32(),
                    ));
                }
            },
            | InstData::FuncAddr { func } => {
                if checker.verifier.module.get_function(func).is_none() {
                    checker.report(VerifierErrorKind::InvalidReference(
                        EntityKind::Function,
                        func.as_u32(),
                    ));
                }
            },
            | InstData::BlockAddr { label } => {
                if checker.verifier.body.label_block(label).is_none() {
                    checker.report(VerifierErrorKind::InvalidReference(
                        EntityKind::Label,
                        label.as_u32(),
                    ));
                }
            },
            | InstData::Atomic {
                op,
                ty,
                order,
                failure,
                args,
                ..
            } => {
                use super::{
                    AtomicOp as A,
                    MemoryOrder as M,
                };
                let args = checker.verifier.body.value_list(args);
                let floating = matches!(op, A::Fadd | A::Fsub | A::Fmin | A::Fmax);
                checker.require(
                    if op.is_fence() || op.is_bytes() {
                        ty == Type::I8
                    } else if floating {
                        matches!(ty, Type::F32 | Type::F64)
                    } else {
                        ty.is_int() && ty != Type::I1
                    },
                    ty,
                );
                let valid_order = match op {
                    | A::Load | A::LoadBytes => !matches!(order, M::Release | M::AcqRel),
                    | A::Store | A::StoreBytes => !matches!(order, M::Acquire | M::AcqRel),
                    | A::CompareExchange | A::CompareExchangeBytes => order.valid_failure(failure),
                    | _ => true,
                } && (matches!(op, A::CompareExchange | A::CompareExchangeBytes)
                    || failure == M::Relaxed);
                if !valid_order || args.len() != op.arity() {
                    checker
                        .verifier
                        .error(Location::Inst(inst), VerifierErrorKind::AtomicContract);
                }
                for (index, &arg) in args.iter().enumerate() {
                    checker.operand(
                        index,
                        arg,
                        if op.is_bytes() {
                            if index + 1 == op.arity() {
                                Type::I64
                            } else {
                                Type::Ptr
                            }
                        } else if index == 0 {
                            Type::Ptr
                        } else {
                            ty
                        },
                    );
                }
            },
            | InstData::Load { addr, .. } => checker.operand(0, addr, Type::Ptr),
            | InstData::Store { ty, args, .. } => checker.operands(&args, &[ty, Type::Ptr]),
            | InstData::MemoryRange { args, .. } => match opcode {
                | Opcode::Copy => checker.operands(&args, &[Type::Ptr, Type::Ptr, Type::I64]),
                | Opcode::Fill => checker.operands(&args, &[Type::Ptr, Type::I8, Type::I64]),
                | _ => checker.require(false, Type::Ptr),
            },
            | InstData::Call { func, args } => {
                let module = checker.verifier.module;
                match module
                    .get_function(func)
                    .and_then(|function| module.get_signature(function.signature))
                {
                    | Some(signature) => {
                        let args = checker.verifier.body.value_list(args);
                        checker.call_arguments(signature, args);
                    },
                    | None => checker.report(VerifierErrorKind::InvalidReference(
                        EntityKind::Function,
                        func.as_u32(),
                    )),
                }
            },
            | InstData::CallIndirect { sig, args } =>
                match checker.verifier.module.get_signature(sig) {
                    | Some(signature) => {
                        verify_signature(signature, checker.verifier.profile, |kind| {
                            checker.report(kind);
                        });
                        let values = checker.verifier.body.value_list(args);
                        if let Some((&callee, args)) = values.split_first() {
                            checker.operand(0, callee, Type::Ptr);
                            checker.call_arguments(signature, args);
                        } else {
                            checker.report(VerifierErrorKind::MissingCallee);
                        }
                    },
                    | None => checker.report(VerifierErrorKind::InvalidReference(
                        EntityKind::Signature,
                        sig.as_u32(),
                    )),
                },
            | InstData::InlineAsm { asm, args } => checker.inline_asm(asm, args),
            | InstData::Jump { dest } => checker.edges(&[dest]),
            | InstData::Brif { cond, dests } => {
                checker.operand(0, cond, Type::I1);
                checker.edges(&dests);
            },
            | InstData::Switch {
                value,
                default,
                cases,
            } => checker.switch(value, default, cases),
            | InstData::IndirectBr { addr, dests } => checker.indirect_br(addr, dests),
            | InstData::Return { value } => {
                let expected = checker.verifier.signature.result;
                let found = value.and_then(|value| checker.value_type(value));
                if value.is_some() && found.is_none() {
                    return;
                }
                if expected != found {
                    checker.report(VerifierErrorKind::ReturnValue { expected, found });
                }
            },
            | InstData::Unreachable => {},
        }
    }

    /// Checks that every use in a reachable block is dominated by its
    /// definition: a parameter of a dominating block, or a result defined
    /// earlier in the same block or in a dominating block. A branch argument
    /// is used at its terminator. Uses in unreachable blocks are not checked.
    fn dominance(&mut self) {
        let body = self.body;
        let cfg = ControlFlowGraph::compute(body, self.scratch);
        let tree = DominatorTree::compute(body, &cfg, self.scratch);
        for &block in tree.reverse_post_order() {
            for (index, &inst) in body.block_insts(block).iter().enumerate() {
                let mut undominated = None;
                body.visit_operands(inst, |value| {
                    if undominated.is_some() || value.index() >= body.value_count() {
                        return;
                    }
                    let dominated = match body.value_def(value) {
                        | ValueDef::Param(def_block, _) => tree.dominates(def_block, block),
                        | ValueDef::Result(def_inst) =>
                            match self.positions.get(def_inst.index()).copied().flatten() {
                                | Some((def_block, def_index)) if def_block == block =>
                                    def_index < index,
                                | Some((def_block, _)) => tree.dominates(def_block, block),
                                | None => false,
                            },
                    };
                    if !dominated {
                        undominated = Some(value);
                    }
                });
                if let Some(value) = undominated {
                    self.error(Location::Inst(inst), VerifierErrorKind::NotDominated(value));
                }
            }
        }
    }

    fn error(&mut self, location: Location, kind: VerifierErrorKind) {
        self.errors.push(VerifierError {
            item: self.name,
            location,
            kind,
        });
    }
}

/// The checks of one instruction, which share its location.
struct InstChecker<'v, 'a, 'm, 'ir, 's> {
    verifier: &'v mut Verifier<'a, 'm, 'ir, 's>,
    inst:     Inst,
    opcode:   Opcode,
}

impl InstChecker<'_, '_, '_, '_, '_> {
    fn report(&mut self, kind: VerifierErrorKind) {
        self.verifier.error(Location::Inst(self.inst), kind);
    }

    /// Reports the controlling type unless `legal`.
    fn require(&mut self, legal: bool, ty: Type) {
        if !legal {
            let opcode = self.opcode;
            self.report(VerifierErrorKind::InstructionType { opcode, ty });
        }
    }

    /// The type of `value`, reporting it if it does not exist.
    fn value_type(&mut self, value: Value) -> Option<Type> {
        if value.index() < self.verifier.body.value_count() {
            Some(self.verifier.body.value_type(value))
        } else {
            self.report(VerifierErrorKind::InvalidReference(
                EntityKind::Value,
                value.as_u32(),
            ));
            None
        }
    }

    fn operand(&mut self, index: usize, value: Value, expected: Type) {
        if let Some(found) = self.value_type(value)
            && found != expected
        {
            self.report(VerifierErrorKind::OperandType {
                index,
                expected,
                found,
            });
        }
    }

    fn operands(&mut self, values: &[Value], expected: &[Type]) {
        for (index, (&value, &ty)) in values.iter().zip(expected).enumerate() {
            self.operand(index, value, ty);
        }
    }

    /// Checks an `asm` instruction against its descriptor: one value per
    /// operand, outputs first, each value of the type its location takes
    /// (a `ptr` for outputs and memory), immediates that are constants, tied
    /// inputs that match a register output, and a template whose operand
    /// references exist.
    /// GNU extension: GCC manual, "Extended Asm".
    fn inline_asm(&mut self, asm: super::AsmId, args: super::ValueList) {
        use super::AsmConstraint as C;
        let Some(descriptor) = self.verifier.module.get_inline_asm(asm) else {
            self.report(VerifierErrorKind::InvalidReference(
                EntityKind::InlineAsm,
                asm.as_u32(),
            ));
            return;
        };
        let values = self.verifier.body.value_list(args);
        if values.len() != descriptor.operands.len() {
            self.report(VerifierErrorKind::InlineAsm(
                "the operand count differs from the descriptor's",
            ));
            return;
        }
        let body = self.verifier.body;
        let constant = |value: Value| {
            value.index() < body.value_count()
                && matches!(body.value_def(value), ValueDef::Result(inst) if matches!(
                    body.inst(inst),
                    InstData::Const { opcode: Opcode::Iconst, .. }
                        | InstData::GlobalAddr { .. }
                        | InstData::FuncAddr { .. }
                ))
        };
        let outputs = descriptor.output_count();
        if descriptor.operands[outputs..]
            .iter()
            .any(|operand| operand.output)
        {
            self.report(VerifierErrorKind::InlineAsm("an output follows an input"));
        }
        for (index, (operand, &value)) in descriptor.operands.iter().zip(values).enumerate() {
            let ty = operand.ty;
            let expected = if operand.output || operand.constraint == C::Memory {
                Type::Ptr
            } else {
                ty
            };
            self.operand(index, value, expected);
            let rule = match operand.constraint {
                | C::General(mask) if mask == 0 || mask & (1 << 4) != 0 =>
                    Some("a general register set is empty or holds %rsp"),
                | C::General(_)
                    if !(ty == Type::Ptr || (ty.is_int() && ty != Type::I1 && ty.bits() <= 64)) =>
                    Some("a general register holds an integer or pointer of at most 64 bits"),
                | C::Sse if !matches!(ty, Type::F32 | Type::F64) =>
                    Some("an SSE register holds a f32 or f64"),
                | C::Memory if operand.bytes == 0 => Some("a memory operand has no bytes"),
                | C::Immediate if operand.output => Some("an immediate is an output"),
                | C::Immediate if !constant(value) =>
                    Some("an immediate is not an iconst, global_addr or func_addr"),
                | C::Tied(output) => match descriptor.operands.get(output as usize) {
                    | _ if operand.output => Some("a tied operand is an output"),
                    | Some(tied)
                        if (output as usize) < outputs
                            && matches!(tied.constraint, C::General(_) | C::Sse)
                            && tied.ty == ty =>
                        None,
                    | _ => Some("a tied input names no register output of its type"),
                },
                | C::Flag(condition)
                    if !operand.output
                        || !ty.is_int()
                        || usize::from(condition) >= super::ASM_CONDITIONS.len() =>
                    Some("a flag operand is not an integer output"),
                | _ if operand.early_clobber && !operand.output =>
                    Some("an input is early-clobber"),
                | _ => None,
            };
            if let Some(rule) = rule {
                self.report(VerifierErrorKind::InlineAsm(rule));
            }
        }
        for piece in super::inline_asm::template_pieces(descriptor.template) {
            let rule = match piece {
                | Err(()) => Some("the template has a malformed `$` escape"),
                | Ok(super::inline_asm::TemplatePiece::Operand { operand, .. })
                    if operand as usize >= values.len() =>
                    Some("the template names a missing operand"),
                | Ok(_) => None,
            };
            if let Some(rule) = rule {
                self.report(VerifierErrorKind::InlineAsm(rule));
                break;
            }
        }
    }

    /// Checks call arguments against a signature: the fixed ones by type,
    /// and any extra ones only if the signature is variadic.
    fn call_arguments(&mut self, signature: Signature<'_>, args: &[Value]) {
        let fixed = signature.params.len();
        if args.len() < fixed || (args.len() > fixed && !signature.variadic) {
            self.report(VerifierErrorKind::CallArgumentCount {
                expected: fixed,
                found:    args.len(),
                variadic: signature.variadic,
            });
            return;
        }
        for (index, (&arg, &expected)) in args.iter().zip(signature.params).enumerate() {
            if let Some(found) = self.value_type(arg)
                && found != expected
            {
                self.report(VerifierErrorKind::CallArgumentType {
                    index,
                    expected,
                    found,
                });
            }
        }
        for &arg in &args[fixed..] {
            _ = self.value_type(arg);
        }
    }

    /// Checks each edge's target and arguments, and that edges to one block
    /// pass identical arguments, so each maps to one LLVM phi entry.
    fn edges(&mut self, dests: &[BlockCall]) {
        let body = self.verifier.body;
        let mut seen: ArenaMap<'_, Block, BlockCall> =
            ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, self.verifier.scratch);
        for &dest in dests {
            let (target, args) = body.block_call(dest);
            if target.index() >= body.block_count() {
                self.report(VerifierErrorKind::InvalidReference(
                    EntityKind::Block,
                    target.as_u32(),
                ));
                continue;
            }
            if Some(target) == body.entry_block() {
                self.report(VerifierErrorKind::BranchToEntry);
            }
            let params = body.block_params(target);
            if params.len() == args.len() {
                for (index, (&arg, &param)) in args.iter().zip(params).enumerate() {
                    // Definitions already reported an invalid destination
                    // parameter; its type cannot inform this edge check.
                    if param.index() >= body.value_count() {
                        continue;
                    }
                    let expected = body.value_type(param);
                    if let Some(found) = self.value_type(arg)
                        && found != expected
                    {
                        self.report(VerifierErrorKind::EdgeArgumentType {
                            target,
                            index,
                            expected,
                            found,
                        });
                    }
                }
            } else {
                self.report(VerifierErrorKind::EdgeArgumentCount {
                    target,
                    expected: params.len(),
                    found: args.len(),
                });
            }
            match seen.get(&target) {
                | Some(&previous) if body.block_call(previous).1 != args =>
                    self.report(VerifierErrorKind::DuplicateEdgeArguments(target)),
                | Some(_) => {},
                | None => _ = seen.insert(target, dest),
            }
        }
    }

    /// Checks an indirect branch: a `ptr` address and distinct targets that
    /// are all address-taken blocks of this function, as LLVM's
    /// `indirectbr` requires of a `blockaddress` destination.
    fn indirect_br(&mut self, addr: Value, list: super::BlockList) {
        let body = self.verifier.body;
        self.operand(0, addr, Type::Ptr);
        let mut labeled: ArenaSet<'_, Block> =
            ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, self.verifier.scratch);
        labeled.extend(body.labels().map(|(_, block)| block));
        let mut seen: ArenaSet<'_, Block> =
            ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, self.verifier.scratch);
        let mut dests =
            ArenaVec::with_capacity_in(body.block_list(list).len(), self.verifier.scratch);
        for dest in body.block_list(list) {
            dests.push(dest);
            let target = body.block_call(dest).0;
            if target.index() >= body.block_count() {
                continue;
            }
            if !labeled.contains(&target) {
                self.report(VerifierErrorKind::IndirectTarget(target));
            }
            if !seen.insert(target) {
                self.report(VerifierErrorKind::DuplicateIndirectTarget(target));
            }
        }
        self.edges(&dests);
    }

    /// Checks a switch: an integer value, cases that fit its type and are
    /// distinct, and its edges.
    ///
    /// C99: §6.8.4.2 paragraph 3, p. 134; PDF p. 146 (distinct case values).
    fn switch(&mut self, value: Value, default: BlockCall, cases: super::CaseList) {
        let body = self.verifier.body;
        let ty = self.value_type(value);
        if let Some(ty) = ty
            && !ty.is_int()
        {
            self.report(VerifierErrorKind::OperandType {
                index:    0,
                expected: Type::I64,
                found:    ty,
            });
        }
        let mut dests =
            ArenaVec::with_capacity_in(1 + body.switch_cases(cases).len(), self.verifier.scratch);
        dests.push(default);
        let mut seen: ArenaSet<'_, u128> =
            ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, self.verifier.scratch);
        for (bits, dest) in body.switch_cases(cases) {
            dests.push(dest);
            if let Some(ty) = ty
                && bits & !ty.mask() != 0
            {
                self.report(VerifierErrorKind::ConstantOutOfRange(ty));
            }
            if !seen.insert(bits) {
                self.report(VerifierErrorKind::DuplicateSwitchCase(bits));
            }
        }
        self.edges(&dests);
    }
}

/// Whether a conversion from `from` to `to` is one `opcode` performs.
fn conversion_is_legal(opcode: Opcode, from: Type, to: Type) -> bool {
    match opcode {
        | Opcode::Zext | Opcode::Sext => from.is_int() && to.is_int() && from.bits() < to.bits(),
        | Opcode::Trunc => from.is_int() && to.is_int() && from.bits() > to.bits(),
        | Opcode::Fpext => from.is_float() && to.is_float() && from.bits() < to.bits(),
        | Opcode::Fptrunc => from.is_float() && to.is_float() && from.bits() > to.bits(),
        | Opcode::Fptosi | Opcode::Fptoui => from.is_float() && to.is_int(),
        | Opcode::Sitofp | Opcode::Uitofp => from.is_int() && to.is_float(),
        | Opcode::Ptrtoint => from == Type::Ptr && to.is_int(),
        | Opcode::Inttoptr => from.is_int() && to == Type::Ptr,
        | Opcode::Bitcast => from.bits() == to.bits() && (from == Type::Ptr) == (to == Type::Ptr),
        | _ => false,
    }
}

/// Checks a global's initializer against its size, and its relocations.
fn verify_global<'m>(
    module: &'m Module<'_>,
    global: GlobalId,
    errors: &mut ArenaVec<'_, VerifierError<'m>>,
) {
    let data = module.global(global);
    let mut error = |kind| {
        errors.push(VerifierError {
            item: data.name,
            location: Location::Item,
            kind,
        });
    };
    if data.name.contains('\0') {
        error(VerifierErrorKind::NulSymbolName);
    }
    verify_symbol_attributes(module, Symbol::Global(global), &mut error);
    let Some(GlobalInit::Bytes { bytes, relocations }) = data.init else {
        return;
    };
    if bytes.len() as u64 != data.size {
        error(VerifierErrorKind::InitializerSize {
            expected: data.size,
            found:    bytes.len() as u64,
        });
    }
    for relocation in relocations {
        if relocation
            .offset
            .checked_add(relocation.size())
            .is_none_or(|end| end > data.size)
        {
            error(VerifierErrorKind::RelocationRange(relocation.offset));
        }
        let label_exists = |func: FuncId, label: super::BlockLabel| {
            module
                .get_function(func)
                .and_then(|function| function.body.as_ref())
                .is_some_and(|body| body.label_block(label).is_some())
        };
        let (kind, index, exists) = match relocation.target {
            | RelocationTarget::Symbol(Symbol::Function(func)) => (
                EntityKind::Function,
                func.as_u32(),
                module.get_function(func).is_some(),
            ),
            | RelocationTarget::Symbol(Symbol::Global(target)) => (
                EntityKind::Global,
                target.as_u32(),
                module.get_global(target).is_some(),
            ),
            | RelocationTarget::Label { func, label } =>
                (EntityKind::Label, label.as_u32(), label_exists(func, label)),
            | RelocationTarget::LabelDifference {
                func,
                label,
                base,
                width,
            } => {
                if !matches!(width, Type::I8 | Type::I16 | Type::I32 | Type::I64) {
                    error(VerifierErrorKind::RelocationWidth(width));
                }
                let missing = if label_exists(func, label) {
                    base
                } else {
                    label
                };
                (
                    EntityKind::Label,
                    missing.as_u32(),
                    label_exists(func, label) && label_exists(func, base),
                )
            },
        };
        if !exists {
            error(VerifierErrorKind::InvalidReference(kind, index));
        } else if let RelocationTarget::Symbol(Symbol::Global(target)) = relocation.target
            && module.global(target).thread_local
        {
            error(VerifierErrorKind::ThreadLocalRelocation(relocation.offset));
        }
    }
}

/// One problem the verifier found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct VerifierError<'m> {
    /// The function or global it is in.
    pub(crate) item:     &'m str,
    pub(crate) location: Location,
    pub(crate) kind:     VerifierErrorKind,
}

/// Where in a function or global a problem is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Location {
    /// The function or global as a whole.
    Item,
    Block(Block),
    Inst(Inst),
}

/// What the verifier found wrong.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum VerifierErrorKind {
    AtomicContract,
    VariadicDescription,
    /// A defined-value contract describes a position that does not exist.
    Noundef(&'static str),
    /// LLVM and the native symbol interfaces require NUL-free names.
    NulSymbolName,
    /// An alias, section, constructor or destructor request breaks the rule
    /// named.
    SymbolAttribute(&'static str),
    /// A defined function has no blocks.
    NoBlocks,
    EmptyBlock,
    /// A block's last instruction is not a terminator.
    MissingTerminator,
    /// A terminator is followed by more instructions.
    TerminatorNotLast,
    /// An instruction appears more than once in the block lists.
    InstructionReused,
    /// An entity index is out of range.
    InvalidReference(EntityKind, u32),
    /// The value table and the instruction or block that defines a value
    /// disagree.
    ValueDefinition(Value),
    EntryParameterCount {
        expected: usize,
        found:    usize,
    },
    EntryParameterType {
        index:    usize,
        expected: Type,
        found:    Type,
    },
    /// The entry block has no predecessors; its parameters are the
    /// function's.
    BranchToEntry,
    /// The controlling type is not one the opcode accepts.
    InstructionType {
        opcode: Opcode,
        ty:     Type,
    },
    OperandType {
        index:    usize,
        expected: Type,
        found:    Type,
    },
    /// The instruction's result is missing, extra or of the wrong type.
    ResultType {
        expected: Option<Type>,
        found:    Option<Type>,
    },
    IllegalFlags {
        opcode: Opcode,
        flags:  InstFlags,
    },
    Conversion {
        opcode: Opcode,
        from:   Type,
        to:     Type,
    },
    /// Constant bits do not fit the type.
    ConstantOutOfRange(Type),
    EdgeArgumentCount {
        target:   Block,
        expected: usize,
        found:    usize,
    },
    EdgeArgumentType {
        target:   Block,
        index:    usize,
        expected: Type,
        found:    Type,
    },
    /// Two edges of one terminator reach a block with different arguments.
    DuplicateEdgeArguments(Block),
    DuplicateSwitchCase(u128),
    CallArgumentCount {
        expected: usize,
        found:    usize,
        variadic: bool,
    },
    CallArgumentType {
        index:    usize,
        expected: Type,
        found:    Type,
    },
    /// A `call_indirect` without a callee operand.
    MissingCallee,
    /// An `asm` instruction breaks the descriptor rule named.
    InlineAsm(&'static str),
    ReturnValue {
        expected: Option<Type>,
        found:    Option<Type>,
    },
    /// A use is not dominated by the value's definition.
    NotDominated(Value),
    NotAllowedInProfile {
        opcode:  Opcode,
        profile: Profile,
    },
    /// A signature's parameter `index` stands for an aggregate wrongly.
    ParamAbi {
        index:   u32,
        problem: ParamAbiProblem,
    },
    /// A signature's parameter `param`, or its result for `None`, has a
    /// wrong extension.
    Extension {
        param:   Option<u32>,
        problem: ExtensionProblem,
    },
    /// A global's bytes are not as long as the global.
    InitializerSize {
        expected: u64,
        found:    u64,
    },
    /// A relocation's bytes do not fit in the global.
    RelocationRange(u64),
    /// A label difference is not an `i8`, `i16`, `i32` or `i64`.
    RelocationWidth(Type),
    /// A label names the entry block, which has no address.
    LabeledEntry,
    /// Two labels name one block.
    DuplicateLabel,
    /// An `indirect_br` lists a block that no label names.
    IndirectTarget(Block),
    /// An `indirect_br` lists a block twice.
    DuplicateIndirectTarget(Block),
    /// A relocation names a thread-local global, whose address differs per
    /// thread and so is no link-time constant.
    /// C11: §6.6 paragraph 9, p. 107; PDF p. 125.
    ThreadLocalRelocation(u64),
}

/// What is wrong with an aggregate parameter of a signature.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ParamAbiProblem {
    /// Descriptions are not in increasing parameter order.
    Order,
    /// The parameter does not exist or is not a `ptr`.
    NotPointer,
    /// The kind belongs to the other profile.
    Profile,
    /// A result kind is not parameter 0, or the signature also has a result.
    Result,
    /// A scalar of the layout extends past the aggregate's size.
    ScalarOutside,
    /// A result register is `i1` or `ptr`.
    RegisterType,
    /// A complex scalar must have exactly two corresponding real components.
    ComplexLayout,
    /// A pair's natural layout does not match two ABI eightbytes.
    RegisterLayout,
    /// A first member is recorded for something other than a pre-ABI
    /// aggregate argument.
    FirstMisplaced,
    /// The first member is larger than the aggregate.
    FirstSize,
    /// A scalar extends past the first member it describes.
    FirstScalarOutside,
    /// The first member's extension does not describe a lone narrow
    /// integer or `_Bool` scalar that is the whole member.
    FirstExtension,
}

/// What is wrong with the first member a transparent union argument's
/// layout records (System V; see [`FirstMember`](super::FirstMember)): it
/// describes only a pre-ABI argument, fits the union and holds every scalar
/// recorded, and an extension describes one narrow integer or `_Bool`
/// scalar that is the whole member.
fn first_member_problem(
    abi: ParamAbi<'_>,
    layout: super::AggregateLayout<'_>,
    first: super::FirstMember,
) -> Option<ParamAbiProblem> {
    if !matches!(abi, ParamAbi::Aggregate(_)) {
        return Some(ParamAbiProblem::FirstMisplaced);
    }
    // An empty first member (GNU empty structure) has size 0.
    if first.size > layout.size {
        return Some(ParamAbiProblem::FirstSize);
    }
    if layout
        .scalars
        .iter()
        .any(|scalar| u64::from(scalar.offset) + u64::from(scalar.ty.bytes()) > first.size)
    {
        return Some(ParamAbiProblem::FirstScalarOutside);
    }
    let extension = first.extension?;
    let narrow = match (extension, layout.scalars) {
        | (
            Extension::Signed | Extension::Unsigned,
            &[super::AggregateScalar { offset: 0, ty }],
        ) => matches!(ty, Type::I8 | Type::I16),
        | (Extension::Bool, &[super::AggregateScalar { offset: 0, ty }]) => ty == Type::I8,
        | _ => false,
    };
    let whole = layout
        .scalars
        .first()
        .is_some_and(|scalar| u64::from(scalar.ty.bytes()) == first.size);
    (!narrow || !whole).then_some(ParamAbiProblem::FirstExtension)
}

/// What is wrong with an extended parameter or result of a signature.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ExtensionProblem {
    /// Extended parameters are not in increasing parameter order.
    Order,
    /// The parameter or result does not exist or is not `i8` or `i16`.
    NotNarrowInteger,
    /// The kind belongs to the other profile.
    Profile,
}

/// The kind of entity an [`VerifierErrorKind::InvalidReference`] names.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum EntityKind {
    Value,
    Block,
    Inst,
    StackSlot,
    Constant,
    Function,
    Global,
    Signature,
    ValueList,
    BlockCall,
    CaseList,
    BlockList,
    Label,
    InlineAsm,
}

impl fmt::Display for VerifierError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@{}", self.item)?;
        match self.location {
            | Location::Item => {},
            | Location::Block(block) => write!(f, ", {block}")?,
            | Location::Inst(inst) => write!(f, ", {inst}")?,
        }
        write!(f, ": {}", self.kind)
    }
}

impl fmt::Display for VerifierErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let optional = |ty: Option<Type>| ty.map_or("nothing", Type::name);
        match *self {
            | Self::VariadicDescription =>
                f.write_str("invalid trailing variadic argument descriptions"),
            | Self::Noundef(problem) => write!(f, "invalid noundef: {problem}"),
            | Self::NulSymbolName => f.write_str("a symbol name contains NUL"),
            | Self::AtomicContract =>
                f.write_str("invalid atomic operand count or memory ordering"),
            | Self::SymbolAttribute(rule) => write!(f, "invalid symbol attribute: {rule}"),
            | Self::NoBlocks => f.write_str("a defined function has no blocks"),
            | Self::EmptyBlock => f.write_str("the block is empty"),
            | Self::MissingTerminator => f.write_str("the block does not end in a terminator"),
            | Self::TerminatorNotLast => f.write_str("a terminator is followed by instructions"),
            | Self::InstructionReused => f.write_str("the instruction appears more than once"),
            | Self::InvalidReference(kind, index) => write!(f, "{kind} {index} does not exist"),
            | Self::ValueDefinition(value) =>
                write!(f, "{value} is not defined where the value table says"),
            | Self::EntryParameterCount { expected, found } => write!(
                f,
                "the entry block has {found} parameters but the signature has {expected}"
            ),
            | Self::EntryParameterType {
                index,
                expected,
                found,
            } => write!(
                f,
                "entry parameter {index} is {found}, but the signature says {expected}"
            ),
            | Self::BranchToEntry => f.write_str("a branch targets the entry block"),
            | Self::InstructionType { opcode, ty } => write!(f, "{opcode} cannot have type {ty}"),
            | Self::OperandType {
                index,
                expected,
                found,
            } => write!(f, "operand {index} is {found}, expected {expected}"),
            | Self::ResultType { expected, found } => write!(
                f,
                "the result is {}, expected {}",
                optional(found),
                optional(expected)
            ),
            | Self::IllegalFlags { opcode, flags } => write!(f, "{opcode} cannot carry `{flags}`"),
            | Self::Conversion { opcode, from, to } =>
                write!(f, "{opcode} cannot convert {from} to {to}"),
            | Self::ConstantOutOfRange(ty) => write!(f, "a constant does not fit in {ty}"),
            | Self::EdgeArgumentCount {
                target,
                expected,
                found,
            } => write!(
                f,
                "the edge to {target} passes {found} arguments for {expected} parameters"
            ),
            | Self::EdgeArgumentType {
                target,
                index,
                expected,
                found,
            } => write!(
                f,
                "argument {index} to {target} is {found}, expected {expected}"
            ),
            | Self::DuplicateEdgeArguments(target) =>
                write!(f, "edges to {target} pass different arguments"),
            | Self::DuplicateSwitchCase(bits) => write!(f, "the case {bits} appears twice"),
            | Self::CallArgumentCount {
                expected,
                found,
                variadic,
            } => write!(
                f,
                "the call passes {found} arguments for {}{expected} parameters",
                if variadic { "at least " } else { "" }
            ),
            | Self::CallArgumentType {
                index,
                expected,
                found,
            } => write!(f, "call argument {index} is {found}, expected {expected}"),
            | Self::MissingCallee => f.write_str("call_indirect has no callee"),
            | Self::InlineAsm(rule) => write!(f, "invalid inline assembly: {rule}"),
            | Self::ReturnValue { expected, found } => write!(
                f,
                "the function returns {}, but the return gives {}",
                optional(expected),
                optional(found)
            ),
            | Self::NotDominated(value) =>
                write!(f, "the use of {value} is not dominated by its definition"),
            | Self::NotAllowedInProfile { opcode, profile } =>
                write!(f, "{opcode} is not allowed in the {profile:?} profile"),
            | Self::ParamAbi { index, problem } => write!(
                f,
                "parameter {index} {}",
                match problem {
                    | ParamAbiProblem::Order => "is described out of order",
                    | ParamAbiProblem::NotPointer => "stands for an aggregate but is not a ptr",
                    | ParamAbiProblem::Profile => "has an aggregate kind of the other profile",
                    | ParamAbiProblem::Result =>
                        "returns an aggregate but is not parameter 0 of a signature without a \
                         result",
                    | ParamAbiProblem::ScalarOutside =>
                        "has an aggregate scalar beyond the aggregate's size",
                    | ParamAbiProblem::ComplexLayout => "has an invalid complex scalar layout",
                    | ParamAbiProblem::RegisterType => "returns a register of type i1 or ptr",
                    | ParamAbiProblem::RegisterLayout =>
                        "has a register pair incompatible with the eightbyte return area",
                    | ParamAbiProblem::FirstMisplaced =>
                        "records a first member outside a pre-ABI aggregate argument",
                    | ParamAbiProblem::FirstSize => "has a first member larger than its aggregate",
                    | ParamAbiProblem::FirstScalarOutside =>
                        "has an aggregate scalar beyond its first member's size",
                    | ParamAbiProblem::FirstExtension =>
                        "extends a first member that is not one narrow integer or _Bool scalar",
                }
            ),
            | Self::Extension { param, problem } => {
                match param {
                    | Some(index) => write!(f, "parameter {index} ")?,
                    | None => f.write_str("the result ")?,
                }
                f.write_str(match problem {
                    | ExtensionProblem::Order => "is extended out of order",
                    | ExtensionProblem::NotNarrowInteger =>
                        "has an extension but is not an i8 or i16",
                    | ExtensionProblem::Profile => "has an extension of the other profile",
                })
            },
            | Self::InitializerSize { expected, found } => write!(
                f,
                "the initializer has {found} bytes for a size of {expected}"
            ),
            | Self::RelocationRange(offset) => write!(
                f,
                "the relocation at offset {offset} does not fit in the global"
            ),
            | Self::RelocationWidth(ty) => write!(f, "a label difference cannot have type {ty}"),
            | Self::LabeledEntry => f.write_str("a label names the entry block"),
            | Self::DuplicateLabel => f.write_str("two labels name the block"),
            | Self::IndirectTarget(target) =>
                write!(f, "indirect_br lists {target}, which no label names"),
            | Self::DuplicateIndirectTarget(target) =>
                write!(f, "indirect_br lists {target} twice"),
            | Self::ThreadLocalRelocation(offset) => write!(
                f,
                "the relocation at offset {offset} names a thread-local global, whose address is \
                 not a link-time constant"
            ),
        }
    }
}

impl fmt::Display for EntityKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            | Self::Value => "value",
            | Self::Block => "block",
            | Self::Inst => "instruction",
            | Self::StackSlot => "stack slot",
            | Self::Constant => "constant",
            | Self::Function => "function",
            | Self::Global => "global",
            | Self::Signature => "signature",
            | Self::ValueList => "value list",
            | Self::BlockCall => "block call",
            | Self::CaseList => "case list",
            | Self::BlockList => "block list",
            | Self::Label => "label",
            | Self::InlineAsm => "inline assembly descriptor",
        })
    }
}
