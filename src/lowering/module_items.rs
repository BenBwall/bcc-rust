//! Module-level items: the functions, objects and string literals of a
//! translation unit as IR functions and globals, and the bytes of static
//! initializers.
//!
//! Functions and objects with linkage are named by their identifier, so
//! every declaration of one linked entity reaches the same symbol (C99
//! §6.2.2 paragraph 2, p. 30; PDF p. 42). Static locals and string literals
//! have no linkage and get internal names such as `f.count` and `.str.1`.
//! All user symbol names, including GNU assembler labels, are reserved
//! before generating those internal names.

mod file_asm;
mod reachability;

use super::{
    Construct,
    LoweringError,
    LoweringErrorKind,
    address_taken,
    floats,
    initializers::{
        self,
        Leaf,
        Storage,
    },
    types::{
        self,
        Repr,
        repr,
    },
    work::BitFieldBits,
};
use crate::{
    diagnostics::format_in,
    ir::{
        AggregateLayout,
        AggregateParam,
        Align,
        BlockLabel,
        Entity,
        ExtendedParam,
        FuncId,
        GlobalDecl,
        GlobalId,
        GlobalInit,
        InlineControl,
        Linkage as IrLinkage,
        Module,
        ParamAbi,
        Relocation,
        RelocationTarget,
        SigId,
        Signature,
        Symbol,
        Type,
    },
    target::Target,
    translation_phases::{
        Context,
        SourceVectors,
        parsing::{
            ParsedTranslationUnit,
            declaration_syntax::Initializer,
            syntax::{
                BinaryOperator,
                Expression,
                ExpressionType,
                UnaryOperator,
            },
        },
        preprocessing::{
            LiteralEncoding,
            LiteralId,
            LiteralUnit,
            StringTokenType,
        },
        semantic_analysis::{
            BindingKind,
            BodyEmission,
            ConstantClass,
            Definition,
            DefinitionKind,
            FunctionRecord,
            Inlining,
            Integer,
            LabelId,
            Linkage,
            ScopeKind,
            SemanticTranslationUnit,
            TypeId,
            TypeKind,
        },
    },
    util::{
        bump::{
            ArenaMap,
            ArenaSet,
            ArenaVec,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

/// Translation-unit state shared by every function's lowering.
pub(super) struct Unit<'a, 'tu, 'ir> {
    /// The module planner's emission closure, including static local objects.
    pub(super) reachable: ArenaSet<'a, usize>,
    /// Best-effort O0-only storage; required uses and `used` remain mandatory.
    pub(super) retain_local_statics: bool,
    /// Work spent associating retained parameter bindings (scaling regression).
    #[cfg(test)]
    pub(super) parameter_alias_visits: usize,
    pub(super) context: &'a Context<'tu>,
    pub(super) sema: &'a SemanticTranslationUnit<'tu>,
    pub(super) module: Module<'ir>,
    pub(super) call_diagnostics: ArenaVec<'ir, super::CallDiagnostic>,
    pub(super) tables: &'a Bump,
    pub(super) target: Target,
    /// Completed types of linked object definitions, including tentative ones,
    /// indexed by emitted symbol name after applying GNU assembler labels.
    /// C99: §6.2.7 paragraph 4, p. 40; PDF p. 52; §6.9.2 paragraph 2,
    /// p. 143; PDF p. 155.
    defined_objects: ArenaMap<'a, &'a str, usize>,
    /// The symbol names of the linked objects this unit defines only
    /// tentatively, which `-fcommon` may make common symbols.
    /// C99: §6.9.2 paragraph 2, p. 143; PDF p. 155.
    tentative_objects: ArenaSet<'a, &'a str>,
    /// Bindings whose address is taken, which therefore live in memory.
    pub(super) address_taken: ArenaSet<'a, usize>,
    /// Globals of static locals and `__func__`, which have no linkage.
    statics: ArenaMap<'a, usize, GlobalId>,
    /// Keyed by literal and [`string_form`]: narrow, `L`, `u8`, `u`, `U`.
    strings: ArenaMap<'a, (LiteralId, u8), GlobalId>,
    /// One static object per file-scope compound literal syntax node. New
    /// initializers join the explicit queue, never recursively lower syntax.
    compound_literals: ArenaMap<'a, usize, GlobalId>,
    /// Static objects to define: global, type, initializer and, for a
    /// flexible array member the initializer fills, the array type with
    /// room for every element it writes
    /// ([`FlexibleArray`](crate::translation_phases::semantic_analysis::FlexibleArray)).
    static_initializers: ArenaVec<'a, (GlobalId, TypeId, &'tu Initializer<'tu>, Option<TypeId>)>,
    /// Next candidate for a string literal's internal symbol.
    next_string_suffix: u64,
    /// User names reserved before generating internal storage.
    reserved_names: ArenaSet<'a, &'a str>,
    /// Internal storage cannot be reused by a linked object declaration.
    generated_globals: ArenaSet<'a, GlobalId>,
    /// Conflicting definitions are isolated while later functions are checked.
    conflicting_names: ArenaSet<'a, &'a str>,
    /// Each generated base probes a suffix at most once.
    next_name_suffix: ArenaMap<'a, &'a str, u64>,
    /// The private copies of `always_inline` GNU and C99 inline bodies,
    /// by the identity of the entity they stand in for at direct calls.
    /// The emitted symbol is not a key: a different entity may name it
    /// with an assembler label (glibc's `__REDIRECT`) and must still call
    /// the external definition.
    private_bodies: ArenaMap<'a, StringCacheId, FuncId>,
    /// The plain inline-only definitions lowered as unreferenced copies
    /// that the LLVM back end emits as `available_externally` bodies
    /// ([`Unit::lower_plain_inline_bodies`]).
    plain_inline: ArenaSet<'a, usize>,
    /// Symbols whose entity names a visibility, which `-fvisibility=`
    /// leaves alone.
    pub(super) explicit_visibility: ArenaSet<'a, Symbol>,
    /// The definitions whose bodies use `__builtin_va_arg_pack ()` or its
    /// length, by entity identity, as for `private_bodies`.
    pub(super) pack_functions: ArenaMap<'a, StringCacheId, &'tu FunctionRecord<'tu>>,
    /// Their copies specialized for calls, in declaration order
    /// (`argument_pack.rs`), each copy's index by function, and the indexes
    /// of the copies a lowered body calls that are still to be lowered.
    pub(super) pack_copies: ArenaVec<'a, super::argument_pack::PackCopy<'a, 'tu>>,
    pub(super) pack_copy_index: ArenaMap<'a, FuncId, usize>,
    /// The declarations of internal functions using the pack whose address
    /// is taken, with their definitions, whose bodies
    /// `argument_pack::lower_live_copies` lowers if the address is live.
    pub(super) pack_addresses: ArenaMap<'a, FuncId, &'tu FunctionRecord<'tu>>,
}

impl<'a, 'tu, 'ir> Unit<'a, 'tu, 'ir> {
    /// Declares reachable function definitions and defines reachable objects.
    /// Returns reachable function definitions in order, each with its function
    /// if it is to be lowered: inline definitions and unsupported
    /// signatures have none.
    /// C99: §6.2.7 paragraph 4, p. 40; PDF p. 52; §6.9.2 paragraph 2,
    /// p. 143; PDF p. 155.
    pub(super) fn declare_items(
        &mut self,
        syntax: &ParsedTranslationUnit<'tu>,
        errors: &mut ArenaVec<'_, LoweringError>,
    ) -> ArenaVec<'a, (&'tu FunctionRecord<'tu>, Option<FuncId>)> {
        self.reachable = self.reachable_items();
        self.check_extensions(errors);
        self.file_scope_asm(syntax, errors);
        // Reserve every user spelling before any initializer or body creates
        // internal storage (GNU labels extend C99 §6.2.2p2).
        for (index, binding) in self.sema.bindings.iter().enumerate() {
            if !binding.needs_entity_policy() {
                continue;
            }
            let name = if binding.linkage == Linkage::None {
                self.symbol_request(index).map(|r| r.name)
            } else {
                Some(self.symbol_name(index))
            };
            if let Some(name) = name {
                _ = self.reserved_names.insert(name);
                if binding.origin.is_intrinsic()
                    && self.symbol_request(index).is_none()
                    && let Some(runtime) = binding.origin.library()
                {
                    _ = self.reserved_names.insert(runtime);
                }
            }
        }
        // Reserve an alias with its definition's completed layout even when
        // the alias's C identifier differs (GNU extension to C99 §6.2.2p2).
        for definition in self.sema.definitions {
            let binding = self.sema.bindings[definition.binding];
            if matches!(
                definition.kind,
                DefinitionKind::Object | DefinitionKind::Tentative
            ) && binding.linkage != Linkage::None
            {
                if definition.kind == DefinitionKind::Tentative {
                    _ = self
                        .tentative_objects
                        .insert(self.symbol_name(definition.binding));
                }
                let previous = self
                    .defined_objects
                    .insert(self.symbol_name(definition.binding), definition.binding);
                if let Some(previous) = previous
                    && self.sema.bindings[previous].name.name != binding.name.name
                {
                    errors.push(LoweringError::unsupported(
                        Construct::AssemblerLabel,
                        binding.name.source_vectors,
                    ));
                    // Refuse these declarations without suppressing later
                    // function diagnostics (C99 §5.1.1.3p1).
                    _ = self
                        .conflicting_names
                        .insert(self.symbol_name(definition.binding));
                }
            }
        }
        let mut functions = ArenaVec::with_capacity_in(self.sema.functions.len(), self.tables);
        let mut bodies = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, self.tables);
        for record in self.declaration_order() {
            if record
                .binding
                .is_some_and(|binding| !self.reachable.contains(&binding))
            {
                continue;
            }
            let mut func = self.declare_definition(record).unwrap_or_else(|error| {
                errors.push(error);
                None
            });
            if let Some(id) = func
                && !bodies.insert(id)
            {
                errors.push(LoweringError::unsupported(
                    Construct::AssemblerLabel,
                    record.syntax.source_vectors,
                ));
                func = None;
            }
            functions.push((record, func));
        }
        for definition in self.sema.definitions {
            let binding = self.sema.bindings[definition.binding];
            if !matches!(
                definition.kind,
                DefinitionKind::Object | DefinitionKind::Tentative
            ) || binding.linkage == Linkage::None
                || !self.reachable.contains(&definition.binding)
            {
                continue;
            }
            if self
                .conflicting_names
                .contains(self.symbol_name(definition.binding))
            {
                continue;
            }
            let initializer = self
                .sema
                .entity(definition.binding)
                .expect(super::ENTITY_FACTS)
                .initializer;
            if definition.kind == DefinitionKind::Object && initializer.is_none() {
                errors.push(LoweringError::missing(
                    "object definition initializer is not retained",
                    binding.name.source_vectors,
                ));
                continue;
            }
            if let Err(error) = self.define_object(definition.binding, None, initializer) {
                errors.push(error);
            }
        }
        // A retained local can be inside an unevaluated operand. Define its
        // storage independently of the executable walk (C99 §6.2.4p3).
        let mut owners = ArenaVec::new_in(self.tables);
        owners.resize(self.sema.scopes.len(), None);
        for function in self.sema.functions {
            owners[function.scope] = function.binding;
        }
        for (index, scope) in self.sema.scopes.iter().enumerate() {
            if scope.kind != ScopeKind::Function {
                owners[index] = scope.parent.and_then(|parent| owners[parent]);
            }
        }
        for definition in self.sema.definitions {
            let binding = self.sema.bindings[definition.binding];
            if !matches!(
                definition.kind,
                DefinitionKind::Object | DefinitionKind::Tentative
            ) || binding.linkage != Linkage::None
                || !binding.duration.is_static_or_thread()
                || !self.reachable.contains(&definition.binding)
            {
                continue;
            }
            let Some(owner) = owners[binding.scope] else {
                continue;
            };
            let name = self.symbol_name(owner);
            let initializer = self
                .sema
                .entity(definition.binding)
                .expect(super::ENTITY_FACTS)
                .initializer;
            if let Err(error) = self.define_object(definition.binding, Some(name), initializer) {
                errors.push(error);
            }
        }
        self.declare_aliases_and_initializers(errors);
        functions
    }

    /// The function definitions in source order, except that the
    /// `always_inline` bodies among them take their slots in the order Clang
    /// creates their LLVM functions ([`Self::clang_creation_order`]), those
    /// it never creates last. The
    /// module's function order is LLVM's, so LLVM's always-inliner, which
    /// visits bodies in it, and the resolution of inline copies for the
    /// other back ends (`optimizer/always_inline.rs`) both follow it, and
    /// both choose as Clang does where expanding one recursive body into
    /// another decides which calls stay calls.
    fn declaration_order(&self) -> ArenaVec<'a, &'tu FunctionRecord<'tu>> {
        let mut records: ArenaVec<'a, &'tu FunctionRecord<'tu>> =
            ArenaVec::with_capacity_in(self.sema.functions.len(), self.tables);
        records.extend(self.sema.functions.iter());
        let always = |binding: usize| {
            self.sema
                .entity_attributes(binding)
                .is_some_and(|attributes| attributes.inlining == Inlining::Always)
                && self.sema.function_policy(binding).is_some_and(|policy| {
                    matches!(
                        policy.body,
                        BodyEmission::PrivateInline | BodyEmission::Definition
                    )
                })
        };
        let mut slots = ArenaVec::new_in(self.tables);
        for (index, record) in records.iter().enumerate() {
            if record
                .binding
                .is_some_and(|binding| self.reachable.contains(&binding) && always(binding))
            {
                slots.push(index);
            }
        }
        if slots.len() < 2 {
            return records;
        }
        let ranks = self.clang_creation_order();
        let mut sorted: ArenaVec<'_, &'tu FunctionRecord<'tu>> =
            ArenaVec::with_capacity_in(slots.len(), self.tables);
        sorted.extend(slots.iter().map(|&index| records[index]));
        // Bodies Clang never creates, referenced only from code it skips,
        // come after the others, so their copies expand nothing first.
        sorted.sort_by_key(|record| {
            let rank = record.binding.and_then(|binding| ranks[binding]);
            (rank.is_none(), rank)
        });
        for (&index, record) in slots.iter().zip(sorted) {
            records[index] = record;
        }
        records
    }

    /// Whether this unit defines the linked object `binding`, tentatively
    /// included. C99: §6.9.2p2, p. 143; PDF p. 155.
    pub(super) fn defines_object(&self, binding: usize) -> bool {
        self.defined_objects.contains_key(self.symbol_name(binding))
    }

    /// Whether this unit defines the linked object `binding` only
    /// tentatively. C99: §6.9.2p2, p. 143; PDF p. 155.
    pub(super) fn defines_tentatively(&self, binding: usize) -> bool {
        self.tentative_objects.contains(self.symbol_name(binding))
    }

    /// Projects the module plan's body decision for declaration modifier gates.
    /// C99: §6.7.4p6-7, pp. 112-113; PDF pp. 124-125; §6.9p3-5,
    /// p. 140; PDF p. 152.
    pub(super) fn emits_function(&self, binding: usize) -> bool {
        self.reachable.contains(&binding)
            && (self
                .sema
                .function_policy(binding)
                .is_some_and(|policy| policy.body != BodyEmission::InlineOnly)
                || self.plain_inline.contains(&binding))
    }

    /// Whether the inline-only definition `binding` is lowered as a private
    /// body: it is `always_inline` and has no inline assembly. Emission,
    /// reachability and the declaration gates share this one decision, so
    /// a private body's own references and refusals count as emitted ones.
    /// C99: §6.7.4p6, pp. 112-113; PDF pp. 124-125. GNU extension: GCC
    /// manual, "Common Function Attributes", `always_inline`.
    pub(super) fn private_inline(&self, binding: usize) -> bool {
        self.sema
            .function_policy(binding)
            .is_some_and(|policy| policy.body == BodyEmission::PrivateInline)
            || self.plain_inline.contains(&binding)
    }

    /// Whether `binding` is a plain inline-only definition lowered as an
    /// optional copy ([`Self::lower_plain_inline_bodies`]).
    pub(super) fn optional_inline_body(&self, binding: usize) -> bool {
        self.plain_inline.contains(&binding)
    }

    /// Lowers every plain inline-only definition without inline assembly,
    /// attribute refusals or `noinline` as well, as a copy that nothing calls:
    /// the LLVM back end emits it as the external function's
    /// `available_externally` body, as Clang does when it optimizes, so
    /// LLVM can expand calls where Clang does; the other back ends drop it.
    /// Returns whether there is any such definition. These bodies were not
    /// lowered before, and their constructs must not refuse the unit: one
    /// that does not lower stays bodiless (`lowering::lower_unit`).
    /// C99: §6.7.4p6, pp. 112-113; PDF pp. 124-125.
    pub(super) fn lower_plain_inline_bodies(&mut self) -> bool {
        for record in self.sema.functions {
            let Some(binding) = record.binding else {
                continue;
            };
            let refused = self
                .sema
                .symbols
                .refusals
                .iter()
                .any(|refusal| refusal.owner == Some(binding))
                || self
                    .sema
                    .entities
                    .refusals
                    .iter()
                    .any(|refusal| refusal.owner == Some(binding));
            if !record.assembly
                && !refused
                && self
                    .sema
                    .function_policy(binding)
                    .is_some_and(|policy| policy.body == BodyEmission::InlineOnly)
                && self
                    .sema
                    .entity_attributes(binding)
                    .is_none_or(|attributes| attributes.inlining != Inlining::Never)
            {
                _ = self.plain_inline.insert(binding);
            }
        }
        !self.plain_inline.is_empty()
    }

    /// Declares a function definition's symbol; `None` when its body is
    /// not lowered for a C99 inline-only or GNU extern-inline definition.
    /// The semantic pass decides emission once for the interpreter and LLVM.
    /// C99: §6.7.4p6-7, pp. 112-113; PDF pp. 124-125; §6.9p5,
    /// p. 140; PDF p. 152. GNU extension: GCC manual, "Inline":
    /// <https://gcc.gnu.org/onlinedocs/gcc/Inline.html>.
    fn declare_definition(
        &mut self,
        record: &FunctionRecord<'tu>,
    ) -> Result<Option<FuncId>, LoweringError> {
        let source = record.syntax.source_vectors;
        if self
            .sema
            .correspondence
            .declaration(record.syntax.declarator)
            .is_none()
        {
            return Err(LoweringError::missing(
                "function definition occurrence is not retained",
                source,
            ));
        }
        let Some(binding) = record.binding else {
            return Err(LoweringError::missing(
                "a function definition has no binding",
                source,
            ));
        };
        let scope = self.sema.bindings[binding].scope;
        if self.sema.scopes[scope].kind != ScopeKind::File {
            return Err(LoweringError::unsupported(
                Construct::NestedFunction,
                source,
            ));
        }
        // GNU extension: GCC manual, "Constructing Calls". Each direct call
        // gets its own copy of a body that uses the argument pack. A `used`
        // one is declared now: it is emitted whatever calls it, so the walk
        // over live code (`argument_pack::lower_live_copies`) lowers its own
        // body, as for a live address.
        if self.copies_only(record) {
            if self.sema.bindings[binding].linkage == Linkage::Internal
                && self
                    .sema
                    .entity_attributes(binding)
                    .is_some_and(|attributes| attributes.used)
            {
                _ = self.function(binding).map_err(|kind| at(kind, source))?;
            }
            return Ok(None);
        }
        if self.sema.function_policy(binding).is_some_and(|policy| {
            matches!(
                policy.body,
                BodyEmission::InlineOnly | BodyEmission::PrivateInline
            )
        }) {
            return self.private_body(binding, record);
        }
        self.function(binding)
            .map(Some)
            .map_err(|kind| at(kind, source))
    }

    /// An inline definition supplies no external definition, but an
    /// `always_inline` one must still reach its direct calls, as GCC and
    /// Clang expand it there (MinGW-w64's `FORCEINLINE` `intsafe.h`
    /// functions, which no library defines). Its body becomes an internal
    /// function that direct calls name and the inliner expands; its address
    /// stays the external symbol's. A body with inline assembly keeps
    /// calling the external definition instead, as MinGW-w64's
    /// `intrin-impl.h` expects: there the library's definition is the
    /// assembly's own.
    /// C99: §6.7.4p6, pp. 112-113; PDF pp. 124-125 (a call to an inline
    /// function may use the inline or the external definition).
    /// GNU extension: GCC manual, "Common Function Attributes",
    /// `always_inline`, and "Inline".
    fn private_body(
        &mut self,
        binding: usize,
        record: &FunctionRecord<'tu>,
    ) -> Result<Option<FuncId>, LoweringError> {
        if !self.private_inline(binding) {
            return Ok(None);
        }
        if self.plain_inline.contains(&binding) {
            return self.plain_inline_body(binding, record);
        }
        let source = record.syntax.source_vectors;
        let external = self.symbol_name(binding);
        let identity = self.binding_identity(binding);
        if let Some(&func) = self.private_bodies.get(&identity) {
            return Ok(Some(func));
        }
        let signature = self
            .signature_for_binding(binding)
            .map_err(|kind| at(kind, source))?;
        let base = format_in!(self.tables, "{external}.always_inline");
        let name = self.unique_name(base);
        let func = self
            .module
            .declare_function(name, signature, IrLinkage::Internal);
        self.module.function_mut(func).inlining = InlineControl::Always;
        // Name the external function the copy stands in for, declaring it
        // when nothing else has, so the LLVM back end can emit Clang's
        // `available_externally` form. A symbol clash keeps the plain
        // private copy, as does a wrapper of its own symbol (glibc's
        // fortify wrappers before 2.40): as that body, its call to the
        // symbol would call itself.
        if let Ok(external) = self.function(binding) {
            let name = self.module.function(external).name;
            if !self.calls_own_symbol(binding, name) {
                self.module.set_inline_copy_of(func, name);
            }
        }
        // Private copies keep the entity's attribute-only return contract.
        // C99: §7.13.2.1p4, p. 244; PDF p. 256 (longjmp resumes setjmp).
        self.module.function_mut(func).returns_twice = self
            .sema
            .entity_attributes(binding)
            .is_some_and(|attributes| attributes.returns_twice);
        _ = self.private_bodies.insert(identity, func);
        Ok(Some(func))
    }

    /// The unreferenced copy of the plain inline-only definition `binding`
    /// ([`Self::lower_plain_inline_bodies`]), when its external function is
    /// an ordinary declaration with the same signature, which the LLVM back
    /// end can give the body. Otherwise the body is not lowered, as before.
    /// C99: §6.7.4p6, pp. 112-113; PDF pp. 124-125.
    fn plain_inline_body(
        &mut self,
        binding: usize,
        record: &FunctionRecord<'tu>,
    ) -> Result<Option<FuncId>, LoweringError> {
        let source = record.syntax.source_vectors;
        let Ok(external) = self.function(binding) else {
            return Ok(None);
        };
        let signature = self
            .signature_for_binding(binding)
            .map_err(|kind| at(kind, source))?;
        let declaration = self.module.function(external);
        if declaration.signature != signature
            || declaration.linkage != IrLinkage::External
            || declaration.body.is_some()
            || self.calls_own_symbol(binding, declaration.name)
        {
            return Ok(None);
        }
        let name = declaration.name;
        let base = format_in!(self.tables, "{name}.inline");
        let copy = self.unique_name(base);
        let func = self
            .module
            .declare_function(copy, signature, IrLinkage::Internal);
        self.module.set_inline_copy_of(func, name);
        self.module.function_mut(func).returns_twice = self
            .sema
            .entity_attributes(binding)
            .is_some_and(|attributes| attributes.returns_twice);
        Ok(Some(func))
    }

    /// Whether the inline definition `binding` refers to its own symbol
    /// through another entity, an assembler label (glibc's `__REDIRECT`)
    /// or a builtin's library name: such a body is no copy of the external
    /// definition but a wrapper of it, and Clang does not emit it as
    /// `available_externally` (its `isTriviallyRecursive`).
    fn calls_own_symbol(&self, binding: usize, name: &str) -> bool {
        let identity = self.binding_identity(binding);
        self.sema.references.iter().any(|reference| {
            let data = self.sema.bindings[reference.binding];
            reference.owner == binding
                && data.kind == BindingKind::Function
                && (data.origin.library() == Some(name)
                    || (data.needs_entity_policy()
                        && data.linkage != Linkage::None
                        && self.symbol_name(reference.binding) == name
                        && self.binding_identity(reference.binding) != identity))
        })
    }

    /// The function a direct call to `binding` runs: the private copy of an
    /// `always_inline` inline body when there is one, else the function
    /// the binding names. Only bindings of the inline definition's entity
    /// use the private copy; another entity labelled with the same symbol
    /// calls the external definition.
    /// C99: §6.7.4p6, pp. 112-113; PDF pp. 124-125.
    pub(super) fn direct_callee(&mut self, binding: usize) -> Result<FuncId, LoweringErrorKind> {
        if self.sema.bindings[binding].linkage == Linkage::External
            && let Some(&func) = self.private_bodies.get(&self.binding_identity(binding))
        {
            return Ok(func);
        }
        self.function(binding)
    }

    /// Whether every direct call of the function `binding` passes its
    /// pointer arguments through `subobject` hints: it is `always_inline`,
    /// or uses the GNU argument pack, whose calls call `always_inline`
    /// copies. Expanded into its callers, its object-size queries can then
    /// find the subobject each argument names.
    /// GNU extension: GCC manual, "Object Size Checking".
    pub(super) fn hints_subobjects(&self, binding: usize) -> bool {
        self.pack_function(binding).is_some()
            || self
                .sema
                .entity_attributes(binding)
                .is_some_and(|attributes| attributes.inlining == Inlining::Always)
    }

    /// The function a function binding names, declaring it on first use
    /// with the signature of the binding's type.
    pub(super) fn function(&mut self, binding: usize) -> Result<FuncId, LoweringErrorKind> {
        let data = self.sema.bindings[binding];
        if data.linkage == Linkage::Internal && !self.reachable.contains(&binding) {
            return Err(LoweringErrorKind::MissingFact(
                "lowering referenced an omitted internal function",
            ));
        }
        let name = self.symbol_name(binding);
        // Builtin aliases describe the C binding, never the exact assembler
        // label chosen by the user (GNU extension to C99 §6.2.2p2).
        let runtime = self
            .symbol_request(binding)
            .is_none()
            .then(|| data.origin.library())
            .flatten();
        let name = runtime.unwrap_or(name);
        if let Some(Symbol::Function(func)) = self.module.symbol(name) {
            // An internal C function can use a library spelling without
            // being that library function. Do not redirect a builtin alias
            // to that unrelated definition (C99 §7.1.3p1; GNU extension).
            if runtime.is_some() && self.module.function(func).linkage == IrLinkage::Internal {
                return Err(LoweringErrorKind::Unsupported(Construct::Builtin));
            }
            return Ok(func);
        }
        if self.module.symbol(name).is_some() {
            return Err(LoweringErrorKind::Unsupported(Construct::AssemblerLabel));
        }
        let signature = if self.unprototyped_external(binding) {
            // C99 §6.5.2.2p6: a declaration without a prototype accepts
            // default-promoted arguments. LLVM represents this open external
            // interface with a variadic declaration, not a zero-argument one.
            let TypeKind::Function { result, .. } = self.sema.types.kind(data.ty) else {
                return Err(LoweringErrorKind::MissingFact(
                    "an external binding has no function type",
                ));
            };
            let signature = self.signature_of([], result, true, self.tables)?;
            self.module.intern(signature)
        } else {
            self.signature_for_binding(binding)?
        };
        let func = self
            .module
            .declare_function(name, signature, self.ir_linkage(binding));
        self.symbol_attributes(binding, Symbol::Function(func));
        // Only calls reach the copies of an internal function using the
        // argument pack; its address designates its own body, lowered only if
        // live code takes it (`argument_pack::lower_live_copies`).
        if data.linkage == Linkage::Internal
            && let Some(record) = self.pack_function(binding)
            && self.copies_only(record)
        {
            _ = self.pack_addresses.insert(func, record);
        }
        Ok(func)
    }

    /// An external declaration without a prototype or a known definition.
    /// Known old-style definitions keep their actual parameter signature.
    /// C99: §6.5.2.2 paragraph 6, pp. 71-72; PDF pp. 83-84.
    pub(super) fn unprototyped_external(&self, binding: usize) -> bool {
        self.sema
            .unprototyped_external(binding)
            .expect(super::ENTITY_FACTS)
    }

    /// The IR signature of a C function type: one parameter per declared
    /// parameter, after the adjustments of §6.7.5.3 paragraphs 7-8, p. 119;
    /// PDF p. 131. An unprototyped type has no fixed parameters.
    fn signature_for_binding(&mut self, binding: usize) -> Result<SigId, LoweringErrorKind> {
        let ty = self.sema.bindings[binding].ty;
        if self.msvc_setjmp(binding)
            && let TypeKind::Function {
                result, parameters, ..
            } = self.sema.types.kind(ty)
        {
            // The frame argument is a pointer, as the buffer is.
            let parameters = [parameters[0], parameters[0]];
            let signature = self.signature_of(parameters, result, false, self.tables)?;
            return Ok(self.module.intern(signature));
        }
        let Some(extension) = self.sema.function_extensions.function(binding) else {
            return self.signature(ty);
        };
        let TypeKind::Function {
            result,
            parameters,
            variadic,
            ..
        } = self.sema.types.kind(ty)
        else {
            return self.signature(ty);
        };
        let mut expanded = ArenaVec::new_in(self.tables);
        for (index, &parameter) in parameters.iter().enumerate() {
            expanded.push(parameter);
            if extension
                .sizes
                .iter()
                .any(|size| size.index as usize == index)
            {
                expanded.push(extension.size_type);
            }
        }
        let signature =
            self.signature_of(expanded.iter().copied(), result, variadic, self.tables)?;
        Ok(self.module.intern(signature))
    }

    /// Whether `binding` is the Microsoft C runtime's x64 `_setjmp` or
    /// `_setjmpex`, which `<setjmp.h>` declares with the buffer alone but
    /// which takes in its second argument register the frame its `longjmp`
    /// unwinds to, 0 for none. `cl` treats them as intrinsics that pass the
    /// caller's frame, and Clang passes `llvm.frameaddress(0)`, so a call
    /// gets that argument too ([`Self::signature_for_binding`]); without it
    /// `longjmp` unwinds to whatever the register held.
    /// MSVC extension: Microsoft Learn, "setjmp" (x64 unwinding);
    /// C99: §7.13.1.1, p. 243; PDF p. 255.
    pub(super) fn msvc_setjmp(&self, binding: usize) -> bool {
        self.target == Target::WindowsMsvc
            && matches!(self.symbol_name(binding), "_setjmp" | "_setjmpex")
            && matches!(
                self.sema.types.kind(self.sema.bindings[binding].ty),
                TypeKind::Function {
                    parameters: [_],
                    prototype: true,
                    variadic: false,
                    ..
                }
            )
    }

    pub(super) fn signature(&mut self, ty: TypeId) -> Result<SigId, LoweringErrorKind> {
        let TypeKind::Function {
            result,
            parameters,
            variadic,
            ..
        } = self.sema.types.kind(ty)
        else {
            return Err(LoweringErrorKind::MissingFact(
                "a function has no function type",
            ));
        };
        let signature =
            self.signature_of(parameters.iter().copied(), result, variadic, self.tables)?;
        Ok(self.module.intern(signature))
    }

    /// The calling-convention facts of a vector parameter or result
    /// ([`super::vectors::abi_layout`]) under the claimed compiler's rules.
    fn vector_abi_layout<'s>(
        &self,
        ty: TypeId,
        result: bool,
        scratch: &'s Bump,
    ) -> Result<AggregateLayout<'s>, LoweringErrorKind> {
        super::vectors::abi_layout(
            &self.sema.types,
            ty,
            crate::abi::Convention::of(self.target),
            self.context.configuration.compiler_identity().rules(),
            result,
            scratch,
        )
    }

    /// The pre-ABI signature of a call or function whose fixed parameters
    /// have the C types `parameters`. A structure or union parameter is a
    /// `ptr` to a copy the caller makes, described by
    /// [`ParamAbi::Aggregate`]; a structure or union result is returned
    /// through a `ptr` to the receiving object inserted as parameter 0 and
    /// described by [`ParamAbi::Result`], and the signature has no result.
    /// A System V transparent union parameter also records its first
    /// member, which takes the registers ([`types::argument_layout`]).
    /// A parameter or result of an integer type narrower than `int` records
    /// its signedness, or that it is `_Bool`
    /// ([`Extension`](crate::ir::Extension)). The lists are allocated in
    /// `scratch`.
    /// C99: §6.5.2.2 paragraph 4, p. 71; PDF p. 83 (arguments are passed
    /// by value); §6.9.1 paragraph 10, p. 142; PDF p. 154 (each parameter is
    /// an object of the callee).
    pub(super) fn signature_of<'s>(
        &self,
        parameters: impl IntoIterator<Item = TypeId>,
        result: TypeId,
        variadic: bool,
        scratch: &'s Bump,
    ) -> Result<Signature<'s>, LoweringErrorKind> {
        let types = &self.sema.types;
        let mut params = ArenaVec::new_in(scratch);
        let mut aggregates = ArenaVec::new_in(scratch);
        let mut extensions = ArenaVec::new_in(scratch);
        let mut noundef = ArenaVec::new_in(scratch);
        let result_representation = repr(types, result)?;
        let result = match result_representation {
            | Repr::Void => None,
            | representation
                if matches!(representation, Repr::Aggregate | Repr::Complex { .. })
                    || types::limb_argument(types, result) =>
            {
                params.push(Type::Ptr);
                let layout = if super::vectors::shape(types, result).is_some() {
                    self.vector_abi_layout(result, true, scratch)?
                } else {
                    types::object_layout(types, result, scratch)?
                };
                aggregates.push(AggregateParam {
                    index: 0,
                    abi:   ParamAbi::Result(layout),
                });
                None
            },
            | representation => representation.value_type(),
        };
        for parameter in parameters {
            match repr(types, parameter)? {
                | representation
                    if matches!(representation, Repr::Aggregate | Repr::Complex { .. })
                        || types::limb_argument(types, parameter) =>
                {
                    let index = u32::try_from(params.len())
                        .map_err(|_| LoweringErrorKind::MissingFact("too many parameters"))?;
                    // Ordinary aggregates constrain only the caller-owned
                    // copy's pointer; complex scalar descriptors also cover
                    // their meaningful component bytes.
                    noundef.push(index);
                    params.push(Type::Ptr);
                    let layout = if super::vectors::shape(types, parameter).is_some() {
                        self.vector_abi_layout(parameter, false, scratch)?
                    } else {
                        types::argument_layout(types, self.target, parameter, scratch)?
                    };
                    aggregates.push(AggregateParam {
                        index,
                        abi: ParamAbi::Aggregate(layout),
                    });
                },
                | representation => {
                    // Clang's scalar-argument model: evaluating an
                    // indeterminate scalar argument is UB. C99 §6.2.6.1p5
                    // covers trap reads; §6.3.2.1p2 is lvalue conversion.
                    // The broader assumption is documented in middle-end.md.
                    noundef.push(
                        u32::try_from(params.len())
                            .map_err(|_| LoweringErrorKind::MissingFact("too many parameters"))?,
                    );
                    if let Some(extension) = representation.extension() {
                        let index = u32::try_from(params.len())
                            .map_err(|_| LoweringErrorKind::MissingFact("too many parameters"))?;
                        extensions.push(ExtendedParam { index, extension });
                    }
                    params.push(representation.value_type().ok_or(
                        LoweringErrorKind::MissingFact("a parameter has no value type"),
                    )?);
                },
            }
        }
        Ok(Signature {
            params: scratch.alloc_slice_copy(&params),
            result,
            variadic,
            variadic_fixed: None,
            aggregates: scratch.alloc_slice_copy(&aggregates),
            extensions: scratch.alloc_slice_copy(&extensions),
            result_extension: result_representation.extension(),
            noundef: scratch.alloc_slice_copy(&noundef),
            // Bundled Clang emits no C return noundef at -O0.
            result_noundef: false,
            variadic_noundef: variadic,
        })
    }

    /// The global holding a static-duration object, declaring it on first
    /// use. An object with linkage is found by name; `__func__` is defined
    /// when first used.
    pub(super) fn object(&mut self, binding: usize) -> Result<GlobalId, LoweringErrorKind> {
        if let Some(&global) = self.statics.get(&binding) {
            return Ok(global);
        }
        let data = self.sema.bindings[binding];
        if data.linkage == Linkage::Internal && !self.reachable.contains(&binding) {
            return Err(LoweringErrorKind::MissingFact(
                "lowering referenced an omitted internal object",
            ));
        }
        if data.linkage == Linkage::None {
            let (stem, function, text) = match self
                .sema
                .entity(binding)
                .and_then(|entity| entity.definition)
                .map(|definition| definition.kind)
            {
                | Some(DefinitionKind::FunctionName(text)) => ("__func__", Some(text), text),
                | Some(DefinitionKind::PrettyFunctionName { function, text }) =>
                    ("__PRETTY_FUNCTION__", function, text),
                | _ =>
                    return Err(LoweringErrorKind::MissingFact(
                        "a static local is used before its declaration",
                    )),
            };
            return Ok(self.function_name(binding, stem, function, text));
        }
        let name = self.symbol_name(binding);
        if self.conflicting_names.contains(name) {
            return Err(LoweringErrorKind::Unsupported(Construct::AssemblerLabel));
        }
        match self.module.symbol(name) {
            | Some(Symbol::Global(global)) => {
                if self.generated_globals.contains(&global) {
                    return Err(LoweringErrorKind::Unsupported(Construct::AssemblerLabel));
                }
                Ok(global)
            },
            | Some(Symbol::Function(_)) =>
                Err(LoweringErrorKind::Unsupported(Construct::AssemblerLabel)),
            | None => {
                // C99 §6.2.7p4, §6.9.2p2: an initializer can refer to an
                // earlier incomplete declaration. Reserve the same symbol
                // using the definition's completed type from the outset.
                // The definition's binding names the same entity, so its
                // linkage and symbol attributes are the entity's.
                let binding = self.defined_objects.get(name).copied().unwrap_or(binding);
                let declaration = self.global_declaration(binding, self.ir_linkage(binding))?;
                let global = self.module.declare_global(name, declaration);
                self.symbol_attributes(binding, Symbol::Global(global));
                Ok(global)
            },
        }
    }

    /// Defines a static-duration object: a file-scope object with linkage
    /// when `local` is `None`, else a static local of the function named
    /// `local`. A missing initializer means zero.
    /// C99: §6.7.8 paragraph 10, p. 126; PDF p. 138.
    pub(super) fn define_object(
        &mut self,
        binding: usize,
        local: Option<&str>,
        initializer: Option<&'tu Initializer<'tu>>,
    ) -> Result<GlobalId, LoweringError> {
        if let Some(&global) = self.statics.get(&binding) {
            return Ok(global);
        }
        let data = self.sema.bindings[binding];
        let source = data.name.source_vectors;
        let global = match local {
            | None => self.object(binding).map_err(|kind| at(kind, source))?,
            | Some(function) => {
                let base = format_in!(
                    self.tables,
                    "{function}.{}",
                    self.context.string_cache.at(data.name.name)
                );
                let name = if let Some(name) = self.symbol_request(binding).map(|r| r.name) {
                    if self.module.symbol(name).is_some() || self.conflicting_names.contains(name) {
                        return Err(LoweringError::unsupported(
                            Construct::AssemblerLabel,
                            source,
                        ));
                    }
                    name
                } else {
                    self.unique_name(base)
                };
                let declaration = self
                    .global_declaration(binding, IrLinkage::Internal)
                    .map_err(|kind| at(kind, source))?;
                let global = self.module.declare_global(name, declaration);
                self.symbol_attributes(binding, Symbol::Global(global));
                if self.symbol_request(binding).is_none() {
                    _ = self.generated_globals.insert(global);
                }
                _ = self.statics.insert(binding, global);
                global
            },
        };
        match initializer {
            | None => self.module.define_global(global, GlobalInit::Zero),
            | Some(initializer) => {
                self.static_initializers.push((
                    global,
                    data.ty,
                    initializer,
                    self.sema.flexible_array(binding).map(|array| array.written),
                ));
                self.define_static_initializers()?;
            },
        }
        Ok(global)
    }

    /// A global's size, alignment and constancy from the object's type. An
    /// incomplete array declared but not defined here has size zero; an
    /// object whose initializer gives its flexible array member elements
    /// extends past its structure (GNU; semantic analysis owns the size).
    fn global_declaration(
        &self,
        binding: usize,
        linkage: IrLinkage,
    ) -> Result<GlobalDecl, LoweringErrorKind> {
        let data = self.sema.bindings[binding];
        let types = &self.sema.types;
        if types.unanalyzed(data.ty) {
            return Err(LoweringError::unanalyzed_kind(types, data.ty));
        }
        let align = super::alignment::variable_alignment(self.sema, self.target, binding, data.ty);
        let (size, align) = match types.layout(data.ty) {
            | Some(layout) => (layout.size, align?),
            | None => {
                let element =
                    types::target_type(types, data.ty).and_then(|element| types.layout(element));
                match element {
                    | Some(_) => (0, align?),
                    // An object declared with a structure, union, enumeration
                    // or void type that this translation unit never
                    // completes is only referenced, never defined, so its
                    // address is all lowering needs.
                    // C99 §6.2.5p22, p. 36; PDF p. 48.
                    | None if !matches!(types.kind(data.ty), TypeKind::Array(..)) =>
                        (0, Align::BYTE),
                    | None =>
                        return Err(types::layout(types, data.ty).err().unwrap_or(
                            LoweringErrorKind::MissingFact("an object type has no layout"),
                        )),
                }
            },
        };
        Ok(GlobalDecl {
            linkage,
            size: self.sema.flexible_object_size(binding).unwrap_or(size),
            align,
            // A common symbol may take another unit's definition, as
            // Clang's is never constant.
            constant: types.readonly_object(data.ty) && linkage != IrLinkage::Common,
            thread_local: data.duration
                == crate::translation_phases::semantic_analysis::Duration::Thread,
        })
    }

    /// The array behind `__func__`, `__FUNCTION__` or `__PRETTY_FUNCTION__`:
    /// the string and a null character, named for `stem` and its function.
    /// C99: §6.4.2.2 paragraph 1, p. 52; PDF p. 64.
    fn function_name(
        &mut self,
        binding: usize,
        stem: &str,
        function: Option<StringCacheId>,
        text: StringCacheId,
    ) -> GlobalId {
        let function = function
            .map(|function| self.context.string_cache.at(function))
            .filter(|function| !function.is_empty());
        let text = self.context.string_cache.at(text);
        let symbol = match function {
            | Some(function) => format_in!(self.tables, "{function}.{stem}"),
            | None => format_in!(self.tables, "{stem}"),
        };
        let symbol = self.unique_name(symbol);
        let global = self.module.declare_global(
            symbol,
            GlobalDecl {
                linkage:      IrLinkage::Internal,
                size:         text.len() as u64 + 1,
                align:        Align::BYTE,
                constant:     true,
                thread_local: false,
            },
        );
        let mut bytes = ArenaVec::with_capacity_in(text.len() + 1, self.tables);
        bytes.extend_from_slice(text.as_bytes());
        bytes.push(0);
        self.module.define_global(
            global,
            GlobalInit::Bytes {
                bytes:       &bytes,
                relocations: &[],
            },
        );
        _ = self.statics.insert(binding, global);
        _ = self.generated_globals.insert(global);
        global
    }

    /// The internal constant global holding a string literal's
    /// array, shared by identical literals as C99 permits (§6.4.5
    /// paragraph 6, p. 63; PDF p. 75).
    pub(super) fn string(
        &mut self,
        expression: &'tu Expression<'tu>,
    ) -> Result<GlobalId, LoweringError> {
        let ExpressionType::StringLiteral(literal) = expression.kind else {
            return Err(LoweringError::missing(
                "a string literal has no literal",
                expression.source_vectors,
            ));
        };
        let key = string_form(literal);
        if let Some(&global) = self.strings.get(&key) {
            return Ok(global);
        }
        let (bytes, align) = self.literal_bytes(expression)?;
        let name = self.string_name();
        let global = self.module.declare_global(
            name,
            GlobalDecl {
                linkage: IrLinkage::Internal,
                size: bytes.len() as u64,
                align,
                constant: true,
                thread_local: false,
            },
        );
        self.module.define_global(
            global,
            GlobalInit::Bytes {
                bytes:       &bytes,
                relocations: &[],
            },
        );
        _ = self.generated_globals.insert(global);
        _ = self.strings.insert(key, global);
        Ok(global)
    }

    /// A fresh internal constant global named after `stem`, holding `bytes`
    /// at an alignment of `align` bytes.
    pub(super) fn internal_constant(
        &mut self,
        stem: &'a str,
        bytes: &[u8],
        align: u64,
    ) -> GlobalId {
        let name = self.unique_name(stem);
        let global = self.module.declare_global(
            name,
            GlobalDecl {
                linkage:      IrLinkage::Internal,
                size:         bytes.len() as u64,
                align:        Align::from_bytes(align).unwrap_or(Align::BYTE),
                constant:     true,
                thread_local: false,
            },
        );
        self.module.define_global(
            global,
            GlobalInit::Bytes {
                bytes,
                relocations: &[],
            },
        );
        _ = self.generated_globals.insert(global);
        global
    }

    /// Encodes a literal and its terminator in the target's character units.
    /// C99: §6.4.5p5, pp. 62-63; PDF pp. 74-75;
    /// §6.7.8p14-15, p. 126; PDF p. 138.
    fn literal_bytes(
        &self,
        expression: &'tu Expression<'tu>,
    ) -> Result<(ArenaVec<'a, u8>, Align), LoweringError> {
        let info = self.sema.expression_info(peel(expression)).ok_or_else(|| {
            LoweringError::missing(
                "a string literal has no semantic type",
                expression.source_vectors,
            )
        })?;
        let align = super::alignment::storage_alignment(&self.sema.types, info.ty)
            .map_err(|kind| at(kind, expression.source_vectors))?;
        match peel(expression).kind {
            | ExpressionType::StringLiteral(StringTokenType::String(id)) =>
                Ok((self.string_bytes(id), align)),
            | ExpressionType::StringLiteral(StringTokenType::WideString(id)) => {
                let layout = self.target.layout();
                let character = layout
                    .scalar(layout.wchar_t)
                    .expect("wchar_t has a scalar layout");
                let width = usize::try_from(character.size).expect("wchar_t fits usize");
                Ok((
                    self.unit_bytes(self.context.wide_literal_units(id), width),
                    align,
                ))
            },
            // C11: §6.4.5 paragraph 6, p. 71; PDF p. 89: UTF-8 bytes,
            // `char16_t` and `char32_t` units.
            | ExpressionType::StringLiteral(StringTokenType::EncodedString(id, encoding)) => {
                let width = match encoding {
                    | LiteralEncoding::Utf8 => 1,
                    | LiteralEncoding::Utf16 => 2,
                    | LiteralEncoding::Utf32 => 4,
                };
                Ok((
                    self.unit_bytes(self.context.encoded_literal_units(id, encoding), width),
                    align,
                ))
            },
            | _ => Err(LoweringError::missing(
                "a string literal has no literal",
                expression.source_vectors,
            )),
        }
    }

    /// Little-endian code units of `width` bytes, then the null unit.
    fn unit_bytes(&self, units: impl Iterator<Item = u32>, width: usize) -> ArenaVec<'a, u8> {
        let mut bytes = ArenaVec::new_in(self.tables);
        for unit in units.chain([0]) {
            bytes.extend_from_slice(&unit.to_le_bytes()[..width]);
        }
        bytes
    }

    /// The execution bytes of a narrow string literal and its null
    /// character: a source character contributes its UTF-8 encoding and a
    /// numeric escape one byte.
    /// C99: §6.4.5 paragraph 5, pp. 62-63; PDF pp. 74-75.
    pub(super) fn string_bytes(&self, id: LiteralId) -> ArenaVec<'a, u8> {
        let units = self.context.literal_units(id);
        let mut bytes = ArenaVec::with_capacity_in(units.len() + 1, self.tables);
        for &unit in units {
            match unit {
                | LiteralUnit::Character(c) =>
                    bytes.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()),
                | LiteralUnit::Numeric(code) => bytes.push(code.to_le_bytes()[0]),
            }
        }
        bytes.push(0);
        bytes
    }

    /// Names each literal once, without retaining repeatedly probed suffixes.
    /// Identical literals may share storage, but different contents must keep
    /// their own arrays. C99: §6.4.5 paragraph 6, p. 63; PDF p. 75.
    fn string_name(&mut self) -> &'a str {
        if self.name_available(".str") {
            return ".str";
        }
        loop {
            let suffix = self.next_string_suffix;
            self.next_string_suffix += 1;
            let name = format_in!(self.tables, ".str.{suffix}");
            if self.name_available(name) {
                return name;
            }
        }
    }

    /// Internal storage must not take a later user's symbol.
    /// C99: §6.2.2 paragraph 2, p. 30; PDF p. 42 (GNU label extension).
    fn name_available(&self, name: &str) -> bool {
        !self.reserved_names.contains(name) && self.module.symbol(name).is_none()
    }

    /// `base`, or the next unoccupied `base.N`, probing each suffix once.
    /// C99: §6.2.2 paragraph 2, p. 30; PDF p. 42 (distinct unlinked objects).
    pub(super) fn unique_name(&mut self, base: &'a str) -> &'a str {
        if self.name_available(base) {
            return base;
        }
        loop {
            let next = self.next_name_suffix.entry(base).or_insert(1);
            let suffix = *next;
            *next += 1;
            let name = format_in!(self.tables, "{base}.{suffix}");
            if self.name_available(name) {
                return name;
            }
        }
    }

    pub(super) fn into_module(mut self) -> Module<'ir> {
        self.settle_dll_storage();
        self.module
    }
}

