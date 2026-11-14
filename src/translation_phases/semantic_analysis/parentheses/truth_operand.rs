//! Whether GCC takes the right operand of `!x & e` or `!x | e` for a truth
//! value. `warn_about_parentheses` asks this of the tree GCC has built for
//! `e`, not of its syntax: a comparison or a logical operation passes, and so
//! does an operand of type `_Bool` or one that folded to 0 or 1 (GCC manual,
//! "Warning Options", `-Wparentheses`). An explicit conversion keeps a
//! comparison or a logical operation only where GCC's conversion and folding
//! code keeps one, so [`Analyzer::gcc_tree`] models what each cast, `!`,
//! comma and conditional builds, after GCC 14's `c_convert`,
//! `convert_to_integer_1`, `c_common_truthvalue_conversion`,
//! `fold_truth_not_expr`, `fold_unary_loc`, `fold_ternary_loc` and the
//! conversion pairs in `match.pd`. Measured against GCC 14: `!a & (int)(b !=
//! 0)`, `!a & (unsigned)(b != 0)`, `!a & __extension__ (b != 0)`, `!a &
//! (int)(long)(b || a)` and `!a & (int)(char *)(b != 0)` pass, while `!a &
//! (unsigned)(b || a)`, `!a & (long)(double)(b < a)`, `!a & (0, b != 0)` and
//! `!a & (b ? !a : !b)` are reported.
//! C99: §6.5.4 paragraph 4, p. 81; PDF p. 93; §6.3.1.2 paragraph 1, p. 43;
//! PDF p. 55.

