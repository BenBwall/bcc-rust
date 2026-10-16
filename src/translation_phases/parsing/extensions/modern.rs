//! ISO additions to translation phase 7, expressed as resumable frames.
//!
//! C11: generic selections §6.5.1.1, p. 78; PDF p. 96; static assertions
//! §6.7.10, p. 145; PDF p. 163. C99: the shared type-name and expression
//! children are §6.7.6, p. 122; PDF p. 134 and §6.5, pp. 67-94;
//! PDF pp. 79-106. Evaluation and type constraints belong to later analysis.

use super::super::{
    Parser,
    declaration_syntax::TypeName,
    errors::ParserErrorType,
    frame_pool::FramePools,
    frames::{
        expression::{
            ExpressionBoundary,
            ExpressionFrame,
            ExpressionMode,
        },
        type_name::TypeNameFrame,
    },
    machine::{
        ParseAction,
        ParseFrame,
        ParseValue,
        any_expression_value,
        unexpected_return,
    },
    syntax::Expression,
};
use crate::{
    configuration::Feature,
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

impl<'tu, 'p> ModernFrame<'tu, 'p> {
    /// Parses later-standard operands, generic associations, assertions and
    /// attributes.
    /// GNU extension: GCC manual, "Attribute Syntax".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Attribute-Syntax.html>
    /// C11: §6.5.1.1 paragraph 1, p. 78; PDF p. 96.
    /// C11: §6.7.10 paragraph 1, p. 145; PDF p. 163.
    /// C23: §6.7.13.2 paragraph 1, pp. 142-143; PDF pp. 155-156.
    pub(in crate::translation_phases::parsing) fn step(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
        returned: Option<ParseValue<'tu>>,
    ) -> ParseAction<'tu, 'p> {
        match self.phase {
            | Phase::Start =>
                if let Some(token) = token {
                    if let TokenType::Keyword(keyword) = token.kind {
                        self.keyword = Some(keyword);
                    }
                    self.own(parser, token);
                    if self.kind == ModernKind::Attributes {
                        self.attribute_start = parser.cursor.consumed;
                        if matches!(token.kind, TokenType::Keyword(KeywordTokenType::Declspec)) {
                            self.attribute_syntax = AttributeSyntax::Msvc;
                            self.tokens.push(token);
                            self.phase = Phase::GnuAttributeInnerOpen;
                            return ParseAction::Consume;
                        }
                        if matches!(token.kind, TokenType::Keyword(KeywordTokenType::Attribute)) {
                            self.attribute_syntax = AttributeSyntax::Gnu;
                            self.tokens.push(token);
                            self.phase = Phase::GnuAttributeOpen;
                            return ParseAction::Consume;
                        }
                        parser.extension(Feature::Attributes, "[[...]]", token);
                        self.tokens.push(token);
                        self.delimiters
                            .push(OperatorTokenType::ClosingSquareBracket);
                        self.phase = Phase::AttributeTokens;
                    } else {
                        self.phase = Phase::Open;
                    }
                    ParseAction::Consume
                } else {
                    self.phase = Phase::Open;
                    ParseAction::Continue
                },
            | Phase::Open => {
                self.phase = Phase::Operand;
                self.punctuation(
                    parser,
                    token,
                    OperatorTokenType::OpeningParenthesis,
                    "`(` in ISO construct",
                )
            },
            | Phase::Operand => {
                let type_only = matches!(
                    self.kind,
                    ModernKind::Operand {
                        type_only: true,
                        ..
                    }
                );
                let is_type = type_only
                    || self.kind != ModernKind::Assertion
                        && self.keyword != Some(KeywordTokenType::BitInt)
                        && token.is_some_and(|x| parser.type_name_starter(x));
                self.phase = Phase::AwaitOperand;
                if is_type {
                    if self.kind == ModernKind::Generic
                        && let Some(token) = token
                    {
                        parser.extension(
                            Feature::GenericTypeOperand,
                            "type-controlling _Generic",
                            token,
                        );
                    }
                    ParseAction::Push(ParseFrame::TypeName(TypeNameFrame::new(
                        parser.hard_error_count,
                    )))
                } else {
                    ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                        parser.arena,
                        if self.kind == ModernKind::Assertion
                            || matches!(self.kind, ModernKind::Operand { constant: true, .. })
                        {
                            ExpressionMode::ConstantExpression
                        } else if self.kind == ModernKind::Generic {
                            ExpressionMode::AssignmentExpression
                        } else {
                            ExpressionMode::Expression
                        },
                        if matches!(
                            self.kind,
                            ModernKind::Operand {
                                constant: false,
                                ..
                            }
                        ) {
                            ExpressionBoundary::ClosingParenthesis
                        } else {
                            ExpressionBoundary::Argument
                        },
                        parser.hard_error_count,
                    )))
                }
            },
            | Phase::AwaitOperand => {
                let operand = match returned {
                    | Some(ParseValue::TypeName(x)) => SyntaxOperand::Type(x),
                    | other => SyntaxOperand::Expression(any_expression_value(other)),
                };
                parser
                    .context
                    .merge_into(&mut self.source_vectors, operand.source());
                self.operand = Some(operand);
                self.phase = if matches!(self.kind, ModernKind::Operand { .. }) {
                    Phase::Close
                } else {
                    Phase::Separator
                };
                ParseAction::Continue
            },
            | Phase::Separator =>
                if self.kind == ModernKind::Assertion {
                    if token.is_some_and(|x| {
                        matches!(x.kind, TokenType::Operator(OperatorTokenType::Comma))
                    }) {
                        self.phase = Phase::Message;
                        self.own(parser, token.expect("comma exists"));
                        ParseAction::Consume
                    } else {
                        if !parser.context.configuration.is_native(Feature::C23Keywords)
                            && let Some(token) = token
                        {
                            parser.extension(
                                Feature::C23Keywords,
                                "static assertion without message",
                                token,
                            );
                        }
                        self.phase = Phase::Close;
                        ParseAction::Continue
                    }
                } else {
                    self.phase = Phase::Association;
                    self.punctuation(
                        parser,
                        token,
                        OperatorTokenType::Comma,
                        "`,` before generic associations",
                    )
                },
            | Phase::Association => {
                if let Some(token) = token
                    && matches!(token.kind, TokenType::Keyword(KeywordTokenType::Default))
                {
                    self.own(parser, token);
                    self.association_type = None;
                    self.phase = Phase::Colon;
                    ParseAction::Consume
                } else {
                    self.phase = Phase::AwaitAssociationType;
                    ParseAction::Push(ParseFrame::TypeName(TypeNameFrame::new(
                        parser.hard_error_count,
                    )))
                }
            },
            | Phase::AwaitAssociationType => {
                let Some(ParseValue::TypeName(x)) = returned else {
                    unexpected_return!("generic association type protocol: {returned:?}")
                };
                self.association_type = Some(x);
                parser
                    .context
                    .merge_into(&mut self.source_vectors, x.source_vectors);
                self.phase = Phase::Colon;
                ParseAction::Continue
            },
            | Phase::Colon => {
                self.phase = Phase::AssociationExpression;
                self.punctuation(
                    parser,
                    token,
                    OperatorTokenType::Colon,
                    "`:` in generic association",
                )
            },
            | Phase::AssociationExpression => {
                self.phase = Phase::AwaitAssociationExpression;
                ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
                    parser.arena,
                    ExpressionMode::AssignmentExpression,
                    ExpressionBoundary::Argument,
                    parser.hard_error_count,
                )))
            },
            | Phase::AwaitAssociationExpression => {
                let expression = any_expression_value(returned);
                parser
                    .context
                    .merge_into(&mut self.source_vectors, expression.source_vectors);
                self.associations.push(GenericAssociation {
                    type_name: self.association_type.take(),
                    expression,
                });
                if let Some(token) = token
                    && matches!(token.kind, TokenType::Operator(OperatorTokenType::Comma))
                {
                    self.own(parser, token);
                    self.phase = Phase::Association;
                    ParseAction::Consume
                } else {
                    self.phase = Phase::Close;
                    ParseAction::Continue
                }
            },
            | Phase::Message => {
                self.phase = Phase::Close;
                if let Some(token) = token
                    && matches!(token.kind, TokenType::String(_))
                {
                    self.message = Some(token);
                    self.own(parser, token);
                    ParseAction::Consume
                } else {
                    Self::expected(parser, token, "string literal in static assertion");
                    ParseAction::Reprocess
                }
            },
            | Phase::Close => {
                self.phase = if self.kind == ModernKind::Assertion {
                    Phase::Semicolon
                } else {
                    Phase::Finish
                };
                self.punctuation(
                    parser,
                    token,
                    OperatorTokenType::ClosingParenthesis,
                    "`)` in ISO construct",
                )
            },
            | Phase::Semicolon => {
                self.phase = Phase::Finish;
                self.punctuation(
                    parser,
                    token,
                    OperatorTokenType::Semicolon,
                    "`;` after static assertion",
                )
            },
            | Phase::GnuAttributeOpen | Phase::GnuAttributeInnerOpen => {
                let inner = matches!(self.phase, Phase::GnuAttributeInnerOpen);
                self.phase = if inner {
                    Phase::AttributeTokens
                } else {
                    Phase::GnuAttributeInnerOpen
                };
                self.attribute_position = AttributePosition::Name;
                if let Some(token) = token
                    && matches!(
                        token.kind,
                        TokenType::Operator(OperatorTokenType::OpeningParenthesis)
                    )
                {
                    self.tokens.push(token);
                    self.delimiters.push(OperatorTokenType::ClosingParenthesis);
                    ParseAction::Consume
                } else {
                    self.attribute_expected(
                        parser,
                        token,
                        if self.attribute_syntax == AttributeSyntax::Msvc {
                            "`(` in MSVC declspec specifier"
                        } else {
                            "`(` in GNU attribute specifier"
                        },
                    );
                    self.phase = Phase::Finish;
                    ParseAction::Continue
                }
            },
            | Phase::AttributeTokens => {
                let Some(token) = token else {
                    self.attribute_expected(parser, None, self.closer_component());
                    self.phase = Phase::Finish;
                    return ParseAction::Continue;
                };
                // Leave caller boundaries unconsumed, but keep semicolons
                // within standard balanced-token arguments (C23 §6.7.13.2p1).
                if self.caller_owns(parser, token) {
                    self.attribute_expected(parser, Some(token), self.closer_component());
                    self.phase = Phase::Finish;
                    return ParseAction::Continue;
                }
                if self.delimiters.len() <= self.attribute_outer_depth() {
                    use AttributePosition::{
                        AfterArguments,
                        AfterName,
                        AfterPrefixedName,
                        Closing,
                        Name,
                        Opening,
                        PrefixedName,
                        SecondColon,
                    };
                    let kind = token.kind;
                    let identifier = matches!(kind, TokenType::Identifier | TokenType::Keyword(_));
                    let position = self.attribute_position;
                    let valid = match position {
                        | Opening => {
                            self.attribute_position = Name;
                            matches!(
                                kind,
                                TokenType::Operator(OperatorTokenType::OpeningSquareBracket)
                            )
                        },
                        | Name | PrefixedName if identifier => {
                            self.attribute_position = if matches!(position, Name) {
                                AfterName
                            } else {
                                AfterPrefixedName
                            };
                            true
                        },
                        | AfterName | AfterArguments
                            if identifier && self.attribute_syntax == AttributeSyntax::Msvc =>
                        {
                            self.attribute_position = AfterName;
                            true
                        },
                        | Name | AfterName | AfterPrefixedName | AfterArguments
                            if matches!(kind, TokenType::Operator(OperatorTokenType::Comma)) =>
                        {
                            self.attribute_position = Name;
                            true
                        },
                        | Name | AfterName | AfterPrefixedName | AfterArguments
                            if matches!(kind, TokenType::Operator(actual) if actual ==
                                if self.attribute_syntax == AttributeSyntax::Standard {
                                    OperatorTokenType::ClosingSquareBracket
                                } else {
                                    OperatorTokenType::ClosingParenthesis
                                }
                            ) =>
                        {
                            self.attribute_position = Closing;
                            true
                        },
                        | AfterName
                            if matches!(kind, TokenType::Operator(OperatorTokenType::Colon)) =>
                        {
                            self.attribute_position = SecondColon;
                            true
                        },
                        | SecondColon
                            if matches!(kind, TokenType::Operator(OperatorTokenType::Colon)) =>
                        {
                            self.attribute_position = PrefixedName;
                            true
                        },
                        | AfterName | AfterPrefixedName
                            if matches!(
                                kind,
                                TokenType::Operator(OperatorTokenType::OpeningParenthesis)
                            ) =>
                        {
                            self.start_attribute_argument(parser);
                            self.attribute_position = AfterArguments;
                            true
                        },
                        | Closing
                            if matches!(kind, TokenType::Operator(actual) if actual ==
                                if self.attribute_syntax == AttributeSyntax::Standard {
                                    OperatorTokenType::ClosingSquareBracket
                                } else {
                                    OperatorTokenType::ClosingParenthesis
                                }
                            ) =>
                            true,
                        | Closing => {
                            // Only the second closer can follow the first;
                            // anything else starts the attributed syntax.
                            self.attribute_expected(parser, Some(token), self.closer_component());
                            self.phase = Phase::Finish;
                            return ParseAction::Continue;
                        },
                        | _ => false,
                    };
                    if !valid {
                        self.attribute_expected(
                            parser,
                            Some(token),
                            "attribute name, arguments, or separator",
                        );
                    }
                }
                if let TokenType::Operator(op) = token.kind {
                    let closing = match op {
                        | OperatorTokenType::OpeningParenthesis =>
                            Some(OperatorTokenType::ClosingParenthesis),
                        | OperatorTokenType::OpeningSquareBracket =>
                            Some(OperatorTokenType::ClosingSquareBracket),
                        | OperatorTokenType::OpeningCurlyBrace =>
                            Some(OperatorTokenType::ClosingCurlyBrace),
                        | _ => None,
                    };
                    if let Some(closing) = closing {
                        self.delimiters.push(closing);
                    } else if matches!(
                        op,
                        OperatorTokenType::ClosingParenthesis
                            | OperatorTokenType::ClosingSquareBracket
                            | OperatorTokenType::ClosingCurlyBrace
                    ) {
                        if self.delimiters.last() == Some(&op) {
                            _ = self.delimiters.pop();
                        } else {
                            self.attribute_expected(
                                parser,
                                Some(token),
                                "matching attribute argument delimiter",
                            );
                            self.phase = Phase::Finish;
                            return ParseAction::Continue;
                        }
                    }
                }
                self.tokens.push(token);
                self.own(parser, token);
                if self.delimiters.is_empty() {
                    self.phase = Phase::Finish;
                }
                ParseAction::Consume
            },
            | Phase::AttributeArgument => self.attribute_argument(parser, token),
            | Phase::AwaitAttributeArgument =>
                self.complete_attribute_argument(parser, token, returned),
            | Phase::Finish => {
                let source_vectors = if self.kind == ModernKind::Attributes {
                    parser.cursor.source_since(self.attribute_start)
                } else {
                    self.source_vectors.unwrap_or_default()
                };
                let recovered = parser.hard_error_count > self.starting_errors;
                let value = match self.kind {
                    | ModernKind::Operand { .. } => ModernValue::Operand(
                        self.operand.expect("operand child completed"),
                        source_vectors,
                    ),
                    | ModernKind::Generic => {
                        let associations = parser.alloc_syntax_list(&mut self.associations);
                        ModernValue::Generic(parser.alloc_syntax(GenericSelection {
                            source_vectors,
                            recovered,
                            controlling: self.operand.expect("generic operand completed"),
                            associations,
                        }))
                    },
                    | ModernKind::Assertion => {
                        let SyntaxOperand::Expression(expression) =
                            self.operand.expect("assertion operand completed")
                        else {
                            unreachable!("assertion uses expressions")
                        };
                        ModernValue::Assertion(parser.alloc_syntax(StaticAssertion {
                            expression,
                            message: self.message,
                            source_vectors,
                            recovered,
                        }))
                    },
                    | ModernKind::Attributes => {
                        let tokens = parser
                            .cursor
                            .retain_since(self.attribute_start, parser.tree);
                        let arguments = parser.alloc_syntax_list(&mut self.arguments);
                        ModernValue::Attributes(parser.alloc_syntax(AttributeSpecifier {
                            syntax: self.attribute_syntax,
                            tokens,
                            arguments,
                            source_vectors,
                            recovered,
                        }))
                    },
                };
                ParseAction::Reduce(ParseValue::Modern(value))
            },
        }
    }

    /// Parses an integer operand, a nonnull index list, or a deferred call
    /// predicate together with its message and severity.
    /// C99: §6.6p6, p. 95; PDF p. 107 (GNU attribute extension).
    fn attribute_argument(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
    ) -> ParseAction<'tu, 'p> {
        let list = self.tokens.iter().rev().nth(1).is_some_and(|token| {
            let name = parser
                .context
                .string_cache
                .at(token.contents)
                .trim_matches('_');
            matches!(name, "nonnull" | "diagnose_if" | "enable_if")
        });
        if list
            && token.is_some_and(|token| {
                matches!(
                    token.kind,
                    TokenType::Operator(OperatorTokenType::ClosingParenthesis)
                )
            })
        {
            self.phase = Phase::AttributeTokens;
            return ParseAction::Continue;
        }
        self.phase = Phase::AwaitAttributeArgument;
        ParseAction::Push(ParseFrame::Expression(ExpressionFrame::new(
            parser.arena,
            if list {
                ExpressionMode::Expression
            } else {
                ExpressionMode::ConstantExpression
            },
            if list {
                ExpressionBoundary::ClosingParenthesis
            } else {
                ExpressionBoundary::Argument
            },
            parser.hard_error_count,
        )))
    }

    /// GCC aligned, and the constructor/destructor priority, have a C integer
    /// constant expression operand. Keep unknown vendor-prefixed attributes
    /// opaque, even if their name is `aligned`.
    /// C99: §6.6p6, p. 95; PDF p. 107 (GNU attribute extension).
    fn start_attribute_argument(&mut self, parser: &mut Parser<'_, 'tu, 'p>) {
        if matches!(
            self.attribute_position,
            AttributePosition::AfterPrefixedName
        ) && self
            .tokens
            .get(self.tokens.len().saturating_sub(4))
            .is_none_or(|namespace| {
                let namespace = parser
                    .context
                    .string_cache
                    .at(namespace.contents)
                    .trim_matches('_');
                namespace != "gnu"
                    && !(namespace == "clang"
                        && self.tokens.last().is_some_and(|name| {
                            matches!(
                                parser
                                    .context
                                    .string_cache
                                    .at(name.contents)
                                    .trim_matches('_'),
                                "pass_object_size"
                                    | "pass_dynamic_object_size"
                                    | "diagnose_if"
                                    | "enable_if"
                            )
                        }))
            })
        {
            return;
        }
        if self.tokens.last().is_some_and(|name| {
            let name = parser
                .context
                .string_cache
                .at(name.contents)
                .trim_matches('_');
            matches!(
                name,
                "aligned"
                    | "constructor"
                    | "destructor"
                    | "nonnull"
                    | "pass_object_size"
                    | "pass_dynamic_object_size"
                    | "diagnose_if"
                    | "enable_if"
            )
        }) && parser.cursor.alignment_argument_closes(parser.arena)
        {
            self.argument_index = parser.cursor.consumed - self.attribute_start - 1;
            self.phase = Phase::AttributeArgument;
        }
    }

    /// Preserve the operand's original tokens alongside its parsed expression.
    /// C99: §6.6p6, p. 95; PDF p. 107 (GNU aligned operand).
    fn complete_attribute_argument(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
        returned: Option<ParseValue<'tu>>,
    ) -> ParseAction<'tu, 'p> {
        let expression = any_expression_value(returned);
        self.arguments.push(AttributeArgument {
            index: self.argument_index,
            expression,
        });
        if !token.is_some_and(|token| {
            matches!(
                token.kind,
                TokenType::Operator(OperatorTokenType::ClosingParenthesis)
            )
        }) {
            self.attribute_expected(parser, token, self.closer_component());
            if token.is_none_or(|token| self.caller_owns(parser, token)) {
                self.phase = Phase::Finish;
                return ParseAction::Continue;
            }
        }
        self.phase = Phase::AttributeTokens;
        ParseAction::Continue
    }
}