// Static initializers

impl<'a, 'tu> Unit<'a, 'tu, '_> {
    /// The bytes an address constant fills: the target's pointer size.
    /// C99: §6.2.5 paragraph 20, pp. 35-36; PDF pp. 47-48.
    fn pointer_bytes(&self) -> u64 {
        self.sema.types.target.pointer.size
    }

    /// Materialize pending static objects iteratively. Address constants may
    /// declare nested compound literals while their containing object emits.
    /// C99: §6.5.2.5p5-6, pp. 75-76; PDF pp. 87-88; §6.6p9, p. 96; PDF p. 108.
    fn define_static_initializers(&mut self) -> Result<(), LoweringError> {
        while let Some((global, ty, initializer, flexible)) = self.static_initializers.pop() {
            let size = self.module.global(global).size;
            let (bytes, relocations) = self.static_bytes(ty, size, initializer, flexible)?;
            let init = if relocations.is_empty() && bytes.iter().all(|&byte| byte == 0) {
                GlobalInit::Zero
            } else {
                GlobalInit::Bytes {
                    bytes:       &bytes,
                    relocations: &relocations,
                }
            };
            self.module.define_global(global, init);
        }
        Ok(())
    }

    /// File-scope compound literals designate distinct unnamed static objects.
    /// C99: §6.5.2.5p5-6, pp. 75-76; PDF pp. 87-88.
    fn static_compound_literal(
        &mut self,
        expression: &'tu Expression<'tu>,
        initializer: &'tu Initializer<'tu>,
        ty: TypeId,
    ) -> Result<GlobalId, LoweringError> {
        let key = std::ptr::from_ref(expression).addr();
        if let Some(&global) = self.compound_literals.get(&key) {
            return Ok(global);
        }
        let layout = types::layout(&self.sema.types, ty)
            .map_err(|kind| at(kind, expression.source_vectors))?;
        let name = self.unique_name(".compound");
        let global = self.module.declare_global(
            name,
            GlobalDecl {
                linkage:      IrLinkage::Internal,
                size:         layout.size,
                align:        super::alignment::storage_alignment(&self.sema.types, ty)
                    .map_err(|kind| at(kind, expression.source_vectors))?,
                constant:     self.sema.types.readonly_object(ty),
                thread_local: false,
            },
        );
        _ = self.compound_literals.insert(key, global);
        _ = self.generated_globals.insert(global);
        self.static_initializers
            .push((global, ty, initializer, None));
        Ok(global)
    }

    /// The bytes and relocations of a static object of type `ty` and `size`
    /// bytes initialized by `initializer`. Every leaf must be an integer
    /// constant or an address constant.
    /// C99: §6.7.8 paragraph 4, p. 125; PDF p. 137; §6.6 paragraphs 7-9,
    /// pp. 95-96; PDF pp. 107-108.
    fn static_bytes(
        &mut self,
        ty: TypeId,
        size: u64,
        initializer: &'tu Initializer<'tu>,
        flexible: Option<TypeId>,
    ) -> Result<(ArenaVec<'a, u8>, ArenaVec<'a, Relocation>), LoweringError> {
        let tables = self.tables;
        let (leaves, _) =
            initializers::leaves(self.sema, ty, initializer, Storage::Static, tables)?;
        let length = usize::try_from(size).map_err(|_| {
            LoweringError::unsupported(Construct::StaticInitializer, initializer.source_vectors)
        })?;
        // GNU: an initializer of a flexible array member that a later list
        // or string replaces may write past the storage the replacement
        // gives the member (§6.7.8p19). Those bytes are written into a
        // longer buffer, which the replacement zeroes, and cut off.
        let written = flexible
            .and_then(|array| {
                let member = self.sema.types.tag(ty)?.members.get().last()?;
                let array = self.sema.types.layout(array)?;
                usize::try_from(member.offset.checked_add(array.size)?).ok()
            })
            .map_or(length, |written| written.max(length));
        // The initializer's bytes are held as one dense image, so an object
        // of 4 GiB or more is refused rather than exhausting memory.
        if u32::try_from(written).is_err() {
            return Err(LoweringError::unsupported(
                Construct::StaticInitializer,
                initializer.source_vectors,
            ));
        }
        let mut bytes = ArenaVec::with_capacity_in(written, tables);
        bytes.resize(written, 0_u8);
        let mut relocations = ArenaVec::new_in(tables);
        for leaf in leaves {
            match leaf {
                | Leaf::Zero { offset, size, .. } => {
                    let start =
                        usize::try_from(offset).expect("an initializer offset fits the object");
                    let end =
                        usize::try_from(offset + size).expect("an initializer fits the object");
                    bytes[start..end].fill(0);
                    // C99 §6.7.8p19: replacement also discards old addresses.
                    discard(
                        &mut relocations,
                        offset,
                        offset + size,
                        self.pointer_bytes(),
                    );
                },
                | Leaf::Scalar {
                    offset,
                    ty,
                    expression,
                    bit_field,
                    ..
                } => self.static_scalar(
                    &mut bytes,
                    &mut relocations,
                    (offset, bit_field),
                    ty,
                    expression,
                )?,
                | Leaf::String {
                    offset,
                    size,
                    literal,
                    ..
                } => {
                    let (string, _) = self.literal_bytes(literal)?;
                    let (start, size) = (index(offset, literal)?, index(size, literal)?);
                    let count = string.len().min(size);
                    bytes[start..start + size].fill(0);
                    discard(
                        &mut relocations,
                        offset,
                        offset + size as u64,
                        self.pointer_bytes(),
                    );
                    bytes[start..start + count].copy_from_slice(&string[..count]);
                },
                | Leaf::Copy {
                    offset,
                    ty,
                    expression,
                    ..
                } => self.static_copy(&mut bytes, &mut relocations, offset, ty, expression)?,
                | Leaf::Replicate {
                    offset,
                    size,
                    stride,
                    count,
                    ..
                } => replicate(
                    &mut bytes,
                    &mut relocations,
                    (offset, size),
                    (stride, count),
                    self.pointer_bytes(),
                    tables,
                )
                .ok_or_else(|| {
                    LoweringError::unsupported(
                        Construct::StaticInitializer,
                        initializer.source_vectors,
                    )
                })?,
            }
        }
        bytes.truncate(length);
        let pointer = self.pointer_bytes();
        relocations.retain(|relocation| relocation.offset + relocation.size(pointer) <= size);
        // Designators and range replacements visit offsets out of order.
        // Native storage emission consumes the final addresses in order.
        relocations.sort_unstable_by_key(|relocation| relocation.offset);
        Ok((bytes, relocations))
    }

    /// Writes the structure or union a compound literal of its type gives a
    /// static object: the bytes and addresses of the literal's own
    /// initializer, as if it were written in braces. Semantic analysis
    /// accepts no other expression here.
    /// GNU extension: GCC manual, "Compound Literals"; C99 §6.6p10, p. 96;
    /// PDF p. 108.
    fn static_copy(
        &mut self,
        bytes: &mut [u8],
        relocations: &mut ArenaVec<'_, Relocation>,
        offset: u64,
        ty: TypeId,
        mut expression: &'tu Expression<'tu>,
    ) -> Result<(), LoweringError> {
        let unsupported =
            || LoweringError::unsupported(Construct::StaticInitializer, expression.source_vectors);
        let literal = loop {
            match expression.kind {
                | ExpressionType::Parenthesized { expression: inner } => expression = inner,
                | ExpressionType::CompoundLiteral { initializer, .. } => break initializer,
                | _ => return Err(unsupported()),
            }
        };
        let size = types::layout(&self.sema.types, ty)
            .map_err(|kind| at(kind, expression.source_vectors))?
            .size;
        let literal_ty = self
            .sema
            .expression_info(expression)
            .ok_or_else(unsupported)?
            .ty;
        let (literal_bytes, literal_relocations) =
            self.static_bytes(literal_ty, size, literal, None)?;
        let start = index(offset, expression)?;
        bytes[start..start + literal_bytes.len()].copy_from_slice(&literal_bytes);
        discard(relocations, offset, offset + size, self.pointer_bytes());
        for relocation in literal_relocations {
            insert(
                relocations,
                Relocation {
                    offset: relocation.offset + offset,
                    ..relocation
                },
            );
        }
        Ok(())
    }

    /// The initializer and type of `expression` when it is a parenthesized
    /// compound literal.
    fn whole_compound_literal(
        &self,
        mut expression: &'tu Expression<'tu>,
    ) -> Option<(&'tu Initializer<'tu>, TypeId)> {
        loop {
            match expression.kind {
                | ExpressionType::Parenthesized { expression: inner } => expression = inner,
                | ExpressionType::CompoundLiteral { initializer, .. } =>
                    return Some((initializer, self.sema.expression_info(expression)?.ty)),
                | _ => return None,
            }
        }
    }

    /// Merges into a static bit-field the value a scalar compound literal
    /// of its type gives it: the literal's own initializer, converted to
    /// its type, lowered as a static object and read back. Semantic analysis
    /// accepts the literal only when it is of the field's declared type.
    /// GNU extension: GCC manual, "Compound Literals"; C99 §6.7.2.1
    /// paragraph 9, p. 102; PDF p. 114 (bit-field values); §6.6p10, p. 96;
    /// PDF p. 108.
    fn static_bit_field_literal(
        &mut self,
        bytes: &mut [u8],
        start: usize,
        field: BitFieldBits,
        literal: &'tu Initializer<'tu>,
        literal_ty: TypeId,
    ) -> Result<(), LoweringError> {
        let source = literal.source_vectors;
        let unsupported = || LoweringError::unsupported(Construct::StaticInitializer, source);
        let size = types::layout(&self.sema.types, literal_ty)
            .map_err(|kind| at(kind, source))?
            .size;
        let (literal_bytes, literal_relocations) =
            self.static_bytes(literal_ty, size, literal, None)?;
        if !literal_relocations.is_empty() || literal_bytes.len() > 16 {
            return Err(unsupported());
        }
        let mut value = [0; 16];
        value[..literal_bytes.len()].copy_from_slice(&literal_bytes);
        merge_bit_field(bytes, start, field, i128::from_le_bytes(value)).ok_or_else(unsupported)
    }

    /// Writes one scalar leaf: an integer or floating constant converted to
    /// the leaf's type, or an address constant as a relocation. A bit-field
    /// leaf merges the low bits of its value into the bytes it occupies.
    /// Later writes discard overlapping relocations (§6.7.8 paragraph 19,
    /// p. 127; PDF p. 139).
    /// C99: §6.7.8 paragraph 4, p. 125; PDF p. 137; §6.6 paragraphs 7-9,
    /// pp. 95-96; PDF pp. 107-108; §6.3.1.4, pp. 43-44; PDF pp. 55-56.
    fn static_scalar(
        &mut self,
        bytes: &mut [u8],
        relocations: &mut ArenaVec<'_, Relocation>,
        (offset, bit_field): (u64, Option<BitFieldBits>),
        ty: TypeId,
        expression: &'tu Expression<'tu>,
    ) -> Result<(), LoweringError> {
        let source = expression.source_vectors;
        let unsupported = || LoweringError::unsupported(Construct::StaticInitializer, source);
        let representation = repr(&self.sema.types, ty).map_err(|kind| at(kind, source))?;
        let info = self
            .sema
            .expression_info(expression)
            .ok_or_else(|| LoweringError::missing("an initializer has no record", source))?;
        let size = types::layout(&self.sema.types, ty)
            .map_err(|kind| at(kind, source))?
            .size;
        let written = bit_field.map_or(size, |field| {
            u64::from((field.bit_offset + field.width).div_ceil(8))
        });
        let (start, size) = (index(offset, expression)?, index(size, expression)?);
        // C99 §6.7.8p19: a later write replaces overlapping addresses,
        // including a scalar member that overwrites only part of a pointer.
        discard(relocations, offset, offset + written, self.pointer_bytes());
        // A literal with a folded value, a Clang-read one or an element GCC
        // converts from another type, stores that value instead.
        if let Some(field) = bit_field
            && info.integer.is_none()
            && info.floating().is_none()
            && let Some((literal, literal_ty)) = self.whole_compound_literal(expression)
        {
            return self.static_bit_field_literal(bytes, start, field, literal, literal_ty);
        }
        let integer = info.integer.filter(|_| {
            representation == Repr::Pointer
                || !matches!(
                    self.sema.types.kind(info.ty),
                    TypeKind::Pointer(_) | TypeKind::Array(..)
                )
        });
        // A wide bit-precise object stores its limbs, its padding bits
        // extended from its width.
        if let Some((width, signed)) = super::wide_bit_int::wide_bit_int(&self.sema.types, ty) {
            let mut limbs = ArenaVec::with_capacity_in(size / 8, self.tables);
            limbs.resize(size / 8, 0_u64);
            // A floating constant truncates toward zero (§6.3.1.4p1).
            let parts = if let Some(folded) = info.binary128() {
                super::wide_bit_int::binary128_parts(folded.real.to_bits())
            } else {
                info.floating()
                    .and_then(|folded| folded.real.x87_parts())
                    .and_then(super::wide_bit_int::x87_parts)
            };
            match (integer, parts) {
                | (Some(value), _) => self.sema.integer_limbs(value, &mut limbs),
                | (None, Some(parts)) =>
                    super::wide_bit_int::truncated_limbs(parts, width, signed, &mut limbs)
                        .ok_or_else(unsupported)?,
                | (None, None) => return Err(unsupported()),
            }
            super::wide_bit_int::extend_limbs(&mut limbs, width, signed);
            if let Some(field) = bit_field {
                for bit in 0..field.width as usize {
                    let at = field.bit_offset as usize + bit;
                    let byte = bytes.get_mut(start + at / 8).ok_or_else(unsupported)?;
                    let mask = 1_u8 << (at % 8);
                    if limbs[bit / 64] >> (bit % 64) & 1 == 1 {
                        *byte |= mask;
                    } else {
                        *byte &= !mask;
                    }
                }
                return Ok(());
            }
            for (target, limb) in bytes[start..start + size]
                .as_chunks_mut::<8>()
                .0
                .iter_mut()
                .zip(&limbs)
            {
                target.copy_from_slice(&limb.to_le_bytes());
            }
            return Ok(());
        }
        if let Repr::Complex { component } = representation {
            let real = match (info.floating(), integer) {
                | _ if let Some(value) = info.binary128() =>
                    floats::binary128_bits(value.real, component),
                | (Some(value), _) => floats::constant_bits(&value.real, component),
                | (_, Some(value)) => self.sema.integer_float_bits(value, component),
                | _ => None,
            }
            .ok_or_else(unsupported)?;
            let imag = if let Some(value) = info.binary128() {
                floats::binary128_bits(value.imag, component)
            } else {
                info.floating().map_or(Some(0), |value| {
                    floats::constant_bits(&value.imag, component)
                })
            }
            .ok_or_else(unsupported)?;
            for (index, bits) in [real, imag].into_iter().enumerate() {
                let stride = size / 2;
                let target = bytes
                    .get_mut(start + index * stride..start + (index + 1) * stride)
                    .ok_or_else(unsupported)?;
                for (byte, bits) in target.iter_mut().zip(floats::bytes(bits, stride)) {
                    *byte = bits;
                }
            }
            return Ok(());
        }
        if let Repr::Float { ty: float } = representation {
            let bits = match (info.floating(), integer) {
                | _ if let Some(folded) = info.binary128() =>
                    floats::binary128_bits(folded.real, float),
                | (Some(folded), _) => floats::constant_bits(&folded.real, float),
                | (None, Some(value)) => self.sema.integer_float_bits(value, float),
                | (None, None) => None,
            }
            .ok_or_else(unsupported)?;
            let target = bytes.get_mut(start..start + size).ok_or_else(unsupported)?;
            for (byte, bits) in target.iter_mut().zip(floats::bytes(bits, size)) {
                *byte = bits;
            }
            return Ok(());
        }
        let value = if let Some(value) = integer {
            // A wide value's low bits are the low bits of `value`, but
            // whether it is zero takes all of them.
            Some(if representation == Repr::Bool {
                i128::from(!value.is_zero())
            } else {
                value.value
            })
        } else if let Some(folded) = info.binary128()
            && matches!(representation, Repr::Int { .. } | Repr::Bool)
        {
            if representation == Repr::Bool {
                Some(i128::from(!folded.real.is_zero() || !folded.imag.is_zero()))
            } else {
                let (width, signed) = integer_destination(representation);
                let bits = floats::binary128_to_integer(folded.real, width, signed);
                Some(bits.ok_or_else(unsupported)?.cast_signed())
            }
        } else if let Some(folded) = info.floating()
            && matches!(representation, Repr::Int { .. } | Repr::Bool)
        {
            // A floating constant truncates toward zero (§6.3.1.4p1); `_Bool`
            // compares with zero (§6.3.1.2p1).
            if representation == Repr::Bool {
                Some(i128::from(!folded.real.is_zero() || !folded.imag.is_zero()))
            } else {
                let (width, signed) = integer_destination(representation);
                let bits = floats::to_integer(&folded.real, width, signed);
                Some(bits.ok_or_else(unsupported)?.cast_signed())
            }
        } else {
            None
        };
        if let Some(value) = value {
            let value = match representation {
                | Repr::Bool => i128::from(value != 0),
                // A narrow bit-precise object stores its padding bits
                // extended from its width, as GCC and Clang do.
                | Repr::Int { .. } | Repr::Pointer => {
                    let bits = match representation.narrow() {
                        | Some((_, _, width)) => width,
                        | None => u32::try_from(size * 8).map_err(|_| unsupported())?,
                    };
                    Integer::int(value)
                        .cast(bits, representation.signed())
                        .value
                },
                | _ => return Err(unsupported()),
            };
            if let Some(field) = bit_field {
                return merge_bit_field(bytes, start, field, value).ok_or_else(unsupported);
            }
            bytes[start..start + size].copy_from_slice(&value.to_le_bytes()[..size]);
            return Ok(());
        }
        if info.constant == ConstantClass::LabelDifference
            && let Repr::Int { ty: width, .. } = representation
            && bit_field.is_none()
        {
            let target = self.label_difference(expression, width)?;
            bytes[start..start + size].fill(0);
            insert(
                relocations,
                Relocation {
                    offset,
                    target,
                    addend: 0,
                },
            );
            return Ok(());
        }
        // Semantic analysis keeps an address converted to an integer as wide
        // as a pointer an address constant (§6.6p10), so its relocation
        // fills that integer.
        let address = representation == Repr::Pointer
            || (matches!(representation, Repr::Int { .. })
                && info.constant == ConstantClass::Address);
        if !address || bit_field.is_some() {
            return Err(unsupported());
        }
        let (target, addend) = self.address_constant(expression)?;
        // Symbolic bytes replace any older scalar representation too;
        // that representation must not reappear if a later write drops the
        // relocation.
        bytes[start..start + size].fill(0);
        insert(
            relocations,
            Relocation {
                offset,
                target,
                addend,
            },
        );
        Ok(())
    }

    /// The relocation of a constant label difference `&&label - &&base`,
    /// truncated to `width`, from the labels semantic analysis retained.
    /// GNU extension: GCC "Labels as Values".
    fn label_difference(
        &mut self,
        expression: &'tu Expression<'tu>,
        width: Type,
    ) -> Result<RelocationTarget, LoweringError> {
        let source = expression.source_vectors;
        if !matches!(width, Type::I32 | Type::I64) {
            return Err(LoweringError::missing(
                "a label difference has a width semantic analysis refuses",
                source,
            ));
        }
        let (label, base) = self
            .sema
            .control
            .label_difference(expression)
            .ok_or_else(|| {
                LoweringError::missing("a label difference has no retained labels", source)
            })?;
        let (func, label) = self.label_target(label, source)?;
        let (_, base) = self.label_target(base, source)?;
        Ok(RelocationTarget::LabelDifference {
            func,
            label,
            base,
            width,
        })
    }

    /// The IR function and label of an address-taken C label: the label's
    /// position among its function's address-taken labels, which that
    /// function's body numbers its IR labels by.
    /// GNU extension: GCC "Labels as Values".
    fn label_target(
        &mut self,
        label: LabelId,
        source: SourceVectors,
    ) -> Result<(FuncId, BlockLabel), LoweringError> {
        let facts = self.sema.control.labels[label.0];
        let record =
            self.sema.functions.get(facts.function.0).ok_or_else(|| {
                LoweringError::missing("a label's function has no record", source)
            })?;
        let binding = record
            .binding
            .ok_or_else(|| LoweringError::missing("a label's function has no binding", source))?;
        let func = self.function(binding).map_err(|kind| at(kind, source))?;
        let index = facts.address.ok_or_else(|| {
            LoweringError::missing("an addressed label has no address index", source)
        })?;
        Ok((func, BlockLabel::new(index as usize)))
    }

    /// The symbol and byte offset an address constant designates: a static
    /// object, function, string literal or, as a GNU extension, label,
    /// optionally selected by a constant conditional or with a constant
    /// subscript, member offset or integer added.
    /// C99: §6.6 paragraph 9, p. 96; PDF p. 108.
    /// C99: §6.5.3.2 paragraph 4, p. 79; PDF p. 91.
    /// C99: §6.5.15 paragraph 4, p. 90; PDF p. 102.
    fn address_constant(
        &mut self,
        mut expression: &'tu Expression<'tu>,
    ) -> Result<(RelocationTarget, i64), LoweringError> {
        let unsupported =
            || LoweringError::unsupported(Construct::StaticInitializer, expression.source_vectors);
        let mut addend = 0_i64;
        // Whether `expression` designates an object (after `&`) rather than
        // computing a pointer value.
        let mut place = false;
        loop {
            let info = self
                .sema
                .expression_info(expression)
                .ok_or_else(unsupported)?;
            if let Some(selected) = info.selected_expression {
                expression = selected;
                continue;
            }
            match expression.kind {
                | ExpressionType::CompoundLiteral {
                    type_name,
                    initializer,
                } => {
                    if super::expressions::literal_storage(type_name, false) {
                        return Err(LoweringError::unsupported(
                            Construct::CompoundLiteralStorage,
                            expression.source_vectors,
                        ));
                    }
                    let global = self.static_compound_literal(expression, initializer, info.ty)?;
                    return Ok((RelocationTarget::Symbol(Symbol::Global(global)), addend));
                },
                // GNU extension: GCC "Labels as Values" tables, as in Lua's
                // threaded `luaV_execute`.
                | ExpressionType::LabelAddress(_) if !place => {
                    let source = expression.source_vectors;
                    let label = self.sema.control.label_address(expression).ok_or_else(|| {
                        LoweringError::missing("a label address has no resolved label", source)
                    })?;
                    let (func, label) = self.label_target(label, source)?;
                    return Ok((RelocationTarget::Label { func, label }, addend));
                },
                | ExpressionType::Unary {
                    operator: UnaryOperator::Indirection,
                    operand_expression,
                } if place
                    || matches!(self.sema.types.kind(info.ty), TypeKind::Function { .. }) =>
                {
                    // Function indirection designates the same function; an
                    // object indirection in value context would require a load.
                    expression = operand_expression;
                    place = false;
                },
                | ExpressionType::Parenthesized { expression: inner }
                | ExpressionType::Unary {
                    operator: UnaryOperator::Extension,
                    operand_expression: inner,
                } => expression = inner,
                | ExpressionType::Cast {
                    operand_expression, ..
                } if !place => expression = operand_expression,
                | ExpressionType::Unary {
                    operator: UnaryOperator::AddressOf,
                    operand_expression,
                } if !place => {
                    place = true;
                    expression = operand_expression;
                },
                | ExpressionType::Identifier(_) => {
                    // Use the same retained value selection as runtime
                    // identifier lowering. A fortify call's private body is
                    // distinct from its external function address, including
                    // static relocations (C99 §6.3.2.1p4, p. 46; PDF p. 58).
                    let binding = self
                        .sema
                        .function_extensions
                        .value(expression)
                        .or(info.binding)
                        .ok_or_else(unsupported)?;
                    let data = self.sema.bindings[binding];
                    let source = expression.source_vectors;
                    let kind = self.sema.types.kind(data.ty);
                    let symbol = if matches!(kind, TypeKind::Function { .. }) {
                        Symbol::Function(self.function(binding).map_err(|kind| at(kind, source))?)
                    } else if data.duration
                        == crate::translation_phases::semantic_analysis::Duration::Static
                        && (place || matches!(kind, TypeKind::Array(..)))
                    {
                        Symbol::Global(self.object(binding).map_err(|kind| at(kind, source))?)
                    } else {
                        return Err(unsupported());
                    };
                    return Ok((RelocationTarget::Symbol(symbol), addend));
                },
                | ExpressionType::StringLiteral(_) =>
                    return Ok((
                        RelocationTarget::Symbol(Symbol::Global(self.string(expression)?)),
                        addend,
                    )),
                | ExpressionType::DirectMember {
                    base_expression, ..
                } if place || matches!(self.sema.types.kind(info.ty), TypeKind::Array(..)) => {
                    let field = self.sema.selected_field(expression).ok_or_else(|| {
                        LoweringError::missing(
                            "member facts are not retained",
                            expression.source_vectors,
                        )
                    })?;
                    if field.width.is_some() {
                        return Err(unsupported());
                    }
                    addend = addend
                        .checked_add(i64::try_from(field.offset).map_err(|_| unsupported())?)
                        .ok_or_else(unsupported)?;
                    expression = base_expression;
                    place = true;
                },
                | ExpressionType::Binary {
                    operator:
                        operator @ (BinaryOperator::Subscript
                        | BinaryOperator::Addition
                        | BinaryOperator::Subtraction),
                    left_expression,
                    right_expression,
                } if (operator == BinaryOperator::Subscript) == place => {
                    let left = self
                        .sema
                        .expression_info(left_expression)
                        .ok_or_else(unsupported)?;
                    let right = self
                        .sema
                        .expression_info(right_expression)
                        .ok_or_else(unsupported)?;
                    let (base, index, pointer) = if right.integer.is_some()
                        && types::target_type(&self.sema.types, left.ty).is_some()
                    {
                        (left_expression, right.integer, left.ty)
                    } else if operator != BinaryOperator::Subtraction
                        && left.integer.is_some()
                        && types::target_type(&self.sema.types, right.ty).is_some()
                    {
                        (right_expression, left.integer, right.ty)
                    } else {
                        return Err(unsupported());
                    };
                    let index = index.and_then(Integer::to_i128).ok_or_else(unsupported)?;
                    let element =
                        types::target_type(&self.sema.types, pointer).ok_or_else(unsupported)?;
                    let size = types::arithmetic_size(&self.sema.types, element)
                        .ok_or_else(unsupported)?;
                    let scaled = index
                        .checked_mul(i128::from(size))
                        .and_then(|scaled| i64::try_from(scaled).ok())
                        .ok_or_else(unsupported)?;
                    addend = if operator == BinaryOperator::Subtraction {
                        addend.checked_sub(scaled)
                    } else {
                        addend.checked_add(scaled)
                    }
                    .ok_or_else(unsupported)?;
                    // A subscripted array is designated in place; a pointer
                    // operand is evaluated as a value.
                    place = matches!(self.sema.types.kind(pointer), TypeKind::Array(..));
                    expression = base;
                },
                | _ => return Err(unsupported()),
            }
        }
    }
}

