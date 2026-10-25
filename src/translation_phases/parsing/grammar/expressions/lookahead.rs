//! Expression-recovery lookahead scanners for translation phase 7.
//!
//! Recovery serves C99: §5.1.1.3, p. 11; PDF p. 23.

use crate::translation_phases::{
    parsing::{
        Parser,
        grammar::{
            expression_operators::is_postfix_starter,
            expressions::ExpressionBoundary,
            statements::is_statement_keyword,
        },
        recovery::ExpressionTerminator,
    },
    preprocessing::{
        KeywordTokenType,
        OperatorTokenType,
        Token,
        TokenType,
    },
};

/// Counts the tokens from the current one up to the next `closer` at
/// delimiter depth zero, when that closer follows before any token that
/// cannot occur inside an expression (`;`, a statement keyword, an unmatched
/// closer, or the end of input). Returns `None` when no such closer is
/// within reach, or when the current token is itself the closer.
///
/// `stop_at_declarations` also rejects a run that reaches a declaration
/// starter, matching the stops of
/// [`SynchronizationKind::StatementExpression`](crate::translation_phases::parsing::recovery::SynchronizationKind)
/// so that a recovery scan reaches the same closer.
///
/// C99: recovery serves §5.1.1.3, p. 11; PDF p. 23. An expression never
/// contains `;` or a statement keyword (§6.5, pp. 67-94; PDF pp. 79-106).
pub(in crate::translation_phases::parsing) fn closer_follows_stray_run(
    parser: &mut Parser<'_, '_, '_>,
    closer: fn(TokenType) -> bool,
    stop_at_declarations: bool,
) -> Option<usize> {
    // Each token opens at most one delimiter, so the run's open delimiters
    // fit in a fixed stack.
    let mut open = [OperatorTokenType::OpeningParenthesis; STRAY_RUN_LOOKAHEAD];
    let mut depth = 0;
    let mut previous = None;
    for index in 0..STRAY_RUN_LOOKAHEAD {
        let token = if index == 0 {
            parser.cursor.current()?
        } else {
            parser.cursor.lookahead(index - 1)?
        };
        if depth == 0 && closer(token.kind) {
            return (index > 0).then_some(index);
        }
        if matches!(
            token.kind,
            TokenType::Operator(OperatorTokenType::Semicolon)
        ) || is_statement_keyword(token.kind)
            || stop_at_declarations
                && (matches!(
                    token.kind,
                    TokenType::Operator(OperatorTokenType::QuestionMark)
                ) || depth == 0 && parser.declaration_starter(token))
        {
            return None;
        }
        match token.kind {
            | TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
                if !matches!(
                    previous,
                    Some(TokenType::Operator(OperatorTokenType::ClosingParenthesis))
                ) =>
            {
                // Only a compound literal's initializer list may open a brace
                // inside an expression.
                return None;
            },
            | TokenType::Operator(
                opening @ (OperatorTokenType::OpeningParenthesis
                | OperatorTokenType::OpeningSquareBracket
                | OperatorTokenType::OpeningCurlyBrace),
            ) => {
                open[depth] = opening;
                depth += 1;
            },
            | TokenType::Operator(
                closing @ (OperatorTokenType::ClosingParenthesis
                | OperatorTokenType::ClosingSquareBracket
                | OperatorTokenType::ClosingCurlyBrace),
            ) => {
                let opening = match closing {
                    | OperatorTokenType::ClosingParenthesis =>
                        OperatorTokenType::OpeningParenthesis,
                    | OperatorTokenType::ClosingSquareBracket =>
                        OperatorTokenType::OpeningSquareBracket,
                    | _ => OperatorTokenType::OpeningCurlyBrace,
                };
                if depth == 0 || open[depth - 1] != opening {
                    return None;
                }
                depth -= 1;
            },
            | _ => {},
        }
        previous = Some(token.kind);
    }
    None
}