/// Resumable grammar positions for ISO additions and GNU attributes.
/// GNU extension: GCC manual, "Attribute Syntax".
/// <https://gcc.gnu.org/onlinedocs/gcc/Attribute-Syntax.html>
/// C11: §6.5.1.1 paragraph 1, p. 78; PDF p. 96.
/// C11: §6.7.10 paragraph 1, p. 145; PDF p. 163.
/// C23: §6.7.13.2 paragraph 1, pp. 142-143; PDF pp. 155-156.
#[derive(Debug, Clone, Copy)]
enum Phase {
    Start,
    Open,
    Operand,
    AwaitOperand,
    Separator,
    Association,
    AwaitAssociationType,
    Colon,
    AssociationExpression,
    AwaitAssociationExpression,
    Message,
    Close,
    Semicolon,
    Finish,
    AttributeTokens,
    AttributeArgument,
    AwaitAttributeArgument,
    GnuAttributeOpen,
    GnuAttributeInnerOpen,
}

/// A grammar operand distinguished before semantic analysis.
/// C11: §6.5.1.1, p. 78; PDF p. 96. Type operands in `_Generic` are C2y.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum SyntaxOperand<'tu> {
    Expression(&'tu Expression<'tu>),
    Type(&'tu TypeName<'tu>),
}

