//! How a declaration wrote a type, which `cl` names in its conversion
//! warnings: the typedef name its specifiers use with their qualifiers
//! (`volatile Byte`), or the basic type in `cl`'s words (`unsigned __int64`),
//! once an operand has gone through every derivation its declarator adds.
//! bcc's types keep no typedef names, so each binding and member records its
//! declaration under the `cl` identity, and an operand whose spelling this
//! cannot tell names nothing: `cl` would name a type bcc cannot.
//! C99: §6.7.7 paragraph 3, p. 123; PDF p. 135 (a typedef name is a synonym);
//! §6.7.5 paragraph 4, p. 115; PDF p. 127 (derived declarator types).

use crate::{
    translation_phases::{
        parsing::{
            declaration_syntax::{
                DeclarationSpecifiers,
                Declarator,
                DirectDeclarator,
                ParameterDeclaration,
                StructDeclaration,
                TypeQualifiers,
                TypeSpecifiers,
            },
            syntax::{
                BinaryOperator as B,
                Constant,
                Expression,
                ExpressionType as E,
                UnaryOperator as U,
            },
        },
        preprocessing::IntegerTokenType,
        semantic_analysis::{
            Analyzer,
            BindingKind,
            types::{
                Scalar,
                TypeId,
                TypeKind,
            },
        },
    },
    util::{
        arena_list::ArenaList,
        bump::ArenaVec,
        string_cache::StringCacheId,
    },
};

/// A type as a declaration wrote it: the base its specifiers name, their
/// qualifiers, and the declarator that derives the declared type from it.
#[derive(Clone, Copy, Debug)]
pub(in crate::translation_phases::semantic_analysis) struct Written<'tu> {
    base:       Base,
    qualifiers: TypeQualifiers,
    declarator: Option<Declarator<'tu>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Base {
    /// A basic type, which `cl` names in its own words.
    Basic,
    /// A typedef name, which `cl` keeps.
    Typedef(StringCacheId),
    /// Something this does not spell: a tag, `typeof`, an attribute.
    Unknown,
}

/// One derivation a declarator adds to its base type, outermost first.
#[derive(Clone, Copy)]
enum Derivation<'tu> {
    Pointer,
    Array,
    /// A function, with its parameter declarations unless it has none.
    Function(Option<ArenaList<'tu, ParameterDeclaration<'tu>>>),
}

/// The subobject an initializer converts its value to, whose written type
/// names the target of C4244 and C4305: the declared object, or a member,
/// either `depth` array elements in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::translation_phases::semantic_analysis) enum ClTarget {
    #[default]
    Unknown,
    Binding(usize, u8),
    Member(usize, StringCacheId, u8),
}

impl ClTarget {
    /// The target one array element further in.
    pub(in crate::translation_phases::semantic_analysis) const fn element(self) -> Self {
        match self {
            | Self::Binding(binding, depth) if depth < u8::MAX => Self::Binding(binding, depth + 1),
            | Self::Member(tag, name, depth) if depth < u8::MAX => Self::Member(tag, name, depth + 1),
            | _ => Self::Unknown,
        }
    }
}

impl<'tu> Written<'tu> {
    /// The type declaration specifiers and a declarator write.
    pub(in crate::translation_phases::semantic_analysis) fn declared(
        specifiers: &DeclarationSpecifiers<'tu>,
        declarator: Option<Declarator<'tu>>,
    ) -> Self {
        let base = if specifiers.extensions.is_some() {
            Base::Unknown
        } else {
            Self::base(specifiers.type_specifiers)
        };
        Self {
            base,
            qualifiers: specifiers.type_qualifiers,
            declarator,
        }
    }

    /// The type a member declaration writes for one of its declarators.
    pub(in crate::translation_phases::semantic_analysis) fn member(
        member: &StructDeclaration<'tu>,
        declarator: Option<Declarator<'tu>>,
    ) -> Self {
        let base = if member.extensions.is_some() {
            Base::Unknown
        } else {
            Self::base(member.type_specifiers)
        };
        Self {
            base,
            qualifiers: member.type_qualifiers,
            declarator,
        }
    }

    const fn base(specifiers: TypeSpecifiers<'tu>) -> Base {
        match specifiers {
            | TypeSpecifiers::TypedefName(name) => Base::Typedef(name.name),
            | TypeSpecifiers::Empty
            | TypeSpecifiers::Extended(_)
            | TypeSpecifiers::StructOrUnion(_)
            | TypeSpecifiers::Enum(_) => Base::Unknown,
            | _ => Base::Basic,
        }
    }
}

