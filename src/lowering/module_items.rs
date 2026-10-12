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

use super::{
    Construct,
    LoweringError,
    LoweringErrorKind,
    address_taken,
    extensions::SymbolNames,
    floats,
    initializers::{
        self,
        Leaf,
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
        AggregateParam,
        Align,
        ExtendedParam,
        FuncId,
        GlobalDecl,
        GlobalId,
        GlobalInit,
        Linkage as IrLinkage,
        Module,
        ParamAbi,
        Relocation,
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
            ExternalDeclaration,
            ParsedTranslationUnit,
            declaration_syntax::{
                Initializer,
                TypeQualifiers,
            },
            syntax::{
                BinaryOperator,
                Expression,
                ExpressionType,
                UnaryOperator,
            },
        },
        preprocessing::{
            LiteralId,
            LiteralUnit,
            StringTokenType,
        },
        semantic_analysis::{
            Binding,
            DefinitionKind,
            FunctionRecord,
            Integer,
            Linkage,
            ScopeKind,
            SemanticTranslationUnit,
            TypeId,
            TypeKind,
            library_builtin_name,
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
    pub(super) context:       &'a Context<'tu>,
    pub(super) sema:          &'a SemanticTranslationUnit<'tu>,
    pub(super) module:        Module<'ir>,
    pub(super) tables:        &'a Bump,
    pub(super) target:        Target,
    pub(super) names:         SymbolNames<'a>,
    /// Each binding by its declared identifier, so a declarator finds its
    /// binding without a search.
    declarators:              ArenaMap<'a, (StringCacheId, SourceVectors), usize>,
    definitions:              ArenaMap<'a, usize, DefinitionKind>,
    /// Completed types of linked object definitions, including tentative ones,
    /// indexed by emitted symbol name after applying GNU assembler labels.
    /// C99: §6.2.7 paragraph 4, p. 40; PDF p. 52; §6.9.2 paragraph 2,
    /// p. 143; PDF p. 155.
    defined_objects:          ArenaMap<'a, &'a str, usize>,
    /// Definitions share their linked name across separate semantic bindings.
    /// C99: §6.2.2 paragraph 2, p. 30; PDF p. 42.
    defined_functions:        ArenaSet<'a, StringCacheId>,
    /// Bindings whose address is taken, which therefore live in memory.
    pub(super) address_taken: ArenaSet<'a, usize>,
    /// Globals of static locals and `__func__`, which have no linkage.
    statics:                  ArenaMap<'a, usize, GlobalId>,
    strings:                  ArenaMap<'a, (LiteralId, bool), GlobalId>,
    /// Next candidate for a string literal's internal symbol.
    next_string_suffix:       u64,
    /// User names reserved before generating internal storage.
    reserved_names:           ArenaSet<'a, &'a str>,
    /// Internal storage cannot be reused by a linked object declaration.
    generated_globals:        ArenaSet<'a, GlobalId>,
    /// Conflicting definitions are isolated while later functions are checked.
    conflicting_names:        ArenaSet<'a, &'a str>,
    /// Each generated base probes a suffix at most once.
    next_name_suffix:         ArenaMap<'a, &'a str, u64>,
}

impl<'a, 'tu, 'ir> Unit<'a, 'tu, 'ir> {
    /// Declares every defined function and defines every file-scope object.
    /// Returns the function definitions in order, each with its function if
    /// it is to be lowered: inline definitions and unsupported signatures
    /// have none.
    /// C99: §6.2.7 paragraph 4, p. 40; PDF p. 52; §6.9.2 paragraph 2,
    /// p. 143; PDF p. 155.
    pub(super) fn declare_items(
        &mut self,
        syntax: &ParsedTranslationUnit<'tu>,
        errors: &mut ArenaVec<'_, LoweringError>,
    ) -> ArenaVec<'a, (&'tu FunctionRecord<'tu>, Option<FuncId>)> {
        // Reserve every user spelling before any initializer or body creates
        // internal storage (GNU labels extend C99 §6.2.2p2).
        for (index, binding) in self.sema.bindings.iter().enumerate() {
            let name = if binding.linkage == Linkage::None {
                self.names.local(index)
            } else {
                Some(self.symbol_name(index))
            };
            if let Some(name) = name {
                _ = self.reserved_names.insert(name);
                if self.names.linked(binding.name.name).is_none()
                    && let Some(runtime) = library_builtin_name(name)
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
        for record in self.sema.functions {
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
        let mut initializers: ArenaMap<'_, usize, &'tu Initializer<'tu>> =
            ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, self.tables);
        for root in syntax.external_declarations() {
            let ExternalDeclaration::Declaration(declaration) = *root else {
                continue;
            };
            for init_declarator in declaration.init_declarators {
                if let Some(initializer) = init_declarator.initializer
                    && let Some(binding) = self.declarator_binding(init_declarator.declarator)
                {
                    _ = initializers.insert(binding, initializer);
                }
            }
        }
        for definition in self.sema.definitions {
            let binding = self.sema.bindings[definition.binding];
            if !matches!(
                definition.kind,
                DefinitionKind::Object | DefinitionKind::Tentative
            ) || binding.linkage == Linkage::None
            {
                continue;
            }
            if self
                .conflicting_names
                .contains(self.symbol_name(definition.binding))
            {
                continue;
            }
            let initializer = initializers.get(&definition.binding).copied();
            if let Err(error) = self.define_object(definition.binding, None, initializer) {
                errors.push(error);
            }
        }
        functions
    }

    /// Declares a function definition's symbol; `None` when its body is
    /// not lowered, as for a C99 inline definition, which provides no
    /// external definition (§6.7.4 paragraph 6, pp. 112-113; PDF pp. 124-125).
    fn declare_definition(
        &mut self,
        record: &FunctionRecord<'tu>,
    ) -> Result<Option<FuncId>, LoweringError> {
        let source = record.syntax.source_vectors;
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
        let func = self.function(binding).map_err(|kind| at(kind, source))?;
        Ok(match self.definitions.get(&binding) {
            | Some(DefinitionKind::Inline) => None,
            | _ => Some(func),
        })
    }

    /// The function a function binding names, declaring it on first use
    /// with the signature of the binding's type.
    pub(super) fn function(&mut self, binding: usize) -> Result<FuncId, LoweringErrorKind> {
        let data = self.sema.bindings[binding];
        let name = self.symbol_name(binding);
        // Builtin aliases describe the C binding, never the exact assembler
        // label chosen by the user (GNU extension to C99 §6.2.2p2).
        let runtime = self
            .names
            .linked(data.name.name)
            .is_none()
            .then(|| library_builtin_name(self.context.string_cache.at(data.name.name)))
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
            self.signature(data.ty)?
        };
        Ok(self.module.declare_function(name, signature, linkage(data)))
    }

    /// An external declaration without a prototype or a known definition.
    /// Known old-style definitions keep their actual parameter signature.
    /// C99: §6.5.2.2 paragraph 6, pp. 71-72; PDF pp. 83-84.
    pub(super) fn unprototyped_external(&self, binding: usize) -> bool {
        matches!(
            self.sema.types.kind(self.sema.bindings[binding].ty),
            TypeKind::Function {
                prototype: false,
                ..
            }
        ) && !self
            .defined_functions
            .contains(&self.sema.bindings[binding].name.name)
    }

    /// The IR signature of a C function type: one parameter per declared
    /// parameter, after the adjustments of §6.7.5.3 paragraphs 7-8, p. 119;
    /// PDF p. 131. An unprototyped type has no fixed parameters.
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

    /// The pre-ABI signature of a call or function whose fixed parameters
    /// have the C types `parameters`. A structure or union parameter is a
    /// `ptr` to a copy the caller makes, described by
    /// [`ParamAbi::Aggregate`]; a structure or union result is returned
    /// through a `ptr` to the receiving object inserted as parameter 0 and
    /// described by [`ParamAbi::Result`], and the signature has no result.
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
            | Repr::Aggregate => {
                params.push(Type::Ptr);
                aggregates.push(AggregateParam {
                    index: 0,
                    abi:   ParamAbi::Result(types::aggregate_layout(types, result, scratch)?),
                });
                None
            },
            | representation => representation.value_type(),
        };
        for parameter in parameters {
            match repr(types, parameter)? {
                | Repr::Aggregate => {
                    let index = u32::try_from(params.len())
                        .map_err(|_| LoweringErrorKind::MissingFact("too many parameters"))?;
                    // Only the pointer to the caller-owned copy is defined.
                    noundef.push(index);
                    params.push(Type::Ptr);
                    aggregates.push(AggregateParam {
                        index,
                        abi: ParamAbi::Aggregate(types::aggregate_layout(
                            types, parameter, scratch,
                        )?),
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
        if data.linkage == Linkage::None {
            let Some(&DefinitionKind::FunctionName(name)) = self.definitions.get(&binding) else {
                return Err(LoweringErrorKind::MissingFact(
                    "a static local is used before its declaration",
                ));
            };
            return Ok(self.function_name(binding, name));
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
                let data = self
                    .defined_objects
                    .get(name)
                    .map_or(data, |&binding| self.sema.bindings[binding]);
                let declaration = self.global_declaration(data, linkage(data))?;
                Ok(self.module.declare_global(name, declaration))
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
                let name = if let Some(name) = self.names.local(binding) {
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
                    .global_declaration(data, IrLinkage::Internal)
                    .map_err(|kind| at(kind, source))?;
                let global = self.module.declare_global(name, declaration);
                if self.names.local(binding).is_none() {
                    _ = self.generated_globals.insert(global);
                }
                _ = self.statics.insert(binding, global);
                global
            },
        };
        match initializer {
            | None => self.module.define_global(global, GlobalInit::Zero),
            | Some(initializer) => {
                let size = self.module.global(global).size;
                let (bytes, relocations) = self.static_bytes(data.ty, size, initializer)?;
                let init = if relocations.is_empty() && bytes.iter().all(|&byte| byte == 0) {
                    GlobalInit::Zero
                } else {
                    GlobalInit::Bytes {
                        bytes:       &bytes,
                        relocations: &relocations,
                    }
                };
                self.module.define_global(global, init);
            },
        }
        Ok(global)
    }

    /// A global's size, alignment and constancy from the object's type. An
    /// incomplete array declared but not defined here has size zero.
    fn global_declaration(
        &self,
        data: Binding,
        linkage: IrLinkage,
    ) -> Result<GlobalDecl, LoweringErrorKind> {
        let types = &self.sema.types;
        if types.unanalyzed(data.ty) {
            return Err(LoweringErrorKind::UnanalyzedType);
        }
        let (size, align) = match types.layout(data.ty) {
            | Some(layout) => (layout.size, types::align(layout)),
            | None => {
                let element =
                    types::target_type(types, data.ty).and_then(|element| types.layout(element));
                match element {
                    | Some(layout) => (0, types::align(layout)),
                    | None =>
                        return Err(types::layout(types, data.ty).err().unwrap_or(
                            LoweringErrorKind::MissingFact("an object type has no layout"),
                        )),
                }
            },
        };
        Ok(GlobalDecl {
            linkage,
            size,
            align,
            constant: constant_object(types, data.ty),
        })
    }

    /// The implicit `__func__` array of a function: its name and a null
    /// character.
    /// C99: §6.4.2.2 paragraph 1, p. 52; PDF p. 64.
    fn function_name(&mut self, binding: usize, name: StringCacheId) -> GlobalId {
        let text = self.context.string_cache.at(name);
        let symbol = format_in!(self.tables, "{text}.__func__");
        let symbol = self.unique_name(symbol);
        let global = self.module.declare_global(
            symbol,
            GlobalDecl {
                linkage:  IrLinkage::Internal,
                size:     text.len() as u64 + 1,
                align:    Align::BYTE,
                constant: true,
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
        let key = match literal {
            | StringTokenType::String(id) => (id, false),
            | StringTokenType::WideString(id) => (id, true),
            | StringTokenType::EncodedString(..) =>
                return Err(LoweringError::unsupported(
                    Construct::EncodedString,
                    expression.source_vectors,
                )),
        };
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

    /// Encodes a literal and its terminator in the target's character units.
    /// C99: §6.4.5p5, pp. 62-63; PDF pp. 74-75;
    /// §6.7.8p14-15, p. 126; PDF p. 138.
    fn literal_bytes(
        &self,
        expression: &'tu Expression<'tu>,
    ) -> Result<(ArenaVec<'a, u8>, Align), LoweringError> {
        match peel(expression).kind {
            | ExpressionType::StringLiteral(StringTokenType::String(id)) =>
                Ok((self.string_bytes(id), Align::BYTE)),
            | ExpressionType::StringLiteral(StringTokenType::WideString(id)) => {
                let layout = self.target.layout();
                let character = layout
                    .scalar(layout.wchar_t)
                    .expect("wchar_t has a scalar layout");
                let width = usize::try_from(character.size).expect("wchar_t fits usize");
                let mut bytes = ArenaVec::new_in(self.tables);
                for unit in self.context.wide_literal_units(id).chain([0]) {
                    bytes.extend_from_slice(&unit.to_le_bytes()[..width]);
                }
                Ok((
                    bytes,
                    Align::from_bytes(character.align).expect("wchar_t alignment"),
                ))
            },
            | _ => Err(LoweringError::unsupported(
                Construct::EncodedString,
                expression.source_vectors,
            )),
        }
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
    fn unique_name(&mut self, base: &'a str) -> &'a str {
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

    /// The binding a declarator declares.
    pub(super) fn declarator_binding(
        &self,
        declarator: crate::translation_phases::parsing::declaration_syntax::Declarator<'tu>,
    ) -> Option<usize> {
        let identifier = declarator.identifier()?;
        self.declarators
            .get(&(identifier.name, identifier.source_vectors))
            .copied()
    }

    pub(super) fn into_module(self) -> Module<'ir> {
        self.module
    }
}

// Static initializers

impl<'a, 'tu> Unit<'a, 'tu, '_> {
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
    ) -> Result<(ArenaVec<'a, u8>, ArenaVec<'a, Relocation>), LoweringError> {
        let tables = self.tables;
        let leaves = initializers::leaves(self.sema, ty, initializer, tables)?;
        let length = usize::try_from(size).map_err(|_| {
            LoweringError::unsupported(Construct::StaticInitializer, initializer.source_vectors)
        })?;
        let mut bytes = ArenaVec::with_capacity_in(length, tables);
        bytes.resize(length, 0_u8);
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
                    relocations.retain(|relocation: &Relocation| {
                        relocation.offset + 8 <= offset || relocation.offset >= offset + size
                    });
                },
                | Leaf::Scalar {
                    offset,
                    ty,
                    expression,
                    bit_field,
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
                    relocations.retain(|relocation| {
                        relocation.offset + 8 <= offset || relocation.offset >= offset + size as u64
                    });
                    bytes[start..start + count].copy_from_slice(&string[..count]);
                },
                | Leaf::Copy { expression, .. } =>
                    return Err(LoweringError::unsupported(
                        Construct::StaticInitializer,
                        expression.source_vectors,
                    )),
            }
        }
        Ok((bytes, relocations))
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
        relocations.retain(|relocation| {
            relocation.offset + 8 <= offset || relocation.offset >= offset + written
        });
        let integer = info.integer.filter(|_| {
            !matches!(
                self.sema.types.kind(info.ty),
                TypeKind::Pointer(_) | TypeKind::Array(..)
            )
        });
        if let Repr::Float { ty: float } = representation {
            let bits = match (info.floating, integer) {
                | (Some(folded), _) => floats::constant_bits(&folded.real, float),
                | (None, Some(value)) =>
                    if value.signed {
                        floats::from_integer(value.value, float)
                    } else {
                        floats::from_unsigned(u128::from_le_bytes(value.value.to_le_bytes()), float)
                    },
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
            Some(value.value)
        } else if let Some(folded) = info.floating
            && matches!(representation, Repr::Int { .. } | Repr::Bool)
        {
            // A floating constant truncates toward zero (§6.3.1.4p1); `_Bool`
            // compares with zero (§6.3.1.2p1).
            if representation == Repr::Bool {
                Some(i128::from(!folded.real.is_zero()))
            } else {
                Some(floats::to_integer(&folded.real).ok_or_else(unsupported)?)
            }
        } else {
            None
        };
        if let Some(value) = value {
            let value = match representation {
                | Repr::Bool => i128::from(value != 0),
                | Repr::Int { .. } | Repr::Pointer => {
                    let bits = u32::try_from(size * 8).map_err(|_| unsupported())?;
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
        if representation != Repr::Pointer || bit_field.is_some() {
            return Err(unsupported());
        }
        let (symbol, addend) = self.address_constant(expression)?;
        // Symbolic bytes replace any older scalar representation too;
        // that representation must not reappear if a later write drops the
        // relocation.
        bytes[start..start + size].fill(0);
        relocations.push(Relocation {
            offset,
            symbol,
            addend,
        });
        Ok(())
    }

    /// The symbol and byte offset an address constant designates: a static
    /// object, function or string literal, optionally with a constant
    /// subscript, member offset or integer added.
    /// C99: §6.6 paragraph 9, p. 96; PDF p. 108.
    fn address_constant(
        &mut self,
        mut expression: &'tu Expression<'tu>,
    ) -> Result<(Symbol, i64), LoweringError> {
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
            match expression.kind {
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
                    let binding = info.binding.ok_or_else(unsupported)?;
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
                    return Ok((symbol, addend));
                },
                | ExpressionType::StringLiteral(_) =>
                    return Ok((Symbol::Global(self.string(expression)?), addend)),
                | ExpressionType::DirectMember {
                    base_expression, ..
                } if place => {
                    let field = self
                        .sema
                        .selected_field(expression)
                        .ok_or_else(unsupported)?;
                    if field.width.is_some() {
                        return Err(unsupported());
                    }
                    addend += i64::try_from(field.offset).map_err(|_| unsupported())?;
                    expression = base_expression;
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
                    let size = self
                        .sema
                        .types
                        .layout(element)
                        .ok_or_else(unsupported)?
                        .size;
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
    let span = usize::try_from((field.bit_offset + field.width).div_ceil(8)).ok()?;
    let target = bytes.get_mut(start..start + span)?;
    let mut word = [0_u8; 16];
    word.get_mut(..span)?.copy_from_slice(target);
    let mut word = u128::from_le_bytes(word);
    let mask = (u128::MAX >> (128 - field.width)) << field.bit_offset;
    word = (word & !mask) | ((value as u128) << field.bit_offset & mask);
    target.copy_from_slice(&word.to_le_bytes()[..span]);
    Some(())
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

/// The IR linkage of a function or object with linkage.
/// C99: §6.2.2, pp. 30-31; PDF pp. 42-43.
fn linkage(binding: Binding) -> IrLinkage {
    match binding.linkage {
        | Linkage::Internal => IrLinkage::Internal,
        | Linkage::External | Linkage::None => IrLinkage::External,
    }
}

/// Whether every byte of an object of this type is const: its type or,
/// for an array, its element type is const-qualified.
/// C99: §6.7.3 paragraph 5, p. 109; PDF p. 121.
fn constant_object(
    types: &crate::translation_phases::semantic_analysis::Types<'_>,
    mut ty: TypeId,
) -> bool {
    loop {
        if ty.qualifiers.contains(TypeQualifiers::CONST) {
            return !ty.qualifiers.contains(TypeQualifiers::VOLATILE);
        }
        match types.kind(ty) {
            | TypeKind::Array(element, _) => ty = element,
            | _ => return false,
        }
    }
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
    ) -> Self {
        let mut module = Module::new(arena);
        module.set_target(target.triple(), types::data_layout(target));
        let mut declarators = ArenaMap::with_capacity_and_hasher_in(
            sema.bindings.len(),
            rustc_hash::FxBuildHasher,
            tables,
        );
        for (index, binding) in sema.bindings.iter().enumerate() {
            _ = declarators
                .entry((binding.name.name, binding.name.source_vectors))
                .or_insert(index);
        }
        let mut definitions = ArenaMap::with_capacity_and_hasher_in(
            sema.definitions.len(),
            rustc_hash::FxBuildHasher,
            tables,
        );
        for definition in sema.definitions {
            _ = definitions.insert(definition.binding, definition.kind);
        }
        let defined_objects = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tables);
        let mut defined_functions = ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, tables);
        for function in sema.functions {
            if let Some(binding) = function.binding {
                _ = defined_functions.insert(sema.bindings[binding].name.name);
            }
        }
        Self {
            context,
            sema,
            module,
            tables,
            target,
            names: SymbolNames::new(tables),
            declarators,
            definitions,
            defined_functions,
            defined_objects,
            address_taken: address_taken::find(sema, tables),
            statics: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tables),
            strings: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tables),
            next_string_suffix: 1,
            reserved_names: ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, tables),
            generated_globals: ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, tables),
            conflicting_names: ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, tables),
            next_name_suffix: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tables),
        }
    }
}