/// Parameterized and newly introduced ISO type specifiers.
/// C99: extends the type-specifier seam of §6.7.2, pp. 99-100; PDF pp. 111-112.
/// C11: atomic type specifiers §6.7.2.4 paragraph 1, p. 121; PDF p. 139.
/// C23: typeof specifiers §6.7.3.6 paragraph 1, p. 117; PDF p. 130.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum ExtendedType<'tu> {
    Atomic(&'tu TypeName<'tu>),
    Typeof {
        operand:     SyntaxOperand<'tu>,
        unqualified: bool,
    },
    BitInt {
        width:      &'tu Expression<'tu>,
        signedness: Option<bool>,
    },
    Decimal32,
    Decimal64,
    Decimal128,
    Inferred,
    Int128 {
        signedness: Option<bool>,
    },
    AutoType,
    /// MSVC fixed-width integer spelling; target layout belongs to analysis.
    /// MSVC extension: Microsoft Learn, "__int8, __int16, __int32, __int64".
    /// <https://learn.microsoft.com/en-us/cpp/cpp/int8-int16-int32-int64>
    MsInteger {
        width:      u8,
        signedness: Option<bool>,
    },
    Float128 {
        complex: bool,
    },
}

/// Specifier additions collected in reverse source order; immutable links avoid
/// enlarging common nodes. Inspection visits the links in source order. C99:
/// extends declaration-specifiers §6.7, p. 97; PDF p. 109. C11: alignment
/// specifiers §6.7.5 paragraph 1, p. 127; PDF p. 145.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct SpecifierExtension<'tu> {
    pub(crate) kind:           SpecifierExtensionKind<'tu>,
    pub(crate) next:           Option<&'tu Self>,
    pub(crate) source_vectors: SourceVectors,
}