impl<'tu, 's> Analyzer<'_, 'tu, 's> {
    /// Records how the declaration of the latest binding wrote its type.
    pub(in crate::translation_phases::semantic_analysis) fn record_written_binding(
        &mut self,
        specifiers: &DeclarationSpecifiers<'tu>,
        declarator: Option<Declarator<'tu>>,
    ) {
        if self.unused.cl && !self.bindings.is_empty() {
            _ = self.cl_written.insert(
                self.bindings.len() - 1,
                Written::declared(specifiers, declarator),
            );
        }
    }

    /// `cl`'s spelling of the type `e` has, if this can tell it: `None` for
    /// one it names by a typedef name bcc did not see, and for `size_t`,
    /// which `cl` reports apart (C4267).
    pub(in crate::translation_phases::semantic_analysis) fn cl_spelling(
        &self,
        e: &'tu Expression<'tu>,
    ) -> Option<&'s str> {
        let ty = self.types.non_atomic(self.expression_info(e).ty).unqualified();
        match e.kind {
            | E::Parenthesized { expression } => self.cl_spelling(expression),
            | E::Identifier(_) => {
                let binding = self.expression_info(e).binding?;
                if self.bindings[binding].kind == BindingKind::Enumerator {
                    return self.cl_basic(ty, TypeQualifiers::empty());
                }
                self.written_spelling(*self.cl_written.get(&binding)?, 0, ty)
            },
            | E::DirectMember { .. }
            | E::IndirectMember { .. }
            | E::Call { .. }
            | E::Unary {
                operator: U::Indirection,
                ..
            }
            | E::Binary {
                operator: B::Subscript,
                ..
            } => {
                let (written, depth) = self.written_operand(e)?;
                self.written_spelling(written, depth, ty)
            },
            | E::Cast { target_type, .. } => self.written_spelling(
                Written::declared(&target_type.declaration_specifiers, target_type.declarator),
                0,
                ty,
            ),
            | E::Constant(Constant::Integer(IntegerTokenType::LongLong(value)))
                if (u64::from(u32::MAX >> 1) + 1..=u64::from(u32::MAX))
                    .contains(&value.get().cast_unsigned()) =>
                None,
            | E::Constant(_) => self.cl_basic(ty, TypeQualifiers::empty()),
            // An assignment has its left operand's type, and a comma
            // expression its right operand's.
            | E::Binary {
                operator:
                    B::Assignment
                    | B::MultiplicationAssignment
                    | B::DivisionAssignment
                    | B::ModuloAssignment
                    | B::AdditionAssignment
                    | B::SubtractionAssignment
                    | B::LeftShiftAssignment
                    | B::RightShiftAssignment
                    | B::BitwiseAndAssignment
                    | B::BitwiseXorAssignment
                    | B::BitwiseOrAssignment,
                left_expression,
                ..
            } => self.cl_spelling(left_expression),
            | E::Binary {
                operator: B::Comma,
                right_expression,
                ..
            } => self.cl_spelling(right_expression),
            | E::Binary {
                operator:
                    B::LessThan
                    | B::GreaterThan
                    | B::LessThanOrEqual
                    | B::GreaterThanOrEqual
                    | B::Equal
                    | B::NotEqual
                    | B::LogicalAnd
                    | B::LogicalOr,
                ..
            }
            | E::Unary {
                operator: U::LogicalNot,
                ..
            } => self.cl_basic(ty, TypeQualifiers::empty()),
            | E::Binary {
                operator: B::LeftShift | B::RightShift,
                left_expression,
                ..
            } => self.operand_spelling(&[left_expression], ty),
            | E::Binary {
                left_expression,
                right_expression,
                ..
            } => self.operand_spelling(&[left_expression, right_expression], ty),
            | E::Conditional(conditional) => self.operand_spelling(
                &[conditional.then_expression, conditional.else_expression],
                ty,
            ),
            | E::Unary {
                operator:
                    U::Plus
                    | U::Minus
                    | U::BitwiseNot
                    | U::PreIncrement
                    | U::PreDecrement
                    | U::PostIncrement
                    | U::PostDecrement,
                operand_expression,
            } => self.operand_spelling(&[operand_expression], ty),
            | _ => None,
        }
    }

    /// The spelling of an operation's result of type `ty` from its
    /// `operands`: that of the operands that already have the type, which
    /// must agree, or the basic type when none has it, as after a
    /// promotion. `cl` keeps the left operand's name for some operands of
    /// one type and the right one's for others, which this does not tell.
    fn operand_spelling(&self, operands: &[&'tu Expression<'tu>], ty: TypeId) -> Option<&'s str> {
        let mut spelling = None;
        let mut typed = false;
        for &operand in operands {
            let operand_ty = self
                .types
                .non_atomic(self.expression_info(operand).ty)
                .unqualified();
            if operand_ty != ty {
                continue;
            }
            let own = self.cl_spelling(operand)?;
            if typed && spelling.is_some_and(|spelling| spelling != own) {
                return None;
            }
            typed = true;
            spelling = Some(own);
        }
        if typed {
            spelling
        } else {
            self.cl_basic(ty, TypeQualifiers::empty())
        }
    }

    /// `cl`'s spelling of the lvalue `e`, the target of an assignment.
    pub(in crate::translation_phases::semantic_analysis) fn cl_lvalue_spelling(
        &self,
        e: &'tu Expression<'tu>,
    ) -> Option<&'s str> {
        self.cl_spelling(e)
    }

    /// `cl`'s spelling of the type of the subobject `target` names, of type
    /// `ty`.
    pub(in crate::translation_phases::semantic_analysis) fn cl_target_spelling(
        &self,
        target: ClTarget,
        ty: TypeId,
    ) -> Option<&'s str> {
        let (written, depth) = match target {
            | ClTarget::Unknown => return None,
            | ClTarget::Binding(binding, depth) => (*self.cl_written.get(&binding)?, depth),
            | ClTarget::Member(tag, name, depth) =>
                (*self.cl_written_fields.get(&(tag, name))?, depth),
        };
        let ty = self.types.non_atomic(ty).unqualified();
        self.written_spelling(written, usize::from(depth), ty)
    }

    /// `cl`'s spelling of the parameter `index` of the function `callee`
    /// calls, of type `ty`.
    pub(in crate::translation_phases::semantic_analysis) fn cl_parameter_spelling(
        &self,
        callee: &'tu Expression<'tu>,
        index: usize,
        ty: TypeId,
    ) -> Option<&'s str> {
        let (written, depth) = self.written_operand(callee)?;
        let (_, _, parameters) = self.called_function(written, depth)?;
        let parameter = parameters?.as_slice().get(index)?;
        let written = Written::declared(&parameter.declaration_specifiers, parameter.declarator);
        self.written_spelling(written, 0, self.types.non_atomic(ty).unqualified())
    }

    /// `cl`'s spelling of the result type of the function being defined.
    pub(in crate::translation_phases::semantic_analysis) fn cl_result_spelling(
        &self,
        ty: TypeId,
    ) -> Option<&'s str> {
        let function = self.functions.current?;
        let written = Written::declared(
            &function.syntax.declaration_specifiers,
            Some(function.syntax.declarator),
        );
        self.written_spelling(written, 1, self.types.non_atomic(ty).unqualified())
    }

    /// The declaration that wrote the type of the designator under `e`, and
    /// how many of its derivations `e` has gone through: an object or
    /// member, through `*`, `[]` and calls.
    fn written_operand(&self, e: &'tu Expression<'tu>) -> Option<(Written<'tu>, usize)> {
        match e.kind {
            | E::Parenthesized { expression } => self.written_operand(expression),
            | E::Identifier(_) => {
                let binding = self.expression_info(e).binding?;
                Some((*self.cl_written.get(&binding)?, 0))
            },
            | E::DirectMember {
                base_expression,
                member,
            }
            | E::IndirectMember {
                base_expression,
                member,
            } => {
                let mut ty = self.types.non_atomic(self.expression_info(base_expression).ty);
                if matches!(e.kind, E::IndirectMember { .. }) {
                    ty = match self.types.nodes[ty.index] {
                        | TypeKind::Pointer(target) | TypeKind::Array(target, _) => target,
                        | _ => return None,
                    };
                }
                let TypeKind::Tag(tag) = self.types.nodes[self.types.non_atomic(ty).index] else {
                    return None;
                };
                let tag = self.declaring_tag(tag, member.name)?;
                Some((*self.cl_written_fields.get(&(tag, member.name))?, 0))
            },
            | E::Unary {
                operator: U::Indirection,
                operand_expression,
            } => {
                let (written, depth) = self.written_operand(operand_expression)?;
                Some((written, depth + 1))
            },
            // A pointer stepped keeps its type.
            | E::Unary {
                operator:
                    U::PreIncrement | U::PreDecrement | U::PostIncrement | U::PostDecrement,
                operand_expression,
            } if self.cl_pointer(operand_expression) => self.written_operand(operand_expression),
            | E::Binary {
                operator: B::Addition | B::Subtraction,
                left_expression,
                right_expression,
            } =>
                if self.cl_pointer(left_expression) {
                    self.written_operand(left_expression)
                } else if self.cl_pointer(right_expression) {
                    self.written_operand(right_expression)
                } else {
                    None
                },
            | E::Binary {
                operator: B::Subscript,
                left_expression,
                right_expression,
            } => {
                let pointer = |operand: &'tu Expression<'tu>| {
                    matches!(
                        self.types.nodes[self
                            .types
                            .non_atomic(self.expression_info(operand).ty)
                            .index],
                        TypeKind::Pointer(_) | TypeKind::Array(..)
                    )
                };
                let base = if pointer(left_expression) {
                    left_expression
                } else if pointer(right_expression) {
                    right_expression
                } else {
                    return None;
                };
                let (written, depth) = self.written_operand(base)?;
                Some((written, depth + 1))
            },
            // A call goes through the function the callee designates, and
            // through the pointer that reaches it.
            | E::Call {
                function_expression,
                ..
            } => {
                let (written, depth) = self.written_operand(function_expression)?;
                let (written, index, _) = self.called_function(written, depth)?;
                Some((written, index + 1))
            },
            | _ => None,
        }
    }

    /// The function an operand that has gone through `depth` derivations of
    /// `written` calls: the declaration that writes it, through typedef
    /// names, its index among that declaration's derivations, and its
    /// parameter declarations.
    #[expect(
        clippy::type_complexity,
        reason = "The declaration, the position and the parameters go together."
    )]
    fn called_function(
        &self,
        mut written: Written<'tu>,
        mut depth: usize,
    ) -> Option<(Written<'tu>, usize, Option<ArenaList<'tu, ParameterDeclaration<'tu>>>)> {
        loop {
            let mut derivations = ArenaVec::new_in(self.scratch);
            if !Self::derivations(written.declarator, &mut derivations) {
                return None;
            }
            if let Some((index, parameters)) =
                derivations
                    .iter()
                    .enumerate()
                    .skip(depth)
                    .find_map(|(index, derivation)| match *derivation {
                        | Derivation::Function(parameters) => Some((index, parameters)),
                        | _ => None,
                    })
            {
                return Some((written, index, parameters));
            }
            depth = depth.saturating_sub(derivations.len());
            written = self.typedef_written(written)?;
        }
    }

    /// The declaration of the typedef name `written`'s specifiers use, for
    /// an operand that goes through more derivations than its own
    /// declarator adds.
    fn typedef_written(&self, written: Written<'tu>) -> Option<Written<'tu>> {
        let Base::Typedef(name) = written.base else {
            return None;
        };
        let entry = self.lookup(super::super::Namespace::Ordinary, name)?;
        if self.bindings[entry.binding].kind != BindingKind::Typedef {
            return None;
        }
        self.cl_written.get(&entry.binding).copied()
    }

    /// Whether `e` has a pointer or an array type.
    fn cl_pointer(&self, e: &'tu Expression<'tu>) -> bool {
        matches!(
            self.types.nodes[self.types.non_atomic(self.expression_info(e).ty).index],
            TypeKind::Pointer(_) | TypeKind::Array(..)
        )
    }

    /// The tag that declares the member `name` of the record `tag`, through
    /// its anonymous members.
    fn declaring_tag(&self, tag: usize, name: StringCacheId) -> Option<usize> {
        let &index = self.member_indices.get(&(tag, name))?;
        let field = *self.types.tags[tag].fields.get().get(index)?;
        let mut declaring = tag;
        let mut path = field.path;
        while let Some(next) = path.next {
            let member = self.types.tags[declaring].members.get()[path.index];
            if let TypeKind::Tag(inner) = self.types.nodes[member.ty.index] {
                declaring = inner;
            }
            path = next;
        }
        Some(declaring)
    }

    /// The spelling of `written` once an operand of type `ty` has gone
    /// through `depth` of its derivations, which must be all of them.
    fn written_spelling(
        &self,
        mut written: Written<'tu>,
        mut depth: usize,
        ty: TypeId,
    ) -> Option<&'s str> {
        loop {
            let mut derivations = ArenaVec::new_in(self.scratch);
            if !Self::derivations(written.declarator, &mut derivations) || depth < derivations.len() {
                return None;
            }
            if depth == derivations.len() {
                break;
            }
            depth -= derivations.len();
            written = self.typedef_written(written)?;
        }
        match written.base {
            | Base::Unknown => None,
            | Base::Basic => self.cl_basic(ty, written.qualifiers),
            | Base::Typedef(name) => {
                let name = self.context.string_cache.at(name);
                if name == "size_t" {
                    return None;
                }
                let qualifiers = Self::qualifier_words(written.qualifiers)?;
                Some(crate::diagnostics::format_in!(self.scratch, "{qualifiers}{name}"))
            },
        }
    }

    /// The derivations `declarator` adds, outermost first, into `out`;
    /// `false` for one this does not read, as one with attributes, which
    /// may change the type.
    fn derivations(declarator: Option<Declarator<'tu>>, out: &mut ArenaVec<'_, Derivation<'tu>>) -> bool {
        let Some(declarator) = declarator else {
            return true;
        };
        for direct in declarator.kind.as_slice() {
            match *direct {
                | DirectDeclarator::Identifier(_)
                | DirectDeclarator::MsModifier(..)
                | DirectDeclarator::AsmLabel(_) => {},
                | DirectDeclarator::Parenthesized(inner) =>
                    if !Self::derivations(Some(inner.declarator), out) {
                        return false;
                    },
                | DirectDeclarator::Attributes(_) => return false,
                | DirectDeclarator::KAndRStyleFunction { .. } => out.push(Derivation::Function(None)),
                | DirectDeclarator::Array { .. } => out.push(Derivation::Array),
                | DirectDeclarator::Function { parameter_list, .. } =>
                    out.push(Derivation::Function(Some(parameter_list))),
            }
        }
        for _ in declarator.pointer.levels.as_slice() {
            out.push(Derivation::Pointer);
        }
        true
    }

    /// `cl`'s words for the basic type `ty` with the `qualifiers` written
    /// on it: `__int64` for `long long`, `bool` for `_Bool`.
    pub(in crate::translation_phases::semantic_analysis) fn cl_basic(
        &self,
        ty: TypeId,
        qualifiers: TypeQualifiers,
    ) -> Option<&'s str> {
        let TypeKind::Scalar(scalar) = self.types.nodes[ty.index] else {
            return None;
        };
        let name = match scalar {
            | Scalar::Bool => "bool",
            | Scalar::Char => "char",
            | Scalar::SignedChar => "signed char",
            | Scalar::UnsignedChar => "unsigned char",
            | Scalar::Short => "short",
            | Scalar::UnsignedShort => "unsigned short",
            | Scalar::Int => "int",
            | Scalar::UnsignedInt => "unsigned int",
            | Scalar::Long => "long",
            | Scalar::UnsignedLong => "unsigned long",
            | Scalar::LongLong => "__int64",
            | Scalar::UnsignedLongLong => "unsigned __int64",
            | Scalar::Float => "float",
            | Scalar::Double => "double",
            | Scalar::LongDouble => "long double",
            | _ => return None,
        };
        let qualifiers = Self::qualifier_words(qualifiers)?;
        Some(crate::diagnostics::format_in!(self.scratch, "{qualifiers}{name}"))
    }

    /// The qualifiers `cl` writes before a type's name, each with a space
    /// after it; `None` for one it is not seen to write.
    fn qualifier_words(qualifiers: TypeQualifiers) -> Option<&'static str> {
        let known = TypeQualifiers::CONST | TypeQualifiers::VOLATILE;
        if !known.contains(qualifiers) {
            return None;
        }
        Some(
            match (
                qualifiers.contains(TypeQualifiers::CONST),
                qualifiers.contains(TypeQualifiers::VOLATILE),
            ) {
                | (false, false) => "",
                | (true, false) => "const ",
                | (false, true) => "volatile ",
                | (true, true) => "const volatile ",
            },
        )
    }
}
