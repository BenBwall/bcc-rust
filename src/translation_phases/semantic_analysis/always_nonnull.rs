//! Clang's warnings about a pointer that cannot be null, compared with a
//! null pointer (`-Wtautological-pointer-compare`) or converted to `_Bool`
//! (`-Wpointer-bool-conversion`): `Sema::DiagnoseAlwaysNonNullPointer` in
//! `SemaChecking.cpp`, which both `CheckCompareOperands` and
//! `CheckImplicitConversion` call. A §6.3.2.3p3 null pointer compares
//! unequal to the address of every object and function, so the result is
//! known; a `nonnull` parameter or a `returns_nonnull` call is only assumed
//! not to be null, "on first encounter": Clang stops assuming it of a
//! parameter once an assignment writes it or `&` takes its address.
//!
//! For `void f(int *p) __attribute__((nonnull)); void f(int *p) { if (p ==
//! 0) return; }`, the comparison names the parameter `p` of the function
//! being defined, which the attribute of an earlier declaration makes
//! nonnull, so Clang warns at `p`, underlines `0`, and notes the attribute.
//! C99: §6.3.2.3p3, p. 47; PDF p. 59; §6.5.9p6, p. 87; PDF p. 99;
//! §6.3.1.2p1, p. 43; PDF p. 55; §5.1.1.3p1, p. 11; PDF p. 23.
//! GNU extension: GCC manual, "Common Function Attributes", `nonnull` and
//! `returns_nonnull`.

use super::{
    Analyzer,
    BindingKind,
    ExpressionInfo,
    SemanticErrorKind,
    expressions::ValueCategory,
    types::{
        Scalar,
        TypeKind,
    },
};
use crate::{
    configuration::{
        DiagnosticGroup,
        RuleSet,
    },
    diagnostics::format_in,
    translation_phases::{
        Context,
        SourceVectors,
        parsing::syntax::{
            BinaryOperator as B,
            Constant,
            Expression,
            ExpressionType as E,
            UnaryOperator as U,
        },
    },
    util::bump::{
        ArenaString,
        Bump,
    },
};

/// The note a [`SemanticErrorKind::Nonnull`] diagnostic gives at its
/// `previous` place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NonnullNote {
    /// None: the diagnostic has no `previous` place.
    Plain,
    /// Clang's `declared 'nonnull' here` at the attribute that makes a
    /// parameter nonnull, or `declared 'returns_nonnull' here`.
    Declared { returns: bool },
    /// Clang's notes at a function's name: that `&` silences the warning,
    /// and, when `call`, that `()` would call the function.
    FunctionName { call: bool },
    /// GCC's note at the latest declaration of the function a null
    /// argument is passed to, which it names.
    GccCall,
}

impl NonnullNote {
    /// The words of the note.
    /// `previous` is the place of the note, whose name GCC's note repeats.
    pub(crate) fn words<'d>(
        self,
        context: &Context<'_>,
        arena: &'d Bump,
        previous: Option<SourceVectors>,
    ) -> &'d str {
        match self {
            | Self::Plain => "",
            | Self::Declared { returns: false } => "declared 'nonnull' here",
            | Self::Declared { returns: true } => "declared 'returns_nonnull' here",
            | Self::FunctionName { .. } =>
                "prefix with the address-of operator to silence this warning",
            | Self::GccCall => format_in!(
                arena,
                "in a call to function '{}' declared 'nonnull'",
                previous
                    .and_then(|previous| context.source_spelling(previous))
                    .unwrap_or_default()
            ),
        }
    }

    /// The words of a second note at the same place, if there is one.
    pub(crate) const fn second_words(self) -> Option<&'static str> {
        match self {
            | Self::FunctionName { call: true } =>
                Some("suffix with parentheses to turn this into a function call"),
            | _ => None,
        }
    }
}

/// A comparison of a pointer with a null pointer constant, as
/// [`Analyzer::always_nonnull`] needs it.
#[derive(Clone, Copy)]
struct NullComparison<'tu> {
    /// `==` rather than `!=`.
    equal: bool,
    /// The null pointer constant.
    null:  ExpressionInfo<'tu>,
}