use super::super::{
    Analyzer,
    types::{
        Scalar,
        TypeId,
        TypeKind,
    },
};
use crate::translation_phases::parsing::syntax::{
    BinaryOperator as B,
    Expression,
    ExpressionType as E,
    UnaryOperator as U,
};

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Whether GCC leaves `!x` unquestioned before `& e` or `| e`.
    pub(in crate::translation_phases::semantic_analysis) fn gcc_truth_operand(
        &self,
        e: &'tu Expression<'tu>,
    ) -> bool {
        let info = self.expression_info(e);
        matches!(self.gcc_tree(e).tree, Tree::Truth(_))
            || self.gcc_type(info.ty).class == Class::Boolean
            || info
                .integer
                .is_some_and(|value| value.value == 0 || value.value == 1)
    }

    /// The tree GCC builds for `e`, as far as the truth of its value goes.
    fn gcc_tree(&self, e: &'tu Expression<'tu>) -> Node<'tu> {
        let ty = self.gcc_type(self.expression_info(e).ty);
        let tree = match e.kind {
            | E::Parenthesized { expression }
            | E::Unary {
                operator: U::Extension,
                operand_expression: expression,
            } => return self.gcc_tree(expression),
            | E::Binary {
                operator:
                    B::LessThan
                    | B::GreaterThan
                    | B::LessThanOrEqual
                    | B::GreaterThanOrEqual
                    | B::Equal
                    | B::NotEqual,
                ..
            } => Tree::Truth(Truth {
                ty,
                comparison: true,
            }),
            | E::Binary {
                operator: B::LogicalAnd | B::LogicalOr,
                ..
            } => Tree::Truth(Truth {
                ty,
                comparison: false,
            }),
            // C99: §6.5.17 paragraph 2, p. 94; PDF p. 106.
            | E::Binary {
                operator: B::Comma,
                left_expression,
                right_expression,
            } => Tree::Compound {
                right: right_expression,
                pure:  !self.has_side_effects(left_expression),
            },
            | E::Unary {
                operator: U::LogicalNot,
                operand_expression,
            } => return self.gcc_not(operand_expression),
            // C99: §6.5.15 paragraph 4, p. 90; PDF p. 102.
            | E::Conditional(c) => {
                let is = |e, n| {
                    self.expression_info(e)
                        .integer
                        .is_some_and(|v| v.value == n)
                };
                match self.truth_value(self.gcc_tree(c.condition_expression)).tree {
                    | Tree::Truth(condition)
                        if is(c.then_expression, 1) && is(c.else_expression, 0) =>
                        Tree::Selection(condition),
                    | _ => Tree::Conditional,
                }
            },
            | E::Cast {
                operand_expression, ..
            } => return self.gcc_cast(operand_expression, ty),
            | _ => Tree::Value,
        };
        Node {
            tree,
            ty,
            pointer: false,
        }
    }

    /// `!e`: `c_objc_common_truthvalue_conversion` of `e`, inverted by
    /// `fold_truth_not_expr`. A relational comparison of floating operands
    /// stays under a `TRUTH_NOT_EXPR`, since its inverse would not raise the
    /// same exceptions.
    /// C99: §6.5.3.3 paragraph 5, p. 79; PDF p. 91.
    fn gcc_not(&self, e: &'tu Expression<'tu>) -> Node<'tu> {
        let int = self.gcc_scalar(Scalar::Int);
        let truth = |comparison| Node {
            tree:    Tree::Truth(Truth {
                ty: int,
                comparison,
            }),
            ty:      int,
            pointer: false,
        };
        if let E::Binary {
            operator: B::LessThan | B::GreaterThan | B::LessThanOrEqual | B::GreaterThanOrEqual,
            left_expression,
            ..
        } = strip(e).kind
            && self
                .gcc_type(self.expression_info(left_expression).ty)
                .class
                == Class::Floating
        {
            return truth(false);
        }
        match self.truth_value(self.gcc_tree(e)).tree {
            | Tree::Truth(t) => truth(t.comparison),
            | Tree::Conditional => Node {
                tree:    Tree::Conditional,
                ty:      int,
                pointer: false,
            },
            | _ => truth(true),
        }
    }

    /// `(T) e`, as `build_c_cast` and `c_convert` build it. A cast to the
    /// operand's own type changes nothing; another drops a unary `+` of the
    /// same type, as `STRIP_TYPE_NOPS` does.
    fn gcc_cast(&self, operand: &'tu Expression<'tu>, to: GccType<'tu>) -> Node<'tu> {
        let node = self.gcc_tree(operand);
        if node.ty.kind == to.kind && !node.pointer {
            return node;
        }
        let mut stripped = strip(operand);
        while let E::Unary {
            operator: U::Plus,
            operand_expression,
        } = stripped.kind
            && self
                .gcc_type(self.expression_info(operand_expression).ty)
                .kind
                == node.ty.kind
        {
            stripped = strip(operand_expression);
        }
        let node = if core::ptr::eq(stripped, strip(operand)) {
            node
        } else {
            self.gcc_tree(stripped)
        };
        match to.class {
            | Class::Boolean => self.convert_to_boolean(node, to),
            | Class::Integer => self.convert_to_integer(node, to),
            | Class::Pointer => self.convert_to_pointer(node, to),
            | Class::Floating => Node {
                tree:    match node.tree {
                    | Tree::Truth(t) if t.comparison => Tree::Selection(t),
                    | Tree::Selection(t) => Tree::Selection(t),
                    | _ => Tree::Value,
                },
                ty:      to,
                pointer: false,
            },
            | Class::Other => Node {
                tree:    Tree::Value,
                ty:      to,
                pointer: false,
            },
        }
    }

    /// `convert_to_integer_1`: a comparison takes the new type; a widening
    /// conversion folds; a narrowing one drops an earlier widening, as
    /// `get_unwidened` does, and otherwise stays.
    /// C99: §6.3.1.3, p. 43; PDF p. 55.
    fn convert_to_integer(&self, node: Node<'tu>, to: GccType<'tu>) -> Node<'tu> {
        if node.ty.kind == to.kind && !node.pointer {
            return node;
        }
        if node.pointer {
            // `convert_to_integer_1` converts a pointer to `long` before `T`.
            let long = self.gcc_scalar(Scalar::Long);
            let view = Node {
                pointer: false,
                ..node
            };
            let long = match node.tree {
                | Tree::Selection(t) => select(t, long),
                | _ => fold_conversion(view, long),
            };
            return fold_conversion(long, to);
        }
        let tree = match node.tree {
            | Tree::Selection(t) => return select(t, to),
            | _ if node.ty.class == Class::Floating => Tree::Value,
            | Tree::Truth(t) if t.comparison => Tree::Truth(Truth { ty: to, ..t }),
            | Tree::Compound { right, pure } => {
                // `fold_binary_loc` drops a left operand without effects.
                let converted = self.convert_to_integer(self.gcc_tree(right), to);
                if pure && self.expression_info(right).integer.is_none() {
                    return converted;
                }
                Tree::Compound { right, pure }
            },
            | _ if to.bits >= node.ty.bits => return fold_conversion(node, to),
            | Tree::Converted(t) => return self.convert_to_integer(t.node(), to),
            | Tree::Truth(t) => Tree::Converted(t),
            | tree => tree,
        };
        Node {
            tree,
            ty: to,
            pointer: false,
        }
    }

    /// `convert_to_pointer_1` converts an integer of another width to `long`
    /// first; the pointer keeps that integer, which converts back without
    /// change. A comparison as wide as a pointer becomes `c ? 1 : 0`.
    /// C99: §6.3.2.3 paragraph 5, p. 47; PDF p. 59.
    fn convert_to_pointer(&self, node: Node<'tu>, to: GccType<'tu>) -> Node<'tu> {
        if node.pointer {
            return node;
        }
        if !matches!(node.ty.class, Class::Integer | Class::Boolean) {
            return Node {
                tree:    Tree::Value,
                ty:      to,
                pointer: true,
            };
        }
        let node = if node.ty.bits == to.bits {
            node
        } else {
            fold_conversion(node, self.gcc_scalar(Scalar::Long))
        };
        Node {
            tree: match node.tree {
                | Tree::Truth(t) if t.comparison => Tree::Selection(t),
                | tree => tree,
            },
            pointer: true,
            ..node
        }
    }

    /// `c_objc_common_truthvalue_conversion` to `_Bool`: GCC compares a
    /// value with zero, and converts a comparison or logical operation, which
    /// already has that meaning.
    /// C99: §6.3.1.2 paragraph 1, p. 43; PDF p. 55.
    fn convert_to_boolean(&self, node: Node<'tu>, to: GccType<'tu>) -> Node<'tu> {
        fold_conversion(self.truth_value(node), to)
    }

    /// `c_common_truthvalue_conversion`, which gives a truth value the type
    /// `int`.
    fn truth_value(&self, node: Node<'tu>) -> Node<'tu> {
        let int = self.gcc_scalar(Scalar::Int);
        let tree = match node.tree {
            | Tree::Truth(t) => Tree::Truth(Truth { ty: int, ..t }),
            | Tree::Converted(t) if node.ty.bits >= t.ty.bits => return self.truth_value(t.node()),
            | tree @ (Tree::Selection(_) | Tree::Conditional) => tree,
            | _ => Tree::Truth(Truth {
                ty:         int,
                comparison: true,
            }),
        };
        Node {
            tree,
            ty: int,
            pointer: false,
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
            | TypeKind::Scalar(s) if !s.complex() && s.floating_rank(3) != 0 =>
                (Class::Floating, 0, false),
            | _ => match self.integer_type(ty) {
                | Some((bits, signed)) => (Class::Integer, bits, !signed),
                | None => (Class::Other, 0, false),
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

/// An operand's tree with its type. A pointer made by a cast keeps the
/// integer it was converted from as its type, since GCC converts it back
/// through an integer of that width.
#[derive(Clone, Copy)]
struct Node<'tu> {
    tree:    Tree<'tu>,
    ty:      GccType<'tu>,
    pointer: bool,
}

#[derive(Clone, Copy)]
enum Tree<'tu> {
    /// A comparison or a logical operation: what GCC takes for a truth value.
    Truth(Truth<'tu>),
    /// A truth value under a conversion to the node's integer type.
    Converted(Truth<'tu>),
    /// `c ? 1 : 0` with a truth value `c`, which folds to `c` once converted
    /// to `c`'s type.
    Selection(Truth<'tu>),
    /// A comma expression, and whether its left operand has no side effects.
    Compound {
        right: &'tu Expression<'tu>,
        pure:  bool,
    },
    /// Any other conditional expression.
    Conditional,
    /// Anything else.
    Value,
}

#[derive(Clone, Copy)]
struct Truth<'tu> {
    ty:         GccType<'tu>,
    /// A comparison, which a conversion to an integer type retypes, rather
    /// than `&&`, `||` or `!`.
    comparison: bool,
}

impl<'tu> Truth<'tu> {
    const fn node(self) -> Node<'tu> {
        Node {
            tree:    Tree::Truth(self),
            ty:      self.ty,
            pointer: false,
        }
    }
}

/// What a conversion asks of a type: its identity, as GCC's
/// `TYPE_MAIN_VARIANT` does, and its class, width and signedness.
#[derive(Clone, Copy)]
struct GccType<'tu> {
    kind:     TypeKind<'tu>,
    class:    Class,
    bits:     u32,
    unsigned: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Boolean,
    Integer,
    Pointer,
    Floating,
    Other,
}

/// `fold_build1 (NOP_EXPR, to, node)` between integer types: a
/// conversion of a conversion folds to one when `match.pd` drops the
/// middle type, and `c ? 1 : 0` of type `c`'s folds to `c`.
fn fold_conversion<'tu>(node: Node<'tu>, to: GccType<'tu>) -> Node<'tu> {
    if node.ty.kind == to.kind {
        return node;
    }
    let tree = match node.tree {
        | Tree::Truth(t) if t.comparison && to.class == Class::Boolean =>
            Tree::Truth(Truth { ty: to, ..t }),
        | Tree::Truth(t) => Tree::Converted(t),
        | Tree::Converted(t) if drops_middle(t.ty, node.ty, to) =>
            return fold_conversion(t.node(), to),
        | Tree::Converted(_) => Tree::Value,
        | Tree::Selection(t) => return select(t, to),
        | tree => tree,
    };
    Node {
        tree,
        ty: to,
        pointer: false,
    }
}

/// `c ? 1 : 0` converted to `to`: `fold_ternary_loc` returns `c` when `to` is
/// `c`'s type.
fn select<'tu>(t: Truth<'tu>, to: GccType<'tu>) -> Node<'tu> {
    Node {
        tree:    if t.ty.kind == to.kind {
            Tree::Truth(t)
        } else {
            Tree::Selection(t)
        },
        ty:      to,
        pointer: false,
    }
}

/// Whether `match.pd` folds `(final)(inter) x`, with `x` of type `inside`, to
/// `(final) x`, for integer types.
fn drops_middle(inside: GccType<'_>, inter: GccType<'_>, last: GccType<'_>) -> bool {
    let (i, m, f) = (inside.bits, inter.bits, last.bits);
    let (iu, mu, fu) = (inside.unsigned, inter.unsigned, last.unsigned);
    // Converting something to its own type via an object at least as wide.
    (last.kind == inside.kind && m >= f)
        // The middle type widens without changing the signedness.
        || (m >= i && mu == iu)
        // A sign extension of a zero extension, a final conversion that keeps
        // the middle width, or a truncation of a sign extension.
        || (i < m && m < f && iu && !mu)
        || f == m
        || (i < m && m > f && !iu && mu)
        // Two conversions in a row that are not needed, beyond the middle
        // widening without a change of signedness above.
        || (m >= f && ((mu && m > i) == (fu && f > m)))
        // A truncation of a widening.
        || (f <= i && m >= i)
}

/// `e` without parentheses or `__extension__`, which build no tree.
fn strip<'tu>(mut e: &'tu Expression<'tu>) -> &'tu Expression<'tu> {
    while let E::Parenthesized { expression: inner }
    | E::Unary {
        operator: U::Extension,
        operand_expression: inner,
    } = e.kind
    {
        e = inner;
    }
    e
}