/// Declaration specifier additions from later ISO revisions and vendor
/// dialects.
/// GNU extension: GCC manual, "Attribute Syntax".
/// <https://gcc.gnu.org/onlinedocs/gcc/Attribute-Syntax.html>
/// MSVC extension: Microsoft Learn, "declspec".
/// <https://learn.microsoft.com/en-us/cpp/cpp/declspec>
/// C11: §6.7.5 paragraph 1, p. 127; PDF p. 145.
/// C11: §6.7.1 paragraph 1, p. 109; PDF p. 127.
/// C23: §6.7.2 paragraph 1, pp. 98-99; PDF pp. 111-112.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum SpecifierExtensionKind<'tu> {
    Alignment(SyntaxOperand<'tu>),
    Attributes(&'tu AttributeSpecifier<'tu>),
    ThreadLocal(ThreadSpelling),
    Constexpr,
    ExtensionMarker,
    /// MSVC declaration modifier, retaining exact spelling and provenance.
    /// MSVC extension: Microsoft Learn, "Microsoft-Specific Modifiers".
    /// <https://learn.microsoft.com/en-us/cpp/cpp/microsoft-specific-modifiers>
    MsModifier(KeywordTokenType),
}

/// How a thread storage-class specifier was written. Every spelling gives
/// thread storage duration, but `__thread` does not combine with the
/// standard ones, and diagnostics name the spelling the source used.
/// C11: §6.7.1 paragraph 1, p. 109; PDF p. 127. C23: `thread_local` is a
/// keyword, §6.4.1 paragraph 1, p. 53; PDF p. 66.
/// GNU extension: GCC manual, "Thread-Local Storage".
/// <https://gcc.gnu.org/onlinedocs/gcc/Thread-Local.html>
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum ThreadSpelling {
    /// GNU `__thread`.
    Gnu,
    /// C11 `_Thread_local`.
    Reserved,
    /// C23 `thread_local`.
    Keyword,
}