/// What Clang finds `e` is, when it can never be null.
enum Nonnull {
    /// A `nonnull` parameter or a `returns_nonnull` call.
    Assumed { call: bool },
    /// The address of an object, written with `&`.
    Address,
    /// A function designator, with whether calling it with no arguments
    /// would give a value the comparison or conversion could have meant.
    Function { call: bool },
    /// An array.
    Array,
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Clang's check of an equality comparison `==` (`equal`) or `!=` of a
    /// pointer with a null pointer constant, which wants one operand null
    /// and the other not (`CheckCompareOperands` in `SemaExpr.cpp`).
    /// C99: §6.5.9p2, p. 86; PDF p. 98.
    pub(super) fn null_comparison(
        &mut self,
        equal: bool,
        left: ExpressionInfo<'tu>,
        right: ExpressionInfo<'tu>,
    ) {
        if self.context.configuration.compiler_identity().rules() != RuleSet::Clang {
            return;
        }
        let (pointer, null) = match (
            self.null_pointer_constant(left),
            self.null_pointer_constant(right),
        ) {
            | (false, true) => (left, right),
            | (true, false) => (right, left),
            | _ => return,
        };
        self.always_nonnull(
            pointer.expression,
            Some(NullComparison { equal, null }),
            null.expression.source_vectors,
        );
    }

    /// Clang's check of the pointer `e` converted to `_Bool`, at the token
    /// `site` the conversion takes its location from. Clang looks through
    /// the parentheses around a converted operand (`CheckBoolLikeConversion`
    /// and `AnalyzeImplicitConversions` take `IgnoreParenImpCasts`), though
    /// not around a compared one, so `(p) ? 1 : 0` warns at `p`.
    /// C99: §6.3.1.2p1, p. 43; PDF p. 55; §6.5.1p5, p. 69; PDF p. 81.
    pub(super) fn bool_conversion_nonnull(&mut self, e: &'tu Expression<'tu>, site: SourceVectors) {
        self.always_nonnull(Self::without_parentheses(e), None, site);
    }

    /// Clang's `DiagnoseAlwaysNonNullPointer` for the pointer operand `e` of
    /// a comparison with a null pointer constant, or of a conversion to
    /// `_Bool` when `comparison` is `None`. `range` is what Clang underlines
    /// beside the operand: the null pointer constant, or the token the
    /// conversion takes its location from. Inside a macro's body Clang says
    /// nothing, nor when `e` is a macro's argument and the macro's body
    /// writes `range`.
    /// C99: §6.3.2.3p3, p. 47; PDF p. 59; §6.5.9p6, p. 87; PDF p. 99.
    fn always_nonnull(
        &mut self,
        e: &'tu Expression<'tu>,
        comparison: Option<NullComparison<'tu>>,
        range: SourceVectors,
    ) {
        // Only a pointer converts so (`CheckImplicitConversion`).
        if comparison.is_none() {
            let ty = self.expression_info(e).ty;
            if self.pointer_target(ty).is_none()
                && !matches!(
                    self.types.nodes[ty.index],
                    TypeKind::Array(..) | TypeKind::Function { .. }
                )
            {
                return;
            }
        }
        let located = Self::clang_location(e);
        if self.context.produced_by_macro_expansion(located)
            && (self.context.written_by_macro_body(located)
                || self
                    .context
                    .written_by_macro_body(Context::first_token_vector(range)))
        {
            return;
        }
        // `&x` names `x`; any other unary operator hides what it applies to.
        let (address, e) = match Self::without_parentheses(e).kind {
            | E::Unary {
                operator: U::AddressOf,
                operand_expression,
            } => (true, operand_expression),
            | E::Unary { .. } => return,
            | _ => (false, e),
        };
        let Some((what, site)) = self.nonnull_kind(e, address, comparison) else {
            return;
        };
        let printed = self.clang_printed(e);
        let comparing = comparison.map(|comparison| comparison.equal);
        let text = match (&what, comparing) {
            | (Nonnull::Assumed { call, .. }, Some(equal)) => format_in!(
                self.scratch,
                "comparison of nonnull {} '{printed}' {}equal to a null pointer is '{}' on first \
                 encounter",
                if *call { "function call" } else { "parameter" },
                if equal { "" } else { "not " },
                if equal { "false" } else { "true" },
            ),
            | (Nonnull::Assumed { call, .. }, None) => format_in!(
                self.scratch,
                "nonnull {} '{printed}' will evaluate to 'true' on first encounter",
                if *call { "function call" } else { "parameter" },
            ),
            | (_, Some(equal)) => format_in!(
                self.scratch,
                "comparison of {} '{printed}' {}equal to a null pointer is always {}",
                match what {
                    | Nonnull::Function { .. } => "function",
                    | Nonnull::Array => "array",
                    | _ => "address of",
                },
                if equal { "" } else { "not " },
                if equal { "false" } else { "true" },
            ),
            | (_, None) => format_in!(
                self.scratch,
                "address of {}'{printed}' will always evaluate to 'true'",
                match what {
                    | Nonnull::Function { .. } => "function ",
                    | Nonnull::Array => "array ",
                    | _ => "",
                },
            ),
        };
        let located = Self::clang_location(e);
        let (note, previous) = match what {
            | Nonnull::Assumed { call } => (NonnullNote::Declared { returns: call }, site),
            | Nonnull::Function { call } => (NonnullNote::FunctionName { call }, Some(located)),
            | Nonnull::Address | Nonnull::Array => (NonnullNote::Plain, None),
        };
        let group = if comparison.is_some() {
            DiagnosticGroup::TautologicalPointerCompare
        } else {
            DiagnosticGroup::PointerBoolConversion
        };
        let name = self.context.string_cache.intern(text);
        // A conversion's location is one token.
        let range = if comparison.is_some() {
            range
        } else {
            Context::first_token_vector(range)
        };
        self.with_highlights([e.source_vectors, range], |analyzer| {
            analyzer.group_diagnostic(
                group,
                SemanticErrorKind::Nonnull(group, note),
                located,
                Some(name),
                previous,
            );
        });
    }

