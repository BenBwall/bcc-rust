//! GCC's, Clang's and TCC's words for the pointer conversion constraints they
//! report by default (`-Wint-conversion`, `-Wincompatible-pointer-types`
//! and Clang's `-Wincompatible-function-pointer-types`), in their
//! diagnostic formats: each names the conversion's context and both types,
//! as each compiler spells them. bcc models no typedef names, so a type is
//! spelled from its canonical form, where both compilers would name a
//! typedef; an unnamed tag or a vector type keeps bcc's own words.
//! C99: §6.5.16.1p1, p. 92; PDF p. 104; §6.5.2.2p7, p. 72; PDF p. 84;
//! §6.8.6.4p3, p. 139; PDF p. 151; §6.7.8p11, p. 126; PDF p. 138.

use super::{
    Analyzer,
    ExpressionInfo,
    errors::{
        ConversionEdit,
        ConversionFixIt,
    },
    expressions::ValueCategory,
    types::{
        ArrayBound,
        Scalar,
        TagKind,
        TypeId,
        TypeKind,
    },
};
use crate::{
    configuration::{
        CStandard,
        DiagnosticFormat,
        DiagnosticGroup,
    },
    translation_phases::parsing::{
        declaration_syntax::TypeQualifiers,
        syntax::{
            BinaryOperator,
            Expression,
            ExpressionType,
            UnaryOperator,
        },
    },
    util::{
        bump::ArenaString,
        string_cache::StringCacheId,
    },
};

/// Where an assignment-like conversion happens, which GCC's and Clang's
/// words name.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ConversionSite {
    /// A builtin's operand, or a context neither compiler words apart.
    Other,
    /// C99: §6.7.8p11, p. 126; PDF p. 138.
    Initialization,
    /// C99: §6.5.16.1p1, p. 92; PDF p. 104.
    Assignment,
    /// The argument's 1-based position and the called function's name.
    /// C99: §6.5.2.2p7, p. 72; PDF p. 84.
    Argument {
        position: usize,
        callee:   Option<StringCacheId>,
    },
    /// C99: §6.8.6.4p3, p. 139; PDF p. 151.
    Return,
}

