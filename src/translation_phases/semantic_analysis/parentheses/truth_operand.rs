//! Whether GCC takes the right operand of `!x & e` or `!x | e` for a truth
//! value. `warn_about_parentheses` asks this of the tree GCC's C front end
//! has built for `e`, not of its syntax: a comparison or a logical operation
//! passes, and so does an operand of type `_Bool` or one that is the integer
//! constant 0 or 1 (GCC manual, "Warning Options", `-Wparentheses`). Casts,
//! `!`, conditionals and commas fold while GCC builds them, so
//! [`Analyzer::gcc_tree`] builds that tree again: `build_c_cast`,
//! `build_unary_op`, `build_conditional_expr` and `build_compound_expr`
//! build it, `c_convert`, `convert_to_integer_1`, `convert_to_real_1` and
//! `c_common_truthvalue_conversion` convert it, and `fold_unary_loc`,
//! `fold_ternary_loc`, `fold_truth_not_expr` and the GENERIC conversion rules
//! of `match.pd` fold it, as GCC 14 has them. Anything else is an opaque
//! value. Measured against GCC 14: `!a & (int)(b != 0)`, `!a &
//! (int)(_Bool)(b ? 0 : 1)`, `!a & (unsigned)(b ? !a : 0)` and `!a &
//! (int)(long)(b || a)` pass, while `!a & (int)(b ? 0 : 1)`, `!a &
//! (int)(_Bool)(double)(b || a)`, `!a & (0, b != 0)`, `!a & !(b ? 1 : 0)`,
//! `!a & (int)(b ? (unsigned)(b || a) : 0)` and `!a & (int)(_Bool)(b ? p :
//! q)` are reported. Arithmetic other than negation stays opaque, so the
//! folding of `x + 0` and similar identities under a narrowing conversion
//! is not modeled.
//! C99: §6.5.4 paragraph 4, p. 81; PDF p. 93; §6.3.1.2 paragraph 1, p. 43;
//! PDF p. 55.

use super::super::{
    Analyzer,
    expressions::ValueCategory,
    types::{
        Scalar,
        TypeId,
        TypeKind,
    },
};
use crate::{
    translation_phases::parsing::syntax::{
        BinaryOperator as B,
        Constant,
        Expression,
        ExpressionType as E,
        UnaryOperator as U,
    },
    util::bump::Bump,
};

impl<'tu, 's> Analyzer<'_, 'tu, 's> {
    /// Whether GCC leaves `!x` unquestioned before `& e` or `| e`: its
    /// `APPEARS_TO_BE_BOOLEAN_EXPR_P`.
    pub(in crate::translation_phases::semantic_analysis) fn gcc_truth_operand(
        &self,
        e: &'tu Expression<'tu>,
    ) -> bool {
        let node = self.gcc_tree(e);
        node.code.is_truth()
            || node.ty.class == Class::Boolean
            || matches!(node.code, Code::Integer(0 | 1))
    }

    /// The tree GCC's parser builds for `e`.
    fn gcc_tree(&self, e: &'tu Expression<'tu>) -> Node<'s, 'tu> {
        if let E::Parenthesized { expression }
        | E::Unary {
            operator: U::Extension,
            operand_expression: expression,
        } = e.kind
        {
            return self.gcc_tree(expression);
        }
        let info = self.expression_info(e);
        let ty = self.gcc_type(info.ty);
        let gcc = self.gcc();
        let value = info.integer.filter(|_| ty.integral()).map(|v| v.value);
        let opaque = || Node {
            code: Code::Value {
                lvalue: matches!(
                    info.category,
                    ValueCategory::Lvalue | ValueCategory::ModifiableLvalue
                ),
                source: Some(e),
            },
            ty,
            effects: self.has_side_effects(e),
        };
        // An operator applied to integer constants folds to one, and one
        // whose operands are all integer constants is marked as such.
        let operator = |operands: &[Node<'s, 'tu>], built: Node<'s, 'tu>| match value {
            | Some(v) if operands.iter().all(|n| matches!(n.code, Code::Integer(_))) =>
                integer(ty, v),
            | _ if operands.iter().all(|&n| int_const_operands(n)) =>
                gcc.note_integer_operands(built),
            | _ => built,
        };
        match e.kind {
            | E::Unary {
                operator: U::LogicalNot,
                operand_expression,
            } => gcc.logical_not(self.gcc_tree(operand_expression)),
            | E::Unary {
                operator: U::Plus,
                operand_expression,
            } => gcc.unary_plus(self.gcc_tree(operand_expression)),
            | E::Unary {
                operator: U::Minus,
                operand_expression,
            } => {
                let operand = self.gcc_tree(operand_expression);
                let promoted = gcc.default_conversion(operand);
                let negate = Node {
                    code: Code::Negate(gcc.alloc(promoted)),
                    ty,
                    effects: operand.effects,
                };
                operator(&[operand], negate)
            },
            | E::Unary {
                operator: U::BitwiseNot,
                operand_expression,
            } => operator(&[self.gcc_tree(operand_expression)], opaque()),
            | E::Binary {
                operator: op,
                left_expression,
                right_expression,
            } => {
                let (left, right) = (
                    self.gcc_tree(left_expression),
                    self.gcc_tree(right_expression),
                );
                let code = match op {
                    | B::LogicalAnd | B::LogicalOr =>
                        return gcc.logical(op == B::LogicalOr, left, right),
                    // C99: §6.5.17 paragraph 2, p. 94; PDF p. 106.
                    | B::Comma => return gcc.compound(left, right),
                    | B::LessThan => Comparison::Lt,
                    | B::GreaterThan => Comparison::Gt,
                    | B::LessThanOrEqual => Comparison::Le,
                    | B::GreaterThanOrEqual => Comparison::Ge,
                    | B::Equal => Comparison::Eq,
                    | B::NotEqual => Comparison::Ne,
                    | _ => return operator(&[left, right], opaque()),
                };
                operator(&[left, right], gcc.comparison(code, ty, left, right))
            },
            // C99: §6.5.15 paragraph 4, p. 90; PDF p. 102.
            | E::Conditional(c) => gcc.conditional(
                self.gcc_tree(c.condition_expression),
                self.gcc_tree(c.then_expression),
                self.gcc_tree(c.else_expression),
                ty,
            ),
            | E::Cast {
                operand_expression, ..
            } if ty.class != Class::Other => gcc.cast(ty, self.gcc_tree(operand_expression)),
            | E::Constant(Constant::Float(_))
                if ty.class == Class::Floating
                    && let Some(value) = info.floating() =>
                Node {
                    code: Code::Real {
                        truth:     value.truth(),
                        truncated: crate::float_parsing::bits::to_integer(&value.real, 128, true)
                            .map(u128::cast_signed),
                    },
                    ty,
                    effects: false,
                },
            // A constant, an enumeration constant or `sizeof`.
            | _ if info.ice
                && let Some(v) = value =>
                integer(ty, v),
            | _ => opaque(),
        }
    }

    fn gcc(&self) -> Gcc<'_, '_, 's, 'tu> {
        Gcc {
            analyzer:  self,
            scratch:   self.scratch,
            inhibited: self.evaluation_inhibited(),
            int:       self.gcc_scalar(Scalar::Int),
            long:      self.gcc_scalar(Scalar::Long),
        }
    }

    fn gcc_type(&self, ty: TypeId) -> GccType<'tu> {
        let kind = self.types.nodes[self.types.non_atomic(ty).index];
        let (class, bits, unsigned) = match kind {
            | TypeKind::Scalar(Scalar::Bool) => (Class::Boolean, 1, true),
            | TypeKind::Pointer(_) => (
                Class::Pointer,
                u32::try_from(self.types.target.pointer.size * 8).unwrap_or(u32::MAX),
                true,
            ),
            // Only the order of floating precisions matters.
            | TypeKind::Scalar(s) if !s.complex() && s.floating_rank(3) != 0 =>
                (Class::Floating, u32::from(s.floating_rank(3)), false),
            | _ => match self.integer_type(ty) {
                | Some((bits, signed)) if bits <= 128 => (Class::Integer, bits, !signed),
                | _ => (Class::Other, 0, false),
            },
        };
        GccType {
            kind,
            class,
            bits,
            unsigned,
        }
    }

    fn gcc_scalar(&self, scalar: Scalar) -> GccType<'tu> {
        let (bits, signed) = self.types.target.integer(scalar).unwrap_or((0, true));
        GccType {
            kind: TypeKind::Scalar(scalar),
            class: Class::Integer,
            bits,
            unsigned: !signed,
        }
    }
}