/// Returns whether the brace group starting at the current `{` reads as a
/// statement block rather than a misplaced initializer list: it contains a
/// `;` or a statement keyword, is empty, or does not close within reach.
pub(in crate::translation_phases::parsing) fn brace_group_is_block(
    parser: &mut Parser<'_, '_, '_>,
) -> bool {
    let mut depth = 0_usize;
    for index in 0..BRACE_GROUP_LOOKAHEAD {
        let token = if index == 0 {
            parser.cursor.current()
        } else {
            parser.cursor.lookahead(index - 1)
        };
        let Some(token) = token else {
            return true;
        };
        match token.kind {
            | TokenType::Operator(OperatorTokenType::OpeningCurlyBrace) => depth += 1,
            | TokenType::Operator(OperatorTokenType::ClosingCurlyBrace) => {
                if index == 1 {
                    return true;
                }
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return false;
                }
            },
            | TokenType::Operator(OperatorTokenType::Semicolon) => return true,
            | kind if is_statement_keyword(kind) => return true,
            | _ => {},
        }
    }
    true
}

/// Returns whether the brace group starting at the current `{` closes within
/// reach and is followed directly by `)`, as in `f({ ... })` or
/// `if ({ ... })`, where the `(` belongs to a call or a statement header
/// rather than opening a GNU statement expression.
///
/// C99: a `{` cannot start a primary expression (§6.5.1, p. 69; PDF p. 81),
/// so the whole group is skipped as one diagnosed operand (§5.1.1.3, p. 11;
/// PDF p. 23).
pub(in crate::translation_phases::parsing) fn brace_group_closes_before_parenthesis(
    parser: &mut Parser<'_, '_, '_>,
) -> bool {
    matches!(
        token_after_brace_group(parser),
        Some(TokenType::Operator(OperatorTokenType::ClosingParenthesis))
    )
}

/// Returns whether the brace group starting at the current `{` closes within
/// reach and is followed by `)`, `]`, or `,`: the group then sits inside an
/// expression, as in `if (x == {}) ...`, rather than being a statement body
/// left after a missing `)`.
pub(in crate::translation_phases::parsing) fn brace_group_continues_expression(
    parser: &mut Parser<'_, '_, '_>,
) -> bool {
    matches!(
        token_after_brace_group(parser),
        Some(TokenType::Operator(
            OperatorTokenType::ClosingParenthesis
                | OperatorTokenType::ClosingSquareBracket
                | OperatorTokenType::Comma
        ))
    )
}

/// The kind of the token after the `}` that closes the brace group starting
/// at the current `{`, or `None` when the group does not close within
/// [`PARENTHESIZED_BRACE_GROUP_LOOKAHEAD`] tokens or nothing follows it.
fn token_after_brace_group(parser: &mut Parser<'_, '_, '_>) -> Option<TokenType> {
    let mut depth = 0_usize;
    for index in 0..PARENTHESIZED_BRACE_GROUP_LOOKAHEAD {
        let token = if index == 0 {
            parser.cursor.current()
        } else {
            parser.cursor.lookahead(index - 1)
        }?;
        match token.kind {
            | TokenType::Operator(OperatorTokenType::OpeningCurlyBrace) => depth += 1,
            | TokenType::Operator(OperatorTokenType::ClosingCurlyBrace) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return parser.cursor.lookahead(index).map(|next| next.kind);
                }
            },
            | _ => {},
        }
    }
    None
}

/// How far recovery lookahead may scan for the closer of a malformed
/// delimited expression. Longer runs are left to the closer's owner.
const STRAY_RUN_LOOKAHEAD: usize = 64;

/// How far recovery lookahead may scan a brace group in operand position
/// before treating it as a statement block.
const BRACE_GROUP_LOOKAHEAD: usize = 256;

/// How far recovery lookahead may scan a brace group directly inside a `(`
/// for its matching `}` and the `)` after it.
const PARENTHESIZED_BRACE_GROUP_LOOKAHEAD: usize = 4096;

