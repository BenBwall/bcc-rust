//! Which calls do not return, as LLVM's `CallBase::doesNotReturn` decides
//! for the call Clang emits: the call site carries `noreturn` from the
//! callee's function type, and a direct call also from the called
//! function's declaration.
//!
//! Clang keeps two facts apart. GNU `noreturn`, `[[gnu::noreturn]]` and
//! `__declspec(noreturn)` are function type attributes, which typedefs,
//! `__typeof__`, pointers, arrays, members and return types carry, and which
//! a conditional keeps only when both operands have them. `_Noreturn` and
//! standard `[[noreturn]]` belong to the declaration, so only a direct call
//! sees them. A library function Clang's builtin table marks gets a
//! `noreturn` function type from Clang's implicit declaration, which every
//! later declaration merges with: `abort`, `exit`, `_Exit` and, in GNU
//! modes, `_exit` always; `longjmp` (and `_longjmp` in GNU modes) only when
//! a file-scope `jmp_buf` typedef precedes its first declaration, and
//! `siglongjmp` only after `sigjmp_buf`, since Clang cannot build their
//! builtin types otherwise. A freestanding unit has no library builtins.
//! C11: §6.7.4 paragraph 8, p. 126; PDF p. 144 (`_Noreturn`).
//! C99: §6.5.2.2 paragraph 1, p. 71; PDF p. 83; §6.5.15 paragraph 6,
//! pp. 90-91; PDF pp. 102-103 (composite types); §7.20.4.1 paragraph 2,
//! p. 315; PDF p. 327; §7.20.4.3 paragraph 6, p. 316; PDF p. 328;
//! §7.20.4.4 paragraph 2, p. 316; PDF p. 328; §7.13.2.1 paragraph 4, p. 244;
//! PDF p. 256.
//! GNU extension: GCC manual, "Common Function Attributes", `noreturn`.

use super::super::{
    Analyzer,
    BindingKind,
    BindingOrigin,
    ExtendedType,
    Linkage,
    ScopeKind,
    SpecifierExtension,
    SyntaxOperand,
    TypeId,
    TypeKind,
    scopes::Namespace,
};
use crate::{
    builtins::BuiltinId,
    translation_phases::parsing::{
        declaration_syntax::{
            DeclarationSpecifiers,
            Declarator,
            StructDeclaration,
            StructDeclarator,
            TypeName,
            TypeSpecifiers,
        },
        syntax::{
            BinaryOperator,
            BlockItem,
            Expression,
            ExpressionSlot,
            ExpressionType,
            StatementType,
            UnaryOperator,
        },
    },
    util::string_cache::StringCacheId,
};