/// A tree of GCC's GENERIC, as far as the truth of its value goes.
#[derive(Clone, Copy)]
struct Node<'s, 'tu> {
    code:    Code<'s, 'tu>,
    ty:      GccType<'tu>,
    /// `TREE_SIDE_EFFECTS`.
    effects: bool,
}

#[derive(Clone, Copy)]
enum Code<'s, 'tu> {
    /// `INTEGER_CST`, with its value in its type.
    Integer(i128),
    /// `REAL_CST`: whether it differs from zero, and what it truncates to.
    Real {
        truth:     bool,
        truncated: Option<i128>,
    },
    /// A conversion: `NOP_EXPR`, `CONVERT_EXPR`, `FLOAT_EXPR`,
    /// `FIX_TRUNC_EXPR` or `NON_LVALUE_EXPR`.
    Convert(Conversion, &'s Node<'s, 'tu>),
    /// `TRUTH_NOT_EXPR`.
    Not(&'s Node<'s, 'tu>),
    /// `NEGATE_EXPR`, which does not change whether a value is zero.
    Negate(&'s Node<'s, 'tu>),
    /// A comparison, and whether its operands are floating. The front end
    /// wraps arithmetic operands in a `C_MAYBE_CONST_EXPR`, which no fold
    /// looks into, and leaves pointer operands as they are.
    Comparison {
        code:     Comparison,
        floating: bool,
        pointers: Option<&'s (Node<'s, 'tu>, Node<'s, 'tu>)>,
    },
    /// `TRUTH_ANDIF_EXPR` or `TRUTH_ORIF_EXPR`.
    Logical {
        or:    bool,
        left:  &'s Node<'s, 'tu>,
        right: &'s Node<'s, 'tu>,
    },
    /// `COND_EXPR`.
    Conditional {
        condition: &'s Node<'s, 'tu>,
        then:      &'s Node<'s, 'tu>,
        otherwise: &'s Node<'s, 'tu>,
    },
    /// `COMPOUND_EXPR`.
    Compound {
        left:  &'s Node<'s, 'tu>,
        right: &'s Node<'s, 'tu>,
    },
    /// `C_MAYBE_CONST_EXPR` marking an expression whose operands are all
    /// integer constants: `C_MAYBE_CONST_EXPR_INT_OPERANDS`.
    IntegerOperands(&'s Node<'s, 'tu>),
    /// Anything else, whether it designates an object, and the expression
    /// it stands for, which `operand_equal_p` compares.
    Value {
        lvalue: bool,
        source: Option<&'tu Expression<'tu>>,
    },
}

impl Code<'_, '_> {
    /// `truth_value_p`.
    const fn is_truth(self) -> bool {
        matches!(
            self,
            Self::Comparison { .. } | Self::Logical { .. } | Self::Not(_)
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Conversion {
    Nop,
    Convert,
    Float,
    FixTrunc,
    NonLvalue,
}

impl Conversion {
    /// `CONVERT_EXPR_CODE_P`.
    const fn is_nop(self) -> bool {
        matches!(self, Self::Nop | Self::Convert)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Comparison {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

impl Comparison {
    /// `fold_truth_not_expr` of a comparison under `-ftrapping-math`: an
    /// ordered comparison of floating operands has no inverse, since the
    /// inverse would not raise the same exceptions.
    const fn invert(self, floating: bool) -> Option<Self> {
        Some(match self {
            | Self::Eq => Self::Ne,
            | Self::Ne => Self::Eq,
            | _ if floating => return None,
            | Self::Lt => Self::Ge,
            | Self::Gt => Self::Le,
            | Self::Le => Self::Gt,
            | Self::Ge => Self::Lt,
        })
    }
}

/// What a conversion asks of a type: its identity, as GCC's
/// `TYPE_MAIN_VARIANT` does, and its class, precision and signedness.
#[derive(Clone, Copy)]
struct GccType<'tu> {
    kind:     TypeKind<'tu>,
    class:    Class,
    bits:     u32,
    unsigned: bool,
}

impl GccType<'_> {
    fn same(self, other: Self) -> bool {
        self.kind == other.kind
    }

    const fn integral(self) -> bool {
        matches!(self.class, Class::Integer | Class::Boolean)
    }

    /// `tree_nop_conversion_p`.
    fn nop_conversion(self, inner: Self) -> bool {
        let address = |t: Self| t.integral() || t.class == Class::Pointer;
        if address(self) && address(inner) {
            self.bits == inner.bits
        } else {
            self.class == inner.class && self.bits == inner.bits
        }
    }

    /// `c_promoting_integer_type_p`.
    fn promotes(self, int: Self) -> bool {
        self.class == Class::Boolean || (self.class == Class::Integer && self.bits < int.bits)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Boolean,
    Integer,
    Pointer,
    Floating,
    Other,
}

/// The front end's builders and the folder, with the types they use.
struct Gcc<'g, 'a, 's, 'tu> {
    /// What `operand_equal_p` asks of the expressions behind two values.
    analyzer:  &'g Analyzer<'a, 'tu, 's>,
    scratch:   &'s Bump,
    /// `c_inhibit_evaluation_warnings`: the expression is not evaluated.
    inhibited: bool,
    int:       GccType<'tu>,
    long:      GccType<'tu>,
}

impl<'s, 'tu> Gcc<'_, '_, 's, 'tu> {
    fn alloc(&self, node: Node<'s, 'tu>) -> &'s Node<'s, 'tu> {
        self.scratch.alloc(node)
    }

    fn build1(
        &self,
        conversion: Conversion,
        ty: GccType<'tu>,
        operand: Node<'s, 'tu>,
    ) -> Node<'s, 'tu> {
        Node {
            code: Code::Convert(conversion, self.alloc(operand)),
            ty,
            effects: operand.effects,
        }
    }

    /// `build_binary_op` for a comparison of `left` and `right`.
    fn comparison(
        &self,
        code: Comparison,
        ty: GccType<'tu>,
        left: Node<'s, 'tu>,
        right: Node<'s, 'tu>,
    ) -> Node<'s, 'tu> {
        let pointer = left.ty.class == Class::Pointer || right.ty.class == Class::Pointer;
        Node {
            code: Code::Comparison {
                code,
                floating: left.ty.class == Class::Floating || right.ty.class == Class::Floating,
                pointers: pointer.then(|| &*self.scratch.alloc((left, right))),
            },
            ty,
            effects: left.effects || right.effects,
        }
    }

    fn cond(
        &self,
        ty: GccType<'tu>,
        condition: Node<'s, 'tu>,
        then: Node<'s, 'tu>,
        otherwise: Node<'s, 'tu>,
    ) -> Node<'s, 'tu> {
        Node {
            code: Code::Conditional {
                condition: self.alloc(condition),
                then:      self.alloc(then),
                otherwise: self.alloc(otherwise),
            },
            ty,
            effects: condition.effects || then.effects || otherwise.effects,
        }
    }

    fn logical_node(
        &self,
        or: bool,
        ty: GccType<'tu>,
        left: Node<'s, 'tu>,
        right: Node<'s, 'tu>,
    ) -> Node<'s, 'tu> {
        Node {
            code: Code::Logical {
                or,
                left: self.alloc(left),
                right: self.alloc(right),
            },
            ty,
            effects: left.effects || right.effects,
        }
    }

    fn compound_node(&self, left: Node<'s, 'tu>, right: Node<'s, 'tu>) -> Node<'s, 'tu> {
        Node {
            code:    Code::Compound {
                left:  self.alloc(left),
                right: self.alloc(right),
            },
            ty:      right.ty,
            effects: left.effects || right.effects,
        }
    }

    fn note_integer_operands(&self, node: Node<'s, 'tu>) -> Node<'s, 'tu> {
        Node {
            code: Code::IntegerOperands(self.alloc(node)),
            ..node
        }
    }

    // -- c-typeck.cc -------------------------------------------------------

    /// `build_unary_op` for `!e`: `c_objc_common_truthvalue_conversion`,
    /// then `invert_truthvalue_loc`. A constant made from a non-constant
    /// is wrapped so that it is not an integer constant expression.
    /// C99: §6.5.3.3 paragraph 5, p. 79; PDF p. 91.
    fn logical_not(&self, operand: Node<'s, 'tu>) -> Node<'s, 'tu> {
        let int_operands = int_const_operands(operand);
        let mut arg = self.truthvalue_conversion(operand, self.int);
        if int_operands {
            arg = remove_maybe_const(arg);
        }
        let ret = self.invert_truthvalue(arg);
        if matches!(ret.code, Code::Integer(_)) && !matches!(operand.code, Code::Integer(_)) {
            self.build1(Conversion::Nop, ret.ty, ret)
        } else if !matches!(ret.code, Code::Integer(_)) && int_operands {
            self.note_integer_operands(ret)
        } else {
            ret
        }
    }

    /// `build_unary_op` for `+e`: the promoted operand, no longer an lvalue,
    /// under a `CONVERT_EXPR`.
    fn unary_plus(&self, operand: Node<'s, 'tu>) -> Node<'s, 'tu> {
        let arg = self.non_lvalue(self.default_conversion(operand));
        if matches!(arg.code, Code::Integer(_)) {
            return arg;
        }
        let ret = self.build1(Conversion::Convert, arg.ty, arg);
        if int_const_operands(operand) {
            self.note_integer_operands(ret)
        } else {
            ret
        }
    }

    /// `build_binary_op` for `&&` and `||`, which compare both operands
    /// with zero.
    fn logical(&self, or: bool, left: Node<'s, 'tu>, right: Node<'s, 'tu>) -> Node<'s, 'tu> {
        let (left_int, right_int) = (int_const_operands(left), int_const_operands(right));
        let operand = |n: Node<'s, 'tu>, int_operands: bool| {
            let converted = self.truthvalue_conversion(n, self.int);
            if int_operands {
                remove_maybe_const(converted)
            } else {
                converted
            }
        };
        let (left, right) = (operand(left, left_int), operand(right, right_int));
        let ret = self.logical_node(or, self.int, left, right);
        if left_int && right_int {
            match self.fold(ret) {
                | folded @ Node {
                    code: Code::Integer(_),
                    ..
                } => folded,
                | _ => self.note_integer_operands(ret),
            }
        } else {
            ret
        }
    }

    /// `build_compound_expr`, which keeps the left operand.
    fn compound(&self, left: Node<'s, 'tu>, right: Node<'s, 'tu>) -> Node<'s, 'tu> {
        let int_operands = int_const_operands(left) && int_const_operands(right);
        let ret = self.compound_node(remove_maybe_const(left), remove_maybe_const(right));
        if int_operands {
            self.note_integer_operands(ret)
        } else {
            ret
        }
    }

    /// `build_conditional_expr`: the condition compared with zero, both arms
    /// promoted and converted to the result type, and nothing folded unless
    /// the result is an integer constant. Where the expression is evaluated,
    /// arithmetic arms of different types whose signedness differs, with an
    /// unsigned result, are folded for `-Wsign-compare` and kept from
    /// further folding.
    fn conditional(
        &self,
        condition: Node<'s, 'tu>,
        then: Node<'s, 'tu>,
        otherwise: Node<'s, 'tu>,
        ty: GccType<'tu>,
    ) -> Node<'s, 'tu> {
        let condition = self.truthvalue_conversion(self.default_conversion(condition), self.int);
        let int_operands = int_const_operands(condition)
            && int_const_operands(then)
            && int_const_operands(otherwise);
        let condition = remove_maybe_const(condition);
        if ty.class == Class::Other {
            return Node {
                code: Code::Value {
                    lvalue: false,
                    source: None,
                },
                ty,
                effects: condition.effects || then.effects || otherwise.effects,
            };
        }
        let signedness_differs = then.ty.unsigned != otherwise.ty.unsigned;
        let mut promoted =
            [then, otherwise].map(|n| self.default_conversion(remove_maybe_const(n)));
        let arithmetic = |n: Node<'s, 'tu>| matches!(n.ty.class, Class::Integer | Class::Floating);
        if !self.inhibited
            && signedness_differs
            && ty.integral()
            && ty.unsigned
            && promoted.iter().all(|&n| arithmetic(n))
            && !promoted[0].ty.same(promoted[1].ty)
        {
            promoted = promoted.map(|n| self.sign_compare_operand(n));
        }
        let [then, otherwise] = promoted.map(|n| self.convert(ty, n));
        if int_operands
            && let Code::Integer(c) = condition.code
            && matches!(
                (if c == 0 { otherwise } else { then }).code,
                Code::Integer(_)
            )
        {
            return self.fold_cond(ty, condition, then, otherwise);
        }
        let ret = self.cond(ty, condition, then, otherwise);
        if int_operands {
            self.note_integer_operands(ret)
        } else {
            ret
        }
    }

    /// An arm of `build_conditional_expr`'s `-Wsign-compare` check:
    /// `c_fully_fold`, then `c_wrap_maybe_const` around anything but a
    /// constant whose operands may be constant, which no fold looks into.
    fn sign_compare_operand(&self, arm: Node<'s, 'tu>) -> Node<'s, 'tu> {
        let mut maybe_const = true;
        let folded = self.fully_fold(arm, &mut maybe_const);
        if maybe_const && matches!(folded.code, Code::Integer(_)) {
            return folded;
        }
        Node {
            code: Code::Value {
                lvalue: false,
                source: None,
            },
            ..strip_type_nops(folded)
        }
    }

    /// `c_fully_fold`: each operand folded, then the expression. A comma
    /// clears `maybe_const`, since it cannot occur in a constant expression.
    fn fully_fold(&self, node: Node<'s, 'tu>, maybe_const: &mut bool) -> Node<'s, 'tu> {
        let ty = node.ty;
        match node.code {
            | Code::IntegerOperands(inner) => self.fully_fold(*inner, maybe_const),
            | Code::Convert(conversion, operand) =>
                self.fold_build1(conversion, ty, self.fully_fold(*operand, maybe_const)),
            | Code::Not(operand) => self.invert_truthvalue(self.fully_fold(*operand, maybe_const)),
            | Code::Negate(operand) => Node {
                code: Code::Negate(self.alloc(self.fully_fold(*operand, maybe_const))),
                ..node
            },
            | Code::Comparison {
                code,
                pointers: Some(&(left, right)),
                ..
            } => {
                let (left, right) = (
                    self.fully_fold(left, maybe_const),
                    self.fully_fold(right, maybe_const),
                );
                self.fold(Node {
                    ty,
                    ..self.comparison(code, ty, left, right)
                })
            },
            | Code::Logical { or, left, right } => {
                let (left, right) = (
                    self.fully_fold(*left, maybe_const),
                    self.fully_fold(*right, maybe_const),
                );
                self.fold(self.logical_node(or, ty, left, right))
            },
            | Code::Conditional {
                condition,
                then,
                otherwise,
            } => {
                let condition = self.fully_fold(*condition, maybe_const);
                let then = self.fully_fold(*then, maybe_const);
                let otherwise = self.fully_fold(*otherwise, maybe_const);
                self.fold_cond(ty, condition, then, otherwise)
            },
            | Code::Compound { left, right } => {
                *maybe_const = false;
                let (left, right) = (
                    self.fully_fold(*left, maybe_const),
                    self.fully_fold(*right, maybe_const),
                );
                self.fold(self.compound_node(left, right))
            },
            | Code::Integer(_)
            | Code::Real { .. }
            | Code::Comparison { .. }
            | Code::Value { .. } => node,
        }
    }

    /// `build_c_cast`. A cast keeps a constant an integer constant only from
    /// an integer or floating constant.
    /// C99: §6.5.4 paragraph 4, p. 81; PDF p. 93.
    fn cast(&self, ty: GccType<'tu>, expr: Node<'s, 'tu>) -> Node<'s, 'tu> {
        let int_operands = int_const_operands(expr);
        let mut value = if int_operands {
            remove_maybe_const(expr)
        } else {
            expr
        };
        if !value.ty.same(ty)
            && int_operands
            && !matches!(value.code, Code::Integer(_))
            && ty.class == Class::Boolean
        {
            value = self.note_integer_operands(value);
        }
        value = self.convert(ty, value);
        value = self.non_lvalue(value);
        let constant_operand = (matches!(expr.code, Code::Integer(_)) && expr.ty.integral())
            || matches!(expr.code, Code::Real { .. });
        if matches!(value.code, Code::Real { .. })
            || (matches!(value.code, Code::Integer(_)) && !constant_operand)
        {
            value = self.build1(Conversion::Nop, ty, value);
        }
        if int_operands && ty.integral() && !int_const_operands(value) {
            value = self.note_integer_operands(value);
        }
        value
    }

    fn non_lvalue(&self, node: Node<'s, 'tu>) -> Node<'s, 'tu> {
        if matches!(node.code, Code::Value { lvalue: true, .. }) {
            self.build1(Conversion::NonLvalue, node.ty, node)
        } else {
            node
        }
    }

    /// `default_conversion`: the integer promotions.
    /// C99: §6.3.1.1 paragraph 2, pp. 42-43; PDF pp. 54-55.
    fn default_conversion(&self, node: Node<'s, 'tu>) -> Node<'s, 'tu> {
        if node.ty.promotes(self.int) {
            self.convert(self.int, node)
        } else {
            node
        }
    }

    // -- c-convert.cc, c-common.cc -----------------------------------------

    /// `c_convert`, which folds what `convert.cc` builds.
    fn convert(&self, ty: GccType<'tu>, expr: Node<'s, 'tu>) -> Node<'s, 'tu> {
        if expr.ty.same(ty) {
            return expr;
        }
        let e = strip_type_nops(expr);
        match ty.class {
            | Class::Boolean => self.truthvalue_conversion(expr, ty),
            | Class::Integer => self.fold(self.convert_to_integer(ty, e)),
            | Class::Floating => self.fold(self.convert_to_real(ty, e)),
            | Class::Pointer => self.fold(self.convert_to_pointer(ty, e)),
            | Class::Other => Node {
                code: Code::Value {
                    lvalue: false,
                    source: None,
                },
                ty,
                effects: e.effects,
            },
        }
    }

    /// `c_objc_common_truthvalue_conversion`: `e != 0` in type `ty`.
    /// C99: §6.3.1.2 paragraph 1, p. 43; PDF p. 55.
    fn truthvalue_conversion(&self, expr: Node<'s, 'tu>, ty: GccType<'tu>) -> Node<'s, 'tu> {
        let int_const = matches!(expr.code, Code::Integer(_))
            || (matches!(expr.code, Code::Real { .. }) && ty.class == Class::Boolean);
        let int_operands = int_const_operands(expr);
        let ret = if int_operands && !matches!(expr.code, Code::Integer(_)) {
            let inner = remove_maybe_const(expr);
            let zero = integer(inner.ty, 0);
            self.note_integer_operands(Node {
                ty,
                ..self.comparison(Comparison::Ne, ty, inner, zero)
            })
        } else {
            self.fold_convert(ty, self.common_truthvalue_conversion(expr))
        };
        match ret.code {
            | Code::Integer(_) if int_operands && !int_const => self.note_integer_operands(ret),
            | Code::Integer(_) if !int_const => self.build1(Conversion::Nop, ret.ty, ret),
            | _ => ret,
        }
    }

    /// `c_common_truthvalue_conversion`, which gives a truth value the type
    /// `int`, distributes into the arms of a conditional, and otherwise
    /// compares with zero.
    fn common_truthvalue_conversion(&self, expr: Node<'s, 'tu>) -> Node<'s, 'tu> {
        let int = self.int;
        match expr.code {
            | Code::Comparison { .. } | Code::Not(_) | Code::Logical { .. } if expr.ty.same(int) =>
                expr,
            | Code::Comparison { .. } => Node { ty: int, ..expr },
            | Code::Not(operand) => Node {
                code:    Code::Not(self.alloc(self.common_truthvalue_conversion(*operand))),
                ty:      int,
                effects: expr.effects,
            },
            | Code::Logical { or, left, right } => self.logical_node(
                or,
                int,
                self.common_truthvalue_conversion(*left),
                self.common_truthvalue_conversion(*right),
            ),
            | Code::Integer(v) => integer(int, i128::from(v != 0)),
            | Code::Real { truth, .. } => integer(int, i128::from(truth)),
            | Code::Convert(Conversion::Float, operand) | Code::Negate(operand) =>
                self.common_truthvalue_conversion(*operand),
            | Code::Conditional {
                condition,
                then,
                otherwise,
            } => self.cond(
                int,
                *condition,
                self.common_truthvalue_conversion(*then),
                self.common_truthvalue_conversion(*otherwise),
            ),
            // A conversion that does not narrow.
            | Code::Convert(conversion, operand)
                if conversion.is_nop() && expr.ty.bits >= operand.ty.bits =>
                self.common_truthvalue_conversion(*operand),
            | _ => self.comparison(Comparison::Ne, int, expr, integer(expr.ty, 0)),
        }
    }

    // -- convert.cc --------------------------------------------------------

    /// `convert_to_integer_1`: a comparison takes the new type, a
    /// widening builds a folded `NOP_EXPR`, a narrowing pushes into the
    /// arms of a conditional or drops an earlier widening, and a floating
    /// value truncates.
    /// C99: §6.3.1.3, p. 43; PDF p. 55; §6.3.1.4 paragraph 1, p. 43; PDF p. 55.
    fn convert_to_integer(&self, ty: GccType<'tu>, expr: Node<'s, 'tu>) -> Node<'s, 'tu> {
        if let Code::Compound { left, right } = expr.code {
            let converted = self.convert_to_integer(ty, *right);
            return self.compound_node(*left, converted);
        }
        match expr.ty.class {
            | Class::Pointer => {
                if let Code::Integer(0) = expr.code {
                    return integer(ty, 0);
                }
                let long = self.fold_build1(Conversion::Convert, self.long, expr);
                self.fold_convert(ty, long)
            },
            | Class::Integer | Class::Boolean => {
                if let Code::Comparison { .. } = expr.code {
                    return Node { ty, ..expr };
                }
                if ty.bits >= expr.ty.bits {
                    return self.fold_build1(Conversion::Nop, ty, expr);
                }
                match expr.code {
                    | Code::Convert(conversion, operand) if conversion.is_nop() =>
                        self.convert(ty, get_unwidened(*operand, ty)),
                    | Code::Conditional {
                        condition,
                        then,
                        otherwise,
                    } => self.fold_cond(
                        ty,
                        *condition,
                        self.convert(ty, *then),
                        self.convert(ty, *otherwise),
                    ),
                    | Code::Integer(_) => self.fold_convert(ty, expr),
                    | _ => self.build1(Conversion::Convert, ty, expr),
                }
            },
            | Class::Floating => self.build1(Conversion::FixTrunc, ty, expr),
            | Class::Other => Node {
                code: Code::Value {
                    lvalue: false,
                    source: None,
                },
                ty,
                effects: expr.effects,
            },
        }
    }

    /// `convert_to_pointer_1`: an integer of another width converts to
    /// `long` first.
    /// C99: §6.3.2.3 paragraph 5, p. 47; PDF p. 59.
    fn convert_to_pointer(&self, ty: GccType<'tu>, expr: Node<'s, 'tu>) -> Node<'s, 'tu> {
        match expr.ty.class {
            | Class::Pointer => self.fold_build1(Conversion::Nop, ty, expr),
            | Class::Integer | Class::Boolean => {
                let expr = if expr.ty.bits == ty.bits {
                    expr
                } else {
                    self.fold_build1(Conversion::Nop, self.long, expr)
                };
                self.fold_build1(Conversion::Convert, ty, expr)
            },
            | _ => Node {
                code: Code::Value {
                    lvalue: false,
                    source: None,
                },
                ty,
                effects: expr.effects,
            },
        }
    }

    /// `convert_to_real_1`: an unfolded `NOP_EXPR` or `FLOAT_EXPR`.
    /// C99: §6.3.1.4 paragraph 2, pp. 43-44; PDF pp. 55-56; §6.3.1.5, p. 44;
    /// PDF p. 56.
    fn convert_to_real(&self, ty: GccType<'tu>, expr: Node<'s, 'tu>) -> Node<'s, 'tu> {
        if let Code::Compound { left, right } = expr.code {
            let converted = self.convert_to_real(ty, *right);
            return self.compound_node(*left, converted);
        }
        match expr.ty.class {
            | Class::Floating => self.build1(Conversion::Nop, ty, expr),
            | Class::Integer | Class::Boolean => self.build1(Conversion::Float, ty, expr),
            | _ => Node {
                code: Code::Value {
                    lvalue: false,
                    source: None,
                },
                ty,
                effects: expr.effects,
            },
        }
    }

    // -- fold-const.cc, match.pd -------------------------------------------

    /// `fold`, as `c_convert` applies it to the conversion it built.
    fn fold(&self, node: Node<'s, 'tu>) -> Node<'s, 'tu> {
        match node.code {
            | Code::Convert(conversion, operand) => self
                .fold_unary(conversion, node.ty, *operand)
                .unwrap_or(node),
            | Code::Conditional {
                condition,
                then,
                otherwise,
            } => self
                .fold_ternary(node.ty, *condition, *then, *otherwise)
                .unwrap_or(node),
            // `fold_binary_loc` drops a left operand without effects.
            | Code::Compound { left, right } if !left.effects && !tree_constant(*right) =>
                match right.code {
                    | Code::Integer(0) => self.build1(Conversion::Nop, node.ty, *right),
                    | _ => self.fold_convert(node.ty, *right),
                },
            // `fold_binary_loc` takes a comma out of a pointer operand, and
            // compares the arms of a conditional with a constant.
            | Code::Comparison {
                code,
                pointers: Some(&(left, right)),
                ..
            } => {
                let compare = |l: Node<'s, 'tu>, r: Node<'s, 'tu>| Node {
                    ty: node.ty,
                    ..self.fold(self.comparison(code, node.ty, l, r))
                };
                match (strip_sign_nops(left).code, strip_sign_nops(right).code) {
                    | (Code::Compound { left: l, right: r }, _) =>
                        self.compound_node(*l, compare(self.fold_convert(left.ty, *r), right)),
                    | (_, Code::Compound { left: l, right: r }) =>
                        self.compound_node(*l, compare(left, self.fold_convert(right.ty, *r))),
                    | (
                        Code::Conditional {
                            condition,
                            then,
                            otherwise,
                        },
                        _,
                    ) if tree_constant(right) => {
                        let then = compare(self.fold_convert(left.ty, *then), right);
                        let otherwise = compare(self.fold_convert(left.ty, *otherwise), right);
                        self.fold_cond(node.ty, *condition, then, otherwise)
                    },
                    | _ => node,
                }
            },
            // A constant operand decides `&&` or `||`, or drops out.
            | Code::Logical { or, left, right } => {
                let decides =
                    |n: Node<'s, 'tu>| matches!(n.code, Code::Integer(v) if (v != 0) == or);
                let (a, b) = (strip_nops(*left), strip_nops(*right));
                if decides(a) {
                    self.fold_convert(node.ty, a)
                } else if matches!(a.code, Code::Integer(_)) {
                    self.non_lvalue(self.fold_convert(node.ty, b))
                } else if matches!(b.code, Code::Integer(_)) && !decides(b) && !a.effects {
                    self.non_lvalue(self.fold_convert(node.ty, a))
                } else if decides(b) {
                    let constant = self.fold_convert(node.ty, b);
                    if a.effects {
                        self.compound_node(a, constant)
                    } else {
                        constant
                    }
                } else {
                    node
                }
            },
            | _ => node,
        }
    }

    fn fold_build1(
        &self,
        conversion: Conversion,
        ty: GccType<'tu>,
        operand: Node<'s, 'tu>,
    ) -> Node<'s, 'tu> {
        self.fold_unary(conversion, ty, operand)
            .unwrap_or_else(|| self.build1(conversion, ty, operand))
    }

    /// `fold_unary_loc` for a conversion: a constant converts, `match.pd`
    /// drops conversions in a row, a conversion goes into a comma or the
    /// arms of a conditional, and a comparison converted to `_Bool` takes
    /// that type, or to a floating type becomes `c ? 1 : 0`.
    fn fold_unary(
        &self,
        conversion: Conversion,
        ty: GccType<'tu>,
        op0: Node<'s, 'tu>,
    ) -> Option<Node<'s, 'tu>> {
        let arg0 = if matches!(conversion, Conversion::NonLvalue | Conversion::FixTrunc) {
            strip_nops(op0)
        } else {
            strip_sign_nops(op0)
        };
        if let Some(folded) = const_unop(conversion, ty, arg0) {
            return Some(folded);
        }
        if let Some(simplified) = self.simplify_conversion(conversion, ty, op0) {
            return Some(simplified);
        }
        match arg0.code {
            | Code::Compound { left, right } => {
                let inner = self.fold_build1(conversion, ty, self.fold_convert(op0.ty, *right));
                return Some(self.compound_node(*left, inner));
            },
            | Code::Conditional {
                condition,
                then,
                otherwise,
            } => {
                let arm = |n: Node<'s, 'tu>| {
                    self.fold_build1(conversion, ty, self.fold_convert(op0.ty, n))
                };
                let tem = self.fold_cond(ty, *condition, arm(*then), arm(*otherwise));
                // Bring a conversion that only moved into both arms back out,
                // unless it is a cheap one between integers.
                if (conversion.is_nop() || conversion == Conversion::NonLvalue)
                    && let Code::Conditional {
                        condition,
                        then:
                            Node {
                                code: Code::Convert(c1, a),
                                ..
                            },
                        otherwise:
                            Node {
                                code: Code::Convert(c2, b),
                                ..
                            },
                    } = tem.code
                    && *c1 == conversion
                    && *c2 == conversion
                    && a.ty.same(b.ty)
                    && !(tem.ty.integral() && a.ty.integral() && tem.ty.bits <= 64)
                {
                    let inner = self.cond(a.ty, *condition, **a, **b);
                    return Some(self.build1(conversion, ty, inner));
                }
                return Some(tem);
            },
            | _ => {},
        }
        match conversion {
            | Conversion::NonLvalue => (!matches!(op0.code, Code::Value { lvalue: true, .. }))
                .then(|| self.fold_convert(ty, op0)),
            | _ => match op0.code {
                | Code::Comparison { .. } if ty.class == Class::Boolean => Some(Node { ty, ..op0 }),
                | Code::Comparison { .. } if !ty.integral() => {
                    let (one, zero) = (constant(ty, 1), constant(ty, 0));
                    Some(self.cond(ty, op0, one, zero))
                },
                | _ => None,
            },
        }
    }

    /// The GENERIC conversion rules of `match.pd`: a conversion to the
    /// operand's own type goes; two conversions in a row become one where
    /// the middle type changes nothing; and a conversion between integer
    /// types goes into the arms of `c ? n : m`.
    fn simplify_conversion(
        &self,
        conversion: Conversion,
        ty: GccType<'tu>,
        op0: Node<'s, 'tu>,
    ) -> Option<Node<'s, 'tu>> {
        if conversion == Conversion::NonLvalue {
            return None;
        }
        if ty.same(op0.ty) {
            return Some(op0);
        }
        if let Code::Convert(inner_conversion, x) = op0.code
            && matches!(
                inner_conversion,
                Conversion::Nop | Conversion::Convert | Conversion::Float
            )
            && let Some(drop) = drops_middle(x.ty, op0.ty, ty)
        {
            return Some(self.fold_build1(if drop { conversion } else { Conversion::Nop }, ty, *x));
        }
        if conversion.is_nop()
            && let Code::Conditional {
                condition,
                then:
                    then @ Node {
                        code: Code::Integer(_),
                        ..
                    },
                otherwise:
                    otherwise @ Node {
                        code: Code::Integer(_),
                        ..
                    },
            } = op0.code
            && ty.integral()
            && op0.ty.integral()
        {
            return Some(self.fold_cond(
                ty,
                *condition,
                self.fold_convert(ty, *then),
                self.fold_convert(ty, *otherwise),
            ));
        }
        None
    }

    /// `fold_convert_loc`.
    fn fold_convert(&self, ty: GccType<'tu>, arg: Node<'s, 'tu>) -> Node<'s, 'tu> {
        if arg.ty.same(ty) {
            return arg;
        }
        match ty.class {
            | Class::Integer | Class::Boolean | Class::Pointer => match arg.code {
                | Code::Integer(v) => integer(ty, v),
                | _ => self.fold_build1(Conversion::Nop, ty, arg),
            },
            | Class::Floating => match arg.code {
                | Code::Integer(v) => constant(ty, v),
                | Code::Real { .. } => Node { ty, ..arg },
                | _ if arg.ty.class == Class::Floating =>
                    self.fold_build1(Conversion::Nop, ty, arg),
                | _ => self.fold_build1(Conversion::Float, ty, arg),
            },
            | Class::Other => Node {
                code: Code::Value {
                    lvalue: false,
                    source: None,
                },
                ty,
                effects: arg.effects,
            },
        }
    }

    /// `invert_truthvalue_loc`, which falls back on a `TRUTH_NOT_EXPR`.
    fn invert_truthvalue(&self, arg: Node<'s, 'tu>) -> Node<'s, 'tu> {
        self.fold_invert_truthvalue(arg).unwrap_or_else(|| Node {
            code: Code::Not(self.alloc(arg)),
            ..arg
        })
    }

    /// `fold_unary_loc` for `TRUTH_NOT_EXPR`: a constant inverts, and
    /// otherwise `fold_truth_not_expr`.
    fn fold_invert_truthvalue(&self, arg: Node<'s, 'tu>) -> Option<Node<'s, 'tu>> {
        let arg0 = strip_nops(arg);
        if let Code::Integer(v) = arg0.code {
            return Some(integer(arg.ty, i128::from(v == 0)));
        }
        let tem = self.fold_truth_not_expr(arg0)?;
        Some(self.fold_convert(arg.ty, tem))
    }

    /// `fold_truth_not_expr`: a comparison inverts, De Morgan's laws apply,
    /// and a conditional or comma inverts its arms without folding.
    fn fold_truth_not_expr(&self, arg: Node<'s, 'tu>) -> Option<Node<'s, 'tu>> {
        let ty = arg.ty;
        Some(match arg.code {
            | Code::Comparison {
                code,
                floating,
                pointers,
            } => Node {
                code: Code::Comparison {
                    code: code.invert(floating)?,
                    floating,
                    pointers,
                },
                ..arg
            },
            | Code::Integer(v) => integer(ty, i128::from(v == 0)),
            | Code::Logical { or, left, right } => self.logical_node(
                !or,
                ty,
                self.invert_truthvalue(*left),
                self.invert_truthvalue(*right),
            ),
            | Code::Not(operand) => *operand,
            | Code::Conditional {
                condition,
                then,
                otherwise,
            } => self.cond(
                ty,
                *condition,
                self.invert_truthvalue(*then),
                self.invert_truthvalue(*otherwise),
            ),
            | Code::Compound { left, right } =>
                self.compound_node(*left, self.invert_truthvalue(*right)),
            | Code::Convert(Conversion::NonLvalue, operand) => self.invert_truthvalue(*operand),
            | Code::Convert(conversion, _) if conversion.is_nop() && ty.class == Class::Boolean =>
                Node {
                    code: Code::Not(self.alloc(arg)),
                    ..arg
                },
            | Code::Convert(
                conversion @ (Conversion::Nop | Conversion::Convert | Conversion::Float),
                operand,
            ) => self.build1(conversion, ty, self.invert_truthvalue(*operand)),
            | _ => return None,
        })
    }

    /// `operand_equal_p` with `types_match`, for operands without side
    /// effects.
    fn operand_equal(&self, first: Node<'s, 'tu>, second: Node<'s, 'tu>) -> bool {
        if first.effects || second.effects || !first.ty.same(second.ty) {
            return false;
        }
        let same = |x: &Node<'s, 'tu>, y: &Node<'s, 'tu>| self.operand_equal(*x, *y);
        match (first.code, second.code) {
            | (Code::Integer(x), Code::Integer(y)) => x == y,
            | (Code::Convert(kind, x), Code::Convert(other_kind, y)) =>
                kind == other_kind && same(x, y),
            | (Code::Not(x), Code::Not(y))
            | (Code::Negate(x), Code::Negate(y))
            | (Code::IntegerOperands(x), Code::IntegerOperands(y)) => same(x, y),
            | (
                Code::Comparison {
                    code,
                    pointers: Some((left, right)),
                    ..
                },
                Code::Comparison {
                    code: other_code,
                    pointers: Some((other_left, other_right)),
                    ..
                },
            ) => code == other_code && same(left, other_left) && same(right, other_right),
            | (
                Code::Logical { or, left, right },
                Code::Logical {
                    or: other_or,
                    left: other_left,
                    right: other_right,
                },
            ) => or == other_or && same(left, other_left) && same(right, other_right),
            | (
                Code::Conditional {
                    condition,
                    then,
                    otherwise,
                },
                Code::Conditional {
                    condition: other_condition,
                    then: other_then,
                    otherwise: other_otherwise,
                },
            ) =>
                same(condition, other_condition)
                    && same(then, other_then)
                    && same(otherwise, other_otherwise),
            | (
                Code::Value {
                    source: Some(x), ..
                },
                Code::Value {
                    source: Some(y), ..
                },
            ) => self.analyzer.same_operand(x, y),
            | _ => false,
        }
    }

    fn fold_cond(
        &self,
        ty: GccType<'tu>,
        condition: Node<'s, 'tu>,
        then: Node<'s, 'tu>,
        otherwise: Node<'s, 'tu>,
    ) -> Node<'s, 'tu> {
        self.fold_ternary(ty, condition, then, otherwise)
            .unwrap_or_else(|| self.cond(ty, condition, then, otherwise))
    }

    /// `fold_ternary_loc` for `COND_EXPR`: a constant condition selects,
    /// `c ? 1 : 0` of `c`'s type is `c`, `c ? 0 : 1` is `!c`, and with a
    /// truth value `c` and arm `b`, `c ? b : 0`, `c ? b : 1`, `c ? 0 : b`
    /// and `c ? 1 : b` become `&&` and `||`.
    fn fold_ternary(
        &self,
        ty: GccType<'tu>,
        op0: Node<'s, 'tu>,
        op1: Node<'s, 'tu>,
        op2: Node<'s, 'tu>,
    ) -> Option<Node<'s, 'tu>> {
        let (arg0, arg1) = (strip_nops(op0), strip_nops(op1));
        if let Code::Integer(v) = arg0.code {
            return Some(if v == 0 { op2 } else { op1 });
        }
        // `match.pd`: `A ? B : B` is `B`.
        if !op0.effects && self.operand_equal(op1, op2) {
            return Some(op1);
        }
        let (zero, one) = (
            |n: Node<'s, 'tu>| matches!(n.code, Code::Integer(0)),
            |n: Node<'s, 'tu>| matches!(n.code, Code::Integer(1)),
        );
        let truth = arg0.code.is_truth();
        if truth
            && swaps(op1, op2)
            && let Some(inverse) = self.fold_invert_truthvalue(arg0)
        {
            return Some(self.fold_cond(ty, inverse, op2, op1));
        }
        if one(op1) && zero(op2) && ty.same(arg0.ty) {
            return Some(arg0);
        }
        if zero(op1) && one(op2) && truth {
            return Some(self.fold_convert(ty, self.invert_truthvalue(arg0)));
        }
        if zero(op2) && truth && arg1.code.is_truth() {
            return Some(self.logical_node(false, ty, self.fold_convert(ty, arg0), op1));
        }
        if one(op2)
            && truth
            && arg1.code.is_truth()
            && let Some(inverse) = self.fold_invert_truthvalue(arg0)
        {
            return Some(self.logical_node(true, ty, self.fold_convert(ty, inverse), op1));
        }
        if zero(arg1)
            && truth
            && op2.code.is_truth()
            && let Some(inverse) = self.fold_invert_truthvalue(arg0)
        {
            return Some(self.logical_node(false, ty, self.fold_convert(ty, inverse), op2));
        }
        if one(arg1) && truth && op2.code.is_truth() {
            return Some(self.logical_node(true, ty, self.fold_convert(ty, arg0), op2));
        }
        None
    }
}

/// An `INTEGER_CST` of type `ty`.
fn integer<'s>(ty: GccType<'_>, value: i128) -> Node<'s, '_> {
    Node {
        code: Code::Integer(wrap(ty, value)),
        ty,
        effects: false,
    }
}

/// `build_one_cst` and `build_zero_cst`.
fn constant<'s>(ty: GccType<'_>, value: i128) -> Node<'s, '_> {
    if ty.class == Class::Floating {
        Node {
            code: Code::Real {
                truth:     value != 0,
                truncated: Some(value),
            },
            ty,
            effects: false,
        }
    } else {
        integer(ty, value)
    }
}

/// `const_unop` for a conversion of a constant.
fn const_unop<'s, 'tu>(
    conversion: Conversion,
    ty: GccType<'tu>,
    arg: Node<'s, 'tu>,
) -> Option<Node<'s, 'tu>> {
    match (ty.class, arg.code) {
        | (Class::Integer | Class::Boolean | Class::Pointer, Code::Integer(v)) =>
            Some(integer(ty, v)),
        | (Class::Integer | Class::Boolean, Code::Real { truncated, .. })
            if conversion == Conversion::FixTrunc =>
            truncated.map(|v| integer(ty, saturate(ty, v))),
        | (Class::Floating, Code::Integer(v)) => Some(constant(ty, v)),
        | (Class::Floating, Code::Real { .. }) => Some(Node { ty, ..arg }),
        | _ => None,
    }
}

/// `EXPR_INT_CONST_OPERANDS`.
fn int_const_operands(node: Node<'_, '_>) -> bool {
    node.ty.integral() && matches!(node.code, Code::Integer(_) | Code::IntegerOperands(_))
}

/// `remove_c_maybe_const_expr`.
fn remove_maybe_const<'s, 'tu>(node: Node<'s, 'tu>) -> Node<'s, 'tu> {
    match node.code {
        | Code::IntegerOperands(inner) => *inner,
        | _ => node,
    }
}

/// `TREE_CONSTANT`.
fn tree_constant(node: Node<'_, '_>) -> bool {
    match node.code {
        | Code::Integer(_) | Code::Real { .. } => true,
        | Code::Convert(_, operand) => tree_constant(*operand),
        | _ => false,
    }
}

/// `STRIP_NOPS`: conversions that keep the precision.
fn strip_nops<'s, 'tu>(mut node: Node<'s, 'tu>) -> Node<'s, 'tu> {
    while let Code::Convert(Conversion::Nop | Conversion::Convert | Conversion::NonLvalue, operand) =
        node.code
        && node.ty.nop_conversion(operand.ty)
    {
        node = *operand;
    }
    node
}

/// `STRIP_SIGN_NOPS`: conversions that keep the precision and signedness.
fn strip_sign_nops<'s, 'tu>(mut node: Node<'s, 'tu>) -> Node<'s, 'tu> {
    while let Code::Convert(Conversion::Nop | Conversion::Convert | Conversion::NonLvalue, operand) =
        node.code
        && node.ty.nop_conversion(operand.ty)
        && node.ty.unsigned == operand.ty.unsigned
        && (node.ty.class == Class::Pointer) == (operand.ty.class == Class::Pointer)
    {
        node = *operand;
    }
    node
}

/// `STRIP_TYPE_NOPS`: conversions to the operand's own type.
fn strip_type_nops<'s, 'tu>(mut node: Node<'s, 'tu>) -> Node<'s, 'tu> {
    while let Code::Convert(Conversion::Nop | Conversion::Convert | Conversion::NonLvalue, operand) =
        node.code
        && node.ty.same(operand.ty)
    {
        node = *operand;
    }
    node
}

/// `get_unwidened`: the operand under widening conversions, as long as it
/// stays as wide as `to`.
fn get_unwidened<'s, 'tu>(mut node: Node<'s, 'tu>, to: GccType<'tu>) -> Node<'s, 'tu> {
    while let Code::Convert(Conversion::Nop | Conversion::Convert, inner) = node.code
        && inner.ty.integral()
        && node.ty.integral()
        && inner.ty.bits <= node.ty.bits
        && inner.ty.bits >= to.bits
    {
        node = *inner;
    }
    node
}

/// `tree_swap_operands_p` for the arms of a conditional: a constant goes
/// second.
fn swaps(op1: Node<'_, '_>, op2: Node<'_, '_>) -> bool {
    let constant = |n: Node<'_, '_>| matches!(n.code, Code::Integer(_) | Code::Real { .. });
    if constant(op2) {
        return false;
    }
    if constant(op1) {
        return true;
    }
    let (a, b) = (strip_nops(op1), strip_nops(op2));
    if tree_constant(b) {
        return false;
    }
    if tree_constant(a) {
        return true;
    }
    !matches!(b.code, Code::Value { lvalue: true, .. })
        && matches!(a.code, Code::Value { lvalue: true, .. })
}

/// Whether `match.pd` folds `(last)(inter) x`, with `x` of type `inside`, to
/// a single conversion: `Some(true)` keeps the outer conversion's code,
/// `Some(false)` makes it a `NOP_EXPR`.
fn drops_middle(inside: GccType<'_>, inter: GccType<'_>, last: GccType<'_>) -> Option<bool> {
    let (i, m, f) = (inside.bits, inter.bits, last.bits);
    let (iu, mu, fu) = (inside.unsigned, inter.unsigned, last.unsigned);
    let (ii, mi, fi) = (inside.integral(), inter.integral(), last.integral());
    let (ip, mp, fp) = (
        inside.class == Class::Pointer,
        inter.class == Class::Pointer,
        last.class == Class::Pointer,
    );
    let (ifl, mfl, ffl) = (
        inside.class == Class::Floating,
        inter.class == Class::Floating,
        last.class == Class::Floating,
    );
    // Converting something to its own type via one at least as wide.
    let own = last.same(inside) && (((mi || mp) && fi) || (mfl && ffl)) && m >= f;
    // The middle type widens without changing the signedness.
    let widens = ((mi && ii) || (mfl && ifl)) && (fi || ffl) && m >= i && (mfl || mu == iu);
    // A sign extension of a zero extension, a final conversion that keeps
    // the middle width, or a truncation of a sign extension.
    let extends = ii
        && mi
        && fi
        && ((i < m && m < f && iu && !mu) || f == m || (i < m && m > f && !iu && mu));
    // Two conversions in a row that are not needed.
    let needless = !ifl
        && !mfl
        && !ffl
        && (m >= i || m >= f)
        && !(ii && mi && mu != iu && m < f)
        && ((mu && m > i) == (fu && f > m))
        && !(ip && m != f)
        && !(fp && i != m);
    if own || widens || extends || needless {
        return Some(true);
    }
    // A truncation of a widening.
    (ii && mi && fi && f <= i && m >= i).then_some(false)
}

/// `fold_convert_const_int_from_real`: a truncated floating value outside
/// the range of `ty` becomes its nearest bound.
fn saturate(ty: GccType<'_>, value: i128) -> i128 {
    if ty.class == Class::Boolean || ty.bits == 0 || ty.bits > 128 {
        return value;
    }
    let (min, max) = match (ty.unsigned, ty.bits) {
        | (true, 127..) => (0, i128::MAX),
        | (true, bits) => (0, (1 << bits) - 1),
        | (false, 128) => (i128::MIN, i128::MAX),
        | (false, bits) => (-(1 << (bits - 1)), (1 << (bits - 1)) - 1),
    };
    value.clamp(min, max)
}

/// `value` as an integer of type `ty`.
fn wrap(ty: GccType<'_>, value: i128) -> i128 {
    match ty.class {
        | Class::Boolean => i128::from(value != 0),
        | _ if ty.bits == 0 || ty.bits >= 128 => value,
        | _ => {
            let shift = 128 - ty.bits;
            if ty.unsigned {
                (value << shift)
                    .cast_unsigned()
                    .wrapping_shr(shift)
                    .cast_signed()
            } else {
                (value << shift) >> shift
            }
        },
    }
}
