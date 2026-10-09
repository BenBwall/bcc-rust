//! Syntax-tree inspection labels for translation phase 7.
//!
//! Character constant types follow C99: §6.4.4.4 paragraphs 10-11, p. 61;
//! PDF p. 73. The inspection output format is bcc-rust's own.

use std::fmt::{
    self,
    Display,
};

use super::super::{
    ParsedTranslationUnit,
    declaration_syntax::{
        StructOrUnion,
        TypeQualifiers,
        TypeSpecifiers,
    },
    syntax::{
        BinaryOperator,
        Constant,
        ExpressionType,
        StatementType,
        UnaryOperator,
    },
};
use crate::{
    diagnostics::write_c_quoted,
    translation_phases::{
        Context,
        preprocessing::{
            CharacterTokenType,
            IntegerTokenType,
            StringTokenType,
        },
    },
    util::bump::Bump,
};

impl<'tu> ParsedTranslationUnit<'tu> {
    pub(super) fn type_label<'a>(
        specifiers: TypeSpecifiers<'a>,
        context: &'a Context<'_>,
    ) -> impl Display {
        fmt::from_fn(move |f| match specifiers {
            | TypeSpecifiers::TypedefName(identifier) => {
                write!(f, "typedef {}", context.string_cache.at(identifier.name))
            },
            | TypeSpecifiers::StructOrUnion(specifier) => {
                let kind = match specifier.struct_or_union {
                    | StructOrUnion::Struct => "struct",
                    | StructOrUnion::Union => "union",
                };
                match specifier.identifier {
                    | None => write!(f, "{kind} <anonymous>"),
                    | Some(identifier) =>
                        write!(f, "{kind} {}", context.string_cache.at(identifier.name)),
                }
            },
            | TypeSpecifiers::Enum(specifier) => match specifier.name {
                | None => f.write_str("enum <anonymous>"),
                | Some(identifier) =>
                    write!(f, "enum {}", context.string_cache.at(identifier.name)),
            },
            | _ => write!(f, "{specifiers}"),
        })
    }

    pub(super) fn statement_label<'a>(
        kind: &'a StatementType<'tu>,
        context: &'a Context<'_>,
    ) -> impl Display {
        fmt::from_fn(move |f| {
            f.write_str(match kind {
                | StatementType::Label(identifier, _) => {
                    return write!(f, "label {}", context.string_cache.at(identifier.name));
                },
                | StatementType::Goto(identifier) => {
                    return write!(f, "goto {}", context.string_cache.at(identifier.name));
                },
                | StatementType::Case(..) => "case",
                | StatementType::Default(..) => "default",
                | StatementType::Compound { .. } => "compound",
                | StatementType::Expression(..) => "expression",
                | StatementType::If { .. } => "if",
                | StatementType::Switch { .. } => "switch",
                | StatementType::While { .. } => "while",
                | StatementType::DoWhile { .. } => "do-while",
                | StatementType::For(_) => "for",
                | StatementType::Continue => "continue",
                | StatementType::Break => "break",
                | StatementType::Return(..) => "return",
                | StatementType::Attributed(_) => "attributed",
                | StatementType::Declaration(_) => "declaration",
                | StatementType::CaseRange(_) => "case-range",
                | StatementType::NamedBreak(x) =>
                    return write!(f, "break {}", context.string_cache.at(x.name)),
                | StatementType::NamedContinue(x) =>
                    return write!(f, "continue {}", context.string_cache.at(x.name)),
                | StatementType::MsAsm(_) => "ms-asm",
                | StatementType::Seh(_) => "seh",
                | StatementType::SehLeave => "__leave",
                | StatementType::Asm(_) => "asm",
                | StatementType::ComputedGoto(_) => "computed-goto",
                | StatementType::LocalLabels(_) => "local-labels",
                | StatementType::Null => "null",
            })
        })
    }

    /// Spells a string literal in `scratch`.
    pub(super) fn expression_label<'a>(
        kind: &'a ExpressionType<'tu>,
        context: &'a Context<'_>,
        scratch: &'a Bump,
    ) -> impl Display {
        fmt::from_fn(move |f| {
            f.write_str(match kind {
                | ExpressionType::Binary { operator, .. } => {
                    return write!(f, "binary {}", binary_operator_spelling(*operator));
                },
                | ExpressionType::Unary { operator, .. } => {
                    return write!(f, "unary {}", unary_operator_spelling(*operator));
                },
                | ExpressionType::DirectMember { member, .. } => {
                    return write!(f, "member .{}", context.string_cache.at(member.name));
                },
                | ExpressionType::IndirectMember { member, .. } => {
                    return write!(f, "member ->{}", context.string_cache.at(member.name));
                },
                | ExpressionType::Identifier(identifier) => {
                    return write!(f, "identifier {}", context.string_cache.at(identifier.name));
                },
                | ExpressionType::Constant(constant) => {
                    return write!(f, "constant {}", constant_label(constant));
                },
                | ExpressionType::StringLiteral(string) => {
                    return match string {
                        | StringTokenType::EncodedString(contents, encoding) => write!(
                            f,
                            "{}-string {}",
                            encoding.type_name(),
                            context.literal_spelling_in(
                                scratch,
                                scratch,
                                *contents,
                                encoding.prefix()
                            )
                        ),
                        | StringTokenType::String(contents) => write!(
                            f,
                            "string {}",
                            context.literal_spelling_in(scratch, scratch, *contents, "")
                        ),
                        | StringTokenType::WideString(contents) => write!(
                            f,
                            "wide-string {}",
                            context.literal_spelling_in(scratch, scratch, *contents, "L")
                        ),
                    };
                },
                | ExpressionType::Parenthesized { .. } => "parenthesized",
                | ExpressionType::Conditional(_) => "conditional ?:",
                | ExpressionType::Call { .. } => "call",
                | ExpressionType::CompoundLiteral { .. } => "compound-literal",
                | ExpressionType::SizeofType(..) => "sizeof type",
                | ExpressionType::SizeofExpr(..) => "sizeof expression",
                | ExpressionType::Cast { .. } => "cast",
                | ExpressionType::AlignofType(_) => "alignof type",
                | ExpressionType::AlignofExpr(_) => "alignof expression",
                | ExpressionType::Countof(_) => "countof",
                | ExpressionType::Generic(_) => "generic-selection",
                | ExpressionType::Boolean(true) => "true",
                | ExpressionType::Boolean(false) => "false",
                | ExpressionType::Nullptr => "nullptr",
                | ExpressionType::StatementExpression(_) => "statement-expression",
                | ExpressionType::Builtin(x) =>
                    return write!(f, "builtin {}", x.keyword.spelling()),
                | ExpressionType::LabelAddress(x) =>
                    return write!(f, "label-address {}", context.string_cache.at(x.name)),
                | ExpressionType::OmittedConditional(_) => "conditional ?: (omitted middle)",
                | ExpressionType::Error => "error-expression",
            })
        })
    }
}