/// Stores the low `field.width` bits of `value` at bit `field.bit_offset` of
/// the little-endian bytes from `start`, keeping the other bits. `None` if the
/// field lies outside `bytes`.
fn merge_bit_field(bytes: &mut [u8], start: usize, field: BitFieldBits, value: i128) -> Option<()> {
    if field.width == 0 || field.width > 128 || field.bit_offset >= 8 {
        return None;
    }
    let span = usize::try_from((field.bit_offset + field.width).div_ceil(8)).ok()?;
    let target = bytes.get_mut(start..start + span)?;
    // GNU packed fields can touch seventeen bytes even though their value
    // fits in 128 bits. Merge in value coordinates, retaining neighbor bits.
    // C99: §6.7.8p19, p. 127; PDF p. 139.
    // <https://gcc.gnu.org/onlinedocs/gcc/Common-Attributes.html> (packed).
    for bit in 0..field.width {
        let position = usize::try_from(field.bit_offset + bit).ok()?;
        let mask = 1_u8 << (position % 8);
        let byte = &mut target[position / 8];
        if (value as u128) & (1_u128 << bit) == 0 {
            *byte &= !mask;
        } else {
            *byte |= mask;
        }
    }
    Some(())
}

/// Copies the first element of a GNU range, its `size` bytes at `offset`
/// and the relocations inside them, into each of the `count` later elements
/// `stride` bytes apart, whose relocations there it replaces in the order
/// element-by-element stores would leave. Adjacent copies double the
/// copied bytes each time. `None` if a copy is outside `bytes`.
/// C99: §6.7.8 paragraph 19, p. 127; PDF p. 139.
/// GNU extension: GCC manual, "Designated Initializers" (ranges).
fn replicate(
    bytes: &mut [u8],
    relocations: &mut ArenaVec<'_, Relocation>,
    (offset, size): (u64, u64),
    (stride, count): (u64, u64),
    pointer: u64,
    scratch: &Bump,
) -> Option<()> {
    let start = usize::try_from(offset).ok()?;
    let length = usize::try_from(size).ok()?;
    let step = usize::try_from(stride).ok()?;
    let elements = usize::try_from(count).ok()?;
    let end = start
        .checked_add(step.checked_mul(elements)?)?
        .checked_add(length)?;
    if end > bytes.len() || length == 0 || length > step {
        return None;
    }
    if length == step {
        let mut done = length;
        while start + done < end {
            let doubled = done.min(end - start - done);
            bytes.copy_within(start..start + doubled, start + done);
            done += doubled;
        }
    } else {
        for copy in 1..=elements {
            bytes.copy_within(start..start + length, start + copy * step);
        }
    }
    let mut inside = ArenaVec::new_in(scratch);
    inside.extend(
        relocations[overlapping(relocations, offset, offset + size, pointer)]
            .iter()
            .copied()
            .filter(|relocation| {
                relocation.offset >= offset
                    && relocation.offset + relocation.size(pointer) <= offset + size
            }),
    );
    // Only relocations under the copies can change. Of the copies ending
    // after a relocation's start, the first overlaps it if it starts before
    // the relocation ends.
    let copies = overlapping(relocations, offset + stride, end as u64, pointer);
    let mut kept = ArenaVec::new_in(scratch);
    kept.extend(
        relocations[copies.clone()]
            .iter()
            .copied()
            .filter(|relocation| {
                let after = relocation.offset.saturating_sub(offset + size) / stride + 1;
                after > count
                    || offset + after * stride >= relocation.offset + relocation.size(pointer)
            }),
    );
    // A kept relocation lies between copies, so merging keeps the order.
    let mut kept = kept.into_iter().peekable();
    let mut merged = ArenaVec::with_capacity_in(kept.len() + inside.len() * elements, scratch);
    for copy in 1..=count {
        for &relocation in &inside {
            let relocation = Relocation {
                offset: relocation.offset + copy * stride,
                ..relocation
            };
            while let Some(earlier) = kept.next_if(|kept| kept.offset < relocation.offset) {
                merged.push(earlier);
            }
            merged.push(relocation);
        }
    }
    merged.extend(kept);
    drop(relocations.splice(copies, merged));
    Some(())
}

