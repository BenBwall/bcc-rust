//! Expression-recovery lookahead scanners for translation phase 7.
//!
//! Recovery serves C99: §5.1.1.3, p. 11; PDF p. 23.

use super::super::{
    Parser,
    statement::is_statement_keyword,
};
use crate::translation_phases::preprocessing::{
    OperatorTokenType,
    TokenType,
};

/// How far recovery lookahead may scan for the closer of a malformed
/// delimited expression. Longer runs are left to the closer's owner.
const STRAY_RUN_LOOKAHEAD: usize = 64;

/// How far recovery lookahead may scan a brace group in operand position
/// before treating it as a statement block.
const BRACE_GROUP_LOOKAHEAD: usize = 256;

/// Counts the tokens from the current one up to the next `closer` at
/// delimiter depth zero, when that closer follows before any token that
/// cannot occur inside an expression (`;`, a statement keyword, an unmatched
/// closer, or the end of input). Returns `None` when no such closer is
/// within reach, or when the current token is itself the closer.
///
/// `stop_at_declarations` also rejects a run that reaches a declaration
/// starter, matching the stops of
/// [`SynchronizationKind::StatementExpression`](super::super::recovery::SynchronizationKind)
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
pub(super) fn brace_group_is_block(parser: &mut Parser<'_, '_, '_>) -> bool {
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

/// How far recovery lookahead may scan a brace group directly inside a `(`
/// for its matching `}` and the `)` after it.
const PARENTHESIZED_BRACE_GROUP_LOOKAHEAD: usize = 4096;

/// Returns whether the brace group starting at the current `{` closes within
/// reach and is followed directly by `)`, as in `f({ ... })` or
/// `if ({ ... })`, where the `(` belongs to a call or a statement header
/// rather than opening a GNU statement expression.
///
/// C99: a `{` cannot start a primary expression (§6.5.1, p. 69; PDF p. 81),
/// so the whole group is skipped as one diagnosed operand (§5.1.1.3, p. 11;
/// PDF p. 23).
pub(super) fn brace_group_closes_before_parenthesis(parser: &mut Parser<'_, '_, '_>) -> bool {
    matches!(
        token_after_brace_group(parser),
        Some(TokenType::Operator(OperatorTokenType::ClosingParenthesis))
    )
}

/// Returns whether the brace group starting at the current `{` closes within
/// reach and is followed by `)`, `]`, or `,`: the group then sits inside an
/// expression, as in `if (x == {}) ...`, rather than being a statement body
/// left after a missing `)`.
pub(super) fn brace_group_continues_expression(parser: &mut Parser<'_, '_, '_>) -> bool {
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
