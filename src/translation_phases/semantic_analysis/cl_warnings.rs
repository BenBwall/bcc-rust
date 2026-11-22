//! `cl`'s warnings about an implicit conversion that may lose data, C4244
//! (levels 2 to 4) and C4305 (level 2), and about a constant condition,
//! C4127 (level 4), which only the `cl` identity reports, by rules measured
//! against `cl` 19.44.
//!
//! Read [`Analyzer::check_conversion_as_cl`] first: a conversion as if by
//! assignment draws C4244 for a floating value made an integer, for a value
//! made a narrower type, and C4305 for a constant its new type does not
//! hold. It names both types as the program wrote them (`written.rs`), and
//! measures how wide the value is rather than its type (`natural.rs`).
//! [`Analyzer::check_compound_conversion_as_cl`] is the same for a compound
//! assignment, and [`Analyzer::check_constant_condition_as_cl`] is C4127.
//!
//! - `written.rs`: how a declaration wrote a type, which `cl` names.
//! - `natural.rs`: how wide `cl` takes an integer operand's value to be.
//!
//! Microsoft Learn: "Compiler Warning (level 2) C4244", "Compiler Warning
//! (levels 3 and 4) C4244", "Compiler Warning (level 1) C4305" (which `cl`
//! 19.44 shows from level 2), "Compiler Warning (level 4) C4127".
//! C99: §6.3.1.3, p. 43; PDF p. 55; §6.3.1.4 paragraph 1, p. 43; PDF p. 55;
//! §6.3.1.5 paragraph 2, p. 44; PDF p. 56 (the conversions);
//! §6.5.16.1p2, p. 92; PDF p. 104; §6.5.16.2 paragraph 3, p. 93; PDF p. 105
//! (assignments); §6.8.4.1p2, p. 133; PDF p. 145; §6.8.5, p. 135; PDF p. 147
//! (conditions).

mod natural;
mod written;

pub(super) use written::{
    ClTarget,
    Written,
};

use super::{
    Analyzer,
    ExpressionInfo,
    SemanticErrorKind,
    types::{
        Scalar,
        TypeId,
        TypeKind,
    },
};
use crate::{
    configuration::DiagnosticGroup as G,
    diagnostics::format_in,
    translation_phases::{
        SourceVectors,
        parsing::{
            declaration_syntax::TypeQualifiers,
            syntax::{
                BinaryOperator as B,
                ConditionalExpression,
                Constant,
                Expression,
                ExpressionSlot,
                ExpressionType as E,
                UnaryOperator as U,
            },
        },
    },
    util::bump::ArenaVec,
};

/// How a value reaches the type it is converted to, which `cl` names
/// first.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ClConversion {
    Initializing,
    /// A simple assignment.
    Assignment,
    /// A prototyped call's argument.
    Function,
    Return,
}

impl ClConversion {
    const fn words(self) -> &'static str {
        match self {
            | Self::Initializing => "initializing",
            | Self::Assignment => "=",
            | Self::Function => "function",
            | Self::Return => "return",
        }
    }
}

/// A conversion warning `cl` gives: C4305 when `truncation`, else C4244,
/// C4242 or C4245 by its group, from and to the types as `cl` names them, at
/// `site`.
#[derive(Clone, Copy, Debug)]
pub(super) struct ClFinding<'s> {
    group:      G,
    truncation: bool,
    from:       &'s str,
    to:         &'s str,
    site:       SourceVectors,
}

/// The operand of a conditional expression `cl` keeps
/// ([`Analyzer::cl_condition`]).
#[derive(Clone, Copy, Debug)]
pub(super) enum ClCondition<'tu> {
    /// A constant: `cl` folds the expression to the operand it selects.
    Folded(&'tu Expression<'tu>),
    /// A condition of [`ClTruth::Unknown`] value: `cl` folds it, to an
    /// operand bcc cannot tell.
    Unknown,
    /// Not a constant to `cl`, which keeps both operands.
    Runtime,
}

/// The truth value `cl` folds a condition to ([`Analyzer::cl_truth`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ClTruth {
    Constant(bool),
    /// A constant whose value bcc did not compute, as one whose arithmetic
    /// overflows.
    Unknown,
    /// Not a constant to `cl`.
    Runtime,
}