impl ThreadSpelling {
    /// The specifier as written.
    pub(crate) const fn spelling(self) -> &'static str {
        match self {
            | Self::Gnu => "__thread",
            | Self::Reserved => "_Thread_local",
            | Self::Keyword => "thread_local",
        }
    }

    /// Whether two specifiers are the same one written twice rather than
    /// the GNU and the standard specifier, which do not combine.
    pub(crate) const fn repeats(self, other: Self) -> bool {
        matches!(self, Self::Gnu) == matches!(other, Self::Gnu)
    }
}

/// Shared attribute syntax for ISO, GNU, and MSVC grammar owners.
/// C23: §6.7.13.2 paragraph 1, pp. 142-143; PDF pp. 155-156.
/// Balanced tokens retain spelling and provenance. Recognized GNU alignment
/// operands also retain expression syntax for semantic constant evaluation.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct AttributeSpecifier<'tu> {
    pub(crate) syntax:         AttributeSyntax,
    pub(crate) tokens:         &'tu [Token],
    /// Attribute-name token index and its parsed alignment operand.
    pub(crate) arguments:      ArenaList<'tu, AttributeArgument<'tu>>,
    pub(crate) source_vectors: SourceVectors,
    pub(crate) recovered:      bool,
}

/// A recognized attribute's expression operand, associated with its name's
/// position in the retained token list. GNU extension: GCC aligned uses an
/// integer constant expression (C99 §6.6p6, p. 95; PDF p. 107).
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct AttributeArgument<'tu> {
    pub(crate) index:      usize,
    pub(crate) expression: &'tu Expression<'tu>,
}