    /// What Clang finds `e` is, the operand of `&` when `address`, if it can
    /// never be null: a call to a `returns_nonnull` function, through
    /// parentheses, whose callee [`Self::called`] finds; or a name or member
    /// as written, other than a weak entity, that is a `nonnull` parameter
    /// Clang still assumes nonnull ([`Self::nonnull_parameter`]), or an
    /// array, a function or the operand of `&`. `&` before a function's name
    /// is how to silence the warning.
    fn nonnull_kind(
        &mut self,
        e: &'tu Expression<'tu>,
        address: bool,
        comparison: Option<NullComparison<'tu>>,
    ) -> Option<(Nonnull, Option<SourceVectors>)> {
        if let E::Call {
            function_expression,
            ..
        } = Self::without_parentheses(e).kind
            && let Some(callee) = self
                .expression_info(Self::called(function_expression))
                .binding
            && self.bindings[callee].kind == BindingKind::Function
            && let Some(site) = self.returns_nonnull_site(callee)
        {
            return Some((Nonnull::Assumed { call: true }, Some(site)));
        }
        let info = self.expression_info(e);
        match e.kind {
            | E::Identifier(name) => {
                let binding = info.binding?;
                // `__func__` is no declaration Clang refers to.
                if self.declared_weak(binding)
                    || matches!(
                        self.context.string_cache.at(name.name),
                        "__func__" | "__FUNCTION__" | "__PRETTY_FUNCTION__"
                    )
                {
                    return None;
                }
                if self.bindings[binding].kind == BindingKind::Parameter
                    && let Some(site) = self.nonnull_parameter(binding)
                {
                    return Some((Nonnull::Assumed { call: false }, Some(site)));
                }
            },
            | E::DirectMember { .. } | E::IndirectMember { .. } => {},
            | _ => return None,
        }
        let function = info.category == ValueCategory::FunctionDesignator;
        let array = matches!(self.types.nodes[info.ty.index], TypeKind::Array(..));
        let what = if address {
            (!function).then_some(Nonnull::Address)?
        } else if array {
            Nonnull::Array
        } else if function {
            Nonnull::Function {
                call: self.call_suggested(info.ty, comparison),
            }
        } else {
            return None;
        };
        Some((what, None))
    }

    /// The name a call's function operand `e` designates through
    /// parentheses, `*` and `&`, as Clang finds its direct callee
    /// (`Expr::getReferencedDeclOfCallee`): `(*f)()` and `(&f)()` call `f`,
    /// though a cast hides it.
    /// C99: §6.5.3.2p4, p. 79; PDF p. 91; §6.3.2.1p4, p. 46; PDF p. 58.
    fn called(mut e: &'tu Expression<'tu>) -> &'tu Expression<'tu> {
        loop {
            match e.kind {
                | E::Parenthesized {
                    expression: operand,
                }
                | E::Unary {
                    operator: U::Indirection | U::AddressOf,
                    operand_expression: operand,
                } => e = operand,
                | _ => return e,
            }
        }
    }