/// How a program uses the value of a void expression, which GCC's and
/// Clang's words tell apart.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum VoidUse {
    /// C99: §6.7.8p11, p. 126; PDF p. 138.
    Initialization,
    /// C99: §6.5.16.1p1, p. 92; PDF p. 104.
    Assignment,
    /// C99: §6.5.4p2, p. 81; PDF p. 93.
    Cast,
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Whether `ty` is `void`, an expression of which has no value.
    /// C99: §6.3.2.2p1, p. 46; PDF p. 58.
    pub(super) fn void_value(&self, ty: TypeId) -> bool {
        matches!(self.types.nodes[ty.index], TypeKind::Scalar(Scalar::Void))
    }

    /// The text of an error that uses a void expression's value, in GCC's,
    /// Clang's or TCC's words, interned, when the diagnostic format is one
    /// of theirs and the target type can be spelled.
    /// C99: §6.3.2.2p1, p. 46; PDF p. 58.
    pub(super) fn void_value_wording(
        &mut self,
        use_: VoidUse,
        target: TypeId,
    ) -> Option<StringCacheId> {
        let format = self.context.configuration.diagnostic_format();
        let text = match (format, use_) {
            | (DiagnosticFormat::Gcc, VoidUse::Cast) => "invalid use of void expression",
            // GCC reads an aggregate's initializer as one that cannot
            // initialize it before it looks at the value.
            | (DiagnosticFormat::Gcc, VoidUse::Initialization) if !self.scalar_type(target) =>
                "invalid initializer",
            | (DiagnosticFormat::Gcc, _) => "void value not ignored as it ought to be",
            | (DiagnosticFormat::Clang, VoidUse::Cast) =>
                "operand of type 'void' where arithmetic or pointer type is required",
            | (DiagnosticFormat::Clang, _) => {
                let to = self.type_spelling(target, false)?;
                if use_ == VoidUse::Initialization {
                    crate::diagnostics::format_in!(
                        self.scratch,
                        "initializing '{to}' with an expression of incompatible type 'void'"
                    )
                } else {
                    crate::diagnostics::format_in!(
                        self.scratch,
                        "assigning to '{to}' from incompatible type 'void'"
                    )
                }
            },
            // TCC's `cast_error` in `tccgen.c`.
            | (DiagnosticFormat::Tcc, _) => {
                let to = self.type_spelling(target, false)?;
                crate::diagnostics::format_in!(self.scratch, "cannot convert 'void' to '{to}'")
            },
            | _ => return None,
        };
        Some(self.context.string_cache.intern(text))
    }

    /// TCC's text for comparing (`conditional` false) or selecting between
    /// pointers to incompatible types `left` and `right`, interned, when the
    /// diagnostic format is TCC's and both types can be spelled
    /// (`type_incompatibility_warning` in `tccgen.c`, whose comparison
    /// wording has no space before the parenthesis).
    /// C99: §6.5.9p2, p. 86; PDF p. 98; §6.5.15p3, p. 90; PDF p. 102.
    pub(super) fn pointer_mismatch_wording(
        &mut self,
        conditional: bool,
        left: TypeId,
        right: TypeId,
    ) -> Option<StringCacheId> {
        if self.context.configuration.diagnostic_format() != DiagnosticFormat::Tcc {
            return None;
        }
        let left = self.type_spelling(left, false)?;
        let right = self.type_spelling(right, false)?;
        let text = if conditional {
            crate::diagnostics::format_in!(
                self.scratch,
                "pointer type mismatch in conditional expression ('{left}' and '{right}')"
            )
        } else {
            crate::diagnostics::format_in!(
                self.scratch,
                "pointer type mismatch in comparison('{left}' and '{right}')"
            )
        };
        Some(self.context.string_cache.intern(text))
    }

    /// The text of a pointer conversion diagnostic in GCC's or Clang's
    /// words, interned, when the diagnostic format is one of theirs and
    /// the context and both types can be put in them.
    /// C99: §6.5.16.1p1, p. 92; PDF p. 104.
    pub(super) fn conversion_wording(
        &mut self,
        group: DiagnosticGroup,
        site: ConversionSite,
        target: TypeId,
        from: TypeId,
        source: ExpressionInfo<'tu>,
    ) -> Option<StringCacheId> {
        let format = self.context.configuration.diagnostic_format();
        if self.context.configuration.msvc_wording() {
            return self.msvc_conversion_wording(group, site, target, from);
        }
        // TCC words every assignment-like conversion as an assignment and
        // names no types (`verify_assign_cast` in TCC's `tccgen.c`).
        if format == DiagnosticFormat::Tcc && group == DiagnosticGroup::IntConversion {
            let text = if self.pointer_target(target).is_some() {
                "assignment makes pointer from integer without a cast"
            } else {
                "assignment makes integer from pointer without a cast"
            };
            return Some(self.context.string_cache.intern(text));
        }
        if format == DiagnosticFormat::Clang && group == DiagnosticGroup::PointerQualifiers {
            return self.clang_discarded_qualifiers_wording(site, target, from, source);
        }
        if format == DiagnosticFormat::Gcc && group == DiagnosticGroup::PointerQualifiers {
            return self.gcc_discarded_qualifiers_wording(site, target, from);
        }
        if site == ConversionSite::Other
            || !matches!(format, DiagnosticFormat::Gcc | DiagnosticFormat::Clang)
            || !matches!(
                group,
                DiagnosticGroup::IntConversion
                    | DiagnosticGroup::IncompatiblePointerTypes
                    | DiagnosticGroup::IncompatibleFunctionPointerTypes
            )
        {
            return None;
        }
        let gcc = format == DiagnosticFormat::Gcc;
        let mut text = ArenaString::new_in(self.scratch);
        if gcc {
            // GCC spells the value after its conversions, Clang the
            // expression's own type.
            let to = self.type_spelling(target, true)?;
            let from = self.type_spelling(from, true)?;
            let pointers = group != DiagnosticGroup::IntConversion;
            match site {
                | ConversionSite::Initialization | ConversionSite::Assignment => {
                    text.push_str(if site == ConversionSite::Initialization {
                        "initialization of '"
                    } else {
                        "assignment to '"
                    });
                    text.push_str(to);
                    text.push_str(if pointers {
                        "' from incompatible pointer type '"
                    } else {
                        "' from '"
                    });
                    text.push_str(from);
                    text.push('\'');
                },
                | ConversionSite::Argument { position, callee } => {
                    let callee = self.context.string_cache.at(callee?);
                    text.push_str(crate::diagnostics::format_in!(
                        self.scratch,
                        "passing argument {position} of '{callee}'"
                    ));
                    if pointers {
                        text.push_str(" from incompatible pointer type");
                    }
                },
                | ConversionSite::Return => {
                    text.push_str("returning '");
                    text.push_str(from);
                    text.push_str(if pointers {
                        "' from a function with incompatible return type '"
                    } else {
                        "' from a function with return type '"
                    });
                    text.push_str(to);
                    text.push('\'');
                },
                | ConversionSite::Other => return None,
            }
            if !pointers {
                text.push_str(if self.pointer_target(target).is_some() {
                    " makes pointer from integer without a cast"
                } else {
                    " makes integer from pointer without a cast"
                });
            }
        } else {
            let to = self.type_spelling(target, false)?;
            let from_spelling = self.type_spelling(source.ty, false)?;
            text.push_str(match group {
                | DiagnosticGroup::IntConversion if self.pointer_target(target).is_some() =>
                    "incompatible integer to pointer conversion ",
                | DiagnosticGroup::IntConversion => "incompatible pointer to integer conversion ",
                | DiagnosticGroup::IncompatibleFunctionPointerTypes =>
                    "incompatible function pointer types ",
                | _ => "incompatible pointer types ",
            });
            let (first, middle, last) = match site {
                | ConversionSite::Initialization =>
                    ("initializing '", to, "' with an expression of type '"),
                | ConversionSite::Assignment => ("assigning to '", to, "' from '"),
                | ConversionSite::Argument { .. } =>
                    ("passing '", from_spelling, "' to parameter of type '"),
                | ConversionSite::Return => (
                    "returning '",
                    from_spelling,
                    "' from a function with result type '",
                ),
                | ConversionSite::Other => return None,
            };
            text.push_str(first);
            text.push_str(middle);
            text.push_str(last);
            text.push_str(
                if matches!(
                    site,
                    ConversionSite::Argument { .. } | ConversionSite::Return
                ) {
                    to
                } else {
                    from_spelling
                },
            );
            text.push('\'');
            if group != DiagnosticGroup::IncompatibleFunctionPointerTypes {
                text.push_str(self.clang_conversion_hint(target, from, source));
            }
        }
        Some(self.context.string_cache.intern(text.as_str()))
    }

    /// Clang's words for `-Wincompatible-pointer-types-discards-qualifiers`
    /// (`ext_typecheck_convert_discards_qualifiers` and
    /// `ext_nested_pointer_qualifier_mismatch` in `DiagnosticSemaKinds.td`):
    /// the conversion as Clang names it at its site, then `discards
    /// qualifiers` when the pointed-to types are compatible apart from their
    /// own qualifiers (`char *` from `const char *`), or `discards qualifiers
    /// in nested pointer types` when a deeper level differs, whatever the
    /// pointed-to type's own qualifiers do (`char **` from `const char **`,
    /// `int **` from `const int *const *`). This is the question
    /// `pointer_conversion_violation` asked to put the conversion in
    /// this group, as Clang's `checkPointerTypesForAssignment` (`SemaExpr.cpp`)
    /// returns `CompatiblePointerDiscardsQualifiers` or
    /// `IncompatibleNestedPointerQualifiers`.
    /// C99: §6.5.16.1p1, p. 92; PDF p. 104.
    fn clang_discarded_qualifiers_wording(
        &mut self,
        site: ConversionSite,
        target: TypeId,
        from: TypeId,
        source: ExpressionInfo<'tu>,
    ) -> Option<StringCacheId> {
        let to = self.type_spelling(target, false)?;
        let from_spelling = self.type_spelling(source.ty, false)?;
        let nested = self
            .pointer_target(target)
            .zip(self.pointer_target(from))
            .is_some_and(|(to, from)| !self.pointer_compatible(to, from, true));
        let suffix = if nested {
            "discards qualifiers in nested pointer types"
        } else {
            "discards qualifiers"
        };
        let text = match site {
            | ConversionSite::Initialization => crate::diagnostics::format_in!(
                self.scratch,
                "initializing '{to}' with an expression of type '{from_spelling}' {suffix}"
            ),
            | ConversionSite::Assignment => crate::diagnostics::format_in!(
                self.scratch,
                "assigning to '{to}' from '{from_spelling}' {suffix}"
            ),
            | ConversionSite::Argument { .. } => crate::diagnostics::format_in!(
                self.scratch,
                "passing '{from_spelling}' to parameter of type '{to}' {suffix}"
            ),
            | ConversionSite::Return => crate::diagnostics::format_in!(
                self.scratch,
                "returning '{from_spelling}' from a function with result type '{to}' {suffix}"
            ),
            | ConversionSite::Other => return None,
        };
        Some(self.context.string_cache.intern(text))
    }

    /// GCC's words for `-Wdiscarded-qualifiers` (`WARNING_FOR_QUALIFIERS` in
    /// `convert_for_assignment`, GCC 14's `c/c-typeck.cc`): the conversion
    /// as GCC names it at its site, then `discards 'const' qualifier from
    /// pointer target type`, naming the qualifiers the value's pointed-to
    /// type has and the target's lacks in GCC's order (`'_Atomic const
    /// volatile'`), in one quote and with the singular `qualifier`.
    /// C99: §6.5.16.1p1, p. 92; PDF p. 104; §6.7.3p9, p. 109; PDF p. 121.
    fn gcc_discarded_qualifiers_wording(
        &mut self,
        site: ConversionSite,
        target: TypeId,
        from: TypeId,
    ) -> Option<StringCacheId> {
        let lost = self.pointer_target(from)?.qualifiers & !self.pointer_target(target)?.qualifiers;
        let mut qualifiers = ArenaString::new_in(self.scratch);
        for (qualifier, spelling) in [
            (TypeQualifiers::ATOMIC, "_Atomic"),
            (TypeQualifiers::CONST, "const"),
            (TypeQualifiers::VOLATILE, "volatile"),
            (TypeQualifiers::RESTRICT, "restrict"),
        ] {
            if lost.contains(qualifier) {
                if !qualifiers.is_empty() {
                    qualifiers.push(' ');
                }
                qualifiers.push_str(spelling);
            }
        }
        if qualifiers.is_empty() {
            return None;
        }
        let suffix = crate::diagnostics::format_in!(
            self.scratch,
            "discards '{qualifiers}' qualifier from pointer target type"
        );
        let text = match site {
            | ConversionSite::Initialization =>
                crate::diagnostics::format_in!(self.scratch, "initialization {suffix}"),
            | ConversionSite::Assignment =>
                crate::diagnostics::format_in!(self.scratch, "assignment {suffix}"),
            | ConversionSite::Argument { position, callee } => {
                let callee = self.context.string_cache.at(callee?);
                crate::diagnostics::format_in!(
                    self.scratch,
                    "passing argument {position} of '{callee}' {suffix}"
                )
            },
            | ConversionSite::Return =>
                crate::diagnostics::format_in!(self.scratch, "return {suffix}"),
            | ConversionSite::Other => return None,
        };
        Some(self.context.string_cache.intern(text))
    }

    /// `cl`'s words for an implicit conversion between a pointer and an
    /// integer or between pointers to incompatible types, both warnings
    /// there: C4047 when the two differ in their levels of indirection
    /// (`'=': 'int *' differs in levels of indirection from 'int'`; the
    /// group is then `int-conversion`, see [`Self::msvc_pointer_group`]),
    /// C4133 otherwise (`'=': incompatible types - from 'float *' to 'int
    /// *'`), each led by the operation, which `cl` names `initializing`, `=`,
    /// `function` (an argument) or `return`. A function pointer, which
    /// `cl` spells with its calling convention, keeps bcc's words.
    /// Microsoft Learn: "Compiler Warning (level 1) C4047", "Compiler
    /// Warning (level 1 and level 3) C4133".
    /// C99: §6.5.16.1p1, p. 92; PDF p. 104.
    fn msvc_conversion_wording(
        &mut self,
        group: DiagnosticGroup,
        site: ConversionSite,
        target: TypeId,
        from: TypeId,
    ) -> Option<StringCacheId> {
        if !matches!(
            group,
            DiagnosticGroup::IntConversion | DiagnosticGroup::IncompatiblePointerTypes
        ) {
            return None;
        }
        let operation = match site {
            | ConversionSite::Initialization => "initializing",
            | ConversionSite::Assignment => "=",
            | ConversionSite::Argument { .. } => "function",
            | ConversionSite::Return => "return",
            | ConversionSite::Other => return None,
        };
        let to = self.msvc_type_spelling(target)?;
        let from_spelling = self.msvc_type_spelling(from)?;
        let text = if group == DiagnosticGroup::IntConversion {
            crate::diagnostics::format_in!(
                self.scratch,
                "'{operation}': '{to}' differs in levels of indirection from '{from_spelling}'"
            )
        } else {
            crate::diagnostics::format_in!(
                self.scratch,
                "'{operation}': incompatible types - from '{from_spelling}' to '{to}'"
            )
        };
        Some(self.context.string_cache.intern(text))
    }

    /// The words for a cast between a pointer and an integer of another
    /// width: Clang's in its format under its rules
    /// ([`Self::clang_cast_wording`]), or `cl`'s: C4311 for a pointer
    /// truncated to an integer, C4312 for an integer widened to a pointer.
    /// Microsoft Learn: "Compiler Warning (level 1) C4311", "Compiler Warning
    /// (level 1) C4312". C99: §6.3.2.3 paragraphs 5-6, p. 47; PDF p. 59.
    pub(super) fn cast_wording(&mut self, target: TypeId, from: TypeId) -> Option<StringCacheId> {
        if self.context.configuration.diagnostic_format() == DiagnosticFormat::Clang
            && self.context.configuration.compiler_identity().rules()
                == crate::configuration::RuleSet::Clang
        {
            return self.clang_cast_wording(target, from);
        }
        if !self.context.configuration.msvc_wording() {
            return None;
        }
        let to = self.msvc_type_spelling(target)?;
        let from_spelling = self.msvc_type_spelling(from)?;
        let text = if self.pointer_target(target).is_some() {
            crate::diagnostics::format_in!(
                self.scratch,
                "'type cast': conversion from '{from_spelling}' to '{to}' of greater size"
            )
        } else {
            crate::diagnostics::format_in!(
                self.scratch,
                "'type cast': pointer truncation from '{from_spelling}' to '{to}'"
            )
        };
        Some(self.context.string_cache.intern(text))
    }

    /// Clang's words for a cast between a pointer and a smaller integer
    /// (`warn_pointer_to_int_cast`, `warn_int_to_pointer_cast` and their
    /// `void *` and enumeration forms in `DiagnosticSemaKinds.td`), with
    /// each type spelled as written without its typedef names, which bcc
    /// does not keep: Clang adds `(aka '...')` after one.
    /// C99: §6.3.2.3p5-6, p. 47; PDF p. 59.
    fn clang_cast_wording(&mut self, target: TypeId, from: TypeId) -> Option<StringCacheId> {
        let to = self.type_spelling(target, false)?;
        let from_spelling = self.type_spelling(from, false)?;
        let text = if self.pointer_target(target).is_some() {
            crate::diagnostics::format_in!(
                self.scratch,
                "cast to '{to}' from smaller integer type '{from_spelling}'"
            )
        } else {
            crate::diagnostics::format_in!(
                self.scratch,
                "cast to smaller integer type '{to}' from '{from_spelling}'"
            )
        };
        Some(self.context.string_cache.intern(text))
    }

    /// `ty` as `cl` spells it, for the types whose spelling is known to
    /// agree with Clang's (`int *`, `const char *`, `struct S *`); `None`
    /// for the others, which `cl` spells its own way (`__int64`, a
    /// function pointer's `__cdecl`).
    pub(super) fn msvc_type_spelling(&self, ty: TypeId) -> Option<&'tu str> {
        let spelling = self.type_spelling(ty, false)?;
        (!spelling.contains(['(', '['])
            && ![
                "long long",
                "_Bool",
                "_Complex",
                "_BitInt",
                "_Atomic",
                "__int128",
            ]
            .iter()
            .any(|word| spelling.contains(word)))
        .then_some(spelling)
    }

    /// The function `name` of type `ty` as `cl` names it in an error:
    /// `int g(int,char *)`, `int f(void)`. `None` for a function without a
    /// prototype or with `...`, and where a type is one `cl` spells its own
    /// way ([`Self::msvc_type_spelling`]).
    pub(super) fn msvc_function_spelling(
        &mut self,
        ty: TypeId,
        name: StringCacheId,
    ) -> Option<&'tu str> {
        let TypeKind::Function {
            result,
            parameters,
            prototype: true,
            variadic: false,
        } = self.types.nodes[ty.index]
        else {
            return None;
        };
        let mut text = ArenaString::new_in(self.types.tu);
        text.push_str(self.msvc_type_spelling(result)?);
        text.push(' ');
        text.push_str(self.context.string_cache.at(name));
        text.push('(');
        if parameters.is_empty() {
            text.push_str("void");
        }
        for (index, &parameter) in parameters.iter().enumerate() {
            if index != 0 {
                text.push(',');
            }
            text.push_str(self.msvc_type_spelling(parameter)?);
        }
        text.push(')');
        Some(text.into_str())
    }

    /// The group of a pointer conversion violation under `cl`'s wording:
    /// `cl` reports pointers that differ in their levels of indirection
    /// (`int *` from `char **`) as it reports a pointer from an integer,
    /// with C4047, so they take the `int-conversion` group, which `/wd4047`
    /// turns off with the rest of C4047.
    pub(super) fn msvc_pointer_group(
        &self,
        group: DiagnosticGroup,
        target: TypeId,
        from: TypeId,
    ) -> DiagnosticGroup {
        if self.context.configuration.msvc_wording()
            && group == DiagnosticGroup::IncompatiblePointerTypes
            && self.indirection_levels(target) != self.indirection_levels(from)
        {
            DiagnosticGroup::IntConversion
        } else {
            group
        }
    }

    /// How many pointers `ty` goes through before its first non-pointer
    /// type.
    fn indirection_levels(&self, mut ty: TypeId) -> usize {
        let mut levels = 0;
        while let TypeKind::Pointer(next) = self.types.nodes[self.types.non_atomic(ty).index] {
            levels += 1;
            ty = next;
        }
        levels
    }

    /// Clang's fix-it in words: dereferencing the value, or taking the
    /// address of an lvalue, would give the target type, and the value's own
    /// `&` or `*` is removed rather than another put before it. The edit
    /// itself waits in [`Analyzer::conversion_fix_it`] for the diagnostic.
    /// A null pointer constant is not dereferenced, and only an ordinary
    /// object's address is taken: not a bit-field's or a vector element's,
    /// though a `register` object's is
    /// (`ConversionFixItGenerator::tryToFixConversion` in
    /// `SemaFixItUtils.cpp`).
    /// C99: §6.5.3.2p1, p. 78; PDF p. 90 (the objects `&` may address).
    fn clang_conversion_hint(
        &mut self,
        target: TypeId,
        from: TypeId,
        source: ExpressionInfo<'tu>,
    ) -> &'static str {
        let expression = source.expression;
        let unary = match expression.kind {
            | ExpressionType::Unary { operator, .. } => Some(operator),
            | _ => None,
        };
        let edit = |operator: &'static str, own: UnaryOperator| match unary {
            | Some(unary) if unary == own => Some(ConversionEdit::RemoveOperator),
            | Some(_) => None,
            | None if Self::clang_needs_parentheses(expression) =>
                Some(ConversionEdit::Parenthesize(operator)),
            | None => Some(ConversionEdit::Prefix(operator)),
        };
        let (words, edit) = if let Some(pointee) = self.pointer_target(from)
            && self
                .types
                .composite(pointee.unqualified(), target.unqualified())
                .is_some()
        {
            // Clang asks it of the value under its parentheses and casts.
            let mut inner = expression;
            while let ExpressionType::Parenthesized {
                expression: operand_expression,
            }
            | ExpressionType::Cast {
                operand_expression, ..
            } = inner.kind
            {
                inner = operand_expression;
            }
            if self.null_pointer_constant(self.expression_info(inner)) {
                return "";
            }
            let words = if unary == Some(UnaryOperator::AddressOf) {
                "; remove &"
            } else {
                "; dereference with *"
            };
            (words, edit("*", UnaryOperator::AddressOf))
        } else if matches!(
            source.category,
            ValueCategory::Lvalue | ValueCategory::ModifiableLvalue
        ) && source.bit_field.is_none()
            && !source.vector_element
            && let Some(pointee) = self.pointer_target(target)
            && pointee.qualifiers.contains(source.ty.qualifiers)
            && self
                .types
                .composite(pointee.unqualified(), source.ty.unqualified())
                .is_some()
        {
            let words = if unary == Some(UnaryOperator::Indirection) {
                "; remove *"
            } else {
                "; take the address with &"
            };
            (words, edit("&", UnaryOperator::Indirection))
        } else {
            return "";
        };
        self.conversion_fix_it = edit.map(|edit| ConversionFixIt {
            value: expression.source_vectors,
            edit,
        });
        words
    }

    /// Whether Clang puts parentheses around `expression` before it writes
    /// `&` or `*` before it: unless it is an identifier, a string literal,
    /// a call, a subscript, a member access, a cast or a unary operator.
    /// A parenthesized expression gets more, as Clang asks only whether the
    /// expression under its implicit conversions is one of these.
    const fn clang_needs_parentheses(expression: &Expression<'_>) -> bool {
        !matches!(
            expression.kind,
            ExpressionType::Identifier(_)
                | ExpressionType::StringLiteral(_)
                | ExpressionType::Call { .. }
                | ExpressionType::Builtin(_)
                | ExpressionType::Binary {
                    operator: BinaryOperator::Subscript,
                    ..
                }
                | ExpressionType::DirectMember { .. }
                | ExpressionType::IndirectMember { .. }
                | ExpressionType::Cast { .. }
                | ExpressionType::Unary { .. }
        )
    }

    /// The text of a member access error in GCC's, Clang's or TCC's words,
    /// interned, when the diagnostic format is one of theirs: the record is
    /// incomplete, or has no member `member`.
    /// C99: §6.5.2.3p1-2, p. 72; PDF p. 84.
    pub(super) fn member_wording(
        &mut self,
        record: TypeId,
        tag: usize,
        member: StringCacheId,
        incomplete: bool,
    ) -> Option<StringCacheId> {
        let format = self.context.configuration.diagnostic_format();
        let member = self.context.string_cache.at(member);
        let text = match format {
            // TCC names the tag alone (`tccgen.c`, `struct_field` and
            // `find_field`).
            | DiagnosticFormat::Tcc if incomplete => {
                let name = self.context.string_cache.at(self.types.tags[tag].name?);
                crate::diagnostics::format_in!(
                    self.scratch,
                    "dereferencing incomplete type '{name}'"
                )
            },
            | DiagnosticFormat::Tcc =>
                crate::diagnostics::format_in!(self.scratch, "field not found: {member}"),
            | DiagnosticFormat::Gcc | DiagnosticFormat::Clang => {
                let gcc = format == DiagnosticFormat::Gcc;
                let record = self.type_spelling(record, gcc)?;
                match (gcc, incomplete) {
                    | (true, true) => crate::diagnostics::format_in!(
                        self.scratch,
                        "invalid use of undefined type '{record}'"
                    ),
                    | (true, false) => crate::diagnostics::format_in!(
                        self.scratch,
                        "'{record}' has no member named '{member}'"
                    ),
                    | (false, true) => crate::diagnostics::format_in!(
                        self.scratch,
                        "incomplete definition of type '{record}'"
                    ),
                    | (false, false) => crate::diagnostics::format_in!(
                        self.scratch,
                        "no member named '{member}' in '{record}'"
                    ),
                }
            },
            | _ => return None,
        };
        Some(self.context.string_cache.intern(text))
    }

    /// `ty` as GCC (`gcc`) or Clang spells it in a diagnostic, or `None`
    /// for a type whose spelling bcc does not model (an unnamed tag, a
    /// vector).
    /// C99: §6.7.6p2, p. 122; PDF p. 134 (type names).
    pub(super) fn type_spelling(&self, ty: TypeId, gcc: bool) -> Option<&'tu str> {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Last {
            None,
            Pointer,
            Array,
            Function,
            Grouped,
        }
        let arena = self.types.tu;
        let mut declarator = ArenaString::new_in(self.scratch);
        let mut last = Last::None;
        let mut ty = ty;
        let base = loop {
            match self.types.nodes[ty.index] {
                | TypeKind::Pointer(next) => {
                    let mut text = ArenaString::new_in(self.scratch);
                    text.push('*');
                    let words = qualifier_words(ty.qualifiers);
                    if !words.is_empty() {
                        if gcc {
                            text.push(' ');
                        }
                        text.push_str(words);
                        if !declarator.is_empty() {
                            text.push(' ');
                        }
                    }
                    text.push_str(declarator.as_str());
                    declarator = text;
                    last = Last::Pointer;
                    ty = next;
                },
                | kind @ (TypeKind::Array(..) | TypeKind::Function { .. }) => {
                    if last == Last::Pointer {
                        let mut text = ArenaString::new_in(self.scratch);
                        text.push('(');
                        text.push_str(declarator.as_str());
                        text.push(')');
                        declarator = text;
                    }
                    let grouped = last == Last::Pointer;
                    match kind {
                        | TypeKind::Array(next, bound) => {
                            match bound {
                                | ArrayBound::Constant(n) => declarator.push_str(
                                    crate::diagnostics::format_in!(self.scratch, "[{n}]"),
                                ),
                                | ArrayBound::Incomplete => declarator.push_str("[]"),
                                | ArrayBound::Variable(_) | ArrayBound::Star =>
                                    declarator.push_str("[*]"),
                            }
                            last = if grouped { Last::Grouped } else { Last::Array };
                            ty = next;
                        },
                        | TypeKind::Function {
                            result,
                            parameters,
                            prototype,
                            variadic,
                        } => {
                            declarator.push('(');
                            for (index, &parameter) in parameters.iter().enumerate() {
                                if index != 0 {
                                    declarator.push_str(", ");
                                }
                                declarator.push_str(self.type_spelling(parameter, gcc)?);
                            }
                            if variadic {
                                declarator.push_str(if parameters.is_empty() {
                                    "..."
                                } else {
                                    ", ..."
                                });
                            } else if prototype && parameters.is_empty() {
                                declarator.push_str("void");
                            }
                            declarator.push(')');
                            last = if grouped {
                                Last::Grouped
                            } else {
                                Last::Function
                            };
                            ty = result;
                        },
                        | _ => return None,
                    }
                },
                | TypeKind::Scalar(scalar) => break self.diagnostic_scalar_spelling(scalar, gcc),
                | TypeKind::BitInt { width, signed } =>
                    break crate::diagnostics::format_in!(
                        self.scratch,
                        "{}_BitInt({width})",
                        if signed { "" } else { "unsigned " }
                    ),
                | TypeKind::Nullptr => break "nullptr_t",
                | TypeKind::Atomic(value) =>
                    break crate::diagnostics::format_in!(
                        self.scratch,
                        "_Atomic({})",
                        self.type_spelling(value, gcc)?
                    ),
                | TypeKind::Tag(id) => {
                    let tag = self.types.tags[id];
                    let keyword = match tag.kind {
                        | TagKind::Struct => "struct",
                        | TagKind::Union => "union",
                        | TagKind::Enum => "enum",
                    };
                    break crate::diagnostics::format_in!(
                        self.scratch,
                        "{keyword} {}",
                        self.context.string_cache.at(tag.name?)
                    );
                },
                | _ => return None,
            }
        };
        let mut text = ArenaString::new_in(arena);
        let words = qualifier_words(ty.qualifiers);
        if !words.is_empty() {
            text.push_str(words);
            text.push(' ');
        }
        text.push_str(base);
        if !declarator.is_empty() {
            // Clang writes `int (int)` for a function type and GCC
            // `int(int)`; both write `int[2]`.
            let space = match last {
                | Last::Array => false,
                | Last::Function => !gcc,
                | _ => true,
            };
            if space {
                text.push(' ');
            }
            text.push_str(declarator.as_str());
        }
        Some(text.into_str())
    }

    /// A scalar type's keywords: GCC's, which spell out `int` and put
    /// `unsigned` after the size, or Clang's; `bool` from C23.
    fn diagnostic_scalar_spelling(&self, scalar: Scalar, gcc: bool) -> &'static str {
        if scalar == Scalar::Bool && self.context.configuration.standard() >= CStandard::C23 {
            return "bool";
        }
        match (scalar, gcc) {
            | (Scalar::Short, true) => "short int",
            | (Scalar::UnsignedShort, true) => "short unsigned int",
            | (Scalar::Long, true) => "long int",
            | (Scalar::UnsignedLong, true) => "long unsigned int",
            | (Scalar::LongLong, true) => "long long int",
            | (Scalar::UnsignedLongLong, true) => "long long unsigned int",
            | (Scalar::UnsignedInt128, true) => "__int128 unsigned",
            | (Scalar::ComplexFloat, true) => "complex float",
            | (Scalar::ComplexDouble, true) => "complex double",
            | (Scalar::ComplexLongDouble, true) => "complex long double",
            | (Scalar::ComplexFloat, false) => "_Complex float",
            | (Scalar::ComplexDouble, false) => "_Complex double",
            | (Scalar::ComplexLongDouble, false) => "_Complex long double",
            | _ => scalar.spelling(),
        }
    }
}

/// The qualifier keywords, in the order both compilers write them.
const fn qualifier_words(qualifiers: TypeQualifiers) -> &'static str {
    match qualifiers.bits() & 7 {
        | 0 => "",
        | 1 => "const",
        | 2 => "volatile",
        | 3 => "const volatile",
        | 4 => "restrict",
        | 5 => "const restrict",
        | 6 => "volatile restrict",
        | _ => "const volatile restrict",
    }
}
