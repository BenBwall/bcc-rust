//! Module-level items: the functions, objects and string literals of a
//! translation unit as IR functions and globals, and the bytes of static
//! initializers.
//!
//! Functions and objects with linkage are named by their identifier, so
//! every declaration of one linked entity reaches the same symbol (C99
//! §6.2.2 paragraph 2, p. 30; PDF p. 42). Static locals and string literals
//! have no linkage and get internal names that cannot clash with a C
//! identifier, such as `f.count` and `.str.1`.

use super::{
    Construct,
    LoweringError,
    LoweringErrorKind,
    address_taken,
    initializers::{
        self,
        Leaf,
    },
    types::{
        self,
        Repr,
        repr,
    },
};
use crate::{
    diagnostics::format_in,
    ir::{
        Align,
        FuncId,
        GlobalDecl,
        GlobalId,
        GlobalInit,
        Linkage as IrLinkage,
        Module,
        Relocation,
        SigId,
        Symbol,
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
    tables:                   &'a Bump,
    /// Each binding by its declared identifier, so a declarator finds its
    /// binding without a search.
    declarators:              ArenaMap<'a, (StringCacheId, SourceVectors), usize>,
    definitions:              ArenaMap<'a, usize, DefinitionKind>,
    /// Bindings whose address is taken, which therefore live in memory.
    pub(super) address_taken: ArenaSet<'a, usize>,
    /// Globals of static locals and `__func__`, which have no linkage.
    statics:                  ArenaMap<'a, usize, GlobalId>,
    strings:                  ArenaMap<'a, LiteralId, GlobalId>,
}

impl<'a, 'tu, 'ir> Unit<'a, 'tu, 'ir> {
    /// Declares every defined function and defines every file-scope object.
    /// Returns the function definitions in order, each with its function if
    /// it is to be lowered: inline definitions and unsupported signatures
    /// have none.
    pub(super) fn declare_items(
        &mut self,
        syntax: &ParsedTranslationUnit<'tu>,
        errors: &mut ArenaVec<'_, LoweringError>,
    ) -> ArenaVec<'a, (&'tu FunctionRecord<'tu>, Option<FuncId>)> {
        let mut functions = ArenaVec::with_capacity_in(self.sema.functions.len(), self.tables);
        for record in self.sema.functions {
            let func = self.declare_definition(record).unwrap_or_else(|error| {
                errors.push(error);
                None
            });
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
        let name = self.context.string_cache.at(data.name.name);
        if let Some(Symbol::Function(func)) = self.module.symbol(name) {
            return Ok(func);
        }
        if self.module.symbol(name).is_some() {
            return Err(LoweringErrorKind::MissingFact(
                "a function and an object share a name",
            ));
        }
        let signature = self.signature(data.ty)?;
        Ok(self.module.declare_function(name, signature, linkage(data)))
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
        let mut params = ArenaVec::with_capacity_in(parameters.len(), self.tables);
        for &parameter in parameters {
            match repr(&self.sema.types, parameter)? {
                | Repr::Aggregate =>
                    return Err(LoweringErrorKind::Unsupported(Construct::AggregateArgument)),
                | representation => params.push(representation.value_type().ok_or(
                    LoweringErrorKind::MissingFact("a parameter has no value type"),
                )?),
            }
        }
        let result = match repr(&self.sema.types, result)? {
            | Repr::Void => None,
            | Repr::Aggregate =>
                return Err(LoweringErrorKind::Unsupported(Construct::AggregateResult)),
            | representation => representation.value_type(),
        };
        Ok(self.module.intern_signature(&params, result, variadic))
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
        let name = self.context.string_cache.at(data.name.name);
        match self.module.symbol(name) {
            | Some(Symbol::Global(global)) => Ok(global),
            | Some(Symbol::Function(_)) => Err(LoweringErrorKind::MissingFact(
                "a function and an object share a name",
            )),
            | None => {
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
                let name = self.unique_name(base);
                let declaration = self
                    .global_declaration(data, IrLinkage::Internal)
                    .map_err(|kind| at(kind, source))?;
                let global = self.module.declare_global(name, declaration);
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
        global
    }

    /// The internal constant global holding a narrow string literal's
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
        let id = match literal {
            | StringTokenType::String(id) => id,
            | StringTokenType::WideString(_) =>
                return Err(LoweringError::unsupported(
                    Construct::WideString,
                    expression.source_vectors,
                )),
            | StringTokenType::EncodedString(..) =>
                return Err(LoweringError::unsupported(
                    Construct::EncodedString,
                    expression.source_vectors,
                )),
        };
        if let Some(&global) = self.strings.get(&id) {
            return Ok(global);
        }
        let bytes = self.string_bytes(id);
        let name = self.unique_name(".str");
        let global = self.module.declare_global(
            name,
            GlobalDecl {
                linkage:  IrLinkage::Internal,
                size:     bytes.len() as u64,
                align:    Align::BYTE,
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
        _ = self.strings.insert(id, global);
        Ok(global)
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

    /// `base`, or `base.N` for the first `N` that no symbol has.
    fn unique_name(&self, base: &'a str) -> &'a str {
        if self.module.symbol(base).is_none() {
            return base;
        }
        let mut suffix = 1_u32;
        loop {
            let name = format_in!(self.tables, "{base}.{suffix}");
            if self.module.symbol(name).is_none() {
                return name;
            }
            suffix += 1;
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
                | Leaf::Scalar {
                    offset,
                    ty,
                    expression,
                } => self.static_scalar(&mut bytes, &mut relocations, offset, ty, expression)?,
                | Leaf::String {
                    offset,
                    size,
                    literal,
                } => {
                    let ExpressionType::StringLiteral(StringTokenType::String(id)) =
                        peel(literal).kind
                    else {
                        return Err(LoweringError::unsupported(
                            Construct::WideString,
                            literal.source_vectors,
                        ));
                    };
                    let string = self.string_bytes(id);
                    let (start, size) = (index(offset, literal)?, index(size, literal)?);
                    let count = string.len().min(size);
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

    /// Writes one scalar leaf: an integer constant converted to the leaf's
    /// type, or an address constant as a relocation.
    fn static_scalar(
        &mut self,
        bytes: &mut [u8],
        relocations: &mut ArenaVec<'_, Relocation>,
        offset: u64,
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
        let (start, size) = (index(offset, expression)?, index(size, expression)?);
        if let Some(value) = info.integer
            && !matches!(
                self.sema.types.kind(info.ty),
                TypeKind::Pointer(_) | TypeKind::Array(..)
            )
        {
            let value = match representation {
                | Repr::Bool => i128::from(value.value != 0),
                | Repr::Int { .. } | Repr::Pointer => {
                    let bits = u32::try_from(size * 8).map_err(|_| unsupported())?;
                    value.cast(bits, representation.signed()).value
                },
                | _ => return Err(unsupported()),
            };
            bytes[start..start + size].copy_from_slice(&value.to_le_bytes()[..size]);
            return Ok(());
        }
        if representation != Repr::Pointer {
            return Err(unsupported());
        }
        let (symbol, addend) = self.address_constant(expression)?;
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
        Self {
            context,
            sema,
            module,
            tables,
            declarators,
            definitions,
            address_taken: address_taken::find(sema, tables),
            statics: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tables),
            strings: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, tables),
        }
    }
}
