//! GNU extensions to translation phase 7, implemented by resumable frames.
//!
//! C99: these extend expression, statement and declarator grammar (§6.5,
//! pp. 67-94; PDF pp. 79-106; §6.8, pp. 131-139; PDF pp. 143-151;
//! §6.7.5, p. 114; PDF p. 126). GCC's C Extensions and Extended Asm
//! manuals specify the vendor grammar. Types, constraints, assembly meaning,
//! builtin evaluation and label resolution belong to later analysis.

use super::{
    Parser,
    errors::ParserErrorType,
    expression::{
        ExpressionBoundary,
        ExpressionFrame,
        ExpressionMode,
    },
    machine::{
        ParseAction,
        ParseFrame,
        ParseValue,
        any_expression_value,
    },
    modern::SyntaxOperand,
    syntax::{
        Expression,
        Identifier,
    },
    type_name::TypeNameFrame,
};
use crate::{
    translation_phases::{
        SourceVectors,
        preprocessing::{
            KeywordTokenType,
            OperatorTokenType,
            Token,
            TokenType,
        },
    },
    util::{
        arena_list::ArenaList,
        bump::{
            ArenaVec,
            Bump,
        },
    },
};

/// GNU assembly grammar: `asm qualifiers ( template : outputs : inputs :
/// clobbers : labels )`, or a declarator's `asm ( template )` label. The
/// template, constraint and clobber string literals keep their provenance;
/// C operands are parsed syntax children. `sections` counts the colons.
/// C99: extension to §6.8, p. 131; PDF p. 143 and §6.7.5, p. 114; PDF p. 126.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct Asm<'tu> {
    pub(crate) qualifiers:     AsmQualifiers,
    /// `None` when the template literal is missing.
    pub(crate) template:       Option<Token>,
    pub(crate) operands:       ArenaList<'tu, AsmOperand<'tu>>,
    pub(crate) clobbers:       ArenaList<'tu, Token>,
    pub(crate) labels:         ArenaList<'tu, Identifier>,
    pub(crate) sections:       u8,
    pub(crate) source_vectors: SourceVectors,
    pub(crate) recovered:      bool,
}
/// The qualifiers written before a GNU assembly statement's `(`.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Default)]
pub(crate) struct AsmQualifiers {
    pub(crate) volatile: bool,
    pub(crate) inline:   bool,
    pub(crate) goto:     bool,
}
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct AsmOperand<'tu> {
    pub(crate) name:       Option<Identifier>,
    pub(crate) constraint: Token,
    pub(crate) expression: &'tu Expression<'tu>,
    pub(crate) output:     bool,
}
/// Typed GNU builtin operands. Offset member paths use their own namespace.
/// C99: extension to primary expressions §6.5.1, p. 69; PDF p. 81.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct Builtin<'tu> {
    pub(crate) keyword:        KeywordTokenType,
    pub(crate) operands:       ArenaList<'tu, SyntaxOperand<'tu>>,
    pub(crate) members:        ArenaList<'tu, OffsetMember<'tu>>,
    pub(crate) source_vectors: SourceVectors,
    pub(crate) recovered:      bool,
}
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum OffsetMember<'tu> {
    Field(Identifier),
    Index(&'tu Expression<'tu>),
}
#[derive(Debug, Clone, Copy)]
pub(super) enum GnuValue<'tu> {
    Asm(&'tu Asm<'tu>),
    Builtin(&'tu Builtin<'tu>),
    LocalLabels(ArenaList<'tu, Identifier>, SourceVectors),
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum GnuKind {
    Asm { label: bool },
    Builtin(KeywordTokenType),
    LocalLabels,
}
#[derive(Debug, Clone, Copy)]
enum Phase {
    Start,
    Qualifiers,
    Open,
    Template,
    Section,
    OperandName,
    OperandNameClose,
    Constraint,
    OperandOpen,
    AwaitAsmExpression,
    OperandClose,
    AsmSeparator,
    BuiltinOperand,
    AwaitBuiltinOperand,
    BuiltinSeparator,
    OffsetField,
    OffsetSuffix,
    AwaitOffsetIndex,
    OffsetIndexClose,
    LocalName,
    LocalSeparator,
    Close,
    Semicolon,
    Finish,
}
/// GNU delimiter owner; all expression/type children run on the parser stack.
/// C99: vendor extension to §6.5 and §6.8, pp. 67-139; PDF pp. 79-151.
#[derive(Debug)]
pub(super) struct GnuFrame<'tu, 'p> {
    kind:                    GnuKind,
    phase:                   Phase,
    source_vectors:          Option<SourceVectors>,
    starting_errors:         usize,
    /// Clobber literals of an assembly statement.
    pub(super) tokens:       ArenaVec<'p, Token>,
    template:                Option<Token>,
    pub(super) asm_operands: ArenaVec<'p, AsmOperand<'tu>>,
    pub(super) operands:     ArenaVec<'p, SyntaxOperand<'tu>>,
    pub(super) members:      ArenaVec<'p, OffsetMember<'tu>>,
    pub(super) labels:       ArenaVec<'p, Identifier>,
    sections:                u8,
    qualifiers:              AsmQualifiers,
    requires_operand:        bool,
    name:                    Option<Identifier>,
    constraint:              Option<Token>,
    expression:              Option<&'tu Expression<'tu>>,
}
impl<'tu, 'p> GnuFrame<'tu, 'p> {
    pub(super) fn new(arena: &'p Bump, kind: GnuKind, errors: usize) -> Self {
        Self {
            kind,
            phase: Phase::Start,
            source_vectors: None,
            starting_errors: errors,
            tokens: ArenaVec::new_in(arena),
            template: None,
            asm_operands: ArenaVec::new_in(arena),
            operands: ArenaVec::new_in(arena),
            members: ArenaVec::new_in(arena),
            labels: ArenaVec::new_in(arena),
            sections: 0,
            qualifiers: AsmQualifiers::default(),
            requires_operand: false,
            name: None,
            constraint: None,
            expression: None,
        }
    }

    pub(super) fn push(parser: &mut Parser<'_, 'tu, 'p>, kind: GnuKind) -> ParseAction<'tu, 'p> {
        let frame = Self::new(parser.arena, kind, parser.hard_error_count);
        ParseAction::Push(ParseFrame::Gnu(parser.pools.gnu(frame)))
    }

    fn own(&mut self, parser: &mut Parser<'_, 'tu, 'p>, token: Token) {
        parser.merge_source(&mut self.source_vectors, token);
    }

    fn expected(parser: &mut Parser<'_, 'tu, 'p>, token: Option<Token>, position: &'static str) {
        parser.report(
            ParserErrorType::ExpectedGnuSyntax(position, token.map(|x| x.kind)),
            token,
        );
    }

    fn punctuation(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
        op: OperatorTokenType,
        position: &'static str,
    ) -> ParseAction<'tu, 'p> {
        if let Some(token) = token
            && matches!(token.kind, TokenType::Operator(actual) if actual == op)
        {
            self.own(parser, token);
            ParseAction::Consume
        } else {
            Self::expected(parser, token, position);
            ParseAction::Reprocess
        }
    }

    pub(super) fn step(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
        returned: Option<ParseValue<'tu>>,
    ) -> ParseAction<'tu, 'p> {
        match self.phase {
            | Phase::Start => {
                self.phase = match self.kind {
                    | GnuKind::Asm { label: false } => Phase::Qualifiers,
                    | GnuKind::LocalLabels => Phase::LocalName,
                    | _ => Phase::Open,
                };
                if let Some(token) = token {
                    // Reserved `__asm` has a deferred GNU/MSVC origin when
                    // both grammars are enabled; this owner chose GNU syntax.
                    if token.contents == KeywordTokenType::MsAsm.cache_id()
                        && matches!(token.kind, TokenType::Keyword(KeywordTokenType::MsAsm))
                    {
                        parser.extension(crate::configuration::Feature::GnuAsm, "__asm", token);
                    }
                    self.own(parser, token);
                    ParseAction::Consume
                } else {
                    ParseAction::Continue
                }
            },
            | Phase::Qualifiers => {
                let qualifier = match token.map(|x| x.kind) {
                    | Some(TokenType::Keyword(KeywordTokenType::Volatile)) =>
                        Some(&mut self.qualifiers.volatile),
                    | Some(TokenType::Keyword(KeywordTokenType::Inline)) =>
                        Some(&mut self.qualifiers.inline),
                    | Some(TokenType::Keyword(KeywordTokenType::Goto)) =>
                        Some(&mut self.qualifiers.goto),
                    | _ => None,
                };
                if let Some(qualifier) = qualifier {
                    let repeated = std::mem::replace(qualifier, true);
                    if repeated {
                        Self::expected(parser, token, "distinct asm qualifier");
                    }
                    self.own(parser, token.expect("qualifier exists"));
                    ParseAction::Consume
                } else {
                    self.phase = Phase::Open;
                    ParseAction::Continue
                }
            },
            | Phase::Open => {
                self.phase = if matches!(self.kind, GnuKind::Asm { .. }) {
                    Phase::Template
                } else {
                    Phase::BuiltinOperand
                };
                self.punctuation(
                    parser,
                    token,
                    OperatorTokenType::OpeningParenthesis,
                    "`(` in GNU construct",
                )
            },
            | Phase::Template => {
                self.phase = Phase::Section;
                if let Some(token) = token
                    && matches!(token.kind, TokenType::String(_))
                {
                    self.template = Some(token);
                    self.own(parser, token);
                    ParseAction::Consume
                } else {
                    Self::expected(parser, token, "string literal asm template or label");
                    ParseAction::Reprocess
                }
            },
            | Phase::Section => {
                if let Some(token) = token
                    && matches!(token.kind, TokenType::Operator(OperatorTokenType::Colon))
                {
                    self.sections = self.sections.saturating_add(1);
                    if self.sections > 4 || self.kind == (GnuKind::Asm { label: true }) {
                        Self::expected(parser, Some(token), "`)` after asm sections");
                    }
                    self.own(parser, token);
                    self.phase = Phase::OperandName;
                    ParseAction::Consume
                } else {
                    self.phase = Phase::Close;
                    ParseAction::Continue
                }
            },
            | Phase::OperandName => {
                if token.is_none_or(|x| {
                    matches!(
                        x.kind,
                        TokenType::Operator(
                            OperatorTokenType::Colon
                                | OperatorTokenType::ClosingParenthesis
                                | OperatorTokenType::Semicolon
                                | OperatorTokenType::ClosingCurlyBrace
                        )
                    )
                }) {
                    if self.requires_operand {
                        Self::expected(parser, token, "asm operand after comma");
                        self.requires_operand = false;
                    }
                    self.phase = Phase::Section;
                    return ParseAction::Continue;
                }
                self.requires_operand = false;
                self.name = None;
                if self.sections >= 4 {
                    self.phase = Phase::AsmSeparator;
                    if let Some(token) = token
                        && matches!(token.kind, TokenType::Identifier)
                    {
                        self.labels.push(Identifier::from_token(token));
                        self.own(parser, token);
                        ParseAction::Consume
                    } else {
                        Self::expected(parser, token, "asm goto label");
                        self.phase = Phase::Close;
                        ParseAction::Continue
                    }
                } else if self.sections == 3 {
                    self.phase = Phase::AsmSeparator;
                    if let Some(token) = token
                        && matches!(token.kind, TokenType::String(_))
                    {
                        self.tokens.push(token);
                        self.own(parser, token);
                        ParseAction::Consume
                    } else {
                        Self::expected(parser, token, "string literal asm clobber");
                        self.phase = Phase::Close;
                        ParseAction::Continue
                    }
                } else if let Some(token) = token
                    && matches!(
                        token.kind,
                        TokenType::Operator(OperatorTokenType::OpeningSquareBracket)
                    )
                {
                    self.own(parser, token);
                    self.phase = Phase::OperandNameClose;
                    ParseAction::Consume
                } else {
                    self.phase = Phase::Constraint;
                    ParseAction::Continue
                }
            },
            | Phase::OperandNameClose => {
                if self.name.is_none()
                    && let Some(token) = token
                    && matches!(token.kind, TokenType::Identifier)
                {
                    self.name = Some(Identifier::from_token(token));
                    self.own(parser, token);
                    return ParseAction::Consume;
                }
                if self.name.is_none() {
                    Self::expected(parser, token, "identifier in asm operand name");
                }
                self.phase = Phase::Constraint;
                self.punctuation(
                    parser,
                    token,
                    OperatorTokenType::ClosingSquareBracket,
                    "`]` in asm operand name",
                )
            },
            | Phase::Constraint => {
                self.phase = Phase::OperandOpen;
                if let Some(token) = token
                    && matches!(token.kind, TokenType::String(_))
                {
                    self.constraint = Some(token);
                    self.own(parser, token);
                    ParseAction::Consume
                } else {
                    Self::expected(parser, token, "string literal asm constraint");
                    self.phase = Phase::Close;
                    ParseAction::Continue
                }
            },
            | Phase::OperandOpen => {
                if token.is_some_and(|x| {
                    matches!(
                        x.kind,
                        TokenType::Operator(OperatorTokenType::OpeningParenthesis)
                    )
                }) {
                    self.own(parser, token.expect("parenthesis exists"));
                    // Child must start after the parent consumes its delimiter.
                    self.phase = Phase::AwaitAsmExpression;
                    ParseAction::Consume
                } else {
                    Self::expected(parser, token, "`(` before asm operand expression");
                    self.phase = Phase::Close;
                    ParseAction::Continue
                }
            },
            | Phase::AwaitAsmExpression => {
                if returned.is_none() {
                    return ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                        parser.arena,
                        ExpressionMode::Expression,
                        ExpressionBoundary::ClosingParenthesis,
                        parser.hard_error_count,
                    )));
                }
                let expression = any_expression_value(returned);
                self.expression = Some(expression);
                parser
                    .context
                    .merge_into(&mut self.source_vectors, expression.source_vectors);
                self.phase = Phase::OperandClose;
                ParseAction::Continue
            },
            | Phase::OperandClose => {
                self.asm_operands.push(AsmOperand {
                    name:       self.name.take(),
                    constraint: self.constraint.take().expect("constraint parsed"),
                    expression: self.expression.take().expect("asm expression completed"),
                    output:     self.sections == 1,
                });
                self.phase = Phase::AsmSeparator;
                self.punctuation(
                    parser,
                    token,
                    OperatorTokenType::ClosingParenthesis,
                    "`)` after asm operand expression",
                )
            },
            | Phase::AsmSeparator => {
                if let Some(token) = token
                    && matches!(token.kind, TokenType::Operator(OperatorTokenType::Comma))
                {
                    self.own(parser, token);
                    self.requires_operand = true;
                    self.phase = Phase::OperandName;
                    ParseAction::Consume
                } else {
                    self.phase = Phase::Section;
                    ParseAction::Continue
                }
            },
            | Phase::BuiltinOperand => {
                let GnuKind::Builtin(keyword) = self.kind else {
                    unreachable!("builtin phase");
                };
                let type_operand = match keyword {
                    | KeywordTokenType::BuiltinVaArg | KeywordTokenType::BuiltinConvertVector =>
                        self.operands.len() == 1,
                    | KeywordTokenType::BuiltinBitCast => self.operands.is_empty(),
                    | KeywordTokenType::BuiltinOffsetof
                    | KeywordTokenType::BuiltinTypesCompatible => true,
                    | _ => false,
                };
                self.phase = Phase::AwaitBuiltinOperand;
                if type_operand {
                    ParseAction::Push(ParseFrame::TypeName(TypeNameFrame::new(
                        parser.hard_error_count,
                    )))
                } else {
                    ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                        parser.arena,
                        if keyword == KeywordTokenType::BuiltinChooseExpr
                            && self.operands.is_empty()
                        {
                            ExpressionMode::ConstantExpression
                        } else {
                            ExpressionMode::AssignmentExpression
                        },
                        ExpressionBoundary::Argument,
                        parser.hard_error_count,
                    )))
                }
            },
            | Phase::AwaitBuiltinOperand => {
                let operand = match returned {
                    | Some(ParseValue::TypeName(x)) => SyntaxOperand::Type(x),
                    | x => SyntaxOperand::Expression(any_expression_value(x)),
                };
                parser
                    .context
                    .merge_into(&mut self.source_vectors, operand.source());
                self.operands.push(operand);
                self.phase = Phase::BuiltinSeparator;
                ParseAction::Continue
            },
            | Phase::BuiltinSeparator => {
                let GnuKind::Builtin(keyword) = self.kind else {
                    unreachable!("builtin phase");
                };
                let count = match keyword {
                    | KeywordTokenType::BuiltinChooseExpr => 3,
                    | KeywordTokenType::BuiltinVaEnd => 1,
                    | _ => 2,
                };
                if self.operands.len() < count {
                    self.phase = if keyword == KeywordTokenType::BuiltinOffsetof {
                        Phase::OffsetField
                    } else {
                        Phase::BuiltinOperand
                    };
                    self.punctuation(
                        parser,
                        token,
                        OperatorTokenType::Comma,
                        "`,` between builtin operands",
                    )
                } else {
                    self.phase = Phase::Close;
                    ParseAction::Continue
                }
            },
            | Phase::OffsetField => {
                self.phase = Phase::OffsetSuffix;
                if let Some(token) = token
                    && matches!(token.kind, TokenType::Identifier)
                {
                    self.members
                        .push(OffsetMember::Field(Identifier::from_token(token)));
                    self.own(parser, token);
                    ParseAction::Consume
                } else {
                    Self::expected(parser, token, "member identifier in __builtin_offsetof");
                    self.phase = Phase::Close;
                    ParseAction::Continue
                }
            },
            | Phase::OffsetSuffix => {
                if let Some(token) = token
                    && matches!(token.kind, TokenType::Operator(OperatorTokenType::Period))
                {
                    self.own(parser, token);
                    self.phase = Phase::OffsetField;
                    ParseAction::Consume
                } else if let Some(token) = token
                    && matches!(
                        token.kind,
                        TokenType::Operator(OperatorTokenType::OpeningSquareBracket)
                    )
                {
                    self.own(parser, token);
                    self.phase = Phase::AwaitOffsetIndex;
                    ParseAction::Consume
                } else {
                    self.phase = Phase::Close;
                    ParseAction::Continue
                }
            },
            | Phase::AwaitOffsetIndex => {
                if returned.is_none() {
                    return ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                        parser.arena,
                        ExpressionMode::Expression,
                        ExpressionBoundary::ClosingSquareBracket,
                        parser.hard_error_count,
                    )));
                }
                let expression = any_expression_value(returned);
                self.members.push(OffsetMember::Index(expression));
                parser
                    .context
                    .merge_into(&mut self.source_vectors, expression.source_vectors);
                self.phase = Phase::OffsetIndexClose;
                ParseAction::Continue
            },
            | Phase::OffsetIndexClose => {
                self.phase = Phase::OffsetSuffix;
                self.punctuation(
                    parser,
                    token,
                    OperatorTokenType::ClosingSquareBracket,
                    "`]` in offsetof member path",
                )
            },
            | Phase::LocalName => {
                self.phase = Phase::LocalSeparator;
                if let Some(token) = token
                    && matches!(token.kind, TokenType::Identifier)
                {
                    self.labels.push(Identifier::from_token(token));
                    self.own(parser, token);
                    ParseAction::Consume
                } else {
                    Self::expected(parser, token, "identifier in __label__ declaration");
                    self.phase = Phase::Semicolon;
                    ParseAction::Continue
                }
            },
            | Phase::LocalSeparator => {
                if let Some(token) = token
                    && matches!(token.kind, TokenType::Operator(OperatorTokenType::Comma))
                {
                    self.own(parser, token);
                    self.phase = Phase::LocalName;
                    ParseAction::Consume
                } else {
                    self.phase = Phase::Semicolon;
                    ParseAction::Continue
                }
            },
            | Phase::Close => {
                if matches!(self.kind, GnuKind::Asm { label: false })
                    && self.qualifiers.goto != (self.sections == 4)
                {
                    Self::expected(parser, token, "four asm goto sections with goto qualifier");
                }
                if self.qualifiers.goto && self.sections == 4 && self.labels.is_empty() {
                    Self::expected(parser, token, "label in asm goto section");
                }
                self.phase = if self.kind == (GnuKind::Asm { label: false }) {
                    Phase::Semicolon
                } else {
                    Phase::Finish
                };
                self.punctuation(
                    parser,
                    token,
                    OperatorTokenType::ClosingParenthesis,
                    "`)` in GNU construct",
                )
            },
            | Phase::Semicolon => {
                self.phase = Phase::Finish;
                self.punctuation(
                    parser,
                    token,
                    OperatorTokenType::Semicolon,
                    "`;` after GNU statement",
                )
            },
            | Phase::Finish => {
                let source_vectors = self.source_vectors.unwrap_or_default();
                let recovered = parser.hard_error_count > self.starting_errors;
                let value = match self.kind {
                    | GnuKind::Asm { .. } => {
                        let clobbers = parser.alloc_syntax_list(&mut self.tokens);
                        let operands = parser.alloc_syntax_list(&mut self.asm_operands);
                        let labels = parser.alloc_syntax_list(&mut self.labels);
                        GnuValue::Asm(parser.alloc_syntax(Asm {
                            qualifiers: self.qualifiers,
                            template: self.template,
                            operands,
                            clobbers,
                            labels,
                            sections: self.sections,
                            source_vectors,
                            recovered,
                        }))
                    },
                    | GnuKind::Builtin(keyword) => {
                        let operands = parser.alloc_syntax_list(&mut self.operands);
                        let members = parser.alloc_syntax_list(&mut self.members);
                        GnuValue::Builtin(parser.alloc_syntax(Builtin {
                            keyword,
                            operands,
                            members,
                            source_vectors,
                            recovered,
                        }))
                    },
                    | GnuKind::LocalLabels => GnuValue::LocalLabels(
                        parser.alloc_syntax_list(&mut self.labels),
                        source_vectors,
                    ),
                };
                ParseAction::Reduce(ParseValue::Gnu(value))
            },
        }
    }
}
