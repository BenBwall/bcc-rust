//! Classifies pointer constraints once, before shared diagnostic policy decides
//! severity. Ordinary conversion records remain the lowering interface.
//! C99: §6.5.16.1p1, p. 92; PDF p. 104; §6.5.9p2, p. 86; PDF p. 98;
//! §6.5.8p2, p. 85; PDF p. 97; §6.5.2.2p2,p7, pp. 71-72; PDF pp. 83-84;
//! §6.8.6.4p3, p. 139; PDF p. 151; §6.7.8p11, p. 126; PDF p. 138.

use super::{
    super::{
        WideStore,
        compiler_wording::ConversionSite,
    },
    ConversionKind,
    Expression,
    ExpressionInfo,
};
use crate::{
    configuration::{
        DiagnosticGroup,
        ExtensionPolicy,
        FeatureOrigin,
    },
    translation_phases::{
        SourceVectors,
        parsing::declaration_syntax::TypeQualifiers,
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
        self.assignment_conversion_for(target, source, ConversionSite::Other)
    }

    /// [`Self::assignment_conversion`] in a context GCC's and Clang's words
    /// name.
    pub(in crate::translation_phases::semantic_analysis) fn assignment_conversion_for(
        &mut self,
        target: TypeId,
        source: ExpressionInfo<'tu>,
        site: ConversionSite,
    ) -> bool {
        self.assignment_conversion_at(
            target,
            source,
            source.expression.source_vectors,
            site,
            false,
        )
    }

    /// [`Self::assignment_conversion`] of the right operand of the simple
    /// assignment `e`, reported at its operator, which is also Clang's
    /// context for the conversion.
    /// C99: §6.5.16.1p1, p. 92; PDF p. 104.
    pub(in crate::translation_phases::semantic_analysis) fn assigned_value_conversion(
        &mut self,
        target: TypeId,
        source: ExpressionInfo<'tu>,
        e: &'tu Expression<'tu>,
    ) -> bool {
        let operator = e.operator_source_vectors.unwrap_or(e.source_vectors);
        self.with_conversion_context(operator, |analyzer| {
            analyzer.assignment_conversion_at(
                target,
                source,
                operator,
                ConversionSite::Assignment,
                false,
            )
        })
    }

    /// [`Self::assignment_conversion`] of an initializer, reported at
    /// `diagnostic_site`; `braced` when it is an element of a brace-enclosed
    /// list written for the subobject that holds it, not one brace elision
    /// or a designator reaches inside.
    /// C99: §6.7.8p11, p. 126; PDF p. 138; §6.7.8p20, p. 127; PDF p. 139.
    pub(in crate::translation_phases::semantic_analysis) fn initializer_conversion(
        &mut self,
        target: TypeId,
        source: ExpressionInfo<'tu>,
        diagnostic_site: SourceVectors,
        braced: bool,
    ) -> bool {
        self.assignment_conversion_at(
            target,
            source,
            diagnostic_site,
            ConversionSite::Initialization,
            braced,
        )
    }

    /// Uses the assignment operator as the diagnostic location while the
    /// conversion remains attached to its operand; `braced` as
    /// [`Self::initializer_conversion`] takes it.
    /// C99: §6.5.16.1p1, p. 92; PDF p. 104.
    pub(in crate::translation_phases::semantic_analysis) fn assignment_conversion_at(
        &mut self,
        target: TypeId,
        source: ExpressionInfo<'tu>,
        diagnostic_site: SourceVectors,
        site: ConversionSite,
        braced: bool,
    ) -> bool {
        let source = self.resolve_function_value(source, Some(target));
        // GCC points at an initializer, and elsewhere where it reports the
        // conversion.
        let gcc_site = if site == ConversionSite::Initialization {
            source.expression.source_vectors
        } else {
            diagnostic_site
        };
        if self.gcc_pointer_rules() {
            let (from, _) = self.value_conversion(source);
            self.gcc_array_qualifier_pedwarn(self.types.non_atomic(target), from, gcc_site);
        }
        if !self.bool_negations.is_empty() {
            let to_bool = matches!(
                self.types.nodes[self.types.non_atomic(target).index],
                TypeKind::Scalar(Scalar::Bool)
            );
            self.convert_negation(source.expression, to_bool);
        }
        if self.assignment_compatible(target, source) {
            self.literal_conversion_warning(target, source);
            self.check_constant_conversion(target, source, braced);
            if self.pointer_target(target).is_some() && self.null_pointer_constant(source) {
                self.null_conversion_warning(source, false, diagnostic_site);
            }
            if matches!(
                self.types.nodes[target.index],
                TypeKind::Scalar(Scalar::Bool)
            ) {
                self.conversion_to_bool_warning(source);
            }
            self.gcc_void_array_qualifiers(target, source, site, gcc_site);
            return true;
        }
        let target = self.types.non_atomic(target);
        // Compatibility has already recorded lvalue conversion or decay.
        let (from, _) = self.value_conversion(source);
        if let Some(group) = self.pointer_conversion_violation(target, from) {
            let group = self.msvc_pointer_group(group, target, from);
            let wording = self.conversion_wording(group, site, target, from, source);
            // GCC points at an initializer, Clang at the declarator.
            let diagnostic_site = if wording.is_some()
                && site == ConversionSite::Initialization
                && self.context.configuration.diagnostic_format()
                    == crate::configuration::DiagnosticFormat::Gcc
            {
                source.expression.source_vectors
            } else {
                diagnostic_site
            };
            // Clang underlines the value beside a caret placed elsewhere.
            let value = source.expression.source_vectors;
            let highlight = if diagnostic_site == value {
                SourceVectors::default()
            } else {
                value
            };
            self.with_highlights([highlight, SourceVectors::default()], |analyzer| {
                analyzer.group_diagnostic(
                    group,
                    crate::translation_phases::semantic_analysis::SemanticErrorKind::PointerConstraint(
                        group,
                    ),
                    diagnostic_site,
                    wording,
                    None,
                );
            });
            // The edit the wording named went with the diagnostic, if one
            // was reported.
            self.conversion_fix_it = None;
            // TCC converts the value as a cast converts it, warning about
            // the width too.
            if group == DiagnosticGroup::IntConversion
                && self.context.configuration.compiler_identity()
                    == crate::configuration::CompilerIdentity::Tcc
            {
                self.pointer_integer_cast_warning(target, from, source, diagnostic_site);
            }
            return true;
        }
        // Clang accepts function/void pointers silently by default and reports
        // the ISO constraint only under pedantic. This is not an incompatible
        // function-pointer signature, which remains a default error.
        if let (Some(to), Some(from)) = (self.pointer_target(target), self.pointer_target(from))
            && ((self.void_target(to) && self.function_target(from))
                || (self.function_target(to) && self.void_target(from)))
        {
            // GCC's words name the context (`convert_for_assignment` in
            // GCC's `c-typeck.cc`), at the initializer rather than the
            // declarator.
            let context = match site {
                | ConversionSite::Initialization => Some("initialization"),
                | ConversionSite::Assignment => Some("assignment"),
                | ConversionSite::Return => Some("return"),
                | ConversionSite::Argument {
                    position,
                    callee: Some(callee),
                } => Some(crate::diagnostics::format_in!(
                    self.scratch,
                    "passing argument {position} of '{}'",
                    self.context.string_cache.at(callee)
                )),
                | ConversionSite::Argument { .. } | ConversionSite::Other => None,
            };
            match context {
                | Some(context) if self.gcc_pointer_rules() => {
                    let text = crate::diagnostics::format_in!(
                        self.scratch,
                        "ISO C forbids {context} between function pointer and 'void *'"
                    );
                    let at = if site == ConversionSite::Initialization {
                        source.expression.source_vectors
                    } else {
                        diagnostic_site
                    };
                    self.gcc_pointer_pedwarn(text, at);
                },
                | _ => self.context.report_extension_since(
                    "conversion between function and void pointers",
                    FeatureOrigin::Gnu,
                    diagnostic_site,
                ),
            }
            return true;
        }
        false
    }

    /// Whether GCC's words and rules decide the constraints between function
    /// pointers and object pointers: the GCC identity's.
    fn gcc_pointer_rules(&self) -> bool {
        self.context.configuration.compiler_identity()
            == crate::configuration::CompilerIdentity::Gcc
    }

    /// Reports `text` at `at` as GCC's `-pedantic` reports a conversion
    /// between function and object pointers, unless a system header spells
    /// `at`, as a system macro's body does, where GCC withholds it even
    /// under `-pedantic-errors`.
    fn gcc_pointer_pedwarn(&mut self, text: &str, at: SourceVectors) {
        if !self.context.spelled_in_system_header(at) {
            self.report_pedantic_extension(text, at);
        }
    }

    /// GCC's `-pedantic` report of the cast `e` of `operand`, of type
    /// `from`, to `target`, when one is a function pointer and the other an
    /// object pointer, at the cast's `(`; a null pointer constant may become
    /// a function pointer silently (`build_c_cast` in GCC's `c-typeck.cc`).
    /// C99 converts function pointers only to function pointers, and lists
    /// the casts as a common extension. Clang reports neither.
    /// C99: §6.3.2.3 paragraphs 1 and 8, pp. 47-48; PDF pp. 59-60; §J.5.7
    /// paragraphs 1-2, p. 514; PDF p. 526.
    pub(in crate::translation_phases::semantic_analysis) fn function_object_cast(
        &mut self,
        e: &'tu Expression<'tu>,
        target: TypeId,
        from: TypeId,
        operand: ExpressionInfo<'tu>,
    ) {
        if !self.gcc_pointer_rules() {
            return;
        }
        let (Some(to), Some(from)) = (self.pointer_target(target), self.pointer_target(from))
        else {
            return;
        };
        let text = match (self.function_target(to), self.function_target(from)) {
            | (false, true) =>
                "ISO C forbids conversion of function pointer to object pointer type",
            | (true, false) if !self.null_pointer_constant(operand) =>
                "ISO C forbids conversion of object pointer to function pointer type",
            | _ => return,
        };
        let at = crate::translation_phases::Context::first_token_vector(e.source_vectors);
        self.gcc_pointer_pedwarn(text, at);
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
    /// a system header's macro. MSVC (whose C4311 and C4312 warn for
    /// narrower integers) and clang-cl follow Clang
    /// ([`crate::configuration::CompilerConfiguration::pointer_cast_rules`]).
    ///
    /// TCC (`gen_cast` in `tccgen.c`, 0.9.27 and mob alike) warns for any
    /// other width either way, an enumeration counting as its integer type
    /// and a `_Bool` source as a byte (a `_Bool` destination tests the
    /// pointer instead), unless the operand folds to a constant that is no
    /// address (`(void *)(1 ? 2 : i)`, `(int)(char *)5`), and never in a
    /// static object's initializer, which it only folds. It converts an
    /// assignment's, initializer's, argument's or `return`'s value the same
    /// way, so [`Self::assignment_conversion_at`] calls this for those too.
    /// C99: §6.3.2.3 paragraphs 5-6, p. 47; PDF p. 59.
    pub(in crate::translation_phases::semantic_analysis) fn pointer_integer_cast_warning(
        &mut self,
        target: TypeId,
        from: TypeId,
        operand: ExpressionInfo<'tu>,
        site: SourceVectors,
    ) {
        let pointer_bits = self.types.target.pointer.size * 8;
        let tcc = self.context.configuration.compiler_identity()
            == crate::configuration::CompilerIdentity::Tcc;
        if tcc && self.tcc_static_data() {
            return;
        }
        let gcc =
            self.context.configuration.pointer_cast_rules() == crate::configuration::RuleSet::Gcc;
        // The width of an integer type, and whether it is an enumeration. A
        // `_Bool` counts only as TCC's source.
        let integer = |ty: TypeId, source: bool| -> Option<(u64, bool)> {
            // Clang's integral types exclude an atomic one, which a cast's
            // value keeps (`(char *)(_Atomic int)i`); GCC drops it.
            if !gcc && matches!(self.types.nodes[ty.index], TypeKind::Atomic(_)) {
                return None;
            }
            let ty = self.types.non_atomic(ty);
            let enumeration = match self.types.nodes[ty.index] {
                | TypeKind::Scalar(Scalar::Bool) if !(tcc && source) => return None,
                | TypeKind::Tag(id) => self.types.tags[id].kind == TagKind::Enum,
                | _ => false,
            };
            self.integer_type(ty)
                .map(|(bits, _)| (u64::from(bits), enumeration))
        };
        let group = if let Some(to) = self.pointer_target(target)
            && let Some((bits, enumeration)) = integer(from, true)
        {
            // Even an unevaluated operand that is no constant (`1 ? 2 : i`)
            // keeps the cast from being an integer constant expression.
            let ice = operand.ice && self.valid_ice_operands(operand.expression);
            if tcc {
                (bits != pointer_bits && operand.integer.is_none())
                    .then_some(DiagnosticGroup::IntToPointerCast)
            } else if gcc {
                // GCC folds `__builtin_assoc_barrier` of a constant before it
                // judges the cast.
                let constant = ice
                    || (operand.assoc_barrier && operand.integer.is_some())
                    || self.casts_address_constant(operand.expression);
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
            && let Some((bits, enumeration)) = integer(target, false)
        {
            if tcc {
                (bits != pointer_bits && operand.integer.is_none())
                    .then_some(DiagnosticGroup::PointerToIntCast)
            } else if gcc {
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
            let wording = self.cast_wording(target, from);
            self.group_diagnostic(
                group,
                crate::translation_phases::semantic_analysis::SemanticErrorKind::PointerConstraint(
                    group,
                ),
                site,
                wording,
                None,
            );
        }
    }

    /// Whether TCC is reading a static object's initializer (or anything
    /// else outside a function), whose values it only folds, generating no
    /// code and so no cast warnings (`STATIC_DATA_WANTED` in `tccgen.c`).
    /// C99: §6.7.8 paragraph 4, p. 125; PDF p. 137.
    fn tcc_static_data(&self) -> bool {
        self.functions.current.is_none()
            || self.functions.initializer.is_some_and(|binding| {
                !matches!(
                    self.bindings[binding].duration,
                    crate::translation_phases::semantic_analysis::Duration::Automatic
                )
            })
    }

    /// Whether `expression` is an address constant, possibly inside casts,
    /// as GCC's `TREE_CONSTANT` finds `(int)(long)&x`.
    /// C99: §6.6 paragraph 9, p. 96; PDF p. 108.
    fn casts_address_constant(&self, mut expression: &'tu Expression<'tu>) -> bool {
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
        // an atomic type, and not where a system macro wrote the context.
        if self.context.configuration.float_conversion_rules() == crate::configuration::RuleSet::Gcc
            || !matches!(self.types.nodes[target.index], TypeKind::Scalar(_))
            || self.integer_type(target).is_none()
            || self.conversion_in_system_macro()
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
    /// type; truth contexts need no synthetic cast. `site` is the token each
    /// compiler takes the conversion's location from: `!`, `&&`, `?` or the
    /// condition's first token.
    /// C99: §6.3.1.2p1, p. 43; PDF p. 55.
    pub(super) fn boolean_conversion_warning(
        &mut self,
        info: ExpressionInfo<'tu>,
        site: SourceVectors,
    ) {
        if self.nullptr_type(info.ty) {
            self.pointer_diagnostic(
                DiagnosticGroup::NullConversion,
                info.expression.source_vectors,
            );
        } else if self.context.configuration.compiler_identity().rules()
            == crate::configuration::RuleSet::Clang
        {
            self.bool_conversion_nonnull(info.expression, site);
        } else {
            self.gcc_address_warning(info.expression, Some(site));
        }
    }

    /// [`Self::boolean_conversion_warning`] for a conversion to `_Bool`, whose
    /// location GCC does not take from the expression, so a macro around it
    /// changes nothing.
    /// C99: §6.3.1.2p1, p. 43; PDF p. 55.
    fn conversion_to_bool_warning(&mut self, info: ExpressionInfo<'tu>) {
        if self.nullptr_type(info.ty) {
            self.boolean_conversion_warning(info, info.expression.source_vectors);
        } else if self.context.configuration.compiler_identity().rules()
            == crate::configuration::RuleSet::Clang
        {
            // Clang's context location: the declarator, the assignment's
            // operator, the call or the `return`.
            let context = if self.conversion_context.length() == 0 {
                info.expression.source_vectors
            } else {
                self.conversion_context
            };
            self.boolean_conversion_warning(
                info,
                crate::translation_phases::Context::first_token_vector(context),
            );
        } else {
            self.gcc_address_warning(info.expression, None);
        }
    }

    /// GCC's `-Waddress` on a truth value, which walks a conditional's arms:
    /// `c_common_truthvalue_conversion` reports the address of a declaration
    /// wherever it is written, and `maybe_warn_for_null_address` the rest of
    /// what [`Self::gcc_known_address`] finds, unless `site`, the conversion's
    /// location where it has one of its own, comes from a macro.
    /// C99: §6.3.1.2p1, p. 43; PDF p. 55; §6.5.3.2p3, pp. 78-79; PDF pp. 90-91.
    pub(super) fn gcc_address_warning(
        &mut self,
        e: &'tu Expression<'tu>,
        site: Option<SourceVectors>,
    ) {
        use crate::translation_phases::parsing::syntax::ExpressionType as E;
        let e = Self::without_parentheses(e);
        if let E::Conditional(c) = e.kind {
            self.gcc_address_warning(c.then_expression, site);
            self.gcc_address_warning(c.else_expression, site);
            return;
        }
        let known = match self.gcc_known_address(e) {
            | GccAddress::Declaration => true,
            | GccAddress::Folded =>
                !site.is_some_and(|site| self.context.produced_by_macro_expansion(site)),
            | GccAddress::Unknown => false,
        };
        if known {
            self.pointer_diagnostic(DiagnosticGroup::PointerBoolConversion, e.source_vectors);
        }
    }

    /// What GCC's tree for the pointer value `e` shows about its address. A
    /// pointer's own value says nothing, nor does a string or a compound
    /// literal, which are no declarations; a weak entity's address can be null.
    /// C99: §6.3.2.1p3-p4, p. 46; PDF p. 58.
    fn gcc_known_address(&mut self, e: &'tu Expression<'tu>) -> GccAddress {
        use crate::translation_phases::parsing::syntax::{
            BinaryOperator as B,
            ExpressionType as E,
            UnaryOperator as U,
        };
        let e = Self::without_parentheses(e);
        let decays = {
            let info = self.expression_info(e);
            info.category == super::ValueCategory::FunctionDesignator
                || matches!(self.types.nodes[info.ty.index], TypeKind::Array(..))
        };
        match e.kind {
            | E::Identifier(_) if decays => self.gcc_declaration_address(e),
            | E::LabelAddress(_) => GccAddress::Declaration,
            | E::Unary {
                operator: U::AddressOf,
                operand_expression,
            } => self.gcc_address_of(operand_expression),
            // `*x` as a pointer is `x`, as `&*x` is.
            | E::Unary {
                operator: U::Indirection,
                operand_expression,
            } if decays => self.gcc_known_address(operand_expression),
            | E::DirectMember { .. }
            | E::IndirectMember { .. }
            | E::Binary {
                operator: B::Subscript,
                ..
            } if decays => self.gcc_address_of(e),
            | E::Binary {
                operator: B::Addition | B::Subtraction,
                left_expression,
                right_expression,
            } => self.gcc_pointer_sum(e, left_expression, right_expression),
            | E::Cast {
                operand_expression, ..
            } => self.gcc_cast_address(e, operand_expression),
            | _ => GccAddress::Unknown,
        }
    }

    /// GCC's `&x`.
    fn gcc_address_of(&mut self, x: &'tu Expression<'tu>) -> GccAddress {
        use crate::translation_phases::parsing::syntax::{
            BinaryOperator as B,
            ExpressionType as E,
            UnaryOperator as U,
        };
        let x = Self::without_parentheses(x);
        match x.kind {
            | E::Identifier(_) => self.gcc_declaration_address(x),
            | E::Unary {
                operator: U::Indirection,
                operand_expression,
            } => self.gcc_known_address(operand_expression),
            | E::Unary {
                operator: U::Real | U::Imag,
                ..
            } => GccAddress::Folded,
            | E::DirectMember { .. } | E::IndirectMember { .. } =>
                if self.gcc_constant_base(x) {
                    GccAddress::Unknown
                } else {
                    GccAddress::Folded
                },
            | E::Binary {
                operator: B::Subscript,
                left_expression,
                right_expression,
            } => {
                let (base, index) = if self.pointer_or_array(left_expression) {
                    (left_expression, right_expression)
                } else {
                    (right_expression, left_expression)
                };
                let array = matches!(
                    self.types.nodes[self.expression_info(base).ty.index],
                    TypeKind::Array(..)
                );
                if self.gcc_constant_base(x) {
                    GccAddress::Unknown
                } else if !array && self.constant_zero(index) {
                    // GCC folds `&p[0]` to `p`.
                    self.gcc_known_address(base)
                } else {
                    GccAddress::Folded
                }
            },
            | _ => GccAddress::Unknown,
        }
    }

    /// `p + i`, `i + p` or `p - i`, which GCC keeps as a `POINTER_PLUS_EXPR`
    /// unless `i` is zero or `p` a constant, and which it lets a `void *` take
    /// out of the warning.
    fn gcc_pointer_sum(
        &mut self,
        sum: &'tu Expression<'tu>,
        left: &'tu Expression<'tu>,
        right: &'tu Expression<'tu>,
    ) -> GccAddress {
        let ty = self.expression_info(sum).ty;
        let Some(target) = self.pointer_target(ty) else {
            return GccAddress::Unknown;
        };
        if self.void_target(target) {
            return GccAddress::Unknown;
        }
        let (base, offset) = if self.pointer_or_array(left) {
            (left, right)
        } else {
            (right, left)
        };
        if self.gcc_constant_pointer(base) {
            GccAddress::Unknown
        } else if self.constant_zero(offset) {
            self.gcc_known_address(base)
        } else {
            GccAddress::Folded
        }
    }

    /// A cast, which GCC strips unless it is to an integer. A sum with a known
    /// address folds into the cast's type, so `(void *)(a + 1)` is a `void *`
    /// sum and `(void *)(p + 1)` is not.
    fn gcc_cast_address(
        &mut self,
        cast: &'tu Expression<'tu>,
        operand: &'tu Expression<'tu>,
    ) -> GccAddress {
        use crate::translation_phases::parsing::syntax::{
            BinaryOperator as B,
            ExpressionType as E,
        };
        let ty = self.expression_info(cast).ty;
        let Some(target) = self.pointer_target(ty) else {
            return GccAddress::Unknown;
        };
        let inner = Self::without_parentheses(operand);
        if self.void_target(target)
            && let E::Binary {
                operator: B::Addition | B::Subtraction,
                left_expression,
                right_expression,
            } = inner.kind
        {
            let base = if self.pointer_or_array(left_expression) {
                left_expression
            } else {
                right_expression
            };
            if matches!(
                self.gcc_known_address(base),
                GccAddress::Declaration | GccAddress::Folded
            ) {
                return GccAddress::Unknown;
            }
        }
        match self.gcc_known_address(operand) {
            | GccAddress::Declaration | GccAddress::Folded => GccAddress::Folded,
            | GccAddress::Unknown => GccAddress::Unknown,
        }
    }

    fn gcc_declaration_address(&mut self, name: &'tu Expression<'tu>) -> GccAddress {
        match self.expression_info(name).binding {
            | Some(binding) if self.declared_weak(binding) => GccAddress::Unknown,
            | Some(_) => GccAddress::Declaration,
            | None => GccAddress::Unknown,
        }
    }

    /// Whether the object a member or element chain ends in is `*p` with `p`
    /// constant, as in `offsetof`'s `&((struct S *)0)->x`: GCC's
    /// `build_unary_op` folds that address to a constant (`fold_offsetof`), so
    /// nothing is left to warn about.
    /// C99: §7.17p3, p. 254; PDF p. 266.
    fn gcc_constant_base(&self, mut x: &'tu Expression<'tu>) -> bool {
        use crate::translation_phases::parsing::syntax::{
            BinaryOperator as B,
            ExpressionType as E,
            UnaryOperator as U,
        };
        loop {
            x = Self::without_parentheses(x);
            match x.kind {
                | E::DirectMember {
                    base_expression, ..
                } => x = base_expression,
                | E::IndirectMember {
                    base_expression, ..
                } => return self.gcc_constant_pointer(base_expression),
                | E::Unary {
                    operator: U::Indirection,
                    operand_expression,
                } => return self.gcc_constant_pointer(operand_expression),
                | E::Binary {
                    operator: B::Subscript,
                    left_expression,
                    right_expression,
                } => {
                    let (base, index) = if self.pointer_or_array(left_expression) {
                        (left_expression, right_expression)
                    } else {
                        (right_expression, left_expression)
                    };
                    if matches!(
                        self.types.nodes[self.expression_info(base).ty.index],
                        TypeKind::Array(..)
                    ) {
                        x = base;
                    } else {
                        return self.gcc_constant_pointer(base)
                            && self.expression_info(index).integer.is_some();
                    }
                },
                | _ => return false,
            }
        }
    }

    /// A pointer whose value is an integer constant, through casts and sums.
    fn gcc_constant_pointer(&self, e: &'tu Expression<'tu>) -> bool {
        use crate::translation_phases::parsing::syntax::{
            BinaryOperator as B,
            ExpressionType as E,
        };
        let e = Self::without_parentheses(e);
        match e.kind {
            | E::Cast {
                operand_expression, ..
            } => self.gcc_constant_pointer(operand_expression),
            | E::Binary {
                operator: B::Addition | B::Subtraction,
                left_expression,
                right_expression,
            } if self.pointer_or_array(left_expression)
                || self.pointer_or_array(right_expression) =>
                self.gcc_constant_pointer(left_expression)
                    && self.gcc_constant_pointer(right_expression),
            | _ => self.expression_info(e).integer.is_some(),
        }
    }

    fn pointer_or_array(&self, e: &'tu Expression<'tu>) -> bool {
        matches!(
            self.types.nodes[self.expression_info(e).ty.index],
            TypeKind::Pointer(_) | TypeKind::Array(..)
        )
    }

    fn constant_zero(&self, e: &'tu Expression<'tu>) -> bool {
        self.expression_info(e)
            .integer
            .is_some_and(super::Integer::is_zero)
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
                // errors. A difference in a deeper level's qualifiers is
                // Clang's nested qualifier diagnostic, and GCC's
                // incompatible pointer types (`char **` from `const char
                // **`). C99: §6.7.5.1 paragraph 2, p. 115; PDF p. 127.
                let gcc = self.context.configuration.compiler_identity()
                    == crate::configuration::CompilerIdentity::Gcc;
                let nested = !gcc && self.targets_without_qualifiers(to, from);
                if self.pointer_compatible(to, from, true) || nested {
                    // GCC files a qualifier lost from an array's elements
                    // under an option of its own. C99: §6.7.3 paragraph 8,
                    // p. 109; PDF p. 121.
                    if gcc && matches!(self.types.nodes[from.index], TypeKind::Array(..)) {
                        return Some(DiagnosticGroup::DiscardedArrayQualifiers);
                    }
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
    /// the existing error and recovered unknown result. Clang underlines
    /// both operands of what it reports.
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
        let operands = [
            left.expression.source_vectors,
            right.expression.source_vectors,
        ];
        self.with_highlights(operands, |analyzer| {
            analyzer.compare_pointers(left, right, (lt, rt), equality, source)
        })
    }

    /// [`Self::pointer_comparison`] of operands of types `lt` and `rt`.
    fn compare_pointers(
        &mut self,
        left: ExpressionInfo<'tu>,
        right: ExpressionInfo<'tu>,
        (lt, rt): (TypeId, TypeId),
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
                // GCC's words (`build_binary_op` in GCC's `c-typeck.cc`).
                if self.gcc_pointer_rules() {
                    self.gcc_pointer_pedwarn(
                        "ISO C forbids comparison of 'void *' with function pointer",
                        source,
                    );
                } else {
                    self.context.report_extension_since(
                        "equality comparison between function pointer and void pointer",
                        FeatureOrigin::Gnu,
                        source,
                    );
                }
            } else if !compatible {
                let group = DiagnosticGroup::CompareDistinctPointerTypes;
                let wording = self.pointer_mismatch_wording(false, lt, rt);
                self.group_diagnostic(
                    group,
                    crate::translation_phases::semantic_analysis::SemanticErrorKind::PointerConstraint(
                        group,
                    ),
                    source,
                    wording,
                    None,
                );
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
    /// converts to the pointer arm's type. GCC pairs `void *` with a
    /// function pointer as with an object pointer, reporting it under
    /// `-pedantic` at `colon`, the `:` its reports point at.
    /// C99: §6.5.15p3,p6, pp. 90-91; PDF pp. 102-103.
    pub(super) fn conditional_pointers(
        &mut self,
        lt: TypeId,
        rt: TypeId,
        source: SourceVectors,
        colon: SourceVectors,
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
        } else if self.gcc_pointer_rules() && (self.void_target(lp) || self.void_target(rp)) {
            // `build_conditional_expr` in GCC's `c-typeck.cc`.
            self.gcc_pointer_pedwarn(
                "ISO C forbids conditional expr between 'void *' and function pointer",
                colon,
            );
            if self.void_target(lp) { lp } else { rp }
        } else if self.types.unanalyzed(lp) || self.types.unanalyzed(rp) {
            lp
        } else if let Some(composite) = self.types.composite(lp.unqualified(), rp.unqualified()) {
            composite
        } else if let Some(merged) = self.gnu_qualified_arrays(lp, rp, colon) {
            merged
        } else {
            // GCC 14 reports it under `-Wincompatible-pointer-types`, an
            // error from C99.
            let group = if self.context.configuration.compiler_identity()
                == crate::configuration::CompilerIdentity::Gcc
            {
                DiagnosticGroup::GccConditionalPointerMismatch
            } else {
                DiagnosticGroup::PointerTypeMismatch
            };
            let wording = self.pointer_mismatch_wording(true, lt, rt);
            self.group_diagnostic(
                group,
                crate::translation_phases::semantic_analysis::SemanticErrorKind::PointerConstraint(
                    group,
                ),
                source,
                wording,
                None,
            );
            let void = self.types.scalar(Scalar::Void);
            return Some(self.types.intern(TypeKind::Pointer(void)));
        };
        Some(self.types.intern(TypeKind::Pointer(
            target.qualified(lp.qualifiers | rp.qualifiers),
        )))
    }

    /// GCC's `-Wpedantic` report before C23 of the pointers `left` and
    /// `right` to arrays whose elements differ only in qualifiers, which it
    /// converts, compares and subtracts as C23 does (`comp_target_types` in
    /// GCC's `c-typeck.cc`). Before C23 such arrays are not compatible,
    /// since their element types are not; Clang reports a mismatch instead.
    /// C99: §6.7.3 paragraph 8, p. 109; PDF p. 121; §6.7.5.2 paragraph 6,
    /// p. 117; PDF p. 129.
    pub(in crate::translation_phases::semantic_analysis) fn gcc_array_qualifier_pedwarn(
        &mut self,
        left: TypeId,
        right: TypeId,
        at: SourceVectors,
    ) {
        if self.gcc_pointer_rules()
            && self.context.configuration.standard() < crate::configuration::CStandard::C23
            && let (Some(lp), Some(rp)) = (self.pointer_target(left), self.pointer_target(right))
            && self.differently_qualified_arrays(lp, rp)
        {
            self.gcc_pointer_pedwarn(ARRAY_QUALIFIERS_BEFORE_C23, at);
        }
    }

    /// [`Self::pointer_compatible`] for the targets `lp` and `rp` of the
    /// operands, of types `lt` and `rt`, of the comparison or subtraction
    /// `e`, with GCC's report of arrays differing in element qualifiers at
    /// its operator ([`Self::gcc_array_qualifier_pedwarn`]).
    /// C99: §6.5.6 paragraph 3, pp. 82-83; PDF pp. 94-95; §6.5.8 paragraph 2,
    /// p. 85; PDF p. 97; §6.5.9 paragraph 2, p. 86; PDF p. 98.
    pub(in crate::translation_phases::semantic_analysis) fn operand_targets_compatible(
        &mut self,
        e: &'tu Expression<'tu>,
        (lt, rt): (TypeId, TypeId),
        (lp, rp): (TypeId, TypeId),
        void: bool,
    ) -> bool {
        let compatible = self.pointer_compatible(lp, rp, void);
        if compatible {
            let operator = e.operator_source_vectors.unwrap_or(e.source_vectors);
            self.gcc_array_qualifier_pedwarn(lt, rt, operator);
        }
        compatible
    }

    /// Whether `lp` and `rp` are array types whose elements differ in
    /// qualifiers and that are compatible without them, `_Atomic` aside.
    fn differently_qualified_arrays(&mut self, lp: TypeId, rp: TypeId) -> bool {
        if !matches!(self.types.nodes[lp.index], TypeKind::Array(..))
            || !matches!(self.types.nodes[rp.index], TypeKind::Array(..))
            || self
                .types
                .composite(lp.unqualified(), rp.unqualified())
                .is_some()
        {
            return false;
        }
        let (left, right) = (
            self.types.unqualified_array(lp),
            self.types.unqualified_array(rp),
        );
        self.types.composite(left, right).is_some()
    }

    /// GCC's `-pedantic` report before C23 of a qualified `void *` `source`
    /// converted to the pointer to an array `target` whose elements have
    /// those qualifiers: it compares the `void`'s qualifiers with the
    /// array's own, which are none, and so reports them all as discarded,
    /// as a pedantic warning (`convert_for_assignment` in GCC's
    /// `c-typeck.cc`). Before C23 the array type itself is not qualified.
    /// C99: §6.7.3 paragraph 8, p. 109; PDF p. 121; §6.5.16.1 paragraph 1,
    /// p. 92; PDF p. 104.
    fn gcc_void_array_qualifiers(
        &mut self,
        target: TypeId,
        source: ExpressionInfo<'tu>,
        site: ConversionSite,
        at: SourceVectors,
    ) {
        if !self.gcc_pointer_rules()
            || self.context.configuration.standard() >= crate::configuration::CStandard::C23
            || self.under_extension()
            || self.context.spelled_in_system_header(at)
            || !self.gcc_pedantic_flag()
        {
            return;
        }
        let target = self.types.non_atomic(target);
        let (from, _) = self.value_conversion(source);
        let (Some(to), Some(from_target)) =
            (self.pointer_target(target), self.pointer_target(from))
        else {
            return;
        };
        if !matches!(self.types.nodes[to.index], TypeKind::Array(..))
            || !self.void_target(from_target)
            || !self.types.object_qualifiers(from_target).intersects(
                TypeQualifiers::CONST | TypeQualifiers::VOLATILE | TypeQualifiers::RESTRICT,
            )
        {
            return;
        }
        let group = DiagnosticGroup::PointerQualifiers;
        let wording = self.conversion_wording(group, site, target, from, source);
        self.group_diagnostic(
            group,
            crate::translation_phases::semantic_analysis::SemanticErrorKind::PointerConstraint(
                group,
            ),
            at,
            wording,
            None,
        );
    }

    /// GCC's `pedantic` flag, which `-pedantic`, `-Wpedantic`,
    /// `-pedantic-errors` and `-Werror=pedantic` set and `-Wno-pedantic`
    /// clears; a diagnostic pragma naming `-Wpedantic` leaves it alone.
    fn gcc_pedantic_flag(&self) -> bool {
        let configuration = self.context.configuration;
        configuration.extension_policy() != ExtensionPolicy::Allow
            || configuration.werror_pedantic().is_some()
    }

    /// GCC's and TCC's target for arms that point to arrays whose elements
    /// differ only in qualifiers: the composite array with the union of
    /// both element qualifiers, as C23 has it, which GCC applies in every
    /// mode and reports under `-pedantic` before C23, but not in a
    /// declaration or function definition marked `__extension__` (GCC 14).
    /// Clang reports a mismatch instead.
    /// C23 (N3220): §6.5.16 paragraph 7, p. 92; PDF p. 105; §6.7.4.1
    /// paragraph 10, p. 121; PDF p. 134. GNU extension: GCC manual,
    /// "Alternate Keywords".
    fn gnu_qualified_arrays(
        &mut self,
        lp: TypeId,
        rp: TypeId,
        colon: SourceVectors,
    ) -> Option<TypeId> {
        if !matches!(
            self.context.configuration.compiler_identity(),
            crate::configuration::CompilerIdentity::Gcc
                | crate::configuration::CompilerIdentity::Tcc
        ) || !matches!(self.types.nodes[lp.index], TypeKind::Array(..))
            || !matches!(self.types.nodes[rp.index], TypeKind::Array(..))
        {
            return None;
        }
        let (left, right) = (
            self.types.unqualified_array(lp),
            self.types.unqualified_array(rp),
        );
        let composite = self.types.composite(left, right)?;
        if self.context.configuration.standard() < crate::configuration::CStandard::C23 {
            self.report_pedantic_extension(ARRAY_QUALIFIERS_BEFORE_C23, colon);
        }
        let qualifiers = self.types.object_qualifiers(lp) | self.types.object_qualifiers(rp);
        Some(self.types.qualified_object(composite, qualifiers))
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

/// GCC's words for pointers to arrays whose elements differ only in
/// qualifiers, before C23.
const ARRAY_QUALIFIERS_BEFORE_C23: &str =
    "invalid use of pointers to arrays with different qualifiers in ISO C before C23";

/// What GCC can tell about the address a pointer value holds.
#[derive(Clone, Copy)]
enum GccAddress {
    /// A variable, function or label's own address, without a cast.
    Declaration,
    /// Anything else known not to be null: a member, an element, a sum or a
    /// cast of an address.
    Folded,
    /// Nothing, as for a pointer's value.
    Unknown,
}