    /// Whether Clang suggests calling the function of type `ty` instead,
    /// which it does when a call with no arguments would give a value of
    /// the kind compared or converted: a pointer for a comparison with a
    /// null pointer, or an integer for one with zero; a `_Bool` for a
    /// conversion to it.
    fn call_suggested(&self, ty: super::TypeId, comparison: Option<NullComparison<'tu>>) -> bool {
        let TypeKind::Function {
            result,
            parameters,
            variadic,
            ..
        } = self.types.nodes[ty.index]
        else {
            return false;
        };
        if !parameters.is_empty() || variadic {
            return false;
        }
        let result = self.types.non_atomic(result);
        match comparison {
            | Some(comparison) =>
                self.pointer_target(result).is_some()
                    || (!self.nullptr_type(comparison.null.ty)
                        && self.integer_type(result).is_some()),
            | None => matches!(
                self.types.nodes[result.index],
                TypeKind::Scalar(Scalar::Bool)
            ),
        }
    }

    /// Where Clang locates the expression `e` (`Expr::getExprLoc`): at the
    /// name of a member, at the operator of a binary or postfix one, and
    /// otherwise at its first token.
    pub(super) fn clang_location(e: &'tu Expression<'tu>) -> SourceVectors {
        match e.kind {
            | E::DirectMember { member, .. } | E::IndirectMember { member, .. } =>
                member.source_vectors,
            | E::Binary {
                operator: B::Subscript,
                ..
            }
            | E::Call { .. } => Context::first_token_vector(e.source_vectors),
            | E::Binary { .. }
            | E::Unary {
                operator: U::PostIncrement | U::PostDecrement,
                ..
            } => Context::first_token_vector(e.operator_source_vectors.unwrap_or(e.source_vectors)),
            | _ => Context::first_token_vector(e.source_vectors),
        }
    }

    /// `e` as Clang prints it in a diagnostic (`Stmt::printPretty`): names,
    /// members, calls, subscripts, operators, `sizeof` of an expression and
    /// integer and character constants as its `StmtPrinter` writes them,
    /// anything else, such as a type name, as written, without spaces.
    pub(super) fn clang_printed(&mut self, e: &'tu Expression<'tu>) -> &'tu str {
        let mut out = ArenaString::new_in(self.scratch);
        self.print_expression(&mut out, e);
        self.types.tu.alloc_str(&out)
    }

    fn print_expression(&mut self, out: &mut ArenaString<'_>, e: &'tu Expression<'tu>) {
        match e.kind {
            | E::Identifier(name) => out.push_str(self.context.string_cache.at(name.name)),
            | E::Parenthesized { expression } => {
                out.push('(');
                self.print_expression(out, expression);
                out.push(')');
            },
            | E::DirectMember {
                base_expression,
                member,
            }
            | E::IndirectMember {
                base_expression,
                member,
            } => {
                self.print_expression(out, base_expression);
                out.push_str(if matches!(e.kind, E::DirectMember { .. }) {
                    "."
                } else {
                    "->"
                });
                out.push_str(self.context.string_cache.at(member.name));
            },
            | E::Call {
                function_expression,
                arguments,
            } => {
                self.print_expression(out, function_expression);
                out.push('(');
                for (index, &argument) in arguments.iter().enumerate() {
                    if index != 0 {
                        out.push_str(", ");
                    }
                    self.print_expression(out, argument);
                }
                out.push(')');
            },
            | E::Binary {
                operator: B::Subscript,
                left_expression,
                right_expression,
            } => {
                self.print_expression(out, left_expression);
                out.push('[');
                self.print_expression(out, right_expression);
                out.push(']');
            },
            | E::Binary {
                operator,
                left_expression,
                right_expression,
            } => {
                self.print_expression(out, left_expression);
                out.push_str(if operator == B::Comma { ", " } else { " " });
                if operator != B::Comma {
                    out.push_str(binary_spelling(operator));
                    out.push(' ');
                }
                self.print_expression(out, right_expression);
            },
            | E::Unary {
                operator: operator @ (U::PostIncrement | U::PostDecrement),
                operand_expression,
            } => {
                self.print_expression(out, operand_expression);
                out.push_str(if operator == U::PostIncrement {
                    "++"
                } else {
                    "--"
                });
            },
            | E::Unary {
                operator,
                operand_expression,
            } if unary_spelling(operator).is_some() => {
                out.push_str(unary_spelling(operator).unwrap_or_default());
                self.print_expression(out, operand_expression);
            },
            | E::Conditional(conditional) => {
                self.print_expression(out, conditional.condition_expression);
                out.push_str(" ? ");
                self.print_expression(out, conditional.then_expression);
                out.push_str(" : ");
                self.print_expression(out, conditional.else_expression);
            },
            | E::SizeofExpr(operand) => {
                out.push_str("sizeof ");
                self.print_expression(out, operand);
            },
            | E::Constant(Constant::Integer(_)) if self.print_integer(out, e) => {},
            | E::Constant(Constant::Char(_)) if self.print_character(out, e) => {},
            | _ => self.print_as_written(out, e),
        }
    }

    /// An integer constant as Clang's `IntegerLiteral` prints: its value in
    /// decimal, with the suffix of its type.
    fn print_integer(&mut self, out: &mut ArenaString<'_>, e: &'tu Expression<'tu>) -> bool {
        let info = self.expression_info(e);
        let Some(value) = info.integer else {
            return false;
        };
        let TypeKind::Scalar(scalar) = self.types.nodes[info.ty.index] else {
            return false;
        };
        let suffix = match scalar {
            | Scalar::Int => "",
            | Scalar::UnsignedInt => "U",
            | Scalar::Long => "L",
            | Scalar::UnsignedLong => "UL",
            | Scalar::LongLong => "LL",
            | Scalar::UnsignedLongLong => "ULL",
            | _ => return false,
        };
        out.push_str(format_in!(self.scratch, "{}{suffix}", value.value));
        true
    }

    /// A plain character constant as Clang's `CharacterLiteral` prints: the
    /// character, or its escape sequence.
    fn print_character(&mut self, out: &mut ArenaString<'_>, e: &'tu Expression<'tu>) -> bool {
        let written = self
            .context
            .source_spelling(Context::first_token_vector(e.source_vectors));
        if !written.is_some_and(|written| written.starts_with('\'')) {
            return false;
        }
        let Some(value) = self.expression_info(e).integer else {
            return false;
        };
        let byte = (value.value & 0xFF) as u8;
        let escaped = match byte {
            | b'\\' => "'\\\\'",
            | b'\'' => "'\\''",
            | 0x07 => "'\\a'",
            | 0x08 => "'\\b'",
            | 0x0C => "'\\f'",
            | b'\n' => "'\\n'",
            | b'\r' => "'\\r'",
            | b'\t' => "'\\t'",
            | 0x0B => "'\\v'",
            | 0x20..=0x7E => format_in!(self.scratch, "'{}'", char::from(byte)),
            | _ => format_in!(self.scratch, "'\\x{byte:02x}'"),
        };
        out.push_str(escaped);
        true
    }

    /// `e`'s tokens as written, without the spaces between them.
    fn print_as_written(&self, out: &mut ArenaString<'_>, e: &'tu Expression<'tu>) {
        for vector in self.context.get_source_vectors(e.source_vectors) {
            if let Some(piece) = self
                .context
                .source_text(vector.source_file_index)
                .and_then(|text| text.get(vector.range()))
            {
                out.push_str(piece);
            }
        }
    }
}