/// How deep [`Analyzer::noreturn_type`] follows the operands of
/// conditionals; deeper ones count as returning.
const DEPTH: u32 = 64;

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Whether a call through `callee` does not return: a direct call of a
    /// function declared `noreturn` in any spelling, of a library function
    /// Clang marks, or of `__builtin_unreachable` or `__builtin_abort`,
    /// through casts too (the call then still names the function); or a
    /// call whose callee's function type is `noreturn`.
    pub(super) fn call_does_not_return(&mut self, callee: &'tu Expression<'tu>) -> bool {
        let mut designator = callee;
        while let ExpressionType::Parenthesized { expression }
        | ExpressionType::Unary {
            operator: UnaryOperator::Indirection | UnaryOperator::AddressOf,
            operand_expression: expression,
        }
        | ExpressionType::Cast {
            operand_expression: expression,
            ..
        } = designator.kind
        {
            designator = expression;
        }
        if matches!(designator.kind, ExpressionType::Identifier(_))
            && let Some(binding) = self.expression_info(designator).binding
        {
            let data = self.bindings[binding];
            match data.origin {
                | BindingOrigin::Intrinsic(id) =>
                    return matches!(id, BuiltinId::Unreachable | BuiltinId::LibraryAbort),
                | BindingOrigin::OpaqueIntrinsic => return false,
                | BindingOrigin::User | BindingOrigin::ImplicitOrdinary =>
                    if data.kind == BindingKind::Function
                        && (self.functions.emission.noreturn.contains_key(&binding)
                            || self.library_does_not_return(binding))
                    {
                        return true;
                    },
            }
        }
        self.noreturn_type(callee, 0)
    }

    /// Whether the function type that `expression` designates or points to
    /// is `noreturn`, as Clang's type of the expression carries it.
    fn noreturn_type(&mut self, expression: &'tu Expression<'tu>, depth: u32) -> bool {
        let mut expression = expression;
        loop {
            match expression.kind {
                | ExpressionType::Parenthesized { expression: inner }
                | ExpressionType::Unary {
                    operator:
                        UnaryOperator::Indirection | UnaryOperator::AddressOf | UnaryOperator::Extension,
                    operand_expression: inner,
                }
                | ExpressionType::Binary {
                    operator: BinaryOperator::Comma,
                    right_expression: inner,
                    ..
                }
                | ExpressionType::Binary {
                    operator: BinaryOperator::Assignment,
                    left_expression: inner,
                    ..
                } => expression = inner,
                | ExpressionType::Binary {
                    operator:
                        BinaryOperator::Subscript
                        | BinaryOperator::Addition
                        | BinaryOperator::Subtraction,
                    left_expression,
                    right_expression,
                } => {
                    let pointer = |analyzer: &Self, operand: &'tu Expression<'tu>| {
                        matches!(
                            analyzer.types.nodes[analyzer.expression_info(operand).ty.index],
                            TypeKind::Pointer(_) | TypeKind::Array(..)
                        )
                    };
                    expression = if pointer(self, left_expression) {
                        left_expression
                    } else {
                        right_expression
                    };
                },
                | ExpressionType::Conditional(conditional)
                | ExpressionType::OmittedConditional(conditional) =>
                    return depth < DEPTH
                        && self.noreturn_type(conditional.then_expression, depth + 1)
                        && self.noreturn_type(conditional.else_expression, depth + 1),
                | ExpressionType::Cast { target_type, .. }
                | ExpressionType::CompoundLiteral {
                    type_name: target_type,
                    ..
                } => return self.type_name_noreturn(target_type),
                | ExpressionType::Identifier(_) => {
                    let Some(binding) = self.expression_info(expression).binding else {
                        return false;
                    };
                    return self.functions.emission.noreturn.get(&binding) == Some(&true)
                        || (self.bindings[binding].kind == BindingKind::Function
                            && self.library_does_not_return(binding));
                },
                | ExpressionType::DirectMember {
                    base_expression,
                    member,
                }
                | ExpressionType::IndirectMember {
                    base_expression,
                    member,
                } => {
                    let mut ty = self.expression_info(base_expression).ty;
                    if matches!(expression.kind, ExpressionType::IndirectMember { .. })
                        && let TypeKind::Pointer(inner) = self.types.nodes[ty.index]
                    {
                        ty = inner;
                    }
                    let ty = self.types.non_atomic(ty);
                    return matches!(self.types.nodes[ty.index], TypeKind::Tag(tag)
                        if self.functions.emission.noreturn_fields.contains_key(&(tag, member.name)));
                },
                | ExpressionType::Call {
                    function_expression,
                    ..
                } => return self.returns_noreturn(function_expression),
                | ExpressionType::StatementExpression(block) => {
                    let StatementType::Compound { items } = block.kind else {
                        return false;
                    };
                    let Some(&BlockItem::Statement(last)) = items.last() else {
                        return false;
                    };
                    let StatementType::Expression(ExpressionSlot::Parsed(value)) = last.kind else {
                        return false;
                    };
                    expression = value;
                },
                | ExpressionType::Generic(_) | ExpressionType::Builtin(_) => {
                    match self.expression_info(expression).selected_expression {
                        | Some(selected) if !std::ptr::eq(selected, expression) =>
                            expression = selected,
                        | _ => return false,
                    }
                },
                | _ => return false,
            }
        }
    }

    /// Whether the function `callee` calls returns a pointer to a
    /// `noreturn` function type: a function declared with such a result.
    fn returns_noreturn(&self, callee: &'tu Expression<'tu>) -> bool {
        let mut callee = callee;
        while let ExpressionType::Parenthesized { expression }
        | ExpressionType::Unary {
            operator: UnaryOperator::Indirection | UnaryOperator::AddressOf,
            operand_expression: expression,
        } = callee.kind
        {
            callee = expression;
        }
        matches!(callee.kind, ExpressionType::Identifier(_))
            && self.expression_info(callee).binding.is_some_and(|binding| {
                self.functions
                    .emission
                    .noreturn_results
                    .contains_key(&binding)
            })
    }

    /// Whether type name `name` is a `noreturn` function type, or a pointer
    /// to or array of one.
    fn type_name_noreturn(&mut self, name: &'tu TypeName<'tu>) -> bool {
        let Some(&ty) = self
            .resolved_type_names
            .get(&super::super::type_name_identity(name))
        else {
            return false;
        };
        self.noreturn_type_named(
            name.declaration_specifiers,
            name.declaration_specifiers.extensions,
            name.declarator,
        ) && self.holds_function_type(ty)
    }

    /// Whether `binding` is a library function Clang marks as not returning
    /// whatever its declarations say: `abort`, `exit`, `_Exit` and, in GNU
    /// modes, `_exit`. The `longjmp` family is recorded with its first
    /// declaration instead ([`Self::record_noreturn`]).
    fn library_does_not_return(&self, binding: usize) -> bool {
        let data = self.bindings[binding];
        if !self.context.configuration.hosted() || data.linkage != Linkage::External {
            return false;
        }
        match self.context.string_cache.at(data.name.name) {
            | "abort" | "exit" | "_Exit" => true,
            | "_exit" => self.context.configuration.gnu_extensions(),
            | _ => false,
        }
    }

    /// Records whether the declaration of `binding` says that what it calls
    /// does not return ([`State::noreturn`](super::State)), merging an
    /// earlier declaration of the same entity (`previous`), as Clang merges
    /// the attribute and the function type's bit. Clang gives a type
    /// attribute to the innermost function type, which is the result's for
    /// a function that returns a function pointer. A first declaration of
    /// `longjmp`, `_longjmp` or `siglongjmp` after its buffer typedef gets
    /// Clang's builtin type.
    pub(in crate::translation_phases::semantic_analysis) fn record_noreturn(
        &mut self,
        binding: usize,
        spec: DeclarationSpecifiers<'tu>,
        declarator: Declarator<'tu>,
        previous: Option<usize>,
    ) {
        let data = self.bindings[binding];
        if data.kind == BindingKind::Typedef && self.scopes[data.scope].kind == ScopeKind::File {
            match self.context.string_cache.at(data.name.name) {
                | "jmp_buf" => self.functions.emission.jmp_buf = true,
                | "sigjmp_buf" => self.functions.emission.sigjmp_buf = true,
                | _ => {},
            }
        }
        if !matches!(
            data.kind,
            BindingKind::Function | BindingKind::Object | BindingKind::Typedef
        ) {
            return;
        }
        let inherited =
            previous.and_then(|previous| self.functions.emission.noreturn.get(&previous).copied());
        let result_inherited = previous.is_some_and(|previous| {
            self.functions
                .emission
                .noreturn_results
                .contains_key(&previous)
        });
        // The type walks run only when the declaration names the fact, so
        // deep pointer chains cost nothing elsewhere.
        let named = self.noreturn_type_named(spec, spec.extensions, Some(declarator))
            && self.holds_function_type(data.ty);
        let result = match self.types.nodes[data.ty.index] {
            | TypeKind::Function { result, .. } => Some(result),
            | _ => None,
        };
        let returns_function = result.is_some_and(|result| self.holds_function_type(result));
        let builtin = data.kind == BindingKind::Function
            && previous.is_none()
            && self.builtin_longjmp(binding);
        let typed = inherited == Some(true) || builtin || (named && !returns_function);
        let declared = inherited.is_some()
            || (data.kind == BindingKind::Function
                && (spec.function_specifiers.is_noreturn
                    || self.noreturn_declared(spec.extensions, Some(declarator))));
        if typed || declared {
            _ = self.functions.emission.noreturn.insert(binding, typed);
        }
        if result_inherited || (named && returns_function) {
            _ = self.functions.emission.noreturn_results.insert(binding, ());
        }
    }

    /// Whether `binding` is a first, hosted declaration of the `longjmp`
    /// family that Clang merges with its builtin, which needs its buffer
    /// typedef declared first at file scope.
    fn builtin_longjmp(&self, binding: usize) -> bool {
        let data = self.bindings[binding];
        if !self.context.configuration.hosted() || data.linkage != Linkage::External {
            return false;
        }
        let emission = &self.functions.emission;
        match self.context.string_cache.at(data.name.name) {
            | "longjmp" => emission.jmp_buf,
            | "_longjmp" => emission.jmp_buf && self.context.configuration.gnu_extensions(),
            | "siglongjmp" => emission.sigjmp_buf && self.context.configuration.gnu_extensions(),
            | _ => false,
        }
    }

    /// Records a prototype parameter whose function pointer type does not
    /// return; the body's parameter binding copies the fact.
    pub(in crate::translation_phases::semantic_analysis) fn record_parameter_noreturn(
        &mut self,
        binding: usize,
        spec: DeclarationSpecifiers<'tu>,
        declarator: Option<Declarator<'tu>>,
    ) {
        if self.noreturn_type_named(spec, spec.extensions, declarator)
            && self.holds_function_type(self.bindings[binding].ty)
        {
            _ = self.functions.emission.noreturn.insert(binding, true);
        }
    }

    /// Records member `name` of tag `tag`, of type `ty`, when its function
    /// type does not return.
    pub(in crate::translation_phases::semantic_analysis) fn record_field_noreturn(
        &mut self,
        tag: usize,
        name: StringCacheId,
        declarator: StructDeclarator<'tu>,
        declaration: StructDeclaration<'tu>,
        ty: TypeId,
    ) {
        let spec = DeclarationSpecifiers {
            type_specifiers: declaration.type_specifiers,
            ..DeclarationSpecifiers::default()
        };
        if (self.noreturn_type_named(spec, declaration.extensions, declarator.declarator)
            || self.noreturn_written(declarator.attributes, None))
            && self.holds_function_type(ty)
        {
            _ = self
                .functions
                .emission
                .noreturn_fields
                .insert((tag, name), ());
        }
    }

    /// Gives member `name` of tag `outer` the fact of the same member of the
    /// anonymous member of type `inner` it comes from.
    pub(in crate::translation_phases::semantic_analysis) fn inherit_field_noreturn(
        &mut self,
        inner: usize,
        outer: usize,
        name: StringCacheId,
    ) {
        if self
            .functions
            .emission
            .noreturn_fields
            .contains_key(&(inner, name))
        {
            _ = self
                .functions
                .emission
                .noreturn_fields
                .insert((outer, name), ());
        }
    }

    /// Whether a declaration names a `noreturn` function type: by a type
    /// attribute in `extensions` or `declarator`, or by the typedef name or
    /// `__typeof__` operand of its specifiers. The declared type must still
    /// hold a function type for the fact to apply.
    fn noreturn_type_named(
        &mut self,
        spec: DeclarationSpecifiers<'tu>,
        extensions: Option<&'tu SpecifierExtension<'tu>>,
        declarator: Option<Declarator<'tu>>,
    ) -> bool {
        if self.noreturn_written(extensions, declarator) {
            return true;
        }
        match spec.type_specifiers {
            | TypeSpecifiers::TypedefName(name) => self
                .lookup(Namespace::Ordinary, name.name)
                .is_some_and(|entry| {
                    self.functions.emission.noreturn.get(&entry.binding) == Some(&true)
                }),
            | TypeSpecifiers::Extended(&ExtendedType::Typeof { operand, .. }) => match operand {
                | SyntaxOperand::Expression(operand) => self.noreturn_type(operand, 0),
                | SyntaxOperand::Type(name) => self.type_name_noreturn(name),
            },
            | _ => false,
        }
    }

    /// Whether a `noreturn` type attribute (GNU, `gnu::` or `__declspec`)
    /// is written in `extensions` or `declarator`.
    fn noreturn_written(
        &self,
        extensions: Option<&'tu SpecifierExtension<'tu>>,
        declarator: Option<Declarator<'tu>>,
    ) -> bool {
        self.noreturn_spellings(extensions, declarator).0
    }

    /// Whether a standard `[[noreturn]]` or `[[_Noreturn]]` declaration
    /// attribute is written in `extensions` or `declarator`.
    fn noreturn_declared(
        &self,
        extensions: Option<&'tu SpecifierExtension<'tu>>,
        declarator: Option<Declarator<'tu>>,
    ) -> bool {
        self.noreturn_spellings(extensions, declarator).1
    }

    /// The `noreturn` attributes written in `extensions` or `declarator`:
    /// whether a type attribute and whether a declaration attribute is.
    fn noreturn_spellings(
        &self,
        extensions: Option<&'tu SpecifierExtension<'tu>>,
        declarator: Option<Declarator<'tu>>,
    ) -> (bool, bool) {
        if extensions.is_none()
            && declarator
                .is_none_or(|declarator| !super::super::attributes::written_on(None, declarator))
        {
            return (false, false);
        }
        let mut spellings = (false, false);
        for list in self.declaration_attribute_lists(extensions, declarator) {
            if !list.recovered {
                let (typed, declared) =
                    super::super::attributes::noreturn_spelling(list, self.context);
                spellings.0 |= typed;
                spellings.1 |= declared;
            }
        }
        spellings
    }

    /// Whether `ty` is a function type, or a pointer to or array of one.
    pub(super) fn holds_function_type(&self, ty: TypeId) -> bool {
        let mut ty = ty;
        loop {
            match self.types.nodes[self.types.non_atomic(ty).index] {
                | TypeKind::Function { .. } => return true,
                | TypeKind::Pointer(inner) | TypeKind::Array(inner, _) => ty = inner,
                | _ => return false,
            }
        }
    }
}