/// Returns whether the parenthesized type name starting at the current
/// token belongs to a compound literal: the `)` closing it is followed
/// directly by `{`. A `;` or `}` outside an aggregate body before that
/// `)` ends the scan.
///
/// C99: `( type-name ) { initializer-list }`, §6.5.2.5 paragraph 1,
/// p. 75; PDF p. 87.
/// Aggregate bodies: §6.7.2.1 paragraph 1, p. 101; PDF p. 113.
/// C23: compound-literal storage, §6.5.3.6 paragraph 1,
/// p. 78; PDF p. 91.
pub(in crate::translation_phases::parsing) fn type_name_is_compound_literal(
    parser: &Parser<'_, '_, '_>,
) -> bool {
    let mut groups = 0usize;
    let mut braces = 0usize;
    let mut offset = 0usize;
    let mut token = parser.cursor.current();
    while let Some(current) = token {
        match current.kind {
            | TokenType::Operator(
                OperatorTokenType::OpeningParenthesis | OperatorTokenType::OpeningSquareBracket,
            ) => groups += 1,
            | TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                if groups == 0 && braces == 0 =>
                return parser.cursor.lookahead(offset).is_some_and(|x| {
                    matches!(
                        x.kind,
                        TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
                    )
                }),
            | TokenType::Operator(
                OperatorTokenType::ClosingParenthesis | OperatorTokenType::ClosingSquareBracket,
            ) => groups = groups.saturating_sub(1),
            | TokenType::Operator(OperatorTokenType::OpeningCurlyBrace) => braces += 1,
            | TokenType::Operator(OperatorTokenType::ClosingCurlyBrace) if braces != 0 =>
                braces -= 1,
            | TokenType::Operator(
                OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace,
            ) if braces == 0 => return false,
            | _ => {},
        }
        token = parser.cursor.lookahead(offset);
        offset += 1;
    }
    false
}

/// Returns whether the current `(` opens a type name, or the storage
/// classes of a compound literal before one, as in `(constexpr int){1}`.
/// Storage-class and `inline` keywords that a compound literal cannot
/// take are diagnosed by the type-name frame, as in `(static int)x`.
/// C23: compound-literal storage-class specifiers §6.5.3.6 paragraph 1,
/// p. 78; PDF p. 91.
pub(in crate::translation_phases::parsing) fn parenthesized_type_name_follows(
    parser: &mut Parser<'_, '_, '_>,
) -> bool {
    let storage = |token: Token| {
        matches!(
            token.kind,
            TokenType::Keyword(
                KeywordTokenType::Auto
                    | KeywordTokenType::Constexpr
                    | KeywordTokenType::Extern
                    | KeywordTokenType::Inline
                    | KeywordTokenType::Register
                    | KeywordTokenType::Static
                    | KeywordTokenType::ThreadLocal
                    | KeywordTokenType::Typedef
            )
        )
    };
    let mut index = 0;
    while let Some(token) = parser.cursor.lookahead(index) {
        if parser.type_name_starter(token) {
            return true;
        }
        if !storage(token) {
            return false;
        }
        index += 1;
    }
    false
}

/// Returns whether the current `(` begins a compound literal: a type name
/// whose `)` is followed by `{`.
/// C99: §6.5.2.5 paragraph 1, p. 75; PDF p. 87.
pub(in crate::translation_phases::parsing) fn parenthesized_compound_literal_follows(
    parser: &mut Parser<'_, '_, '_>,
) -> bool {
    if !parenthesized_type_name_follows(parser) {
        return false;
    }
    let mut groups = 0usize;
    let mut braces = 0usize;
    let mut index = 0;
    while let Some(token) = parser.cursor.lookahead(index) {
        index += 1;
        match token.kind {
            | TokenType::Operator(
                OperatorTokenType::OpeningParenthesis | OperatorTokenType::OpeningSquareBracket,
            ) => groups += 1,
            | TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                if groups == 0 && braces == 0 =>
                return parser.cursor.lookahead(index).is_some_and(|x| {
                    matches!(
                        x.kind,
                        TokenType::Operator(OperatorTokenType::OpeningCurlyBrace)
                    )
                }),
            | TokenType::Operator(
                OperatorTokenType::ClosingParenthesis | OperatorTokenType::ClosingSquareBracket,
            ) => groups = groups.saturating_sub(1),
            | TokenType::Operator(OperatorTokenType::OpeningCurlyBrace) => braces += 1,
            | TokenType::Operator(OperatorTokenType::ClosingCurlyBrace) if braces != 0 =>
                braces -= 1,
            | TokenType::Operator(
                OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace,
            ) if braces == 0 => return false,
            | _ => {},
        }
    }
    false
}