/// A binary operator as written.
const fn binary_spelling(operator: B) -> &'static str {
    match operator {
        | B::Multiplication => "*",
        | B::Division => "/",
        | B::Modulo => "%",
        | B::Addition => "+",
        | B::Subtraction => "-",
        | B::LeftShift => "<<",
        | B::RightShift => ">>",
        | B::LessThan => "<",
        | B::GreaterThan => ">",
        | B::LessThanOrEqual => "<=",
        | B::GreaterThanOrEqual => ">=",
        | B::Equal => "==",
        | B::NotEqual => "!=",
        | B::BitwiseAnd => "&",
        | B::BitwiseXor => "^",
        | B::BitwiseOr => "|",
        | B::LogicalAnd => "&&",
        | B::LogicalOr => "||",
        | B::Comma => ",",
        | B::Subscript => "[]",
        | B::Assignment => "=",
        | B::MultiplicationAssignment => "*=",
        | B::DivisionAssignment => "/=",
        | B::ModuloAssignment => "%=",
        | B::AdditionAssignment => "+=",
        | B::SubtractionAssignment => "-=",
        | B::LeftShiftAssignment => "<<=",
        | B::RightShiftAssignment => ">>=",
        | B::BitwiseAndAssignment => "&=",
        | B::BitwiseXorAssignment => "^=",
        | B::BitwiseOrAssignment => "|=",
    }
}

/// A prefix unary operator as Clang prints it, when it prints it.
const fn unary_spelling(operator: U) -> Option<&'static str> {
    Some(match operator {
        | U::AddressOf => "&",
        | U::Indirection => "*",
        | U::Plus => "+",
        | U::Minus => "-",
        | U::BitwiseNot => "~",
        | U::LogicalNot => "!",
        | U::PreIncrement => "++",
        | U::PreDecrement => "--",
        | U::Real => "__real ",
        | U::Imag => "__imag ",
        | U::Extension => "__extension__ ",
        | U::PostIncrement | U::PostDecrement => return None,
    })
}
