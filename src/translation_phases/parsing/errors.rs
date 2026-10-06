//! Parser diagnostics and their rendering.
//!
//! Translation phase 7 syntax analysis (§5.1.1.2 paragraph 1, p. 10;
//! PDF p. 22) must produce at least one diagnostic for every syntax-rule or
//! constraint violation in a translation unit (§5.1.1.3 paragraph 1, p. 11;
//! PDF p. 23). Their form is implementation-defined; bcc-rust renders each
//! as a structured message with source provenance. Diagnostics the standard
//! does not require, such as repeated qualifiers, are warnings (footnote 8,
//! p. 11; PDF p. 23 lets an implementation emit any number).

use std::fmt::{
    self,
    Debug,
    Display,
    Formatter,
    Result as FmtResult,
};

use super::{
    machine::ParseFrameKind,
    syntax::StorageClass,
};
use crate::{
    diagnostics::{
        Diagnostic,
        Explanation,
        ToDiagnostic,
        format_in,
        quote_spelling,
    },
    translation_phases::{
        Context,
        ErrorSeverity,
        GetPosition,
        GetSeverity,
        GetSourceVectors,
        SourcePosition,
        SourceVector,
        SourceVectors,
        preprocessing::{
            OperatorTokenType,
            TokenType,
        },
    },
    util::bump::Bump,
};

/// Structured parser diagnostic paired with original-source provenance.
///
/// C99: the diagnostic requirement is §5.1.1.3, p. 11; PDF p. 23.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum ParserDiagnosticCode {
    /// Violation of a syntax rule (§5.1.1.3 paragraph 1, p. 11; PDF p. 23).
    Syntax,
    /// Violation of a Constraints paragraph (§5.1.1.3 paragraph 1, p. 11;
    /// PDF p. 23; constraint is defined in §3.8, p. 5; PDF p. 17).
    Constraint,
    /// A diagnostic C99 does not require (footnote 8, p. 11; PDF p. 23).
    Quality,
    /// A bcc-rust bug, not a property of the input.
    InternalInvariant,
    /// A bcc-rust resource ceiling; see `ParserResource`.
    ResourceLimit,
}

/// A parser resource with a catchable ceiling.
///
/// C99: §5.2.4.1, pp. 20-21; PDF pp. 32-33 sets minimum translation limits
/// only, and footnote 13, p. 20; PDF p. 32 asks implementations to avoid
/// fixed ones, so these ceilings are representation bounds.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum ParserResource {
    ExternalDeclarations,
    SyntaxNodes,
    FrameDepth,
    SourceSegments,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum ParserWarningGroup {
    RepeatedSpecifiers,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum ExpectedSyntax {
    ExternalDeclaration,
    DeclarationSpecifier,
    TypeSpecifier,
    Declarator,
    Expression,
    Statement,
    Identifier,
    DeclarationContinuation,
    SeparatorOrCloser,
    OwnedDelimiter,
    None,
}

/// Grammar position of a declaration whose continuation was rejected.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum DeclarationPlace {
    /// A file-scope external declaration.
    /// C99: §6.9 paragraph 1, p. 140; PDF p. 152.
    External,
    /// A block-item declaration.
    /// C99: §6.8.2 paragraph 1, p. 132; PDF p. 144.
    Block,
    /// The declaration clause of a `for` statement.
    /// C99: §6.8.5 paragraph 1, p. 135; PDF p. 147.
    ForInitializer,
    /// A declaration in an old-style function's declaration list.
    /// C99: §6.9.1 paragraph 1, p. 141; PDF p. 153.
    OldStyleParameter,
}

/// Tokens that could still continue a declaration after its latest
/// declarator, so a diagnostic never offers the token it rejects.
///
/// C99: init-declarator-list is §6.7, p. 97; PDF p. 109; a function body
/// follows only a sole external declarator under §6.9.1, p. 141; PDF p. 153.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) struct DeclarationContinuation {
    pub(crate) place:         DeclarationPlace,
    /// The latest declarator has no initializer yet, so `=` may follow.
    pub(crate) initializer:   bool,
    /// The declaration is a sole uninitialized external declarator, so `{`
    /// may begin a function body.
    pub(crate) function_body: bool,
}

impl DeclarationContinuation {
    /// Returns the "expected ..." phrase and the primary label.
    fn expected(self, arena: &Bump) -> (&str, &str) {
        let mut phrases = ["`,`"; 4];
        let mut tokens = ["`,`"; 4];
        let mut count = 1;
        let mut offer = |phrase, token| {
            phrases[count] = phrase;
            tokens[count] = token;
            count += 1;
        };
        if self.initializer {
            offer("`=`", "`=`");
        }
        // In a `for` header, `)` only ends recovery: the declaration still
        // needs its `;` (C99 6.8.5p1), so it is not offered here.
        offer("`;`", "`;`");
        if self.function_body {
            offer("a function body", "`{`");
        }
        let (phrases, tokens) = (&phrases[..count], &tokens[..count]);
        let label = if tokens.len() == 2 {
            format_in!(arena, "expected {}", alternatives(tokens))
        } else {
            format_in!(arena, "expected one of {}", alternatives(tokens))
        };
        (
            format_in!(arena, "{} after the declarator", alternatives(phrases)),
            label,
        )
    }

    /// Explains why a found `{` cannot begin a function body here.
    fn function_body_note(self, found: Option<TokenType>) -> Option<&'static str> {
        if self.function_body
            || found != Some(TokenType::Operator(OperatorTokenType::OpeningCurlyBrace))
        {
            return None;
        }
        Some(match self.place {
            | DeclarationPlace::OldStyleParameter =>
                "C99 §6.9.1: each declaration before an old-style function body ends with `;`",
            | DeclarationPlace::Block | DeclarationPlace::ForInitializer =>
                "C99 §6.9.1: functions can only be defined at file scope",
            | DeclarationPlace::External =>
                "C99 §6.9.1: a function definition has exactly one declarator and no initializer",
        })
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct RelatedParserDiagnostic {
    pub(crate) message:        &'static str,
    pub(crate) source_vectors: SourceVectors,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct RecoverySummary {
    pub(crate) owner:            ParseFrameKind,
    pub(crate) discarded:        Option<SourceVectors>,
    pub(crate) discarded_tokens: usize,
    pub(crate) stopped_at:       Option<TokenType>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct ParserError<'tu> {
    pub(crate) code:              ParserDiagnosticCode,
    pub(crate) severity:          ErrorSeverity,
    pub(crate) warning_group:     Option<ParserWarningGroup>,
    pub(crate) frame:             ParseFrameKind,
    pub(crate) expected:          ExpectedSyntax,
    pub(crate) found:             Option<TokenType>,
    /// Source spelling of the found token, captured when the diagnostic is
    /// reported so messages can quote what was written.
    pub(crate) found_spelling:    Option<&'tu str>,
    /// Where a missing `;` most likely belongs, when the found token starts
    /// a new line.
    pub(crate) insertion_point:   Option<SourceVectors>,
    /// Dedicated diagnostic kind and its grammar-specific payload.
    pub(crate) error_type:        ParserErrorType<'tu>,
    /// Source segments to underline when the diagnostic is rendered.
    pub(crate) source_vectors:    SourceVectors,
    pub(crate) ranges:            &'tu mut [SourceVectors],
    pub(crate) related:           &'tu mut [RelatedParserDiagnostic],
    pub(crate) recovery:          Option<RecoverySummary>,
    /// Tokens the parser had consumed when it reported this diagnostic. Two
    /// errors at one place with no input consumed between them come from one
    /// mistake; equal locations alone can also be two uses of one macro.
    pub(crate) consumed_tokens:   usize,
    /// Physical use-site position captured before macro metadata is reclaimed.
    pub(crate) ordering_location: Option<(u32, u32)>,
}

impl ParserError<'_> {
    /// Whether a later error at the same place may be folded into this one,
    /// or this one into an earlier error. A resource limit always shows,
    /// because it explains why the rest of the input was not parsed.
    pub(crate) fn may_fold(&self) -> bool {
        !matches!(
            self.error_type,
            ParserErrorType::ResourceLimitExceeded { .. }
        )
    }

    pub(crate) fn is_empty_translation_unit(&self) -> bool {
        matches!(self.error_type, ParserErrorType::EmptyTranslationUnit)
    }

    /// Visits every provenance range this diagnostic reads from a context
    /// arena.
    pub(crate) fn for_each_source_vectors_mut(
        &mut self,
        visit: &mut impl FnMut(&mut SourceVectors),
    ) {
        visit(&mut self.source_vectors);
        if let Some(insertion_point) = &mut self.insertion_point {
            visit(insertion_point);
        }
        for range in self.ranges.iter_mut() {
            visit(range);
        }
        for related in self.related.iter_mut() {
            visit(&mut related.source_vectors);
        }
        if let Some(discarded) = self
            .recovery
            .as_mut()
            .and_then(|recovery| recovery.discarded.as_mut())
        {
            visit(discarded);
        }
    }
}

impl Display for ParserError<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.error_type)
    }
}

