//! Two warnings GCC and Clang give by default about values a program
//! computes: a function returning the address of its own automatic object
//! (GCC's `-Wreturn-local-addr`, Clang's `-Wreturn-stack-address`), whose
//! lifetime ends as it returns, and an integer constant an implicit
//! conversion changes (GCC's `-Woverflow`, Clang's `-Wconstant-conversion`).
//! C99 makes the first pointer indeterminate (§6.2.4p2) and gives the second
//! an implementation-defined result (§6.3.1.3p3), so each is a warning
//! §5.1.1.3p1 permits.
//! GCC: `c_finish_return` in `c-typeck.cc`, `unsafe_conversion_p` in
//! `c-common.cc`. Clang: `checkExprLifetime`, `CheckImplicitConversion`.
//! C99: §6.2.4p2, p. 32; PDF p. 44; §6.3.1.3p3, p. 43; PDF p. 55.

use super::{
    Analyzer,
    BindingKind,
    Duration,
    ExpressionInfo,
    SemanticErrorKind,
    types::{
        Scalar,
        TypeId,
        TypeKind,
    },
};
use crate::{
    configuration::{
        CompilerIdentity,
        DiagnosticGroup as G,
    },
    diagnostics::format_in,
    translation_phases::{
        Context,
        SourceVectors,
        parsing::syntax::{
            BinaryOperator as B,
            Expression,
            ExpressionType as E,
            UnaryOperator as U,
        },
        preprocessing::KeywordTokenType,
    },
    util::bump::ArenaVec,
};