/// ISO, GNU and Microsoft attribute introducer grammars.
/// GNU extension: GCC manual, "Attribute Syntax".
/// <https://gcc.gnu.org/onlinedocs/gcc/Attribute-Syntax.html>
/// MSVC extension: Microsoft Learn, "declspec".
/// <https://learn.microsoft.com/en-us/cpp/cpp/declspec>
/// C23: §6.7.13.2 paragraph 1, pp. 142-143; PDF pp. 155-156.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum AttributeSyntax {
    Standard,
    Gnu,
    Msvc,
}

/// C11: §6.5.1.1 paragraph 1, p. 78; PDF p. 96.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct GenericSelection<'tu> {
    pub(crate) source_vectors: SourceVectors,
    pub(crate) recovered:      bool,
    pub(crate) controlling:    SyntaxOperand<'tu>,
    pub(crate) associations:   ArenaList<'tu, GenericAssociation<'tu>>,
}

/// A type-named or default generic association and its result expression.
/// C11: §6.5.1.1 paragraph 1, p. 78; PDF p. 96.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct GenericAssociation<'tu> {
    pub(crate) type_name:  Option<&'tu TypeName<'tu>>,
    pub(crate) expression: &'tu Expression<'tu>,
}

/// C11: §6.7.10 paragraph 1, p. 145; PDF p. 163. C23 permits no message.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct StaticAssertion<'tu> {
    pub(crate) expression:     &'tu Expression<'tu>,
    pub(crate) message:        Option<Token>,
    pub(crate) source_vectors: SourceVectors,
    pub(crate) recovered:      bool,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::translation_phases::parsing) enum ModernValue<'tu> {
    Operand(SyntaxOperand<'tu>, SourceVectors),
    Generic(&'tu GenericSelection<'tu>),
    Assertion(&'tu StaticAssertion<'tu>),
    Attributes(&'tu AttributeSpecifier<'tu>),
}