/// The positions of the relocations that overlap bytes `start..end`. Each
/// write first discards the relocations it overlaps and [`insert`] keeps the
/// list sorted by offset, so relocations never overlap and their ends are
/// sorted too: one search finds the run, without scanning the whole list.
fn overlapping(
    relocations: &[Relocation],
    start: u64,
    end: u64,
    pointer: u64,
) -> std::ops::Range<usize> {
    let first = relocations
        .partition_point(|relocation| relocation.offset + relocation.size(pointer) <= start);
    first..first + relocations[first..].partition_point(|relocation| relocation.offset < end)
}

/// Discards the relocations a write of bytes `start..end` replaces (C99
/// §6.7.8 paragraph 19, p. 127; PDF p. 139).
fn discard(relocations: &mut ArenaVec<'_, Relocation>, start: u64, end: u64, pointer: u64) {
    drop(relocations.drain(overlapping(relocations, start, end, pointer)));
}

/// Adds a relocation whose bytes no other relocation overlaps, in offset
/// order.
fn insert(relocations: &mut ArenaVec<'_, Relocation>, relocation: Relocation) {
    let at = relocations.partition_point(|earlier| earlier.offset < relocation.offset);
    relocations.insert(at, relocation);
}

/// A byte position in an object being initialized, which a static object
/// small enough to define always has.
fn index(position: u64, expression: &Expression<'_>) -> Result<usize, LoweringError> {
    usize::try_from(position).map_err(|_| {
        LoweringError::unsupported(Construct::StaticInitializer, expression.source_vectors)
    })
}

