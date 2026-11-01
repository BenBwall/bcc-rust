//! GCC's and Clang's words for the pointer conversion constraints they
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

impl<'tu> Analyzer<'_, 'tu, '_> {
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
            text.push_str(if matches!(site, ConversionSite::Argument { .. } | ConversionSite::Return) {
                to
            } else {
                from_spelling
            });
            text.push('\'');
            if group != DiagnosticGroup::IncompatibleFunctionPointerTypes {
                text.push_str(self.clang_conversion_hint(target, from, source));
            }
        }
        Some(self.context.string_cache.intern(text.as_str()))
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
        )
            && let Some(pointee) = self.pointer_target(target)
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
        if scalar == Scalar::Bool
            && self.context.configuration.standard() >= CStandard::C23
        {
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