pub(super) mod int_float;

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Checks the value `e` a function returns as a pointer.
    /// C99: §6.2.4p2, p. 32; PDF p. 44; §6.8.6.4p3, p. 139; PDF p. 151.
    pub(super) fn check_returned_address(&mut self, e: &'tu Expression<'tu>, result: TypeId) {
        if self.tainted {
            return;
        }
        if self.context.configuration.compiler_identity() == CompilerIdentity::Gcc {
            if self.pointer_target(result).is_none() {
                return;
            }
            // GCC points at the outermost node its folding keeps, which a
            // parenthesis or a selection does not make.
            let e = self.selected_operand(e);
            if self.gcc_local_address(e) {
                let site = match e.kind {
                    | E::Binary { .. } => e.operator_source_vectors.unwrap_or(e.source_vectors),
                    // GCC places a conditional at its `:`.
                    | E::Conditional(_) | E::OmittedConditional(_) => e
                        .operator_source_vectors
                        .map_or(e.source_vectors, Context::last_source_vector),
                    | _ => e.source_vectors,
                };
                self.value_warning(
                    G::ReturnLocalAddr,
                    site,
                    "function returns address of local variable",
                );
            }
        } else if self.holds_address(result) {
            // The conversion to a `_Bool` result keeps only the truth.
            self.clang_local_address(e);
        }
    }

    /// `e` through the forms that only pass an operand's value on:
    /// parentheses, `__extension__` and the operand `_Generic` or
    /// `__builtin_choose_expr` selects.
    /// C11: §6.5.1.1p3, p. 79; PDF p. 97.
    fn selected_operand(&self, mut e: &'tu Expression<'tu>) -> &'tu Expression<'tu> {
        loop {
            e = match e.kind {
                | E::Parenthesized { expression }
                | E::Unary {
                    operator: U::Extension,
                    operand_expression: expression,
                } => expression,
                | E::Generic(_) => match self.expression_info(e).selected_expression {
                    | Some(selected) => selected,
                    | None => return e,
                },
                | E::Builtin(builtin) if builtin.keyword == KeywordTokenType::BuiltinChooseExpr =>
                    match self.expression_info(e).selected_expression {
                        | Some(selected) => selected,
                        | None => return e,
                    },
                | _ => return e,
            };
        }
    }

    /// Whether `e` is an automatic object of this function.
    fn local_object(&self, e: &'tu Expression<'tu>) -> Option<(usize, bool)> {
        let binding = self.expression_info(e).binding?;
        let data = self.bindings[binding];
        (data.duration == Duration::Automatic
            && matches!(data.kind, BindingKind::Object | BindingKind::Parameter))
        .then_some((binding, data.kind == BindingKind::Parameter))
    }

    fn array_valued(&self, e: &'tu Expression<'tu>) -> bool {
        let ty = self.types.non_atomic(self.expression_info(e).ty);
        matches!(self.types.nodes[ty.index], TypeKind::Array(..))
    }

    /// GCC's walk: through conversions and pointer arithmetic to an address
    /// whose object is one of this function's.
    fn gcc_local_address(&mut self, mut e: &'tu Expression<'tu>) -> bool {
        loop {
            e = self.selected_operand(e);
            e = match e.kind {
                // GCC's folding drops a comma's left operand that has no
                // side effects, and a conditional whose condition is a
                // constant, which an address no link can make null is.
                // C99: §6.5.17p2, p. 94; PDF p. 106; §6.5.15p4, p. 90;
                // PDF p. 102.
                | E::Binary {
                    operator: B::Comma,
                    left_expression,
                    right_expression,
                } if !self.has_side_effects(left_expression) => right_expression,
                | E::Conditional(c) | E::OmittedConditional(c) =>
                    match self.constant_truth_of(c.condition_expression, 0) {
                        | Some(true) => c.then_expression,
                        | Some(false) => c.else_expression,
                        | None => return false,
                    },
                // GCC strips every conversion between pointers and
                // integers, so an address survives a round trip through
                // `long`.
                | E::Cast {
                    operand_expression, ..
                } if self.holds_address(self.expression_info(e).ty) => operand_expression,
                // Its first operand, after folding has moved a constant
                // addend second and pointer arithmetic has put the pointer
                // first.
                | E::Binary {
                    operator: B::Addition,
                    left_expression,
                    right_expression,
                } =>
                    if self.address_valued(right_expression)
                        && !self.address_valued(left_expression)
                        || self.expression_info(left_expression).integer.is_some()
                    {
                        right_expression
                    } else {
                        left_expression
                    },
                // A difference with an address taken away may be valid, so
                // GCC stops there.
                | E::Binary {
                    operator: B::Subtraction,
                    left_expression,
                    right_expression,
                } =>
                    if self.subtracts_address(right_expression) {
                        return false;
                    } else {
                        left_expression
                    },
                | E::Unary {
                    operator: U::AddressOf,
                    operand_expression,
                } => return self.lvalue_is_local(operand_expression),
                | E::StatementExpression(_) => return false,
                | _ if self.array_valued(e) => return self.lvalue_is_local(e),
                | _ => return false,
            };
        }
    }

    fn address_valued(&self, e: &'tu Expression<'tu>) -> bool {
        self.array_valued(e) || self.pointer_target(self.expression_info(e).ty).is_some()
    }

    /// Whether a value of type `ty` can carry an address on: a pointer, or an
    /// integer other than `_Bool`, whose conversion keeps only the truth.
    /// C99: §6.3.1.2p1, p. 43; PDF p. 55; §6.3.2.3p6, p. 47; PDF p. 59.
    fn holds_address(&self, ty: TypeId) -> bool {
        let ty = self.types.non_atomic(ty);
        self.pointer_target(ty).is_some()
            || !matches!(self.types.nodes[ty.index], TypeKind::Scalar(Scalar::Bool))
                && self.integer_type(ty).is_some()
    }

    /// Whether the subtrahend `e` is an address, through the integer
    /// conversions GCC strips from it.
    fn subtracts_address(&self, mut e: &'tu Expression<'tu>) -> bool {
        loop {
            if self.address_valued(e) {
                return true;
            }
            e = match e.kind {
                | E::Parenthesized { expression } => expression,
                | E::Cast {
                    operand_expression, ..
                } if self.holds_address(self.expression_info(e).ty) => operand_expression,
                | _ => return false,
            };
        }
    }

    /// Whether the object the lvalue `e` designates is one of this
    /// function's, through members, subscripts of an array and `*&`.
    fn lvalue_is_local(&self, e: &'tu Expression<'tu>) -> bool {
        self.local_lvalue(e).is_some()
    }

    fn local_lvalue(&self, mut e: &'tu Expression<'tu>) -> Option<&'tu Expression<'tu>> {
        loop {
            e = match e.kind {
                | E::Parenthesized { expression } => expression,
                | E::DirectMember {
                    base_expression, ..
                } => base_expression,
                | E::Binary {
                    operator: B::Subscript,
                    left_expression,
                    right_expression,
                } =>
                    if self.array_valued(left_expression) {
                        left_expression
                    } else if self.array_valued(right_expression) {
                        right_expression
                    } else {
                        return None;
                    },
                | E::Unary {
                    operator: U::Indirection,
                    operand_expression,
                } => return self.pointer_to_local(operand_expression),
                | E::Identifier(_) => return self.local_object(e).map(|_| e),
                | _ => return None,
            };
        }
    }

    /// The reference to a local object whose address `e` holds, through
    /// the same conversions as [`Self::gcc_local_address`].
    fn pointer_to_local(&self, mut e: &'tu Expression<'tu>) -> Option<&'tu Expression<'tu>> {
        loop {
            e = match e.kind {
                | E::Parenthesized { expression } => expression,
                | E::Unary {
                    operator: U::AddressOf,
                    operand_expression,
                } => return self.local_lvalue(operand_expression),
                | _ if self.array_valued(e) => return self.local_lvalue(e),
                | _ => return None,
            };
        }
    }

    /// Clang's walk, which also takes both arms of every conditional and a
    /// comma's right operand, and names the object at its reference.
    fn clang_local_address(&mut self, e: &'tu Expression<'tu>) {
        // The arms still to visit, last first, so the warnings follow the
        // source as Clang's depth-first walk does.
        let mut pending = ArenaVec::new_in(self.scratch);
        pending.push(e);
        while let Some(mut e) = pending.pop() {
            let found = loop {
                e = self.selected_operand(e);
                e = match e.kind {
                    // C99: §6.5.17p2, p. 94; PDF p. 106.
                    | E::Binary {
                        operator: B::Comma,
                        right_expression,
                        ..
                    } => right_expression,
                    // Clang follows an address through a cast to a pointer
                    // or an integer, but not to `_Bool`.
                    | E::Cast {
                        operand_expression, ..
                    } if self.holds_address(self.expression_info(e).ty) => operand_expression,
                    // Only pointer arithmetic, to its pointer operand.
                    | E::Binary {
                        operator: B::Addition | B::Subtraction,
                        left_expression,
                        right_expression,
                    } if self.pointer_target(self.expression_info(e).ty).is_some() =>
                        if self.address_valued(left_expression) {
                            left_expression
                        } else if self.address_valued(right_expression) {
                            right_expression
                        } else {
                            break None;
                        },
                    // C99: §6.5.15p4, p. 90; PDF p. 102.
                    | E::Conditional(c) => {
                        pending.push(c.else_expression);
                        c.then_expression
                    },
                    // Clang's walk does not see through the condition that
                    // stands for the omitted operand.
                    | E::OmittedConditional(c) => c.else_expression,
                    | E::Unary {
                        operator: U::AddressOf,
                        operand_expression,
                    } => break self.local_lvalue(operand_expression),
                    | _ if self.array_valued(e) => break self.local_lvalue(e),
                    | _ => break None,
                };
            };
            if let Some(reference) = found
                && let Some((binding, parameter)) = self.local_object(reference)
            {
                let name = self
                    .context
                    .string_cache
                    .at(self.bindings[binding].name.name);
                let what = if parameter {
                    "parameter"
                } else {
                    "local variable"
                };
                let text = format_in!(
                    self.scratch,
                    "address of stack memory associated with {what} '{name}' returned"
                );
                self.value_warning(G::ReturnStackAddress, reference.source_vectors, text);
            }
        }
    }

    /// Warns about an integer constant whose implicit conversion to `target`
    /// changes its value.
    /// C99: §6.3.1.3p3, p. 43; PDF p. 55.
    pub(super) fn check_constant_conversion(
        &mut self,
        target: TypeId,
        source: ExpressionInfo<'tu>,
    ) {
        if self.tainted {
            return;
        }
        // Clang's `constexpr` refusal stands alone here too.
        if self.underspecified == 0 {
            self.check_int_float_source(target, source);
        }
        let Some(value) = source.integer.and_then(super::Integer::to_i128) else {
            return;
        };
        let target = self.types.non_atomic(target).unqualified();
        if !matches!(self.types.nodes[target.index], TypeKind::Scalar(scalar) if scalar != Scalar::Bool)
        {
            return;
        }
        let Some((bits, signed)) = self.integer_type(target) else {
            return;
        };
        let Some((source_bits, source_signed)) = self.integer_type(source.ty) else {
            return;
        };
        if bits >= 128 || source_bits <= bits && source_signed == signed {
            return;
        }
        let modulus = 1_i128 << bits;
        let kept = value.rem_euclid(modulus);
        let kept = if signed && kept >= modulus / 2 {
            kept - modulus
        } else {
            kept
        };
        let gcc = self.context.configuration.compiler_identity() == CompilerIdentity::Gcc;
        // Clang's `constexpr` refusal stands alone.
        if !gcc && self.underspecified > 0 {
            return;
        }
        let warns = if gcc {
            // GCC only reports a value that neither signedness holds.
            let low = -(modulus / 2);
            let high = modulus - 1;
            !(low..=high).contains(&value)
        } else {
            // Clang compares the bits the value needs with the target's, and
            // a signed source's nonnegative value that fills a signed
            // target, which then turns negative.
            let needed = if value >= 0 {
                128 - value.leading_zeros()
            } else {
                129 - (!value).leading_zeros()
            };
            needed > bits || (needed == bits && signed && value >= 0 && source_signed)
        };
        if !warns {
            return;
        }
        let (Some(from), Some(to)) = (
            self.type_spelling(source.ty, gcc),
            self.type_spelling(target, gcc),
        ) else {
            return;
        };
        let (group, text) = if gcc {
            let kind = if signed {
                "overflow in conversion"
            } else {
                "unsigned conversion"
            };
            (
                G::Overflow,
                format_in!(
                    self.scratch,
                    "{kind} from '{from}' to '{to}' changes value from '{value}' to '{kept}'"
                ),
            )
        } else {
            (
                G::ConstantConversion,
                format_in!(
                    self.scratch,
                    "implicit conversion from '{from}' to '{to}' changes value from {value} to \
                     {kept}"
                ),
            )
        };
        self.value_warning(group, source.expression.source_vectors, text);
    }

    fn value_warning(&mut self, group: G, site: SourceVectors, text: &str) {
        let name = self.context.string_cache.intern(text);
        self.spelled_group_diagnostic(
            group,
            SemanticErrorKind::ValueWarning(group),
            site,
            Some(name),
            site,
        );
    }
}
