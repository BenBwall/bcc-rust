//! Classifies pointer constraints once, before shared diagnostic policy decides
//! severity. Ordinary conversion records remain the lowering interface.
//! C99: §6.5.16.1p1, p. 92; PDF p. 104; §6.5.9p2, p. 86; PDF p. 98;
//! §6.5.8p2, p. 85; PDF p. 97; §6.5.2.2p2,p7, pp. 71-72; PDF pp. 83-84;
//! §6.8.6.4p3, p. 139; PDF p. 151; §6.7.8p11, p. 126; PDF p. 138.

use super::{
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