/// Selects the operand, generic, assertion or attribute grammar owned by a
/// frame.
/// C11: §6.5.1.1 paragraph 1, p. 78; PDF p. 96.
/// C11: §6.7.10 paragraph 1, p. 145; PDF p. 163.
/// C23: §6.7.13.2 paragraph 1, pp. 142-143; PDF pp. 155-156.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::translation_phases::parsing) enum ModernKind {
    Operand { type_only: bool, constant: bool },
    Generic,
    Assertion,
    Attributes,
}

/// Resumable positions in the standard attribute token grammar.
/// C23: §6.7.13.2 paragraph 1, pp. 142-143; PDF pp. 155-156.
#[derive(Debug, Clone, Copy)]
enum AttributePosition {
    Opening,
    Name,
    AfterName,
    SecondColon,
    PrefixedName,
    AfterPrefixedName,
    AfterArguments,
    Closing,
}

/// Delimiter-owning frame for the ISO additions: generic selections, static
/// assertions, attribute specifiers, and the parenthesized operands of
/// `_Alignof`, `_Alignas`, `_Atomic`, `_BitInt`, and `typeof`. Its grammar
/// children run on the shared machine stack.
/// C11: §6.5.1.1 and §6.7.10, pp. 78, 145; PDF pp. 96, 163.
#[derive(Debug)]
pub(in crate::translation_phases::parsing) struct ModernFrame<'tu, 'p> {
    keyword: Option<KeywordTokenType>,
    kind: ModernKind,
    phase: Phase,
    attribute_position: AttributePosition,
    attribute_syntax: AttributeSyntax,
    pub(in crate::translation_phases::parsing) source_vectors: Option<SourceVectors>,
    operand: Option<SyntaxOperand<'tu>>,
    association_type: Option<&'tu TypeName<'tu>>,
    pub(in crate::translation_phases::parsing) associations: ArenaVec<'p, GenericAssociation<'tu>>,
    pub(in crate::translation_phases::parsing) tokens: ArenaVec<'p, Token>,
    pub(in crate::translation_phases::parsing) arguments: ArenaVec<'p, AttributeArgument<'tu>>,
    attribute_start: usize,
    argument_index: usize,
    delimiters: ArenaVec<'p, OperatorTokenType>,
    message: Option<Token>,
    starting_errors: usize,
}

impl SyntaxOperand<'_> {
    pub(in crate::translation_phases::parsing) fn source(self) -> SourceVectors {
        match self {
            | Self::Expression(x) => x.source_vectors,
            | Self::Type(x) => x.source_vectors,
        }
    }
}

impl<'tu, 'p> ModernFrame<'tu, 'p> {
    pub(in crate::translation_phases::parsing) fn lend_pooled(
        &mut self,
        pools: &mut FramePools<'tu, 'p>,
    ) {
        pools.modern_associations.lend(&mut self.associations);
        pools.opaque_tokens.lend(&mut self.tokens);
        pools.attribute_arguments.lend(&mut self.arguments);
        pools.delimiters.lend(&mut self.delimiters);
    }

    pub(in crate::translation_phases::parsing) fn reclaim_pooled(
        &mut self,
        pools: &mut FramePools<'tu, 'p>,
    ) {
        pools.modern_associations.reclaim(&mut self.associations);
        pools.opaque_tokens.reclaim(&mut self.tokens);
        pools.attribute_arguments.reclaim(&mut self.arguments);
        pools.delimiters.reclaim(&mut self.delimiters);
    }

    /// Starts parsing the parenthesized operand of an extended type or
    /// alignment specifier.
    /// C11: §6.7.2.4 paragraph 1, p. 121; PDF p. 139.
    /// C11: §6.7.5 paragraph 1, p. 127; PDF p. 145.
    /// C23: §6.7.3.6 paragraph 1, p. 117; PDF p. 130.
    pub(in crate::translation_phases::parsing) fn operand_after_keyword(
        arena: &'p Bump,
        errors: usize,
        source: SourceVectors,
    ) -> Self {
        let mut frame = Self::new(
            arena,
            ModernKind::Operand {
                type_only: true,
                constant:  false,
            },
            errors,
        );
        frame.phase = Phase::Open;
        frame.source_vectors = Some(source);
        frame
    }

    fn own(&mut self, parser: &mut Parser<'_, 'tu, 'p>, token: Token) {
        if self.kind != ModernKind::Attributes {
            parser.merge_source(&mut self.source_vectors, token);
        }
    }

    fn attribute_outer_depth(&self) -> usize {
        if self.attribute_syntax == AttributeSyntax::Msvc {
            1
        } else {
            2
        }
    }