/// An arithmetic type as `cl`'s conversion warnings sort it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Arithmetic {
    /// `_Bool`.
    Bool,
    /// An integer type of this width; `int` and `unsigned int` are `wide`
    /// as `int` is, the narrowing from which is level 4.
    Integer {
        bits:   u8,
        signed: bool,
        int:    bool,
    },
    /// `float`, `double` and `long double` by rank: `cl` warns from
    /// `long double` to `double` though both are 64 bits.
    Floating(u8),
}

impl<'tu, 's> Analyzer<'_, 'tu, 's> {
    /// `cl`'s C4244 and C4305 for the conversion of `source` to `target`
    /// as if by assignment, which `spelled_target` names, reported at
    /// `site`:
    ///
    /// - a floating value made an integer is C4244 at level 2, and a floating
    ///   constant too, `_Bool` included, where a value or a conditional
    ///   expression is not;
    /// - a value made a narrower floating type is C4244 at level 3, or at level
    ///   4 from `long double` to `double`; a constant, C4305 when the type does
    ///   not hold it exactly;
    /// - an integer made a floating type is C4244 at level 2 from a 32-bit type
    ///   to `float` and from a 64-bit type; a constant, C4305 when the type
    ///   does not hold it exactly;
    /// - an integer made a narrower integer type is C4244 when the value is
    ///   wider than the type (`natural.rs`), at level 4 from `int` or `unsigned
    ///   int` and level 3 from the others; a constant, C4305 when it fits
    ///   neither the signed nor the unsigned type of the target's width (`cl`'s
    ///   C4245 for a negative one made unsigned is not reported).
    ///
    /// Neither applies to an enumeration on either side, nor to a value
    /// made `_Bool`. `cl` names a type as written (`written.rs`), and an
    /// operand whose type this cannot name draws nothing.
    pub(super) fn check_conversion_as_cl(
        &mut self,
        target: TypeId,
        spelled_target: Option<&'s str>,
        source: ExpressionInfo<'tu>,
        conversion: ClConversion,
        site: SourceVectors,
    ) {
        if let Some(finding) = self.cl_conversion(target, spelled_target, source, site) {
            self.cl_conversion_warning(finding, conversion.words());
        }
    }

    /// [`Self::check_conversion_as_cl`] of the simple assignment `e` of
    /// `right` to `left`, under the `cl` identity.
    pub(super) fn cl_assignment(
        &mut self,
        e: &'tu Expression<'tu>,
        left: ExpressionInfo<'tu>,
        right: ExpressionInfo<'tu>,
    ) {
        if self.unused.cl {
            let spelled = self.cl_lvalue_spelling(left.expression);
            let site = e.operator_source_vectors.unwrap_or(e.source_vectors);
            self.check_conversion_as_cl(left.ty, spelled, right, ClConversion::Assignment, site);
        }
    }

    /// What [`Self::check_conversion_as_cl`] reports, if anything.
    fn cl_conversion(
        &self,
        target: TypeId,
        spelled_target: Option<&'s str>,
        source: ExpressionInfo<'tu>,
        site: SourceVectors,
    ) -> Option<ClFinding<'s>> {
        let (Some(to), Some(from)) = (self.cl_arithmetic(target), self.cl_arithmetic(source.ty))
        else {
            return None;
        };
        let source = self.cl_folded_constant(source)?;
        if !self.cl_conversions_checked(site) || self.cl_unknown_constant(source) {
            return None;
        }
        let site = self.cl_value_site(source.expression);
        let constant = self.cl_constant(source);
        let (mut group, truncation) = match (from, to) {
            // Not for a conditional expression, folded or not.
            | (Arithmetic::Floating(_), Arithmetic::Bool)
                if !constant
                    || !self.cl_floating_one(source)
                    || matches!(
                        Self::without_parentheses(source.expression).kind,
                        E::Conditional(_)
                    ) =>
                return None,
            | (Arithmetic::Floating(_), Arithmetic::Bool | Arithmetic::Integer { .. }) =>
                (G::ClFloatingConversion, false),
            | (Arithmetic::Floating(source_rank), Arithmetic::Floating(target_rank))
                if source_rank > target_rank =>
                if constant {
                    if self.cl_holds_exactly(source, target) {
                        return None;
                    }
                    (G::ClTruncation, true)
                } else if target_rank == 2 {
                    (G::ClIntConversion, false)
                } else {
                    (G::ClWideConversion, false)
                },
            | (Arithmetic::Integer { bits, .. }, Arithmetic::Floating(target_rank)) =>
                if constant {
                    if self.cl_holds_exactly(source, target) {
                        return None;
                    }
                    (G::ClTruncation, true)
                } else if (bits == 64 || (bits == 32 && target_rank == 1))
                    && !Self::cl_truth_value(source.expression)
                {
                    (G::ClFloatingConversion, false)
                } else {
                    return None;
                },
            // C4245, a negative constant made unsigned, whatever the widths.
            | (Arithmetic::Integer { .. }, Arithmetic::Integer { signed: false, .. })
                if constant
                    && source
                        .integer
                        .is_some_and(|value| value.signed && value.value < 0) =>
                (G::ClSignedConversion, false),
            | (
                Arithmetic::Integer { bits, int, .. },
                Arithmetic::Integer {
                    bits: width,
                    signed,
                    ..
                },
            ) if width < bits =>
                if constant {
                    if Self::cl_constant_fits(source, u32::from(width), signed) {
                        return None;
                    }
                    (G::ClTruncation, true)
                } else if self
                    .cl_natural_bits(source.expression)
                    .is_none_or(|natural| natural <= u32::from(width))
                {
                    return None;
                } else if int {
                    (G::ClIntConversion, false)
                } else {
                    (G::ClWideConversion, false)
                },
            | _ => return None,
        };
        if matches!(group, G::ClIntConversion | G::ClWideConversion)
            && matches!(from, Arithmetic::Integer { .. })
            && (matches!(from, Arithmetic::Integer { bits: 64, .. })
                || self.cl_designated(source.expression))
            && self.cl_shown(G::ClNarrowingConversion, site)
        {
            group = G::ClNarrowingConversion;
        }
        let (Some(to), Some(from)) = (spelled_target, self.cl_spelling(source.expression)) else {
            return None;
        };
        Some(ClFinding {
            group,
            truncation,
            from,
            to,
            site,
        })
    }

    /// [`Self::check_conversion_as_cl`] of an initializer's value `source`
    /// to the subobject `target` names, of type `ty`, under the `cl`
    /// identity. `cl` reports an automatic object's after the warnings of
    /// the rest of the function, before those it gives at its end; a static
    /// one's in place.
    pub(super) fn cl_initializer(
        &mut self,
        ty: TypeId,
        source: ExpressionInfo<'tu>,
        target: ClTarget,
        static_storage: bool,
    ) {
        if !self.unused.cl {
            return;
        }
        let site = self.cl_value_site(source.expression);
        let spelled = self.cl_target_spelling(target, ty);
        let Some(finding) = self.cl_conversion(ty, spelled, source, site) else {
            return;
        };
        if self.functions.current.is_some() && !static_storage {
            self.cl_initializers.push(finding);
        } else {
            self.cl_conversion_warning(finding, ClConversion::Initializing.words());
        }
    }

    /// Reports the automatic objects' initializer conversions of the
    /// function being left, under the settings where each is, after its
    /// other warnings and before the ones `cl` gives at its end.
    pub(super) fn report_cl_initializers(&mut self) {
        let pending = std::mem::replace(&mut self.cl_initializers, ArenaVec::new_in(self.scratch));
        let order = match self.functions.current {
            | Some(function) => super::ReportOrder::After(self.context.file_location(
                crate::translation_phases::Context::last_source_vector(
                    function.syntax.body.source_vectors,
                ),
            )),
            | None => self.report_order,
        };
        self.with_report_order(order, |analyzer| {
            for &finding in &pending {
                analyzer.cl_conversion_warning(finding, ClConversion::Initializing.words());
            }
        });
    }

    /// C4244 and C4305 for the compound assignment `e` of `right` to `left`,
    /// whose operation is in `operation`, at `right`, under the `cl`
    /// identity: as for a
    /// simple one from the operation's type, which `cl` names, except that
    /// `+=`, `-=`, `*=` and `/=` narrow an integer only when `right` is
    /// wider than `left`, the other operators never, an integer made a
    /// floating type draws nothing, and `_Bool` is a narrower type like
    /// another. A constant `right` of an integer operation is left to
    /// `cl`.
    pub(super) fn check_compound_conversion_as_cl(
        &mut self,
        e: &'tu Expression<'tu>,
        left: &'tu Expression<'tu>,
        right: &'tu Expression<'tu>,
        operation: TypeId,
    ) {
        let E::Binary { operator, .. } = e.kind else {
            return;
        };
        if !self.unused.cl {
            return;
        }
        let left_info = self.expression_info(left);
        let Some(right_info) = self.cl_folded_constant(self.expression_info(right)) else {
            return;
        };
        let right = right_info.expression;
        let operation = self.types.non_atomic(operation).unqualified();
        let (Some(to), Some(from)) = (
            self.cl_arithmetic(left_info.ty),
            self.cl_arithmetic(operation),
        ) else {
            return;
        };
        let site = self.cl_value_site(right);
        if !self.cl_conversions_checked(site) || self.cl_unknown_constant(right_info) {
            return;
        }
        let constant = self.cl_constant(right_info);
        let (mut group, truncation) = match (from, to) {
            | (Arithmetic::Floating(_), Arithmetic::Bool | Arithmetic::Integer { .. }) =>
                (G::ClFloatingConversion, false),
            | (Arithmetic::Floating(source_rank), Arithmetic::Floating(target_rank))
                if source_rank > target_rank =>
                if constant {
                    if self.cl_holds_exactly(right_info, left_info.ty) {
                        return;
                    }
                    (G::ClTruncation, true)
                } else if target_rank == 2 {
                    (G::ClIntConversion, false)
                } else {
                    (G::ClWideConversion, false)
                },
            | (Arithmetic::Integer { int, .. }, Arithmetic::Integer { .. } | Arithmetic::Bool)
                if !constant
                    && matches!(
                        operator,
                        B::AdditionAssignment
                            | B::SubtractionAssignment
                            | B::MultiplicationAssignment
                            | B::DivisionAssignment
                    ) =>
            {
                let width = match to {
                    | Arithmetic::Integer { bits, .. } => u32::from(bits),
                    | _ => 8,
                };
                if self
                    .cl_natural_bits(right)
                    .is_none_or(|natural| natural <= width)
                {
                    return;
                }
                if int {
                    (G::ClIntConversion, false)
                } else {
                    (G::ClWideConversion, false)
                }
            },
            | _ => return,
        };
        // C4242 takes an operand that already has the operation's type.
        let right_ty = self.types.non_atomic(right_info.ty).unqualified();
        if matches!(group, G::ClIntConversion | G::ClWideConversion)
            && matches!(from, Arithmetic::Integer { .. })
            && (matches!(from, Arithmetic::Integer { bits: 64, .. })
                || (to != Arithmetic::Bool && right_ty == operation && self.cl_designated(right)))
            && self.cl_shown(G::ClNarrowingConversion, site)
        {
            group = G::ClNarrowingConversion;
        }
        let (Some(to), Some(from)) = (
            self.cl_lvalue_spelling(left),
            self.cl_operation_spelling(right, operation),
        ) else {
            return;
        };
        let words = super::always_nonnull::binary_spelling(operator);
        self.cl_conversion_warning(
            ClFinding {
                group,
                truncation,
                from,
                to,
                site,
            },
            words,
        );
    }

    /// Where `cl` reports the conversion of the value `e`: at the operator
    /// of the expression that computes it (the `?` of a conditional
    /// expression), or where it begins; for a conditional expression it
    /// folds, where it reports the operand it selects. A floating value in
    /// parentheses, but not a constant, it reports at the first of them.
    fn cl_value_site(&self, mut e: &'tu Expression<'tu>) -> SourceVectors {
        let info = self.expression_info(e);
        if matches!(e.kind, E::Parenthesized { .. })
            && matches!(self.cl_arithmetic(info.ty), Some(Arithmetic::Floating(_)))
            && !self.cl_constant(info)
        {
            return e.source_vectors;
        }
        loop {
            match e.kind {
                | E::Parenthesized { expression } => e = expression,
                | E::Conditional(conditional) => match self.cl_condition(conditional) {
                    | ClCondition::Folded(selected) => e = selected,
                    | ClCondition::Unknown | ClCondition::Runtime =>
                        return e.operator_source_vectors.unwrap_or(e.source_vectors),
                },
                | E::Binary { .. } => return e.operator_source_vectors.unwrap_or(e.source_vectors),
                | _ => return e.source_vectors,
            }
        }
    }

    /// Whether `cl` checks conversions at all at `site`: only where the
    /// warning level is 2 or more, whatever `/w1`, `/we` or a pragma says
    /// of the warnings themselves.
    fn cl_conversions_checked(&self, site: SourceVectors) -> bool {
        self.context.configuration_at(site).cl_warnings().level() >= 2
    }

    /// Whether `source` is a constant bcc did not fold, as one whose
    /// arithmetic overflows, of which `cl` reports C4307 and its own value.
    fn cl_unknown_constant(&self, source: ExpressionInfo<'tu>) -> bool {
        self.cl_comma_free(source.expression)
            && source.constant == super::ConstantClass::Arithmetic
            && source.integer.is_none()
            && source.floating().is_none()
    }

    /// Whether `e` is a value `cl` has not computed at `int`, which C4242
    /// reports in place of C4244 when it is shown: an object or member
    /// read, a call, a cast to the operand's own type, or the operand a
    /// constant `?:` selects when it has the type of the whole.
    fn cl_designated(&self, e: &'tu Expression<'tu>) -> bool {
        match e.kind {
            | E::Parenthesized { expression } => self.cl_designated(expression),
            | E::Identifier(_) => self.expression_info(e).binding.is_some_and(|binding| {
                self.bindings[binding].kind != super::BindingKind::Enumerator
            }),
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
            } => true,
            | E::Cast {
                operand_expression, ..
            } =>
                self.types
                    .non_atomic(self.expression_info(operand_expression).ty)
                    .unqualified()
                    == self
                        .types
                        .non_atomic(self.expression_info(e).ty)
                        .unqualified()
                    && self.cl_designated(operand_expression),
            // The selected operand, unless `cl` converts it to the type of
            // the whole.
            | E::Conditional(conditional) => match self.cl_condition(conditional) {
                | ClCondition::Folded(selected) =>
                    self.types
                        .non_atomic(self.expression_info(selected).ty)
                        .unqualified()
                        == self
                            .types
                            .non_atomic(self.expression_info(e).ty)
                            .unqualified()
                        && self.cl_designated(selected),
                | ClCondition::Unknown | ClCondition::Runtime => false,
            },
            | _ => false,
        }
    }

    /// The operand `cl` folds `conditional` to, by its condition's truth
    /// value ([`Self::cl_truth`]).
    /// C99: §6.5.15 paragraph 4, p. 90; PDF p. 102.
    pub(super) fn cl_condition(
        &self,
        conditional: &ConditionalExpression<'tu>,
    ) -> ClCondition<'tu> {
        match self.cl_truth(conditional.condition_expression) {
            | ClTruth::Constant(true) => ClCondition::Folded(conditional.then_expression),
            | ClTruth::Constant(false) => ClCondition::Folded(conditional.else_expression),
            | ClTruth::Unknown => ClCondition::Unknown,
            | ClTruth::Runtime => ClCondition::Runtime,
        }
    }

    /// The truth value `cl` folds the condition `e` to, which decides the
    /// operand of `?:` it keeps, and whether it keeps the right operand of
    /// `&&` and `||`: a constant when `e` is an integer constant, an integer
    /// made a pointer (a null pointer false), or an arithmetic floating
    /// constant (`-0.0` false), but not an address or string literal, nor a
    /// comma expression. Measured against `cl` 19.44.
    pub(super) fn cl_truth(&self, mut e: &'tu Expression<'tu>) -> ClTruth {
        while let E::Parenthesized { expression } = e.kind {
            e = expression;
        }
        if matches!(
            e.kind,
            E::Binary {
                operator: B::Comma,
                ..
            }
        ) {
            return ClTruth::Runtime;
        }
        let info = self.expression_info(e);
        match info.integer {
            | Some(value) => ClTruth::Constant(!value.is_zero()),
            | None if info.constant == super::ConstantClass::Arithmetic => match self
                .floating_value(info)
            {
                | Some(value) => ClTruth::Constant(!(value.real.is_zero() && value.imag.is_zero())),
                | None => ClTruth::Unknown,
            },
            | None => ClTruth::Runtime,
        }
    }

    /// Whether the settings at `site` show the warnings of `group`.
    fn cl_shown(&self, group: G, site: SourceVectors) -> bool {
        self.context
            .configuration_at(site)
            .diagnostic_severity(group)
            .is_some()
    }

    /// Whether the floating constant `source` is 1, the one `cl` reports
    /// made `_Bool`.
    fn cl_floating_one(&self, source: ExpressionInfo<'tu>) -> bool {
        self.floating_value(source).is_some_and(|value| {
            value.imag.is_zero()
                && value
                    .real
                    .compare(crate::float_parsing::LongDouble::from_double(1.0))
                    == 0
        })
    }

    /// `cl`'s name for the type `operation` of a compound assignment's
    /// arithmetic: `right`'s when it has the type, else the basic type.
    fn cl_operation_spelling(
        &self,
        right: &'tu Expression<'tu>,
        operation: TypeId,
    ) -> Option<&'s str> {
        let right_ty = self
            .types
            .non_atomic(self.expression_info(right).ty)
            .unqualified();
        if right_ty == operation {
            self.cl_spelling(right)
        } else {
            self.cl_basic(operation, TypeQualifiers::empty())
        }
    }

    /// Reports `finding` in `cl`'s words, for the conversion `words` names.
    fn cl_conversion_warning(&mut self, finding: ClFinding<'s>, words: &str) {
        let ClFinding {
            group,
            truncation,
            from,
            to,
            site,
        } = finding;
        let text = if truncation {
            format_in!(
                self.scratch,
                "'{words}': truncation from '{from}' to '{to}'"
            )
        } else if group == G::ClSignedConversion {
            format_in!(
                self.scratch,
                "'{words}': conversion from '{from}' to '{to}', signed/unsigned mismatch"
            )
        } else {
            format_in!(
                self.scratch,
                "'{words}': conversion from '{from}' to '{to}', possible loss of data"
            )
        };
        let text = self.context.string_cache.intern(text);
        self.group_diagnostic(
            group,
            SemanticErrorKind::ValueWarning(group),
            site,
            Some(text),
            None,
        );
    }

    /// How `cl`'s conversion warnings sort the arithmetic type `ty`: `None`
    /// for an enumeration and for any type but the standard real ones.
    fn cl_arithmetic(&self, ty: TypeId) -> Option<Arithmetic> {
        let ty = self.types.non_atomic(ty).unqualified();
        let TypeKind::Scalar(scalar) = self.types.nodes[ty.index] else {
            return None;
        };
        Some(match scalar {
            | Scalar::Bool => Arithmetic::Bool,
            | Scalar::Float => Arithmetic::Floating(1),
            | Scalar::Double => Arithmetic::Floating(2),
            | Scalar::LongDouble => Arithmetic::Floating(3),
            | Scalar::Char
            | Scalar::SignedChar
            | Scalar::UnsignedChar
            | Scalar::Short
            | Scalar::UnsignedShort
            | Scalar::Int
            | Scalar::UnsignedInt
            | Scalar::Long
            | Scalar::UnsignedLong
            | Scalar::LongLong
            | Scalar::UnsignedLongLong => {
                let (bits, signed) = self.integer_type(ty)?;
                Arithmetic::Integer {
                    bits: u8::try_from(bits).ok()?,
                    signed,
                    int: matches!(scalar, Scalar::Int | Scalar::UnsignedInt),
                }
            },
            | _ => return None,
        })
    }

    /// Whether `source` is a constant to `cl`: an integer constant
    /// expression, or a floating one, without a comma operator
    /// ([`Self::cl_comma_free`]).
    fn cl_constant(&self, source: ExpressionInfo<'tu>) -> bool {
        self.cl_comma_free(source.expression) && Self::cl_folded(source)
    }

    /// [`Self::cl_constant`] but for the comma operator.
    fn cl_folded(source: ExpressionInfo<'tu>) -> bool {
        (source.integer.is_some()
            && (source.ice || source.constant == super::ConstantClass::Arithmetic))
            || (source.constant == super::ConstantClass::Arithmetic
                && source.floating().is_some()
                && source.integer.is_none())
    }

    /// Whether the constant `source` converts to the floating type `target`
    /// exactly, or to an infinity or a zero, which `cl` reports otherwise
    /// (C4056) or not at all.
    fn cl_holds_exactly(&self, source: ExpressionInfo<'tu>, target: TypeId) -> bool {
        let target = self.types.non_atomic(target).unqualified();
        let exact = match source.integer {
            | Some(integer) if source.floating().is_none() => self
                .integer_value(integer, crate::ir::Type::F80)
                .zip(self.converted_floating(source, target))
                .map(|(wide, narrow)| (wide, narrow.real)),
            | _ => self
                .floating_value(source)
                .zip(self.converted_floating(source, target))
                .map(|(wide, narrow)| (wide.real, narrow.real)),
        };
        exact.is_none_or(|(wide, narrow)| {
            narrow.is_infinite() || narrow.is_nan() || narrow.is_zero() || wide.compare(narrow) == 0
        })
    }

    /// Whether the integer constant `source` fits the signed or the
    /// unsigned type of `width` bits, or is negative and made unsigned,
    /// which `cl` reports as C4245. `cl` reads a 64-bit unsigned constant as
    /// a signed one: `0xFFFFFFFFFFFFFF80ull` is -128, which fits.
    fn cl_constant_fits(source: ExpressionInfo<'tu>, width: u32, signed: bool) -> bool {
        let Some(value) = source.integer.filter(|value| value.wide.is_none()) else {
            return true;
        };
        if (1_i128 << 63..1_i128 << 64).contains(&value.value) {
            return natural::cl_signed_reading(value.value) >= -(1_i128 << (width - 1));
        }
        let value = value.value;
        if value < 0 {
            !signed || value >= -(1_i128 << (width - 1))
        } else {
            value < 1_i128 << width
        }
    }

    /// Whether `e` is a truth value `cl` gives its own type: `!`, whose
    /// `int` it does not convert to `float` as another's.
    fn cl_truth_value(mut e: &'tu Expression<'tu>) -> bool {
        while let E::Parenthesized { expression } = e.kind {
            e = expression;
        }
        matches!(
            e.kind,
            E::Unary {
                operator: U::LogicalNot,
                ..
            } | E::Binary {
                operator: B::LogicalAnd | B::LogicalOr,
                ..
            }
        )
    }

    /// `cl`'s C4127, at level 4: an `if`, `while`, `do` or `for` condition
    /// that is a constant, unless it is an integer literal of 0 or 1, as
    /// `while (1)` and `do ... while (0)` are, under parentheses, unary
    /// operators, a cast to its own type and the selected operand of a
    /// constant `?:`, all of which `cl` folds to such a literal. A constant
    /// to `cl` is an arithmetic one, a null pointer or another integer made
    /// a pointer, or a string literal, but not an object's or a function's
    /// address, nor one with a comma operator; and `0 && x` is one. Reported
    /// at the operator of the folded expression, or where it begins, under
    /// its parentheses and casts and in the operand a constant `?:` selects.
    pub(super) fn check_constant_condition_as_cl(&mut self, slot: ExpressionSlot<'tu>) {
        let Some(e) = super::statements::slot_expression(slot) else {
            return;
        };
        if !self.cl_constant_condition(e) {
            return;
        }
        if self
            .cl_literal_value(e)
            .is_some_and(|value| value == 0 || value == 1)
        {
            return;
        }
        let mut folded = e;
        loop {
            folded = match folded.kind {
                | E::Parenthesized { expression: inner }
                | E::Cast {
                    operand_expression: inner,
                    ..
                } => inner,
                | E::Conditional(conditional) => match self.cl_condition(conditional) {
                    | ClCondition::Folded(selected) => selected,
                    | ClCondition::Unknown | ClCondition::Runtime => break,
                },
                | _ => break,
            };
        }
        let site = folded
            .operator_source_vectors
            .unwrap_or(folded.source_vectors);
        let text = self
            .context
            .string_cache
            .intern("conditional expression is constant");
        let group = G::ClConstantCondition;
        self.group_diagnostic(
            group,
            SemanticErrorKind::ValueWarning(group),
            site,
            Some(text),
            None,
        );
    }

    /// Whether no comma operator remains in `e` once `cl` has folded each
    /// constant `?:` to the operand it selects, outside `sizeof` and
    /// `_Alignof`: `cl` folds no expression with one, nor one with a `?:`
    /// whose condition is not a constant, though bcc may.
    fn cl_comma_free(&self, e: &'tu Expression<'tu>) -> bool {
        let mut work = ArenaVec::new_in(self.scratch);
        work.push(e);
        while let Some(operand) = work.pop() {
            match operand.kind {
                | E::Binary {
                    operator: B::Comma, ..
                } => return false,
                | E::Conditional(conditional) => match self.cl_condition(conditional) {
                    | ClCondition::Folded(selected) =>
                        work.extend([conditional.condition_expression, selected]),
                    | ClCondition::Unknown | ClCondition::Runtime => return false,
                },
                | E::Parenthesized { expression } => work.push(expression),
                | E::Unary {
                    operand_expression, ..
                }
                | E::Cast {
                    operand_expression, ..
                } => work.push(operand_expression),
                | E::Binary {
                    left_expression,
                    right_expression,
                    ..
                } => work.extend([left_expression, right_expression]),
                | _ => {},
            }
        }
        true
    }

    /// The value `cl` converts for `source`: `source` itself, unless `cl`
    /// folds it, through constant `?:`s, to a constant where bcc folded
    /// none, as for a pointer or floating condition. That constant takes its
    /// place when it has the type of `source`; otherwise `cl` converts it
    /// first, which bcc does not model, and reports nothing here (`None`).
    fn cl_folded_constant(&self, source: ExpressionInfo<'tu>) -> Option<ExpressionInfo<'tu>> {
        let mut e = source.expression;
        loop {
            e = match e.kind {
                | E::Parenthesized { expression } => expression,
                | E::Conditional(conditional) => match self.cl_condition(conditional) {
                    | ClCondition::Folded(selected) => selected,
                    | ClCondition::Unknown | ClCondition::Runtime => break,
                },
                | _ => break,
            };
        }
        let selected = self.expression_info(e);
        if std::ptr::eq(e, source.expression)
            || !self.cl_constant(selected)
            || self.cl_constant(source)
        {
            return Some(source);
        }
        let ty = |info: ExpressionInfo<'tu>| self.types.non_atomic(info.ty).unqualified();
        (ty(selected) == ty(source)).then_some(selected)
    }

    /// Whether `cl` folds the condition `e` to a constant, without a comma
    /// operator ([`Self::cl_comma_free`]).
    fn cl_constant_condition(&self, mut e: &'tu Expression<'tu>) -> bool {
        while let E::Parenthesized { expression } = e.kind {
            e = expression;
        }
        if !self.cl_comma_free(e) {
            return false;
        }
        let info = self.expression_info(e);
        if info.integer.is_some() {
            return true;
        }
        if matches!(e.kind, E::StringLiteral(_)) {
            return true;
        }
        info.constant == super::ConstantClass::Arithmetic && info.floating().is_some()
    }

    /// The value of `e` when `cl` folds it to an integer literal: one under
    /// parentheses, `+`, `-`, `~` and `!`, a cast to its own type, or the
    /// operand a constant `?:` selects.
    fn cl_literal_value(&self, e: &'tu Expression<'tu>) -> Option<i128> {
        match e.kind {
            | E::Parenthesized { expression } => self.cl_literal_value(expression),
            | E::Constant(Constant::Integer(_) | Constant::Char(_)) =>
                self.expression_info(e).integer.map(|value| value.value),
            | E::Unary {
                operator: U::Plus | U::Minus | U::BitwiseNot | U::LogicalNot,
                operand_expression,
            } => {
                _ = self.cl_literal_value(operand_expression)?;
                self.expression_info(e).integer.map(|value| value.value)
            },
            | E::Cast {
                operand_expression, ..
            } => {
                let own = self
                    .types
                    .non_atomic(self.expression_info(operand_expression).ty)
                    .unqualified();
                let cast = self
                    .types
                    .non_atomic(self.expression_info(e).ty)
                    .unqualified();
                if own != cast {
                    return None;
                }
                self.cl_literal_value(operand_expression)
            },
            | E::Conditional(conditional) => match self.cl_condition(conditional) {
                | ClCondition::Folded(selected) => self.cl_literal_value(selected),
                | ClCondition::Unknown | ClCondition::Runtime => None,
            },
            | _ => None,
        }
    }
}