impl ToDiagnostic for ParserError<'_> {
    fn diagnostic_in<'d>(
        &self,
        context: &Context<'_>,
        source: SourceVectors,
        arena: &'d Bump,
    ) -> Diagnostic<'d> {
        let mut explanation = self.error_type.explain_in(arena, self.found_spelling);
        if self.insertion_point.is_some() {
            if self.error_type.expects_terminating_semicolon() {
                // The likely fix is known, so lead with it.
                explanation.message = format_in!(
                    arena,
                    "expected `;`, found {}",
                    found_token(self.found, self.found_spelling)
                );
            }
            explanation.help.clear();
        }
        if self.warning_group == Some(ParserWarningGroup::RepeatedSpecifiers) {
            explanation = explanation.note(
                "`repeated-specifiers` warnings are on by default; pass \
                 `--no-repeated-specifier-warnings` to silence them",
            );
        }
        let mut diagnostic = explanation.at(self.severity, source);
        let primary = context.get_source_vectors(source);
        for range in self.ranges.iter() {
            // Show only input skipped beyond the token the error is about.
            let skipped: &[SourceVector] = arena.alloc_slice_fill_iter(
                context
                    .get_source_vectors(*range)
                    .iter()
                    .filter(|vector| !primary.contains(vector))
                    .cloned(),
            );
            if !skipped.is_empty() {
                diagnostic = diagnostic.secondary_segments(skipped, "skipped to recover");
            }
        }
        if let Some(insertion_point) = self.insertion_point {
            diagnostic = diagnostic.secondary(insertion_point, "help: add `;` here");
        }
        // Where parsing resumes is only worth showing when it is on another
        // line than the error.
        let primary_line = primary
            .first()
            .map(|vector| (vector.source_file_index, vector.line));
        if self.insertion_point.is_some()
            || self
                .recovery
                .is_some_and(|recovery| recovery.discarded_tokens > 0)
        {
            for related in self.related.iter() {
                let related_line = context
                    .get_source_vectors(related.source_vectors)
                    .first()
                    .map(|vector| (vector.source_file_index, vector.line));
                if related_line != primary_line {
                    diagnostic = diagnostic.secondary(related.source_vectors, related.message);
                }
            }
        }
        diagnostic
    }
}

impl GetSeverity for ParserError<'_> {
    fn severity(&self) -> ErrorSeverity {
        self.severity
    }
}

impl GetPosition for ParserError<'_> {
    fn position(&self, context: &Context<'_>) -> SourcePosition {
        self.source_vectors.position(context)
    }
}

impl GetSourceVectors for ParserError<'_> {
    fn source_vectors(&self, _context: &mut Context<'_>) -> SourceVectors {
        self.source_vectors
    }
}

impl std::error::Error for ParserError<'_> {}