    fn closer_component(&self) -> &'static str {
        match self.attribute_syntax {
            | AttributeSyntax::Msvc => "`)` in MSVC declspec specifier",
            | AttributeSyntax::Gnu => "`))` in GNU attribute specifier",
            | AttributeSyntax::Standard => "`]]` in attribute specifier",
        }
    }

    /// Whether `token` ends the enclosing declaration rather than continuing
    /// the attribute: a `;` outside standard argument delimiters or in an
    /// argument that does not close, a `}` outside braces the attribute
    /// opened, or a `{` that cannot be an argument token. Standard balanced
    /// tokens allow semicolons, while GNU and MSVC arguments are expressions
    /// and retain semicolons as recovery boundaries.
    /// C23: balanced-token §6.7.13.2 paragraph 1, pp. 142-143; PDF pp. 155-156.
    fn caller_owns(&self, parser: &Parser<'_, 'tu, 'p>, token: Token) -> bool {
        let TokenType::Operator(op) = token.kind else {
            return false;
        };
        match op {
            | OperatorTokenType::Semicolon if self.attribute_syntax == AttributeSyntax::Standard =>
                self.delimiters.len() <= self.attribute_outer_depth()
                    || !self.standard_argument_closes(parser),
            | OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace => !self
                .delimiters
                .contains(&OperatorTokenType::ClosingCurlyBrace),
            | OperatorTokenType::OpeningCurlyBrace =>
                self.attribute_syntax != AttributeSyntax::Standard
                    || self.delimiters.len() <= self.attribute_outer_depth(),
            | _ => false,
        }
    }

    /// Whether the open standard attribute argument closes after the current
    /// `;`. An unclosed argument leaves the `;` to the enclosing declaration,
    /// so malformed input cannot swallow the declarations that follow it. The
    /// scan is bounded; a longer argument is treated as unclosed.
    /// C23: balanced-token §6.7.13.2 paragraph 1, pp. 142-143; PDF pp. 155-156.
    fn standard_argument_closes(&self, parser: &Parser<'_, 'tu, 'p>) -> bool {
        const SCAN_LIMIT: usize = 1024;
        let mut open = &self.delimiters[self.attribute_outer_depth()..];
        let mut nested = 0_usize;
        for offset in 0..SCAN_LIMIT {
            let Some(token) = parser.cursor.lookahead(offset) else {
                return false;
            };
            let TokenType::Operator(op) = token.kind else {
                continue;
            };
            match op {
                | OperatorTokenType::OpeningParenthesis
                | OperatorTokenType::OpeningSquareBracket
                | OperatorTokenType::OpeningCurlyBrace => nested += 1,
                | OperatorTokenType::ClosingParenthesis
                | OperatorTokenType::ClosingSquareBracket
                | OperatorTokenType::ClosingCurlyBrace =>
                    if nested > 0 {
                        nested -= 1;
                    } else if let Some((&closing, rest)) = open.split_last()
                        && closing == op
                    {
                        open = rest;
                        if open.is_empty() {
                            return true;
                        }
                    } else {
                        return false;
                    },
                | _ => {},
            }
        }
        false
    }

    #[cold]
    #[inline(never)]
    fn expected(parser: &mut Parser<'_, 'tu, 'p>, token: Option<Token>, position: &'static str) {
        parser.report(
            ParserErrorType::ExpectedIsoSyntax(position, token.map(|x| x.kind)),
            token,
        );
    }

    #[cold]
    #[inline(never)]
    fn attribute_expected(
        &self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
        component: &'static str,
    ) {
        let error = if self.attribute_syntax == AttributeSyntax::Msvc {
            ParserErrorType::ExpectedMsSyntax(component, token.map(|x| x.kind))
        } else if self.attribute_syntax == AttributeSyntax::Gnu {
            ParserErrorType::ExpectedGnuSyntax(component, token.map(|x| x.kind))
        } else {
            ParserErrorType::ExpectedIsoSyntax(component, token.map(|x| x.kind))
        };
        parser.report(error, token);
    }

    fn punctuation(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
        operator: OperatorTokenType,
        position: &'static str,
    ) -> ParseAction<'tu, 'p> {
        if let Some(token) = token
            && matches!(token.kind, TokenType::Operator(actual) if actual == operator)
        {
            self.own(parser, token);
            ParseAction::Consume
        } else {
            Self::expected(parser, token, position);
            ParseAction::Reprocess
        }
    }

    pub(in crate::translation_phases::parsing) fn new(
        arena: &'p Bump,
        kind: ModernKind,
        starting_errors: usize,
    ) -> Self {
        Self {
            kind,
            keyword: None,
            phase: Phase::Start,
            attribute_position: AttributePosition::Opening,
            attribute_syntax: AttributeSyntax::Standard,
            source_vectors: None,
            operand: None,
            association_type: None,
            associations: ArenaVec::new_in(arena),
            tokens: ArenaVec::new_in(arena),
            arguments: ArenaVec::new_in(arena),
            attribute_start: 0,
            argument_index: 0,
            delimiters: ArenaVec::new_in(arena),
            message: None,
            starting_errors,
        }
    }
}