/// The expression inside any parentheses.
pub(super) fn peel<'tu>(mut expression: &'tu Expression<'tu>) -> &'tu Expression<'tu> {
    while let ExpressionType::Parenthesized { expression: inner } = expression.kind {
        expression = inner;
    }
    expression
}

pub(super) fn at(kind: LoweringErrorKind, source: SourceVectors) -> LoweringError {
    LoweringError {
        kind,
        source: Some(source),
    }
}

impl<'a, 'tu, 'ir> Unit<'a, 'tu, 'ir> {
    pub(super) fn new(
        context: &'a Context<'tu>,
        sema: &'a SemanticTranslationUnit<'tu>,
        target: Target,
        arena: &'ir Bump,
        tables: &'a Bump,
    ) -> Result<Self, LoweringError> {
        let address_taken = address_taken::find(sema, tables)?;
        let mut module = Module::new(arena);
        module.set_target(target.triple(), types::data_layout(target));
        for (index, binding) in sema.bindings.iter().enumerate() {
            if binding.needs_entity_policy() && sema.entity(index).is_none() {
                return Err(LoweringError::missing(
                    "entity policy is not retained",
                    binding.name.source_vectors,
                ));
            }
        }
        for function in sema.functions {
            if let Some(binding) = function.binding
                && sema.function_policy(binding).is_none()
            {
                return Err(LoweringError::missing(
                    "function policy is not retained",
                    function.syntax.source_vectors,
                ));
            }
        }
        let defined_objects = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tables);
        let mut pack_functions = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tables);
        for function in sema.functions {
            // Only the definition calls reach: a later definition replaces
            // a GNU `extern inline` body (GCC manual, "Inline").
            if function.va_arg_pack
                && let Some(entity) = function.binding.and_then(|binding| sema.entity(binding))
                && entity.function == function.binding
            {
                _ = pack_functions.insert(entity.identity, function);
            }
        }
        Ok(Self {
            retain_local_statics: true,
            reachable: ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, tables),
            #[cfg(test)]
            parameter_alias_visits: 0,
            context,
            sema,
            module,
            call_diagnostics: ArenaVec::new_in(arena),
            tables,
            target,
            defined_objects,
            tentative_objects: ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, tables),
            address_taken,
            statics: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tables),
            strings: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tables),
            compound_literals: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tables),
            static_initializers: ArenaVec::new_in(tables),
            next_string_suffix: 1,
            reserved_names: ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, tables),
            generated_globals: ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, tables),
            conflicting_names: ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, tables),
            next_name_suffix: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tables),
            private_bodies: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tables),
            plain_inline: ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, tables),
            explicit_visibility: ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, tables),
            pack_functions,
            pack_copies: ArenaVec::new_in(tables),
            pack_copy_index: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tables),
            pack_addresses: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tables),
        })
    }
}

/// The string-literal table key's form: narrow, `L`, `u8`, `u` or `U`.
fn string_form(literal: StringTokenType) -> (LiteralId, u8) {
    match literal {
        | StringTokenType::String(id) => (id, 0),
        | StringTokenType::WideString(id) => (id, 1),
        | StringTokenType::EncodedString(id, LiteralEncoding::Utf8) => (id, 2),
        | StringTokenType::EncodedString(id, LiteralEncoding::Utf16) => (id, 3),
        | StringTokenType::EncodedString(id, LiteralEncoding::Utf32) => (id, 4),
    }
}

/// The width and signedness of an integer leaf, which a floating constant
/// converts to directly, so an `unsigned __int128` takes values from 2^127
/// up (C99 §6.3.1.4 paragraph 1, p. 43; PDF p. 55).
fn integer_destination(representation: Repr) -> (u32, bool) {
    let width = match representation.narrow() {
        | Some((_, _, width)) => width,
        | None => representation.value_type().map_or(128, Type::bits),
    };
    (width, representation.signed())
}
