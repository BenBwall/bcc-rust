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
        SourceVectors,
        parsing::syntax::{
            BinaryOperator as B,
            Expression,
            ExpressionType as E,
            UnaryOperator as U,
        },
    },
};

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
            if self.gcc_local_address(e) {
                let site = match e.kind {
                    | E::Binary { .. } => e.operator_source_vectors.unwrap_or(e.source_vectors),
                    | _ => e.source_vectors,
                };
                self.value_warning(
                    G::ReturnLocalAddr,
                    site,
                    "function returns address of local variable",
                );
            }
        } else {
            self.clang_local_address(e);
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
    fn gcc_local_address(&self, mut e: &'tu Expression<'tu>) -> bool {
        loop {
            e = match e.kind {
                | E::Parenthesized { expression } => expression,
                | E::Cast {
                    operand_expression, ..
                } if self.pointer_target(self.expression_info(e).ty).is_some() =>
                    operand_expression,
                | E::Binary {
                    operator: B::Addition | B::Subtraction,
                    left_expression,
                    right_expression,
                } =>
                    if self.address_valued(left_expression) {
                        left_expression
                    } else {
                        right_expression
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

    /// Clang's walk, which also takes both arms of a conditional, and names
    /// the object at its reference.
    fn clang_local_address(&mut self, e: &'tu Expression<'tu>) {
        let mut pending = [Some(e), None];
        while let Some(index) = pending.iter().position(Option::is_some) {
            let Some(mut e) = pending[index].take() else {
                break;
            };
            let found = loop {
                e = match e.kind {
                    | E::Parenthesized { expression } => expression,
                    // Clang follows an address through any cast, to an
                    // integer too.
                    | E::Cast {
                        operand_expression, ..
                    } if self.address_valued(operand_expression) => operand_expression,
                    | E::Binary {
                        operator: B::Addition | B::Subtraction,
                        left_expression,
                        right_expression,
                    } =>
                        if self.address_valued(left_expression) {
                            left_expression
                        } else {
                            right_expression
                        },
                    | E::Conditional(c) if pending[1].is_none() => {
                        pending[1] = Some(c.else_expression);
                        c.then_expression
                    },
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