pub(super) fn function_specifier_list(
    specifiers: super::super::declaration_syntax::FunctionSpecifiers,
) -> &'static str {
    match (specifiers.is_inline, specifiers.is_noreturn) {
        | (false, false) => "none",
        | (true, false) => "inline",
        | (false, true) => "_Noreturn",
        | (true, true) => "inline _Noreturn",
    }
}

/// Lists qualifiers in C spelling, such as `const volatile`, or `none`.
pub(super) fn qualifier_list(qualifiers: TypeQualifiers) -> impl Display {
    fmt::from_fn(move |f| {
        let mut names = [
            (TypeQualifiers::CONST, "const"),
            (TypeQualifiers::VOLATILE, "volatile"),
            (TypeQualifiers::RESTRICT, "restrict"),
            (TypeQualifiers::ATOMIC, "_Atomic"),
            (TypeQualifiers::PTR32, "__ptr32"),
            (TypeQualifiers::PTR64, "__ptr64"),
            (TypeQualifiers::UNALIGNED, "__unaligned"),
            (TypeQualifiers::W64, "__w64"),
            (TypeQualifiers::SPTR, "__sptr"),
            (TypeQualifiers::UPTR, "__uptr"),
        ]
        .into_iter()
        .filter(|&(flag, _)| qualifiers.contains(flag))
        .map(|(_, name)| name);
        let Some(first) = names.next() else {
            return f.write_str("none");
        };
        f.write_str(first)?;
        for name in names {
            write!(f, " {name}")?;
        }
        Ok(())
    })
}

