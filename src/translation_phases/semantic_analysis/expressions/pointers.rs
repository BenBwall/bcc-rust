//! Classifies pointer constraints once, before shared diagnostic policy decides
//! severity. Ordinary conversion records remain the lowering interface.
//! C99: §6.5.16.1p1, p. 92; PDF p. 104; §6.5.9p2, p. 86; PDF p. 98;
//! §6.5.8p2, p. 85; PDF p. 97; §6.5.2.2p2,p7, pp. 71-72; PDF pp. 83-84;
//! §6.8.6.4p3, p. 139; PDF p. 151; §6.7.8p11, p. 126; PDF p. 138.

use super::{
    super::WideStore,
    ConversionKind,
    ExpressionInfo,
};
use crate::{
    configuration::{
        DiagnosticGroup,
        FeatureOrigin,
    },
    translation_phases::{
        SourceVectors,
        semantic_analysis::{
            Analyzer,
            types::{
                Scalar,
                TagKind,
                TypeId,
                TypeKind,
            },
        },
    },
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Assignment, scalar initialization, prototype arguments, return and
    /// atomic builtin value operands share this adapter. Other
    /// intrinsic-specific contracts keep `assignment_compatible`.
    /// C99: §6.5.16.1p1, p. 92; PDF p. 104; §6.5.2.2p2,p7, pp. 71-72;
    /// PDF pp. 83-84; §6.8.6.4p3, p. 139; PDF p. 151; §6.7.8p11, p. 126;
    /// PDF p. 138.
    pub(in crate::translation_phases::semantic_analysis) fn assignment_conversion(
        &mut self,
        target: TypeId,
        source: ExpressionInfo<'tu>,
    ) -> bool {
        self.assignment_conversion_at(target, source, source.expression.source_vectors)
    }

    /// Uses the assignment operator as the diagnostic location while the
    /// conversion remains attached to its operand.
    /// C99: §6.5.16.1p1, p. 92; PDF p. 104.
    pub(in crate::translation_phases::semantic_analysis) fn assignment_conversion_at(
        &mut self,
        target: TypeId,
        source: ExpressionInfo<'tu>,
        diagnostic_site: SourceVectors,
    ) -> bool {
        let source = self.resolve_function_value(source, Some(target));
        if self.assignment_compatible(target, source) {
            self.literal_conversion_warning(target, source);
            if self.pointer_target(target).is_some() && self.null_pointer_constant(source) {
                self.null_conversion_warning(source, false, diagnostic_site);
            }
            if matches!(
                self.types.nodes[target.index],
                TypeKind::Scalar(Scalar::Bool)
            ) {
                self.boolean_conversion_warning(source);
            }
            return true;
        }
        let target = self.types.non_atomic(target);
        // Compatibility has already recorded lvalue conversion or decay.
        let (from, _) = self.value_conversion(source);
        if let Some(group) = self.pointer_conversion_violation(target, from) {
            self.pointer_diagnostic(group, diagnostic_site);
            return true;
        }
        // Clang accepts function/void pointers silently by default and reports
        // the ISO constraint only under pedantic. This is not an incompatible
        // function-pointer signature, which remains a default error.
        if let (Some(to), Some(from)) = (self.pointer_target(target), self.pointer_target(from))
            && ((self.void_target(to) && self.function_target(from))
                || (self.function_target(to) && self.void_target(from)))
        {
            self.context.report_extension_since(
                "conversion between function and void pointers",
                FeatureOrigin::Gnu,
                diagnostic_site,
            );
            return true;
        }
        false
    }

    /// Warns about a cast between a pointer and an integer of another width,
    /// whose result C leaves implementation-defined and possibly
    /// unrepresentable. GCC (`build_c_cast`) warns when a plain integer type
    /// (neither `_Bool` nor an enumeration) and the pointer differ in width
    /// either way, toward a pointer only when the operand is not constant
    /// (an address constant is one). Clang (`SemaCast.cpp`) warns only when
    /// the integer is narrower, toward a pointer only when the operand is no
    /// integer constant expression, and splits its groups by a `void *` and
    /// by an enumeration destination; neither compiler warns for `_Bool`. A
    /// `_BitInt` counts with its width. GCC also withholds a cast spelled in
    /// a system header's macro. TCC follows GCC; MSVC (whose C4311 and
    /// C4312 warn for narrower integers) and clang-cl follow Clang
    /// ([`crate::configuration::CompilerConfiguration::pointer_cast_rules`]).
    /// C99: §6.3.2.3 paragraphs 5-6, p. 47; PDF p. 59.
    pub(in crate::translation_phases::semantic_analysis) fn pointer_integer_cast_warning(
        &mut self,
        target: TypeId,
        from: TypeId,
        operand: ExpressionInfo<'tu>,
        site: SourceVectors,
    ) {
        let pointer_bits = self.types.target.pointer.size * 8;
        let gcc =
            self.context.configuration.pointer_cast_rules() == crate::configuration::RuleSet::Gcc;
        // The width of an integer type, and whether it is an enumeration.
        let integer = |ty: TypeId| -> Option<(u64, bool)> {
            // Clang's integral types exclude an atomic one, which a cast's
            // value keeps (`(char *)(_Atomic int)i`); GCC drops it.
            if !gcc && matches!(self.types.nodes[ty.index], TypeKind::Atomic(_)) {
                return None;
            }
            let ty = self.types.non_atomic(ty);
            let enumeration = match self.types.nodes[ty.index] {
                | TypeKind::Scalar(Scalar::Bool) => return None,
                | TypeKind::Tag(id) => self.types.tags[id].kind == TagKind::Enum,
                | _ => false,
            };
            self.integer_type(ty)
                .map(|(bits, _)| (u64::from(bits), enumeration))
        };
        let group = if let Some(to) = self.pointer_target(target)
            && let Some((bits, enumeration)) = integer(from)
        {
            // Even an unevaluated operand that is no constant (`1 ? 2 : i`)
            // keeps the cast from being an integer constant expression.
            let ice = operand.ice && self.valid_ice_operands(operand.expression);
            if gcc {
                let constant = ice || self.casts_address_constant(operand.expression);
                (bits != pointer_bits && !enumeration && !constant)
                    .then_some(DiagnosticGroup::IntToPointerCast)
            } else if bits < pointer_bits && !enumeration && !ice {
                Some(if self.void_target(to) {
                    DiagnosticGroup::IntToVoidPointerCast
                } else {
                    DiagnosticGroup::IntToPointerCast
                })
            } else {
                None
            }
        } else if let Some(pointee) = self.pointer_target(from)
            && let Some((bits, enumeration)) = integer(target)
        {
            if gcc {
                (bits != pointer_bits && !enumeration).then_some(DiagnosticGroup::PointerToIntCast)
            } else if bits < pointer_bits {
                Some(match (self.void_target(pointee), enumeration) {
                    | (true, true) => DiagnosticGroup::VoidPointerToEnumCast,
                    | (true, false) => DiagnosticGroup::VoidPointerToIntCast,
                    | (false, true) => DiagnosticGroup::PointerToEnumCast,
                    | (false, false) => DiagnosticGroup::PointerToIntCast,
                })
            } else {
                None
            }
        } else {
            None
        };
        // GCC also withholds a cast written in a system header's macro
        // expanded in user code, which Clang reports at the expansion.
        if let Some(group) = group
            && !(gcc
                && site.length() != 0
                && self
                    .context
                    .in_system_header(self.context.first_source_vector(site)))
        {
            self.pointer_diagnostic(group, site);
        }
    }

    /// Whether `expression` is an address constant, possibly inside casts,
    /// as GCC's `TREE_CONSTANT` finds `(int)(long)&x`.
    /// C99: §6.6 paragraph 9, p. 96; PDF p. 108.
    fn casts_address_constant(&self, mut expression: &'tu super::Expression<'tu>) -> bool {
        use crate::translation_phases::parsing::syntax::ExpressionType as E;
        loop {
            let info = self.expression_info(expression);
            if info.constant == super::ConstantClass::Address || info.folded_address() {
                return true;
            }
            match expression.kind {
                | E::Parenthesized { expression: inner }
                | E::Cast {
                    operand_expression: inner,
                    ..
                } => expression = inner,
                | _ => return false,
            }
        }
    }

    /// Clang's `-Wliteral-conversion` (`DiagnoseFloatingImpCast`): a
    /// floating literal, possibly signed and parenthesized, whose implicit
    /// conversion to an integer type is out of its range ("is undefined") or
    /// drops a fraction, or that converts to `_Bool` from a value other
    /// than 0 or 1. GCC reports only an out-of-range conversion, and only in
    /// a static initializer (`-Woverflow`, beside the static value it
    /// folds). Clang checks only a builtin integer type, not an enumeration
    /// or an atomic type. clang-cl follows Clang, TCC and MSVC GCC
    /// ([`crate::configuration::CompilerConfiguration::float_conversion_rules`]).
    /// C99: §6.3.1.4 paragraph 1, p. 43; PDF p. 55; §6.3.1.2 paragraph 1,
    /// p. 43; PDF p. 55.
    fn literal_conversion_warning(&mut self, target: TypeId, source: ExpressionInfo<'tu>) {
        use crate::translation_phases::{
            parsing::syntax::{
                Constant,
                ExpressionType as E,
                UnaryOperator,
            },
            preprocessing::FloatTokenType,
        };
        // Clang checks builtin integer targets only, not an enumeration or
        // an atomic type.
        if self.context.configuration.float_conversion_rules() == crate::configuration::RuleSet::Gcc
            || !matches!(self.types.nodes[target.index], TypeKind::Scalar(_))
            || self.integer_type(target).is_none()
        {
            return;
        }
        let mut literal = source.expression;
        let mut signed = false;
        loop {
            match literal.kind {
                | E::Parenthesized { expression } => literal = expression,
                | E::Unary {
                    operator: UnaryOperator::Plus | UnaryOperator::Minus,
                    operand_expression,
                } if !signed => {
                    signed = true;
                    literal = operand_expression;
                },
                | _ => break,
            }
        }
        if !matches!(literal.kind, E::Constant(Constant::Float(_))) {
            return;
        }
        let (value, integral) = match (source.floating(), source.binary128()) {
            | (Some(value), _) => (
                FloatTokenType::LongDouble(value.real),
                integral_x87(value.real),
            ),
            | (_, Some(value)) => (
                FloatTokenType::Float128(value.real),
                match value.real.class() {
                    | crate::binary128::Class::Zero => !value.real.is_negative(),
                    | crate::binary128::Class::Finite {
                        exponent,
                        significand,
                    } =>
                        exponent >= 0
                            || exponent > -128
                                && significand & ((1_u128 << exponent.unsigned_abs()) - 1) == 0,
                    | _ => false,
                },
            ),
            | _ => return,
        };
        let Some((stored, out_of_range)) = self.saturated_float_cast(target, value, None) else {
            return;
        };
        let bool_target = matches!(
            self.types.nodes[self.types.non_atomic(target).index],
            TypeKind::Scalar(Scalar::Bool)
        );
        // `_Bool` keeps a 0 or a 1 exactly.
        let exact = if bool_target {
            let int = self.types.scalar(Scalar::Int);
            integral
                && self
                    .float_cast(int, value)
                    .is_some_and(|v| v.value == stored.value)
        } else {
            integral
        };
        let (from, to) = (
            self.conversion_spelling(source.ty),
            self.conversion_spelling(target),
        );
        let text = if out_of_range {
            crate::diagnostics::format_in!(
                self.scratch,
                "implicit conversion of out of range value from `{from}` to `{to}` is undefined"
            )
        } else if exact {
            return;
        } else if bool_target {
            crate::diagnostics::format_in!(
                self.scratch,
                "implicit conversion from `{from}` to `{to}` changes value to {}",
                if stored.is_zero() { "false" } else { "true" }
            )
        } else {
            crate::diagnostics::format_in!(
                self.scratch,
                "implicit conversion from `{from}` to `{to}` changes value to {}",
                self.wide.display(stored)
            )
        };
        let text = self.context.string_cache.intern(text);
        self.group_diagnostic(
            DiagnosticGroup::LiteralConversion,
            crate::translation_phases::semantic_analysis::SemanticErrorKind::LiteralConversion(
                text,
            ),
            source.expression.source_vectors,
            None,
            None,
        );
    }

    /// Diagnoses suspicious but valid C null pointer constants. Parentheses
    /// do not change whether the expression was a literal.
    /// C99: §6.3.2.3p3, p. 47; PDF p. 59.
    pub(super) fn null_conversion_warning(
        &mut self,
        info: ExpressionInfo<'tu>,
        comparison: bool,
        site: SourceVectors,
    ) {
        use crate::translation_phases::parsing::syntax::{
            Constant,
            ExpressionType as E,
        };
        let mut expression = info.expression;
        while let E::Parenthesized { expression: inner } = expression.kind {
            expression = inner;
        }
        if comparison && matches!(expression.kind, E::Constant(Constant::Char(_))) {
            self.pointer_diagnostic(DiagnosticGroup::PointerCompare, site);
        } else if !comparison
            && !self.nullptr_type(info.ty)
            && !matches!(expression.kind, E::Constant(Constant::Integer(_)))
            && !matches!(self.types.nodes[info.ty.index], TypeKind::Pointer(_))
        {
            self.pointer_diagnostic(
                DiagnosticGroup::NonLiteralNullConversion,
                expression.source_vectors,
            );
        }
    }

    /// Diagnoses implicit Boolean conversion while retaining the operand's
    /// type; truth contexts need no synthetic cast.
    /// C99: §6.3.1.2p1, p. 43; PDF p. 55.
    pub(super) fn boolean_conversion_warning(&mut self, info: ExpressionInfo<'tu>) {
        if self.nullptr_type(info.ty) {
            self.pointer_diagnostic(
                DiagnosticGroup::NullConversion,
                info.expression.source_vectors,
            );
        } else {
            self.pointer_bool_warning(info);
        }
    }

    /// Clang's pointer-to-bool warning concerns a known address, not the
    /// unknown value read from a pointer variable.
    /// C99: §6.3.1.2p1, p. 43; PDF p. 55.
    pub(super) fn pointer_bool_warning(&mut self, info: ExpressionInfo<'tu>) {
        use crate::translation_phases::parsing::syntax::{
            ExpressionType as E,
            UnaryOperator as U,
        };
        let mut expression = info.expression;
        while let E::Parenthesized { expression: inner } = expression.kind {
            expression = inner;
        }
        let address = if let E::Unary {
            operator: U::AddressOf,
            operand_expression,
        } = expression.kind
        {
            let mut operand = operand_expression;
            loop {
                match operand.kind {
                    | E::Identifier(_) | E::CompoundLiteral { .. } => break true,
                    | E::Parenthesized { expression } => operand = expression,
                    | E::DirectMember {
                        base_expression, ..
                    } => operand = base_expression,
                    | _ => break false,
                }
            }
        } else {
            false
        };
        if info.category == super::ValueCategory::FunctionDesignator
            || matches!(self.types.nodes[info.ty.index], TypeKind::Array(..))
            || address
        {
            self.pointer_diagnostic(
                DiagnosticGroup::PointerBoolConversion,
                expression.source_vectors,
            );
        }
    }

    /// The sole conversion-constraint classifier. Callers first test standard
    /// assignment compatibility, including null pointers and pointer-to-bool.
    /// C99: §6.5.16.1p1, p. 92; PDF p. 104.
    fn pointer_conversion_violation(
        &mut self,
        to: TypeId,
        from: TypeId,
    ) -> Option<DiagnosticGroup> {
        match (self.pointer_target(to), self.pointer_target(from)) {
            | (Some(to), Some(from)) => {
                if (self.void_target(to) && self.function_target(from))
                    || (self.function_target(to) && self.void_target(from))
                {
                    return None;
                }
                if self.function_target(to) && self.function_target(from) {
                    return Some(DiagnosticGroup::IncompatibleFunctionPointerTypes);
                }
                // Immediate qualifiers are a qualifier diagnostic only if the
                // unqualified types are compatible; unrelated types stay
                // errors.
                if self.pointer_compatible(to, from, true)
                    || self.targets_without_qualifiers(to, from)
                {
                    return Some(DiagnosticGroup::PointerQualifiers);
                }
                if self.pointer_signedness(to, from) {
                    return Some(DiagnosticGroup::PointerSign);
                }
                Some(DiagnosticGroup::IncompatiblePointerTypes)
            },
            | (Some(_), None) if self.integer_type(from).is_some() =>
                Some(DiagnosticGroup::IntConversion),
            | (None, Some(_)) if self.integer_type(to).is_some() =>
                Some(DiagnosticGroup::IntConversion),
            | _ => None,
        }
    }

    /// Accepts only scalar pointer comparisons. Other invalid operands retain
    /// the existing error and recovered unknown result.
    /// C99: §6.5.9p2, p. 86; PDF p. 98; §6.5.8p2, p. 85; PDF p. 97.
    pub(super) fn pointer_comparison(
        &mut self,
        left: ExpressionInfo<'tu>,
        right: ExpressionInfo<'tu>,
        lt: TypeId,
        rt: TypeId,
        equality: bool,
        source: SourceVectors,
    ) -> bool {
        if let (Some(lp), Some(rp)) = (self.pointer_target(lt), self.pointer_target(rt)) {
            let compatible = self.pointer_compatible(lp, rp, equality);
            let ordered_functions =
                !equality && self.function_target(lp) && self.function_target(rp);
            if ordered_functions {
                // Clang emits this independently of a distinct-type warning.
                self.pointer_diagnostic(DiagnosticGroup::OrderedCompareFunctionPointers, source);
            }
            if equality
                && ((self.void_target(lp) && self.function_target(rp))
                    || (self.function_target(lp) && self.void_target(rp)))
            {
                self.context.report_extension_since(
                    "equality comparison between function pointer and void pointer",
                    FeatureOrigin::Gnu,
                    source,
                );
            } else if !compatible {
                self.pointer_diagnostic(DiagnosticGroup::CompareDistinctPointerTypes, source);
            } else if !ordered_functions {
                self.context.report_extension_since(
                    "comparison of function or void pointers",
                    FeatureOrigin::Gnu,
                    source,
                );
            }
            self.convert(right.expression, lt, ConversionKind::Assignment);
            return true;
        }
        let (integer, pointer) =
            if self.pointer_target(lt).is_some() && self.integer_type(rt).is_some() {
                (right, lt)
            } else if self.pointer_target(rt).is_some() && self.integer_type(lt).is_some() {
                (left, rt)
            } else {
                return false;
            };
        if !equality && self.null_pointer_constant(integer) {
            self.context.report_extension_since(
                "ordered comparison between pointer and zero",
                FeatureOrigin::Gnu,
                source,
            );
        } else {
            self.pointer_diagnostic(
                if equality {
                    DiagnosticGroup::PointerIntegerCompare
                } else {
                    DiagnosticGroup::OrderedPointerIntegerCompare
                },
                source,
            );
        }
        self.convert(integer.expression, pointer, ConversionKind::Assignment);
        true
    }

    /// The result type of conditional arms that are a pointer pair or a
    /// pointer and an integer, after null pointer constants have chosen the
    /// other arm's type; `None` for every other pairing. As in Clang, a
    /// void/object pair yields `void *`, compatible targets yield their
    /// composite, and both carry the union of the targets' qualifiers. The
    /// rest violate the constraint, which Clang reports as a warning:
    /// mismatched targets yield an unqualified `void *` and an integer arm
    /// converts to the pointer arm's type.
    /// C99: §6.5.15p3,p6, pp. 90-91; PDF pp. 102-103.
    pub(super) fn conditional_pointers(
        &mut self,
        lt: TypeId,
        rt: TypeId,
        source: SourceVectors,
    ) -> Option<TypeId> {
        let (lp, rp) = match (self.pointer_target(lt), self.pointer_target(rt)) {
            | (Some(lp), Some(rp)) => (lp, rp),
            | (Some(_), None) | (None, Some(_)) => {
                let (pointer, integer) = if self.pointer_target(lt).is_some() {
                    (lt, rt)
                } else {
                    (rt, lt)
                };
                _ = self.integer_type(integer)?;
                self.pointer_diagnostic(DiagnosticGroup::ConditionalTypeMismatch, source);
                return Some(pointer);
            },
            | (None, None) => return None,
        };
        // Array element qualifiers stay part of the target here: Clang
        // reports `int (*)[3]` against `const int (*)[3]` as a mismatch.
        let target = if self.void_target(lp) && !self.function_target(rp) {
            lp
        } else if self.void_target(rp) && !self.function_target(lp) {
            rp
        } else if self.types.unanalyzed(lp) || self.types.unanalyzed(rp) {
            lp
        } else if let Some(composite) = self.types.composite(lp.unqualified(), rp.unqualified()) {
            composite
        } else {
            self.pointer_diagnostic(DiagnosticGroup::PointerTypeMismatch, source);
            let void = self.types.scalar(Scalar::Void);
            return Some(self.types.intern(TypeKind::Pointer(void)));
        };
        Some(self.types.intern(TypeKind::Pointer(
            target.qualified(lp.qualifiers | rp.qualifiers),
        )))
    }

    /// Walk only pointer/array derivations, never recurse. Qualifiers do not
    /// turn mismatched tags, array extents or function signatures into
    /// warnings.
    fn targets_without_qualifiers(&mut self, mut to: TypeId, mut from: TypeId) -> bool {
        loop {
            match (self.types.nodes[to.index], self.types.nodes[from.index]) {
                | (TypeKind::Pointer(t), TypeKind::Pointer(f)) => {
                    to = t;
                    from = f;
                },
                | (TypeKind::Array(t, tb), TypeKind::Array(f, fb)) if tb == fb => {
                    to = t;
                    from = f;
                },
                | _ =>
                    return self
                        .types
                        .composite(to.unqualified(), from.unqualified())
                        .is_some(),
            }
        }
    }

    /// Clang's sign check for otherwise incompatible targets: they differ
    /// only in sign when they agree after a character type becomes
    /// `unsigned char` and a signed integer type, or an enumeration whose
    /// compatible type is signed, becomes the corresponding unsigned type.
    /// An unsigned enumeration stays itself, so it differs from `int`.
    fn pointer_signedness(&self, to: TypeId, from: TypeId) -> bool {
        // Identical kinds were compatible already, so equality here means
        // integer targets that differ in sign.
        self.sign_erased(to) == self.sign_erased(from)
    }

    fn sign_erased(&self, ty: TypeId) -> TypeKind<'tu> {
        let unsigned = |scalar| match scalar {
            | Scalar::Char | Scalar::SignedChar | Scalar::UnsignedChar =>
                Some(Scalar::UnsignedChar),
            | Scalar::Short => Some(Scalar::UnsignedShort),
            | Scalar::Int => Some(Scalar::UnsignedInt),
            | Scalar::Long => Some(Scalar::UnsignedLong),
            | Scalar::LongLong => Some(Scalar::UnsignedLongLong),
            | Scalar::Int128 => Some(Scalar::UnsignedInt128),
            | _ => None,
        };
        let kind = self.types.nodes[ty.index];
        let scalar = match kind {
            | TypeKind::Scalar(scalar) => scalar,
            | TypeKind::Tag(id) if self.types.tags[id].kind == TagKind::Enum => {
                let compatible = self.types.tags[id].compatible.get();
                // An enumeration is not a character type, so only a signed
                // compatible type changes it.
                if matches!(compatible, Scalar::Char | Scalar::UnsignedChar) {
                    return kind;
                }
                compatible
            },
            | _ => return kind,
        };
        unsigned(scalar).map_or(kind, TypeKind::Scalar)
    }

    fn void_target(&self, ty: TypeId) -> bool {
        matches!(self.types.nodes[ty.index], TypeKind::Scalar(Scalar::Void))
    }

    fn function_target(&self, ty: TypeId) -> bool {
        matches!(self.types.nodes[ty.index], TypeKind::Function { .. })
    }
}

/// Whether a host `long double` converts to an integer exactly: it has no
/// fraction and is not -0. C99: §6.3.1.4 paragraph 1, p. 43; PDF p. 55.
fn integral_x87(value: crate::float_parsing::LongDouble) -> bool {
    let Some((negative, exponent, significand)) = value.x87_parts() else {
        return false;
    };
    // Clang's `APFloat::convertToInteger` finds -0 inexact: the integer
    // loses its sign.
    if significand == 0 {
        return !negative;
    }
    let Some(shift) = (16383 + 63_u16).checked_sub(exponent) else {
        return true;
    };
    shift < 64 && significand & ((1_u64 << shift) - 1) == 0
}
