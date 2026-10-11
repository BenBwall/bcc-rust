//! Describes a preprocessing token with its source spelling and converted
//! literal value for CLI output and test snapshots.

use std::fmt::Write as _;

use super::{
    ArenaString,
    Bump,
    CharacterTokenType,
    Context,
    IntegerTokenType,
    StringTokenType,
    Token,
    TokenType,
};

/// One line per token: its location, kind, source spelling, and for
/// constants the value and type the preprocessor assigned. The line is
/// written in `scratch`.
pub(crate) fn describe_token<'a>(
    token: Token,
    context: &Context<'_>,
    scratch: &'a Bump,
) -> &'a str {
    let spelling = context
        .string_cache
        .at(token.contents)
        .trim_end_matches('\0');
    let literal = match token.kind {
        | TokenType::String(StringTokenType::EncodedString(contents, encoding)) =>
            context.literal_spelling_in(scratch, scratch, contents, encoding.prefix()),
        | TokenType::String(StringTokenType::String(contents)) =>
            context.literal_spelling_in(scratch, scratch, contents, ""),
        | TokenType::String(StringTokenType::WideString(contents)) =>
            context.literal_spelling_in(scratch, scratch, contents, "L"),
        | _ => "",
    };
    let mut line = ArenaString::new_in(scratch);
    if let Some(vector) = context.get_source_vectors(token.source_vectors).first() {
        _ = write!(
            line,
            "{}:{}:{}: ",
            context.get_source_file(vector.source_file_index).display(),
            vector.line,
            vector.column,
        );
    }
    _ = match token.kind {
        | TokenType::Identifier => write!(line, "identifier `{spelling}`"),
        | TokenType::Keyword(_) => write!(line, "keyword `{spelling}`"),
        | TokenType::Operator(operator) => write!(line, "punctuator `{}`", operator.spelling()),
        | TokenType::String(StringTokenType::EncodedString(_, encoding)) =>
            write!(line, "{} string literal {literal}", encoding.type_name()),
        | TokenType::String(StringTokenType::String(_)) => write!(line, "string literal {literal}"),
        | TokenType::String(StringTokenType::WideString(_)) =>
            write!(line, "wide string literal {literal}"),
        | TokenType::Character(character) => {
            let (value, type_name) = match character {
                | CharacterTokenType::EncodedChar(c, encoding) =>
                    (i64::from(c), encoding.type_name()),
                | CharacterTokenType::Char(c) => (i64::from(u32::from(c)), "int"),
                | CharacterTokenType::WideChar(c) => (i64::from(c), "wchar_t"),
                | CharacterTokenType::MultiChar(value) => (i64::from(value), "int"),
            };
            write!(
                line,
                "character constant `{spelling}` = {value} ({type_name})"
            )
        },
        | TokenType::Integer(integer) => {
            let (value, type_name) = match integer {
                | IntegerTokenType::BitInt(value, width, unsigned) => {
                    _ = write!(
                        line,
                        "integer constant `{spelling}` = {} ({}_BitInt({width}))",
                        value.get(),
                        if unsigned { "unsigned " } else { "" }
                    );
                    return line.into_str();
                },
                | IntegerTokenType::Imaginary(value, component) => {
                    // GNU imaginary integer constants extend C99 §6.4.4.1
                    // under §4p6; preserve the imaginary component as for
                    // floats.
                    _ = write!(
                        line,
                        "integer constant `{spelling}` = {}i ({})",
                        value.get(),
                        component.type_name()
                    );
                    return line.into_str();
                },
                | IntegerTokenType::Int(value) => (i128::from(value), "int"),
                | IntegerTokenType::Long(value) => (i128::from(value.get()), "long"),
                | IntegerTokenType::LongLong(value) => (i128::from(value.get()), "long long"),
                | IntegerTokenType::UnsignedInt(value) => (i128::from(value), "unsigned int"),
                | IntegerTokenType::UnsignedLong(value) =>
                    (i128::from(value.get()), "unsigned long"),
                | IntegerTokenType::UnsignedLongLong(value) =>
                    (i128::from(value.get()), "unsigned long long"),
            };
            write!(
                line,
                "integer constant `{spelling}` = {value} ({type_name})"
            )
        },
        | TokenType::Float(float) => write!(
            line,
            "floating constant `{spelling}` = {float} ({})",
            float.type_name()
        ),
    };
    line.into_str()
}