/// Every diagnostic produced by the language parser.
///
/// Variants accepting `Option<TokenType>` use `Some` for an unexpected token
/// and `None` for EOF, keeping token and EOF messages specific without
/// stringly-typed context. Severity and display text are exhaustively defined
/// below, so adding a new parser failure requires an explicit policy.
///
/// C99: the obligation to diagnose syntax and constraint violations is
/// §5.1.1.3, p. 11; PDF p. 23. Each variant below also cites the production,
/// constraint, or semantic rule it concerns.
#[derive(Debug, PartialEq, Clone)]
pub(crate) enum ParserErrorType<'tu> {
    /// The preprocessed token stream contained no external declaration.
    /// C99: §6.9, p. 140; PDF p. 152.
    EmptyTranslationUnit,
    /// A configured, catchable parser resource ceiling was exceeded.
    /// C99: an implementation limit, not a rule of the standard; see
    /// §5.2.4.1 and footnote 13, p. 20; PDF p. 32.
    ResourceLimitExceeded {
        resource: ParserResource,
        limit:    usize,
    },
    /// Internal invariant failure: a typed frame attempted to consume EOF.
    /// C99: implementation guard supporting §5.1.1.3, p. 11; PDF p. 23.
    ParserFrameConsumedAtEndOfInput(ParseFrameKind),
    /// A function-definition head was not followed by its compound body.
    /// C99: §6.9.1 paragraph 1, p. 141; PDF p. 153.
    ExpectedFunctionBody(Option<TokenType>),
    /// A declaration list followed a prototype-style function declarator.
    /// C99: §6.9.1 paragraph 5, p. 141; PDF p. 153.
    DeclarationListAfterParameterTypeList,
    /// A compound statement did not begin with `{`.
    /// C99: §6.8.2 paragraph 1, p. 132; PDF p. 144.
    ExpectedOpeningCurlyBraceInCompoundStatement(Option<TokenType>),
    /// A compound statement did not end with `}`.
    /// C99: §6.8.2 paragraph 1, p. 132; PDF p. 144.
    ExpectedClosingCurlyBraceInCompoundStatement(Option<TokenType>),
    /// No valid statement production began at the current token.
    /// C99: §6.8 paragraph 1, p. 131; PDF p. 143.
    ExpectedStatement(Option<TokenType>),
    /// `goto` was not followed by an identifier.
    /// C99: §6.8.6 paragraph 1, p. 136; PDF p. 148.
    ExpectedGotoLabel(Option<TokenType>),
    /// A required expression or operator was absent; the string names the
    /// position.
    /// C99: statement expressions are §6.8.1-§6.8.6, pp. 131-136;
    /// PDF pp. 143-148; operands and operators are §6.5, pp. 67-94;
    /// PDF pp. 79-106; a postfix suffix needs a postfix-expression and a
    /// compound literal its brace list (§6.5.2 paragraph 1, p. 69;
    /// PDF p. 81); an assignment's left operand is a unary-expression
    /// (§6.5.16 paragraph 1, p. 91; PDF p. 103).
    ExpectedStatementExpression(&'static str, Option<TokenType>),
    /// `.` or `->` was not followed by a member identifier.
    /// C99: `postfix-expression . identifier` is §6.5.2 paragraph 1, p. 69;
    /// PDF p. 81; member access is §6.5.2.3, pp. 72-73; PDF pp. 84-85.
    ExpectedMemberIdentifier(Option<TokenType>),
    /// A postfix subscript omitted its closing `]`.
    /// C99: §6.5.2 paragraph 1, p. 69; PDF p. 81; §6.5.2.1, p. 70;
    /// PDF p. 82.
    ExpectedClosingSquareBracketInSubscript(Option<TokenType>),
    /// An array designator omitted its closing `]`.
    /// C99: §6.7.8, p. 125; PDF p. 137.
    ExpectedClosingSquareBracketInArrayDesignator(Option<TokenType>),
    /// A brace-enclosed initializer list omitted its closing `}`.
    /// C99: §6.7.8 paragraph 1, p. 125; PDF p. 137.
    ExpectedClosingCurlyBraceInInitializerList(Option<TokenType>),
    /// A designator list omitted its required `=`.
    /// C99: `designation` is §6.7.8 paragraph 1, p. 125; PDF p. 137.
    ExpectedEqualsAfterInitializerDesignation(Option<TokenType>),
    /// A statement header omitted its opening parenthesis.
    /// C99: §6.8.4 paragraph 1, p. 133; PDF p. 145; §6.8.5 paragraph 1,
    /// p. 135; PDF p. 147.
    ExpectedOpeningParenthesisInStatement(&'static str, Option<TokenType>),
    /// A statement header, grouped expression, call, or parenthesized type
    /// name omitted its closing parenthesis.
    /// C99: §6.8.4 paragraph 1, p. 133; PDF p. 145; §6.8.5 paragraph 1,
    /// p. 135; PDF p. 147; §6.5.1-§6.5.4, pp. 69-81; PDF pp. 81-93.
    ExpectedClosingParenthesisInStatement(&'static str, Option<TokenType>),
    /// A statement omitted its owned semicolon.
    /// C99: §6.8.3 paragraph 1, p. 132; PDF p. 144; §6.8.5 paragraph 1,
    /// p. 135; PDF p. 147; §6.8.6 paragraph 1, p. 136; PDF p. 148.
    ExpectedSemicolonInStatement(&'static str, Option<TokenType>),
    /// A label or conditional expression omitted its colon.
    /// C99: §6.8.1 paragraph 1, p. 131; PDF p. 143; §6.5.15 paragraph 1,
    /// p. 90; PDF p. 102.
    ExpectedColonInLabel(&'static str, Option<TokenType>),
    /// A switch body contained more than one `default` label.
    /// C99: §6.8.4.2 paragraph 3, p. 134; PDF p. 146.
    DuplicateDefaultLabel,
    /// A `do` body was not followed by `while`.
    /// C99: §6.8.5 paragraph 1, p. 135; PDF p. 147.
    ExpectedWhileAfterDoBody(Option<TokenType>),
    /// A typedef declaration ended before naming its typedef.
    /// C99: §6.7, p. 97; PDF p. 109; §6.7.7, pp. 123-124;
    /// PDF pp. 135-136.
    ExpectedDeclaratorInTypedef(Option<TokenType>),
    /// A typedef declaration without declarators whose specifiers declare a
    /// tag or enumeration constants. It satisfies the constraint, but the
    /// `typedef` names nothing, so this is a warning, as in GCC.
    /// C99: §6.7 paragraph 2, p. 97; PDF p. 109; §6.7.7 paragraph 3, p. 123;
    /// PDF p. 135.
    TypedefDeclaresNoName,
    /// A declaration required a named declarator but none could be parsed.
    /// C99: §6.7, p. 97; PDF p. 109.
    ExpectedDeclaratorInDeclaration(Option<TokenType>),
    /// Token after a declarator was not a legal declaration continuation.
    /// C99: §6.7, p. 97; PDF p. 109; function-definition continuation is
    /// §6.9.1, p. 141; PDF p. 153.
    ExpectedDeclarationContinuationAfterDeclarator(Option<TokenType>, DeclarationContinuation),
    /// EOF occurred before the first declaration specifier.
    /// C99: §6.7, p. 97; PDF p. 109.
    UnexpectedEndBeforeDeclarationSpecifier,
    /// EOF occurred after other specifiers but before a type specifier.
    /// C99: §6.7.2 paragraph 2, p. 99; PDF p. 111.
    UnexpectedEndBeforeTypeSpecifier,
    /// Named direct declarator did not begin with an identifier or `(`.
    /// C99: §6.7.5, p. 114; PDF p. 126.
    DirectDeclaratorMustStartWithIdentifierOrOpeningParenthesis(Option<TokenType>),
    /// Parenthesized direct declarator had no nested declarator.
    /// C99: §6.7.5, p. 114; PDF p. 126.
    ExpectedDeclaratorAfterOpeningParenthesisInDirectDeclarator(Option<TokenType>),
    /// Parenthesized declarator was not closed by `)`.
    /// C99: §6.7.5, p. 114; PDF p. 126; abstract form §6.7.6,
    /// p. 122; PDF p. 134.
    ExpectedClosingParenthesisAfterParenthesizedDeclarator(Option<TokenType>),
    /// Array declarator was not closed by `]`.
    /// C99: §6.7.5, p. 114; PDF p. 126; abstract form §6.7.6,
    /// p. 122; PDF p. 134.
    ExpectedClosingSquareBracketInArrayDirectDeclarator(Option<TokenType>),
    /// EOF occurred inside a function declarator parameter list.
    /// C99: §6.7.5, p. 114; PDF p. 126.
    UnexpectedEndOfFunctionDeclaratorParameterList,
    /// K&R identifier list contained no legal non-typedef identifier.
    /// C99: identifier-list is §6.7.5, p. 114; PDF p. 126; typedef preference
    /// is §6.7.5.3 paragraph 11, p. 119; PDF p. 131.
    ExpectedIdentifierInKAndRFunctionDeclaratorParameterList(Option<TokenType>),
    /// K&R identifier was not followed by `,` or `)`.
    /// C99: identifier-list is §6.7.5, p. 114; PDF p. 126.
    ExpectedCommaOrClosingParenthesisInKAndRFunctionDeclaratorParameterList(Option<TokenType>),
    /// Prototype parameter was not followed by `,` or `)`.
    /// C99: parameter-list is §6.7.5, p. 114; PDF p. 126.
    ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(Option<TokenType>),
    /// A prototype comma was not followed by a parameter or `...`.
    /// C99: parameter-type-list and parameter-list are §6.7.5,
    /// p. 114; PDF p. 126.
    ExpectedParameterDeclarationAfterCommaInFunctionDeclarator(Option<TokenType>),
    /// A function-call argument was not followed by `,` or `)`.
    /// C99: argument-expression-list is §6.5.2, p. 70; PDF p. 82.
    ExpectedCommaOrClosingParenthesisInFunctionCall(Option<TokenType>),
    /// Struct/union child was entered without its owning keyword.
    /// C99: §6.7.2.1, p. 101; PDF p. 113.
    ExpectedStructOrUnionKeyword(Option<TokenType>),
    /// Struct/union specifier had neither a tag nor a body.
    /// C99: §6.7.2.1, p. 101; PDF p. 113.
    StructOrUnionSpecifierWithoutNameAndBody(Option<TokenType>),
    /// Struct member list was not closed by `}`.
    /// C99: §6.7.2.1, p. 101; PDF p. 113.
    ExpectedClosingCurlyBraceInStructDeclarationList(Option<TokenType>),
    /// A struct or union definition had no member declaration.
    /// C99: struct-declaration-list is nonempty in §6.7.2.1, p. 101;
    /// PDF p. 113.
    ExpectedStructDeclarationBeforeClosingCurlyBrace,
    /// Struct member declaration reached `}` without its semicolon.
    /// C99: struct-declaration is §6.7.2.1, p. 101; PDF p. 113.
    ExpectedSemicolonBeforeClosingCurlyBraceInStructDeclaratorList,
    /// Struct declarator was not followed by `,` or `;`.
    /// C99: struct-declarator-list and struct-declaration are §6.7.2.1,
    /// p. 101; PDF p. 113.
    ExpectedCommaOrSemicolonInStructDeclaratorList(Option<TokenType>),
    /// Enum child was entered without its owning keyword.
    /// C99: §6.7.2.2, p. 105; PDF p. 117.
    ExpectedEnumKeyword(Option<TokenType>),
    /// Enum specifier had neither a tag nor a body.
    /// C99: §6.7.2.2, p. 105; PDF p. 117.
    EnumSpecifierWithoutNameAndBody(Option<TokenType>),
    /// Enumerator list expected a name or its closing brace.
    /// C99: §6.7.2.2, p. 105; PDF p. 117.
    ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(Option<TokenType>),
    /// An enum definition had no enumerator.
    /// C99: enumerator-list is nonempty in §6.7.2.2, p. 105; PDF p. 117.
    ExpectedEnumeratorBeforeClosingCurlyBrace,
    /// Enumerator was not followed by `,` or `}`.
    /// C99: §6.7.2.2, p. 105; PDF p. 117.
    ExpectedCommaOrClosingCurlyInEnumeratorList(Option<TokenType>),
    /// Declaration specified more than one storage class.
    /// C99: §6.7.1 paragraph 2, p. 98; PDF p. 110.
    StorageClassRedefinition(StorageClass, TokenType),
    /// A declaration-only specifier appeared in a specifier-qualifier list.
    /// C99: struct member specifier-qualifier-list is §6.7.2.1, p. 101;
    /// PDF p. 113, and type-name is §6.7.6, p. 122; PDF p. 134.
    DeclarationSpecifierNotAllowedHere(TokenType),
    /// `const` occurred more than once in one qualifier sequence.
    /// C99: §6.7.3 paragraph 4, p. 108; PDF p. 120 says repetition has the
    /// same behavior as one occurrence; this diagnostic is therefore a warning.
    ConstSpecifiedTwice,
    /// `volatile` occurred more than once in one qualifier sequence.
    /// C99: §6.7.3 paragraph 4, p. 108; PDF p. 120 says repetition has the
    /// same behavior as one occurrence; this diagnostic is therefore a warning.
    VolatileSpecifiedTwice,
    /// `restrict` occurred more than once in one qualifier sequence.
    /// C99: §6.7.3 paragraph 4, p. 108; PDF p. 120 says repetition has the
    /// same behavior as one occurrence; this diagnostic is therefore a warning.
    RestrictSpecifiedTwice,
    /// `inline` occurred more than once in declaration specifiers.
    /// C99: inline is specified by §6.7.4, p. 112; PDF p. 124. The warning is
    /// an implementation quality diagnostic, not a required C99 diagnostic.
    InlineSpecifiedTwice,
    /// `static` occurred more than once in an array declarator.
    /// C99: §6.7.5 and §6.7.5.2, pp. 114 and 116-117;
    /// PDF pp. 126 and 128-129.
    StaticSpecifiedTwice,
    /// Array qualifiers appeared on both sides of `static`.
    /// C99: §6.7.5, p. 114; PDF p. 126.
    TypeQualifiersBothBeforeAndAfterStaticInArrayDirectDeclarator,
    /// A new type specifier conflicts with the already accumulated type.
    /// C99: the permitted specifier sets are §6.7.2 paragraph 2,
    /// pp. 99-100; PDF pp. 111-112. Both fields are user-facing spellings
    /// resolved when the diagnostic is reported.
    ConflictingTypeSpecifiers {
        existing:    &'tu str,
        conflicting: &'tu str,
    },
    /// A type keyword was repeated where no repetition is legal.
    /// C99: the permitted specifier sets are §6.7.2 paragraph 2,
    /// pp. 99-100; PDF pp. 111-112.
    TypeSpecifierSpecifiedTwice(TokenType),
    /// `_Imaginary` is reserved but is not a normative C99 type specifier.
    /// C99: keyword inventory §6.4.1, p. 50; PDF p. 62; type-specifiers
    /// §6.7.2, pp. 99-100; PDF pp. 111-112. §6.4.1 paragraph 2 and
    /// footnote 59, p. 50; PDF p. 62 reserve it for imaginary types, which
    /// only the informative Annex G describes; bcc-rust does not implement
    /// them.
    UnsupportedImaginaryTypeSpecifier,
    /// More than two `long` keywords occurred in one type.
    /// C99: the permitted specifier sets are §6.7.2 paragraph 2,
    /// pp. 99-100; PDF pp. 111-112.
    LongSpecifiedThrice,
    /// `double` was combined with `long long`.
    /// C99: the permitted specifier sets are §6.7.2 paragraph 2,
    /// pp. 99-100; PDF pp. 111-112.
    LongLongDoubleSpecified,
    /// Declaration-specifier parsing started on a non-specifier token.
    /// C99: declaration-specifiers are §6.7, p. 97; PDF p. 109.
    EmptyDeclarationSpecifiers(TokenType),
    /// A specifier sequence ended without a C99 type specifier.
    /// C99: §6.7.2 paragraph 2, p. 99; PDF p. 111.
    NoTypeSpecifiersInDeclarationSpecifiers(TokenType),
    /// An identifier that is not a visible typedef-name stood in the type
    /// specifier slot, directly before another declarator.
    /// C99: typedef-name is §6.7.7, pp. 123-124; PDF pp. 135-136.
    UnknownTypeName,
    /// `_Complex` appeared without `float`, `double`, or `long double`.
    /// C99: the permitted specifier sets are §6.7.2 paragraph 2,
    /// pp. 99-100; PDF pp. 111-112.
    IncompleteComplexTypeSpecifier,
    /// An array declarator combined `static` and the `*` form.
    /// C99: the array alternatives are distinct productions in §6.7.5,
    /// p. 114; PDF p. 126.
    BothStaticAndPointerInArrayDirectDeclarator,
    /// `static` array syntax omitted its required assignment expression.
    /// C99: §6.7.5, p. 114; PDF p. 126.
    ExpectedAssignmentExpressionAfterStaticInArrayDirectDeclarator,
    /// A token other than `]` followed `*` in an array declarator.
    /// C99: §6.7.5, p. 114; PDF p. 126; abstract form §6.7.6,
    /// p. 122; PDF p. 134.
    ExpectedClosingSquareBracketAfterPointerInArrayDirectDeclarator(TokenType),
    /// EOF followed `*` in an array declarator.
    /// C99: §6.7.5, p. 114; PDF p. 126; abstract form §6.7.6,
    /// p. 122; PDF p. 134.
    UnexpectedEndOfArrayDeclaratorAfterPointer,
    /// The `*` variable-length-array marker occurred more than once.
    /// C99: §6.7.5, p. 114; PDF p. 126; abstract form §6.7.6,
    /// p. 122; PDF p. 134.
    PointerSpecifiedTwice,
    /// Pointer qualifiers were present without a direct declarator.
    /// C99: named declarator requires direct-declarator under §6.7.5,
    /// p. 114; PDF p. 126.
    TypeQualifiersWithoutDeclarator,
    /// Abstract array qualifiers illegally preceded the `*` marker.
    /// C99: direct-abstract-declarator permits `[*]`, not a qualified `*`,
    /// under §6.7.6, p. 122; PDF p. 134.
    TypeQualifiersBeforePointerInArrayAbstractDirectDeclarator,
    /// One function suffix mixed K&R names with prototype declarations.
    /// C99: function suffix alternatives are distinct productions in §6.7.5,
    /// p. 114; PDF p. 126.
    KAndRFunctionDeclaratorMixedWithModernDeclarator,
    /// A token other than `)` followed `...`.
    /// C99: parameter-type-list ends in `, ...` under §6.7.5,
    /// p. 114; PDF p. 126.
    ExpectedClosingParenthesisAfterEllipsisInFunctionDeclaratorParameterList(TokenType),
    /// EOF followed `...` before its closing parenthesis.
    /// C99: parameter-type-list ends in `, ...` under §6.7.5,
    /// p. 114; PDF p. 126.
    UnexpectedEndOfVariadicFunctionDeclaratorParameterList,
    /// Struct member declaration contained no declarator or bit-field.
    /// C99: struct-declarator-list is nonempty under §6.7.2.1,
    /// p. 101; PDF p. 113.
    EmptyStructDeclarator,
}

impl GetSeverity for ParserErrorType<'_> {
    fn severity(&self) -> ErrorSeverity {
        match self {
            | Self::EmptyTranslationUnit
            | Self::ResourceLimitExceeded { .. }
            | Self::ParserFrameConsumedAtEndOfInput(..)
            | Self::ExpectedFunctionBody(..)
            | Self::DeclarationListAfterParameterTypeList
            | Self::ExpectedOpeningCurlyBraceInCompoundStatement(..)
            | Self::ExpectedClosingCurlyBraceInCompoundStatement(..)
            | Self::ExpectedStatement(..)
            | Self::ExpectedGotoLabel(..)
            | Self::ExpectedStatementExpression(..)
            | Self::ExpectedMemberIdentifier(..)
            | Self::ExpectedClosingSquareBracketInSubscript(..)
            | Self::ExpectedClosingSquareBracketInArrayDesignator(..)
            | Self::ExpectedClosingCurlyBraceInInitializerList(..)
            | Self::ExpectedEqualsAfterInitializerDesignation(..)
            | Self::ExpectedOpeningParenthesisInStatement(..)
            | Self::ExpectedClosingParenthesisInStatement(..)
            | Self::ExpectedSemicolonInStatement(..)
            | Self::ExpectedColonInLabel(..)
            | Self::DuplicateDefaultLabel
            | Self::ExpectedWhileAfterDoBody(..)
            | Self::ExpectedDeclaratorInTypedef(..)
            | Self::ExpectedDeclaratorInDeclaration(..)
            | Self::ExpectedDeclarationContinuationAfterDeclarator(..)
            | Self::UnexpectedEndBeforeDeclarationSpecifier
            | Self::UnexpectedEndBeforeTypeSpecifier
            | Self::DirectDeclaratorMustStartWithIdentifierOrOpeningParenthesis(..)
            | Self::ExpectedDeclaratorAfterOpeningParenthesisInDirectDeclarator(..)
            | Self::ExpectedClosingParenthesisAfterParenthesizedDeclarator(..)
            | Self::ExpectedClosingSquareBracketInArrayDirectDeclarator(..)
            | Self::UnexpectedEndOfFunctionDeclaratorParameterList
            | Self::ExpectedIdentifierInKAndRFunctionDeclaratorParameterList(..)
            | Self::ExpectedCommaOrClosingParenthesisInKAndRFunctionDeclaratorParameterList(..)
            | Self::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(..)
            | Self::ExpectedParameterDeclarationAfterCommaInFunctionDeclarator(..)
            | Self::ExpectedCommaOrClosingParenthesisInFunctionCall(..)
            | Self::ExpectedStructOrUnionKeyword(..)
            | Self::StructOrUnionSpecifierWithoutNameAndBody(..)
            | Self::ExpectedClosingCurlyBraceInStructDeclarationList(..)
            | Self::ExpectedStructDeclarationBeforeClosingCurlyBrace
            | Self::ExpectedSemicolonBeforeClosingCurlyBraceInStructDeclaratorList
            | Self::ExpectedCommaOrSemicolonInStructDeclaratorList(..)
            | Self::ExpectedEnumKeyword(..)
            | Self::EnumSpecifierWithoutNameAndBody(..)
            | Self::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(..)
            | Self::ExpectedEnumeratorBeforeClosingCurlyBrace
            | Self::ExpectedCommaOrClosingCurlyInEnumeratorList(..)
            | Self::EmptyDeclarationSpecifiers(..)
            | Self::NoTypeSpecifiersInDeclarationSpecifiers(..)
            | Self::UnknownTypeName
            | Self::IncompleteComplexTypeSpecifier
            | Self::TypeQualifiersBothBeforeAndAfterStaticInArrayDirectDeclarator
            | Self::BothStaticAndPointerInArrayDirectDeclarator
            | Self::ExpectedAssignmentExpressionAfterStaticInArrayDirectDeclarator
            | Self::UnexpectedEndOfArrayDeclaratorAfterPointer
            | Self::TypeQualifiersWithoutDeclarator
            | Self::ExpectedClosingParenthesisAfterEllipsisInFunctionDeclaratorParameterList(
                ..,
            )
            | Self::UnexpectedEndOfVariadicFunctionDeclaratorParameterList
            | Self::TypeQualifiersBeforePointerInArrayAbstractDirectDeclarator
            | Self::KAndRFunctionDeclaratorMixedWithModernDeclarator
            | Self::EmptyStructDeclarator
            | Self::PointerSpecifiedTwice
            | Self::ExpectedClosingSquareBracketAfterPointerInArrayDirectDeclarator(..)
            | Self::UnsupportedImaginaryTypeSpecifier
            | Self::DeclarationSpecifierNotAllowedHere(..)
            | Self::StorageClassRedefinition(..)
            | Self::StaticSpecifiedTwice
            | Self::ConflictingTypeSpecifiers { .. }
            | Self::TypeSpecifierSpecifiedTwice(..)
            | Self::LongSpecifiedThrice
            | Self::LongLongDoubleSpecified => ErrorSeverity::Error,
            | Self::ConstSpecifiedTwice
            | Self::VolatileSpecifiedTwice
            | Self::RestrictSpecifiedTwice
            | Self::InlineSpecifiedTwice
            | Self::TypedefDeclaresNoName => ErrorSeverity::Warning,
        }
    }
}

impl ParserResource {
    fn description(self) -> &'static str {
        match self {
            | Self::ExternalDeclarations => "external declaration",
            | Self::SyntaxNodes => "syntax node",
            | Self::FrameDepth => "nesting depth",
            | Self::SourceSegments => "source segment",
        }
    }
}

/// Describes where a statement-level error occurred, quoting the keyword of
/// statement names such as "if statement".
fn statement_position(position: &str) -> impl Display {
    fmt::from_fn(move |f| match position.split_once(' ') {
        | Some((
            keyword @ ("if" | "while" | "for" | "do" | "switch" | "return" | "goto" | "case"),
            rest,
        )) => write!(f, "`{keyword}` {rest}"),
        | _ => f.write_str(position),
    })
}

/// The spelling of a keyword token, or a description of any other token.
fn token_spelling(token: TokenType) -> impl Display {
    fmt::from_fn(move |f| match token {
        | TokenType::Keyword(keyword) => f.write_str(keyword.spelling()),
        | TokenType::Operator(operator) => f.write_str(operator.spelling()),
        | other => write!(f, "{}", other.found(None)),
    })
}

/// Lists alternatives as "a or b" or "a, b, or c".
fn alternatives<'a>(items: &'a [&'a str]) -> impl Display {
    fmt::from_fn(move |f| match items {
        | [first, second] => write!(f, "{first} or {second}"),
        | [rest @ .., last] => {
            for (index, item) in rest.iter().enumerate() {
                if index > 0 {
                    f.write_str(", ")?;
                }
                f.write_str(item)?;
            }
            write!(f, ", or {last}")
        },
        | [] => Ok(()),
    })
}

/// Describes a found token, or the end of the file when there is none.
fn found_token(token: Option<TokenType>, spelling: Option<&str>) -> impl Display {
    fmt::from_fn(move |f| match token {
        | Some(token) => write!(f, "{}", token.found(spelling)),
        | None => f.write_str("end of file"),
    })
}

const SPECIFIER_COMBINATIONS_NOTE: &str =
    "C99 §6.7.2p2 lists every valid combination of type specifiers";

impl ParserErrorType<'_> {
    /// Classifies the diagnostic: `Constraint` for a rule stated in a
    /// Constraints paragraph, `Syntax` for a grammar violation, both required
    /// by §5.1.1.3 paragraph 1, p. 11; PDF p. 23.
    pub(super) fn code(&self) -> ParserDiagnosticCode {
        match self {
            | Self::ResourceLimitExceeded { .. } => ParserDiagnosticCode::ResourceLimit,
            | Self::ParserFrameConsumedAtEndOfInput(..) => ParserDiagnosticCode::InternalInvariant,
            | Self::ConstSpecifiedTwice
            | Self::VolatileSpecifiedTwice
            | Self::RestrictSpecifiedTwice
            | Self::InlineSpecifiedTwice
            | Self::TypedefDeclaresNoName => ParserDiagnosticCode::Quality,
            | Self::StorageClassRedefinition(..)
            | Self::StaticSpecifiedTwice
            | Self::TypeQualifiersBothBeforeAndAfterStaticInArrayDirectDeclarator
            | Self::ConflictingTypeSpecifiers { .. }
            | Self::TypeSpecifierSpecifiedTwice(..)
            | Self::LongSpecifiedThrice
            | Self::LongLongDoubleSpecified
            | Self::IncompleteComplexTypeSpecifier
            | Self::BothStaticAndPointerInArrayDirectDeclarator
            | Self::PointerSpecifiedTwice
            | Self::TypeQualifiersBeforePointerInArrayAbstractDirectDeclarator
            | Self::KAndRFunctionDeclaratorMixedWithModernDeclarator
            | Self::DeclarationListAfterParameterTypeList
            | Self::DuplicateDefaultLabel => ParserDiagnosticCode::Constraint,
            | _ => ParserDiagnosticCode::Syntax,
        }
    }

    /// Whether the error is a missing declaration or statement terminator.
    pub(super) fn expects_terminating_semicolon(&self) -> bool {
        matches!(
            self,
            Self::ExpectedDeclarationContinuationAfterDeclarator(Some(_), _)
                | Self::ExpectedSemicolonInStatement(_, Some(_))
                | Self::ExpectedStatementExpression("operator in expression", Some(_))
                | Self::ExpectedCommaOrSemicolonInStructDeclaratorList(Some(_))
        )
    }

    /// Constraint errors on specifier combinations leave the parsed syntax
    /// exactly as written, so they do not mark the enclosing construct as
    /// recovered.
    pub(super) fn leaves_syntax_intact(&self) -> bool {
        matches!(
            self,
            Self::StorageClassRedefinition(..)
                | Self::StaticSpecifiedTwice
                | Self::ConflictingTypeSpecifiers { .. }
                | Self::TypeSpecifierSpecifiedTwice(..)
                | Self::LongSpecifiedThrice
                | Self::LongLongDoubleSpecified
                | Self::IncompleteComplexTypeSpecifier
        )
    }

    pub(super) fn warning_group(&self) -> Option<ParserWarningGroup> {
        match self {
            | Self::ConstSpecifiedTwice
            | Self::VolatileSpecifiedTwice
            | Self::RestrictSpecifiedTwice
            | Self::InlineSpecifiedTwice => Some(ParserWarningGroup::RepeatedSpecifiers),
            | _ => None,
        }
    }

    pub(super) fn expected_syntax(&self) -> ExpectedSyntax {
        match self {
            | Self::EmptyTranslationUnit => ExpectedSyntax::ExternalDeclaration,
            | Self::UnexpectedEndBeforeDeclarationSpecifier
            | Self::EmptyDeclarationSpecifiers(..) => ExpectedSyntax::DeclarationSpecifier,
            | Self::UnexpectedEndBeforeTypeSpecifier
            | Self::NoTypeSpecifiersInDeclarationSpecifiers(..)
            | Self::UnknownTypeName => ExpectedSyntax::TypeSpecifier,
            | Self::ExpectedDeclaratorInTypedef(..)
            | Self::ExpectedDeclaratorInDeclaration(..)
            | Self::DirectDeclaratorMustStartWithIdentifierOrOpeningParenthesis(..)
            | Self::ExpectedDeclaratorAfterOpeningParenthesisInDirectDeclarator(..) =>
                ExpectedSyntax::Declarator,
            | Self::ExpectedStatement(..) => ExpectedSyntax::Statement,
            | Self::ExpectedStatementExpression(..)
            | Self::ExpectedAssignmentExpressionAfterStaticInArrayDirectDeclarator =>
                ExpectedSyntax::Expression,
            | Self::ExpectedGotoLabel(..)
            | Self::ExpectedMemberIdentifier(..)
            | Self::ExpectedIdentifierInKAndRFunctionDeclaratorParameterList(..)
            | Self::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(..) =>
                ExpectedSyntax::Identifier,
            | Self::ExpectedDeclarationContinuationAfterDeclarator(..) =>
                ExpectedSyntax::DeclarationContinuation,
            | Self::ExpectedCommaOrClosingParenthesisInKAndRFunctionDeclaratorParameterList(..)
            | Self::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(..)
            | Self::ExpectedParameterDeclarationAfterCommaInFunctionDeclarator(..)
            | Self::ExpectedCommaOrClosingParenthesisInFunctionCall(..)
            | Self::ExpectedCommaOrSemicolonInStructDeclaratorList(..)
            | Self::ExpectedCommaOrClosingCurlyInEnumeratorList(..) =>
                ExpectedSyntax::SeparatorOrCloser,
            | Self::ExpectedFunctionBody(..)
            | Self::ExpectedOpeningCurlyBraceInCompoundStatement(..)
            | Self::ExpectedClosingCurlyBraceInCompoundStatement(..)
            | Self::ExpectedClosingSquareBracketInSubscript(..)
            | Self::ExpectedClosingSquareBracketInArrayDesignator(..)
            | Self::ExpectedClosingCurlyBraceInInitializerList(..)
            | Self::ExpectedEqualsAfterInitializerDesignation(..)
            | Self::ExpectedOpeningParenthesisInStatement(..)
            | Self::ExpectedClosingParenthesisInStatement(..)
            | Self::ExpectedSemicolonInStatement(..)
            | Self::ExpectedColonInLabel(..)
            | Self::ExpectedWhileAfterDoBody(..)
            | Self::ExpectedClosingParenthesisAfterParenthesizedDeclarator(..)
            | Self::ExpectedClosingSquareBracketInArrayDirectDeclarator(..)
            | Self::UnexpectedEndOfFunctionDeclaratorParameterList
            | Self::ExpectedClosingCurlyBraceInStructDeclarationList(..)
            | Self::ExpectedSemicolonBeforeClosingCurlyBraceInStructDeclaratorList
            | Self::ExpectedClosingSquareBracketAfterPointerInArrayDirectDeclarator(..)
            | Self::UnexpectedEndOfArrayDeclaratorAfterPointer
            | Self::ExpectedClosingParenthesisAfterEllipsisInFunctionDeclaratorParameterList(
                ..,
            )
            | Self::UnexpectedEndOfVariadicFunctionDeclaratorParameterList =>
                ExpectedSyntax::OwnedDelimiter,
            | Self::ResourceLimitExceeded { .. }
            | Self::ParserFrameConsumedAtEndOfInput(..)
            | Self::DeclarationListAfterParameterTypeList
            | Self::ExpectedStructOrUnionKeyword(..)
            | Self::StructOrUnionSpecifierWithoutNameAndBody(..)
            | Self::ExpectedStructDeclarationBeforeClosingCurlyBrace
            | Self::ExpectedEnumKeyword(..)
            | Self::EnumSpecifierWithoutNameAndBody(..)
            | Self::ExpectedEnumeratorBeforeClosingCurlyBrace
            | Self::StorageClassRedefinition(..)
            | Self::DeclarationSpecifierNotAllowedHere(..)
            | Self::ConstSpecifiedTwice
            | Self::VolatileSpecifiedTwice
            | Self::RestrictSpecifiedTwice
            | Self::InlineSpecifiedTwice
            | Self::StaticSpecifiedTwice
            | Self::TypeQualifiersBothBeforeAndAfterStaticInArrayDirectDeclarator
            | Self::ConflictingTypeSpecifiers { .. }
            | Self::TypeSpecifierSpecifiedTwice(..)
            | Self::UnsupportedImaginaryTypeSpecifier
            | Self::LongSpecifiedThrice
            | Self::LongLongDoubleSpecified
            | Self::IncompleteComplexTypeSpecifier
            | Self::BothStaticAndPointerInArrayDirectDeclarator
            | Self::PointerSpecifiedTwice
            | Self::TypeQualifiersWithoutDeclarator
            | Self::TypeQualifiersBeforePointerInArrayAbstractDirectDeclarator
            | Self::KAndRFunctionDeclaratorMixedWithModernDeclarator
            | Self::EmptyStructDeclarator
            | Self::TypedefDeclaresNoName
            | Self::DuplicateDefaultLabel => ExpectedSyntax::None,
        }
    }

    /// Describes the error; `spelling` is the source spelling of the token
    /// the parser found, when there was one.
    pub(crate) fn explain_in<'d>(
        &self,
        arena: &'d Bump,
        spelling: Option<&str>,
    ) -> Explanation<'d> {
        let new = |message: &'d str| Explanation::new(arena, message);
        let found = |token: Option<TokenType>| found_token(token, spelling);
        // "expected X, found Y", labelled with what was expected.
        let expected = |what: &str, token: Option<TokenType>| {
            new(format_in!(arena, "expected {what}, found {}", found(token)))
                .label(format_in!(arena, "expected {what}"))
        };
        let expected_with_label = |what: &str, label: &'d str, token: Option<TokenType>| {
            new(format_in!(arena, "expected {what}, found {}", found(token))).label(label)
        };
        let missing_semicolon_help = |explanation: Explanation<'d>, token: Option<TokenType>| {
            if matches!(token, Some(TokenType::Keyword(_) | TokenType::Identifier)) {
                explanation.help("if this starts a new declaration, add `;` before it")
            } else {
                explanation
            }
        };
        match self {
            | Self::EmptyTranslationUnit => new("translation unit is empty")
                .label("expected a declaration or function definition")
                .note("C99 §6.9: a translation unit contains at least one external declaration"),
            | Self::ResourceLimitExceeded { resource, limit } => new(format_in!(
                arena,
                "input exceeds the parser's {} limit of {limit}",
                resource.description()
            ))
            .label("parsing stopped here")
            .note("bcc bounds parser memory and nesting; split or simplify this input"),
            | Self::ParserFrameConsumedAtEndOfInput(frame) => new(format_in!(
                arena,
                "internal compiler error: the {} parser read past the end of input",
                frame.label()
            ))
            .note("this is a bug in bcc; please report it with the input that triggered it"),
            | Self::ExpectedFunctionBody(token) =>
                expected_with_label("a function body", "expected `{`", *token),
            | Self::DeclarationListAfterParameterTypeList =>
                new("parameter declarations after a prototype")
                    .label("old-style parameter declarations need an identifier list")
                    .note(
                        "C99 §6.9.1p5-6: declarations between `)` and `{` are only allowed when \
                         the declarator lists bare parameter names",
                    ),
            | Self::ExpectedOpeningCurlyBraceInCompoundStatement(token) => expected("`{`", *token),
            | Self::ExpectedClosingCurlyBraceInCompoundStatement(token) => expected("`}`", *token),
            | Self::ExpectedStatement(token) => expected("a statement", *token),
            | Self::ExpectedGotoLabel(token) => expected_with_label(
                "a label name after `goto`",
                "expected an identifier",
                *token,
            ),
            | Self::ExpectedStatementExpression("operator in expression", token) =>
                missing_semicolon_help(
                    expected_with_label(
                        "an operator",
                        "expected an operator or the end of the expression",
                        *token,
                    ),
                    *token,
                ),
            | Self::ExpectedStatementExpression("expression operand", token) =>
                expected("an expression", *token),
            | Self::ExpectedStatementExpression("operator before brace list", token) =>
                expected_with_label(
                    "an operator",
                    "a brace list follows only a parenthesized type name",
                    *token,
                )
                .note("C99 §6.5.2.5: a compound literal is `( type-name ) { initializer-list }`"),
            | Self::ExpectedStatementExpression("operator in delimited expression", token) =>
                expected_with_label(
                    "an operator",
                    "expected an operator or the closing delimiter",
                    *token,
                ),
            | Self::ExpectedStatementExpression(
                "postfix operator after a non-postfix expression",
                token,
            ) => new(format_in!(
                arena,
                "postfix {} cannot follow a cast, `sizeof`, or unary expression",
                found(*token)
            ))
            .label("add parentheses around the expression it applies to")
            .note("C99 §6.5.2: a postfix operator applies only to a postfix-expression"),
            | Self::ExpectedStatementExpression("compound literal initializer", token) =>
                expected_with_label(
                    "`{` to begin a compound literal after the parenthesized type name",
                    "expected `{`",
                    *token,
                )
                .note(
                    "C99 §6.5.3p1: the operand of `++`, `--`, or `sizeof` is a unary-expression, \
                     so a cast cannot appear there",
                ),
            | Self::ExpectedStatementExpression(
                "unary-expression left operand of assignment",
                _,
            ) => new("invalid left-hand side of assignment")
                .label("cannot assign to this expression")
                .note(
                    "C99 §6.5.16: the left operand of an assignment operator is a unary \
                     expression, such as a name, `*p`, `a[i]`, or `s.m`",
                ),
            | Self::ExpectedStatementExpression(position, token) => expected_with_label(
                format_in!(arena, "an expression in {}", statement_position(position)),
                "expected an expression",
                *token,
            ),
            | Self::ExpectedMemberIdentifier(token) => expected_with_label(
                "a member name after `.` or `->`",
                "expected an identifier",
                *token,
            ),
            | Self::ExpectedClosingSquareBracketInSubscript(token) =>
                expected_with_label("`]` to close the subscript", "expected `]`", *token),
            | Self::ExpectedClosingSquareBracketInArrayDesignator(token) =>
                expected_with_label("`]` to close the array designator", "expected `]`", *token),
            | Self::ExpectedClosingCurlyBraceInInitializerList(token) => expected_with_label(
                "`,` or `}` in the initializer list",
                "expected `,` or `}`",
                *token,
            ),
            | Self::ExpectedEqualsAfterInitializerDesignation(token) =>
                expected_with_label("`=` after the designator", "expected `=`", *token).note(
                    "C99 §6.7.8: a designation is written `[index] = value` or `.member = value`",
                ),
            | Self::ExpectedOpeningParenthesisInStatement(position, token) => expected_with_label(
                format_in!(arena, "`(` in {}", statement_position(position)),
                "expected `(`",
                *token,
            ),
            | Self::ExpectedClosingParenthesisInStatement(position, token) => expected_with_label(
                format_in!(arena, "`)` in {}", statement_position(position)),
                "expected `)`",
                *token,
            ),
            | Self::ExpectedSemicolonInStatement(position, token) => missing_semicolon_help(
                expected_with_label(
                    format_in!(arena, "`;` after {}", statement_position(position)),
                    "expected `;`",
                    *token,
                ),
                *token,
            ),
            | Self::ExpectedColonInLabel(position, token) => expected_with_label(
                format_in!(arena, "`:` after {}", statement_position(position)),
                "expected `:`",
                *token,
            ),
            | Self::UnsupportedImaginaryTypeSpecifier => new("`_Imaginary` is not supported")
                .label("imaginary types are not implemented")
                .note(
                    "C99 §6.4.1p2 reserves `_Imaginary`; imaginary types are only defined by the \
                     optional Annex G (footnote 59)",
                ),
            | Self::DuplicateDefaultLabel => new("multiple `default` labels in one `switch`")
                .label("second `default` label")
                .note("C99 §6.8.4.2p3: a `switch` body has at most one `default` label"),
            | Self::ExpectedWhileAfterDoBody(token) =>
                expected_with_label("`while` after the `do` body", "expected `while`", *token)
                    .note("C99 §6.8.5: write `do statement while (condition);`"),
            | Self::ExpectedDeclaratorInTypedef(token) =>
                expected_with_label("a name for the typedef", "expected an identifier", *token)
                    .note("a `typedef` declaration must name the type it defines"),
            | Self::TypedefDeclaresNoName => new("`typedef` declares no type name")
                .label("no name follows the type")
                .note("C99 §6.7p2: the declaration still declares its tag or enumeration constants")
                .help("name the type before `;`, or remove `typedef`"),
            | Self::ExpectedDeclaratorInDeclaration(token) =>
                expected_with_label("a declarator", "expected a name to declare", *token),
            | Self::ExpectedDeclarationContinuationAfterDeclarator(token, continuation) => {
                let (what, label) = continuation.expected(arena);
                let explanation = expected_with_label(what, label, *token);
                let explanation = match continuation.function_body_note(*token) {
                    | Some(note) => explanation.note(note),
                    | None => explanation,
                };
                missing_semicolon_help(explanation, *token)
            },
            | Self::UnexpectedEndBeforeDeclarationSpecifier =>
                expected("declaration specifiers", None),
            | Self::UnexpectedEndBeforeTypeSpecifier => expected("a type specifier", None),
            | Self::DirectDeclaratorMustStartWithIdentifierOrOpeningParenthesis(token) => {
                let explanation = expected_with_label(
                    "an identifier or `(` in the declarator",
                    "expected a name",
                    *token,
                );
                match token {
                    | Some(TokenType::Keyword(keyword)) => explanation.note(format_in!(
                        arena,
                        "`{}` is a keyword and cannot be used as a name",
                        keyword.spelling()
                    )),
                    | _ => explanation,
                }
            },
            | Self::ExpectedDeclaratorAfterOpeningParenthesisInDirectDeclarator(token) =>
                expected_with_label("a declarator after `(`", "expected a declarator", *token),
            | Self::ExpectedClosingParenthesisAfterParenthesizedDeclarator(token) =>
                expected_with_label("`)` to close the declarator", "expected `)`", *token),
            | Self::ExpectedClosingSquareBracketInArrayDirectDeclarator(token) =>
                expected_with_label("`]` to close the array declarator", "expected `]`", *token),
            | Self::UnexpectedEndOfFunctionDeclaratorParameterList =>
                expected_with_label("`)` to close the parameter list", "expected `)`", None),
            | Self::ExpectedIdentifierInKAndRFunctionDeclaratorParameterList(token) =>
                expected_with_label(
                    "a parameter name in the identifier list",
                    "expected an identifier",
                    *token,
                )
                .note(
                    "C99 §6.7.5: an old-style identifier list contains only parameter names \
                     separated by commas",
                ),
            | Self::ExpectedCommaOrClosingParenthesisInKAndRFunctionDeclaratorParameterList(
                token,
            ) => expected_with_label(
                "`,` or `)` after the parameter name",
                "expected `,` or `)`",
                *token,
            ),
            | Self::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(token) =>
                expected_with_label(
                    "`,` or `)` after the parameter",
                    "expected `,` or `)`",
                    *token,
                ),
            | Self::ExpectedParameterDeclarationAfterCommaInFunctionDeclarator(token) =>
                expected_with_label(
                    "a parameter declaration or `...` after `,`",
                    "expected a parameter",
                    *token,
                ),
            | Self::ExpectedCommaOrClosingParenthesisInFunctionCall(token) => expected_with_label(
                "`,` or `)` after the argument",
                "expected `,` or `)`",
                *token,
            ),
            | Self::ExpectedStructOrUnionKeyword(token) => expected("`struct` or `union`", *token),
            | Self::StructOrUnionSpecifierWithoutNameAndBody(token) => expected_with_label(
                "a tag name or `{` after `struct` or `union`",
                "expected a tag name or `{`",
                *token,
            ),
            | Self::ExpectedClosingCurlyBraceInStructDeclarationList(token) =>
                expected_with_label("`}` to close the member list", "expected `}`", *token),
            | Self::ExpectedStructDeclarationBeforeClosingCurlyBrace =>
                new("struct or union has no members")
                    .label("expected a member declaration before `}`")
                    .note(
                        "C99 §6.7.2.1: the member list of a struct or union contains at least one \
                         declaration",
                    ),
            | Self::ExpectedSemicolonBeforeClosingCurlyBraceInStructDeclaratorList =>
                new("expected `;` after the last member declaration, found `}`")
                    .label("expected `;`")
                    .note(
                        "C99 §6.7.2.1: every member declaration ends with `;`, including the last",
                    ),
            | Self::ExpectedCommaOrSemicolonInStructDeclaratorList(token) => expected_with_label(
                "`,` or `;` after the member declarator",
                "expected `,` or `;`",
                *token,
            ),
            | Self::ExpectedEnumKeyword(token) => expected("`enum`", *token),
            | Self::EnumSpecifierWithoutNameAndBody(token) => expected_with_label(
                "a tag name or `{` after `enum`",
                "expected a tag name or `{`",
                *token,
            ),
            | Self::ExpectedEnumerationConstantOrClosingCurlyInEnumeratorList(token) =>
                expected_with_label(
                    "an enumerator name or `}`",
                    "expected an identifier or `}`",
                    *token,
                ),
            | Self::ExpectedEnumeratorBeforeClosingCurlyBrace => new("enum has no enumerators")
                .label("expected an enumerator before `}`")
                .note("C99 §6.7.2.2: an enumerator list contains at least one enumerator"),
            | Self::ExpectedCommaOrClosingCurlyInEnumeratorList(token) => expected_with_label(
                "`,` or `}` after the enumerator",
                "expected `,` or `}`",
                *token,
            ),
            | Self::StorageClassRedefinition(previous, repeated) => new(format_in!(
                arena,
                "cannot combine storage classes `{}` and `{}`",
                previous.spelling(),
                token_spelling(*repeated)
            ))
            .label(format_in!(
                arena,
                "`{}` was already specified",
                previous.spelling()
            ))
            .note("C99 §6.7.1p2: a declaration has at most one storage-class specifier"),
            | Self::DeclarationSpecifierNotAllowedHere(token) => new(format_in!(
                arena,
                "`{}` is not allowed here",
                token_spelling(*token)
            ))
            .label("only type specifiers and qualifiers may appear here")
            .note(
                "C99 §6.7.2.1 and §6.7.6: member declarations and type names use a \
                 specifier-qualifier list, which excludes storage classes and `inline`",
            ),
            | Self::ConstSpecifiedTwice => new("duplicate `const`")
                .label("`const` was already specified")
                .note("C99 §6.7.3p4: repeating a qualifier has no effect"),
            | Self::VolatileSpecifiedTwice => new("duplicate `volatile`")
                .label("`volatile` was already specified")
                .note("C99 §6.7.3p4: repeating a qualifier has no effect"),
            | Self::RestrictSpecifiedTwice => new("duplicate `restrict`")
                .label("`restrict` was already specified")
                .note("C99 §6.7.3p4: repeating a qualifier has no effect"),
            | Self::InlineSpecifiedTwice => new("duplicate `inline`")
                .label("`inline` was already specified")
                .note("repeating `inline` has no effect"),
            | Self::StaticSpecifiedTwice => new("duplicate `static` in array declarator")
                .label("`static` was already specified")
                .note("C99 §6.7.5: an array declarator takes `static` at most once"),
            | Self::TypeQualifiersBothBeforeAndAfterStaticInArrayDirectDeclarator =>
                new("type qualifiers on both sides of `static`")
                    .label("qualifiers may appear before or after `static`, not both")
                    .note("C99 §6.7.5: write `[static const 3]` or `[const static 3]`"),
            | Self::ConflictingTypeSpecifiers {
                existing,
                conflicting,
            } => new(format_in!(
                arena,
                "cannot combine `{conflicting}` with `{existing}`"
            ))
            .label(format_in!(arena, "conflicts with `{existing}`"))
            .note(SPECIFIER_COMBINATIONS_NOTE),
            | Self::TypeSpecifierSpecifiedTwice(token) => {
                let keyword = token_spelling(*token);
                new(format_in!(arena, "duplicate `{keyword}`"))
                    .label(format_in!(arena, "`{keyword}` was already specified"))
                    .note(SPECIFIER_COMBINATIONS_NOTE)
                    .help(format_in!(arena, "remove the repeated `{keyword}`"))
            },
            | Self::LongSpecifiedThrice => new("`long long long` is too long")
                .label("third `long`")
                .note("C99 §6.7.2p2: `long long` is the longest integer type"),
            | Self::LongLongDoubleSpecified => new("cannot combine `long long` with `double`")
                .label("`long long double` is not a type")
                .note(SPECIFIER_COMBINATIONS_NOTE)
                .help("use `long double` for extended precision"),
            | Self::EmptyDeclarationSpecifiers(token) => {
                let explanation = expected_with_label(
                    "a declaration",
                    "expected a type or storage class",
                    Some(*token),
                );
                if *token == TokenType::Operator(OperatorTokenType::Semicolon) {
                    explanation
                        .note("C99 §6.7p2: a declaration must declare something")
                        .help("remove this `;`")
                } else {
                    explanation
                }
            },
            | Self::NoTypeSpecifiersInDeclarationSpecifiers(_) => new("missing type specifier")
                .label("expected a type such as `int` before this")
                .note(
                    "C99 §6.7.2p2: every declaration needs at least one type specifier; C99 \
                     removed implicit `int`",
                ),
            | Self::UnknownTypeName => new(match spelling {
                | Some(spelling) =>
                    format_in!(arena, "unknown type name {}", quote_spelling(spelling)),
                | None => "unknown type name",
            })
            .label("not a type name in scope")
            .note("C99 §6.7.7: an identifier names a type only after a `typedef` declares it"),
            | Self::IncompleteComplexTypeSpecifier =>
                new("`_Complex` requires `float`, `double`, or `long double`")
                    .label("incomplete complex type")
                    .note(SPECIFIER_COMBINATIONS_NOTE),
            | Self::BothStaticAndPointerInArrayDirectDeclarator =>
                new("`static` and `*` in one array declarator")
                    .label("`[*]` cannot be combined with `static`")
                    .note("C99 §6.7.5: `[static N]` and `[*]` are separate forms"),
            | Self::ExpectedAssignmentExpressionAfterStaticInArrayDirectDeclarator =>
                new("expected an array size after `static`")
                    .label("expected an expression")
                    .note("C99 §6.7.5: `[static N]` requires the minimum size `N`"),
            | Self::ExpectedClosingSquareBracketAfterPointerInArrayDirectDeclarator(token) =>
                expected_with_label("`]` after `*`", "expected `]`", Some(*token))
                    .note("C99 §6.7.5: `[*]` declares a variable-length array of unspecified size"),
            | Self::UnexpectedEndOfArrayDeclaratorAfterPointer =>
                expected_with_label("`]` after `*`", "expected `]`", None),
            | Self::PointerSpecifiedTwice => new("duplicate `*` in array declarator")
                .label("`*` was already specified")
                .note("C99 §6.7.5: write `[*]` with a single `*`"),
            | Self::TypeQualifiersWithoutDeclarator =>
                new("expected a declarator after the pointer")
                    .label("qualifiers must be followed by the declared name"),
            | Self::TypeQualifiersBeforePointerInArrayAbstractDirectDeclarator =>
                new("type qualifiers before `*` in an abstract array declarator")
                    .label("not allowed in a type name")
                    .note("C99 §6.7.6: an abstract array declarator allows only `[*]` here"),
            | Self::KAndRFunctionDeclaratorMixedWithModernDeclarator =>
                new("identifier list mixed with parameter declarations")
                    .label("prototype parameter in an old-style identifier list")
                    .note(
                        "C99 §6.7.5: a parameter list is either all names (old style) or all \
                         declarations (prototype)",
                    ),
            | Self::ExpectedClosingParenthesisAfterEllipsisInFunctionDeclaratorParameterList(
                token,
            ) => expected_with_label("`)` after `...`", "expected `)`", Some(*token))
                .note("C99 §6.7.5: `...` must be the last parameter"),
            | Self::UnexpectedEndOfVariadicFunctionDeclaratorParameterList =>
                expected_with_label("`)` after `...`", "expected `)`", None),
            | Self::EmptyStructDeclarator => new("expected a member name")
                .label("this member declaration declares nothing")
                .note(
                    "C99 §6.7.2.1: each member declarator names a member or gives a bit-field \
                     width",
                ),
        }
    }
}

impl ParserErrorType<'_> {
    /// The explanation with owned text, for tests to inspect.
    #[cfg(test)]
    pub(crate) fn explain(&self, spelling: Option<&str>) -> crate::diagnostics::OwnedExplanation {
        let arena = Bump::new();
        self.explain_in(&arena, spelling).to_owned_explanation()
    }
}

impl Display for ParserErrorType<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let arena = Bump::new();
        f.write_str(self.explain_in(&arena, None).message)
    }
}