pub(in crate::translation_phases::parsing) fn is_owning_boundary_for(
    boundary: ExpressionBoundary,
    token: TokenType,
) -> bool {
    if super::is_closing_delimiter(token) {
        return true;
    }
    match boundary {
        | ExpressionBoundary::Statement(
            ExpressionTerminator::Semicolon | ExpressionTerminator::ForSemicolon,
        ) => matches!(token, TokenType::Operator(OperatorTokenType::Semicolon)),
        | ExpressionBoundary::Statement(ExpressionTerminator::ClosingParenthesis)
        | ExpressionBoundary::ClosingParenthesis => matches!(
            token,
            TokenType::Operator(OperatorTokenType::ClosingParenthesis)
        ),
        | ExpressionBoundary::Statement(ExpressionTerminator::Colon) =>
            matches!(token, TokenType::Operator(OperatorTokenType::Colon)),
        | ExpressionBoundary::ClosingSquareBracket
        | ExpressionBoundary::ArrayBound
        | ExpressionBoundary::Designator => matches!(
            token,
            TokenType::Operator(OperatorTokenType::ClosingSquareBracket)
        ),
        | ExpressionBoundary::Argument => matches!(
            token,
            TokenType::Operator(OperatorTokenType::Comma | OperatorTokenType::ClosingParenthesis)
        ),
        | ExpressionBoundary::Initializer => matches!(
            token,
            TokenType::Operator(
                OperatorTokenType::Comma
                    | OperatorTokenType::ClosingCurlyBrace
                    | OperatorTokenType::Semicolon
            )
        ),
        | ExpressionBoundary::StructMember => matches!(
            token,
            TokenType::Operator(
                OperatorTokenType::Comma
                    | OperatorTokenType::Semicolon
                    | OperatorTokenType::ClosingCurlyBrace
            )
        ),
        | ExpressionBoundary::Enumerator => matches!(
            token,
            TokenType::Operator(OperatorTokenType::Comma | OperatorTokenType::ClosingCurlyBrace)
        ),
    }
}

pub(in crate::translation_phases::parsing) fn is_strong_grammar_boundary_for(
    parser: &mut Parser<'_, '_, '_>,
    token: Token,
    boundary: ExpressionBoundary,
) -> bool {
    let identifier_continues_as_postfix = matches!(token.kind, TokenType::Identifier)
        && parser
            .cursor
            .following()
            .is_some_and(|following| is_postfix_starter(following.kind));
    let declaration_starts_here = parser.declaration_recovery_starts_here(token);
    match boundary {
        | ExpressionBoundary::Initializer =>
            is_statement_keyword(token.kind)
                || declaration_starts_here
                    && !matches!(
                        parser.cursor.following().map(|following| following.kind),
                        Some(TokenType::Operator(
                            OperatorTokenType::Comma
                                | OperatorTokenType::Semicolon
                                | OperatorTokenType::ClosingCurlyBrace
                        ))
                    ),
        | ExpressionBoundary::ArrayBound =>
            !matches!(token.kind, TokenType::Identifier)
                && parser.declaration_starter(token)
                && !matches!(
                    parser.cursor.following().map(|following| following.kind),
                    Some(TokenType::Operator(OperatorTokenType::ClosingSquareBracket))
                ),
        | ExpressionBoundary::StructMember =>
            !matches!(token.kind, TokenType::Identifier)
                && parser.declaration_starter(token)
                && !matches!(
                    parser.cursor.following().map(|following| following.kind),
                    Some(TokenType::Operator(
                        OperatorTokenType::Comma
                            | OperatorTokenType::Semicolon
                            | OperatorTokenType::ClosingCurlyBrace
                    ))
                ),
        | ExpressionBoundary::Enumerator =>
            !matches!(token.kind, TokenType::Identifier)
                && parser.declaration_starter(token)
                && !matches!(
                    parser.cursor.following().map(|following| following.kind),
                    Some(TokenType::Operator(
                        OperatorTokenType::Comma | OperatorTokenType::ClosingCurlyBrace
                    ))
                ),
        | ExpressionBoundary::Statement(_) | ExpressionBoundary::ClosingParenthesis =>
            declaration_starts_here && !identifier_continues_as_postfix
                || is_statement_keyword(token.kind)
                || matches!(
                    boundary,
                    ExpressionBoundary::Statement(ExpressionTerminator::Semicolon)
                ) && matches!(token.kind, TokenType::Identifier)
                    && parser.cursor.following().is_some_and(|following| {
                        matches!(
                            following.kind,
                            TokenType::Operator(OperatorTokenType::Colon)
                        )
                    }),
        | ExpressionBoundary::ClosingSquareBracket
        | ExpressionBoundary::Argument
        | ExpressionBoundary::Designator => false,
    }
}
