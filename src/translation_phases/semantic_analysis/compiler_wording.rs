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
    translation_phases::parsing::declaration_syntax::TypeQualifiers,
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

    /// `cl`'s words for a cast between a pointer and an integer of another
    /// width: C4311 for a pointer truncated to an integer, C4312 for an
    /// integer widened to a pointer. Microsoft Learn: "Compiler Warning
    /// (level 1) C4311", "Compiler Warning (level 1) C4312".
    /// C99: §6.3.2.3 paragraphs 5-6, p. 47; PDF p. 59.
    pub(super) fn msvc_cast_wording(
        &mut self,
        target: TypeId,
        from: TypeId,
    ) -> Option<StringCacheId> {
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

    /// `ty` as `cl` spells it, for the types whose spelling is known to
    /// agree with Clang's (`int *`, `const char *`, `struct S *`); `None`
    /// for the others, which `cl` spells its own way (`__int64`, a
    /// function pointer's `__cdecl`).
    fn msvc_type_spelling(&self, ty: TypeId) -> Option<&'tu str> {
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
    /// address of an lvalue, would give the target type.
    fn clang_conversion_hint(
        &mut self,
        target: TypeId,
        from: TypeId,
        source: ExpressionInfo<'tu>,
    ) -> &'static str {
        if let Some(pointee) = self.pointer_target(from)
            && self
                .types
                .composite(pointee.unqualified(), target.unqualified())
                .is_some()
        {
            return "; dereference with *";
        }
        if matches!(
            source.category,
            ValueCategory::Lvalue | ValueCategory::ModifiableLvalue
        ) && let Some(pointee) = self.pointer_target(target)
            && pointee.qualifiers.contains(source.ty.qualifiers)
            && self
                .types
                .composite(pointee.unqualified(), source.ty.unqualified())
                .is_some()
        {
            return "; take the address with &";
        }
        ""
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