fn binary_operator_spelling(operator: BinaryOperator) -> &'static str {
    match operator {
        | BinaryOperator::Multiplication => "*",
        | BinaryOperator::Division => "/",
        | BinaryOperator::Modulo => "%",
        | BinaryOperator::Addition => "+",
        | BinaryOperator::Subtraction => "-",
        | BinaryOperator::LeftShift => "<<",
        | BinaryOperator::RightShift => ">>",
        | BinaryOperator::LessThan => "<",
        | BinaryOperator::GreaterThan => ">",
        | BinaryOperator::LessThanOrEqual => "<=",
        | BinaryOperator::GreaterThanOrEqual => ">=",
        | BinaryOperator::Equal => "==",
        | BinaryOperator::NotEqual => "!=",
        | BinaryOperator::BitwiseAnd => "&",
        | BinaryOperator::BitwiseXor => "^",
        | BinaryOperator::BitwiseOr => "|",
        | BinaryOperator::LogicalAnd => "&&",
        | BinaryOperator::LogicalOr => "||",
        | BinaryOperator::Comma => ",",
        | BinaryOperator::Subscript => "[]",
        | BinaryOperator::Assignment => "=",
        | BinaryOperator::MultiplicationAssignment => "*=",
        | BinaryOperator::DivisionAssignment => "/=",
        | BinaryOperator::ModuloAssignment => "%=",
        | BinaryOperator::AdditionAssignment => "+=",
        | BinaryOperator::SubtractionAssignment => "-=",
        | BinaryOperator::LeftShiftAssignment => "<<=",
        | BinaryOperator::RightShiftAssignment => ">>=",
        | BinaryOperator::BitwiseAndAssignment => "&=",
        | BinaryOperator::BitwiseXorAssignment => "^=",
        | BinaryOperator::BitwiseOrAssignment => "|=",
    }
}

fn unary_operator_spelling(operator: UnaryOperator) -> &'static str {
    match operator {
        | UnaryOperator::Real => "__real__",
        | UnaryOperator::Imag => "__imag__",
        | UnaryOperator::Extension => "__extension__",
        | UnaryOperator::AddressOf => "&",
        | UnaryOperator::Indirection => "*",
        | UnaryOperator::Plus => "+",
        | UnaryOperator::Minus => "-",
        | UnaryOperator::BitwiseNot => "~",
        | UnaryOperator::LogicalNot => "!",
        | UnaryOperator::PreIncrement => "++ (prefix)",
        | UnaryOperator::PreDecrement => "-- (prefix)",
        | UnaryOperator::PostIncrement => "++ (postfix)",
        | UnaryOperator::PostDecrement => "-- (postfix)",
    }
}

/// Renders a constant's value and C type. Floating values print exactly:
/// `float` and `double` as the shortest decimal that round-trips, `long
/// double` in hexadecimal.
///
/// C99: integer constant types follow §6.4.4.1 paragraph 5, p. 55;
/// PDF p. 67; a character constant has type `int` and a wide one `wchar_t`
/// (§6.4.4.4 paragraphs 10-11, p. 61; PDF p. 73), and a multi-character
/// constant's value is implementation-defined (§6.4.4.4 paragraph 10).
/// GNU imaginary constants extend C99 under §4 paragraph 6, p. 7; PDF p. 19.
/// Inspection retains their imaginary suffix as well as the component type.
fn constant_label(constant: &Constant) -> impl Display {
    fmt::from_fn(move |f| match *constant {
        | Constant::Integer(integer) => {
            let (value, type_name) = match integer {
                | IntegerTokenType::BitInt(value, width, unsigned) =>
                    return write!(
                        f,
                        "{} ({}_BitInt({width}))",
                        value.get(),
                        if unsigned { "unsigned " } else { "" }
                    ),
                | IntegerTokenType::Imaginary(value, component) =>
                    return write!(f, "{}i ({})", value.get(), component.type_name()),
                | IntegerTokenType::Int(value) => (i128::from(value), "int"),
                | IntegerTokenType::Long(value) => (i128::from(value.get()), "long"),
                | IntegerTokenType::LongLong(value) => (i128::from(value.get()), "long long"),
                | IntegerTokenType::UnsignedInt(value) => (i128::from(value), "unsigned int"),
                | IntegerTokenType::UnsignedLong(value) =>
                    (i128::from(value.get()), "unsigned long"),
                | IntegerTokenType::UnsignedLongLong(value) =>
                    (i128::from(value.get()), "unsigned long long"),
            };
            write!(f, "{value} ({type_name})")
        },
        | Constant::Float(float) => write!(f, "{float} ({})", float.type_name()),
        | Constant::Char(CharacterTokenType::EncodedChar(c, encoding)) =>
            write!(f, "{c} ({})", encoding.type_name()),
        | Constant::Char(CharacterTokenType::Char(c)) => {
            write_c_quoted(f, "", '\'', c.encode_utf8(&mut [0; 4]))?;
            f.write_str(" (int)")
        },
        | Constant::Char(CharacterTokenType::WideChar(c)) => {
            match char::from_u32(c) {
                | Some(c) => write_c_quoted(f, "L", '\'', c.encode_utf8(&mut [0; 4]))?,
                | None => write!(f, "L\'\\x{c:x}\'")?,
            }
            f.write_str(" (wchar_t)")
        },
        | Constant::Char(CharacterTokenType::MultiChar(value)) =>
            write!(f, "{value} (int, multi-character)"),
    })
}
