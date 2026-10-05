//! Declaration-specifier frame and specifier classification helpers.

use std::fmt::Debug;

use super::{
    Parser,
    declaration_syntax::{
        DeclarationSpecifiers,
        TypeQualifiers,
        TypeSpecifiers,
    },
    enum_specifier::EnumSpecifierFrame,
    errors::ParserErrorType,
    machine::{
        EnumSpecifierResult,
        ParseAction,
        ParseFrame,
        ParseValue,
    },
    struct_or_union::StructOrUnionSpecifierFrame,
    syntax::{
        Identifier,
        StorageClass,
    },
};
use crate::translation_phases::{
    Context,
    SourceVectors,
    preprocessing::{
        KeywordTokenType,
        OperatorTokenType,
        Token,
        TokenType,
    },
};

/// Grammar context controlling which specifier families are legal.
///
/// C99: declaration-specifiers are §6.7, p. 97; PDF p. 109, while
/// specifier-qualifier-list is §6.7.2.1, p. 101; PDF p. 113.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SpecifierMode {
    /// Full declaration specifiers, including storage and function specifiers.
    Declaration,
    /// Struct member specifiers: types and qualifiers followed by named
    /// declarators or bit-fields.
    StructMember,
    /// Type-name specifiers: types and qualifiers followed only by an
    /// optional abstract declarator.
    TypeName,
}

/// Accumulates one declaration-specifier or specifier-qualifier sequence.
///
/// C99: §6.7, p. 97; PDF p. 109; §6.7.2.1, p. 101; PDF p. 113.
#[derive(Debug, Clone, Copy)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "The booleans record independent facts about the specifier sequence."
)]
pub(super) struct DeclarationSpecifiersFrame<'tu> {
    /// Current collection/child-wait transition.
    phase:                  DeclarationSpecifiersPhase,
    /// Grammar context limiting legal specifier families.
    mode:                   SpecifierMode,
    /// Accumulated normalized specifier result.
    specifiers:             DeclarationSpecifiers<'tu>,
    /// Whether at least one legal specifier has been consumed.
    consumed:               bool,
    /// Whether a storage-class specifier has already appeared.
    storage_seen:           bool,
    /// Whether an invalid token occupied the mandatory type-specifier slot.
    invalid_type_seen:      bool,
    /// Whether a type specifier conflicted with the ones before it; the
    /// list is then already diagnosed once.
    type_conflict_seen:     bool,
    /// Owning tag keyword retained while its child frame runs.
    pending_type_specifier: Option<Token>,
    /// The `_Complex` keyword, retained to diagnose a list that never
    /// supplies its required real floating type.
    complex_token:          Option<Token>,
    /// Provenance accumulated across the complete specifier sequence.
    source_vectors:         Option<SourceVectors>,
}

/// Child-wait states used while collecting declaration specifiers.
///
/// C99: the child alternatives are type-specifiers from §6.7.2,
/// pp. 99-100; PDF pp. 111-112.
#[derive(Debug, Clone, Copy)]
pub(super) enum DeclarationSpecifiersPhase {
    /// Consume primitive, storage, qualifier, function, and typedef specifiers.
    Collect,
    /// Receive the struct/union specifier pushed by its keyword.
    AwaitStructOrUnion,
    /// Receive the enum specifier pushed by its keyword.
    AwaitEnum,
}

impl<'tu, 'p> DeclarationSpecifiersFrame<'tu> {
    pub(super) fn new(mode: SpecifierMode) -> Self {
        Self {
            phase: DeclarationSpecifiersPhase::Collect,
            mode,
            specifiers: DeclarationSpecifiers::new(),
            consumed: false,
            storage_seen: false,
            invalid_type_seen: false,
            type_conflict_seen: false,
            pending_type_specifier: None,
            complex_token: None,
            source_vectors: None,
        }
    }

    pub(super) fn step(
        &mut self,
        parser: &mut Parser<'tu, 'p>,
        context: &mut Context<'_>,
        token: Option<Token>,
        returned: Option<ParseValue<'tu>>,
    ) -> ParseAction<'tu, 'p> {
        match self.phase {
            | DeclarationSpecifiersPhase::AwaitStructOrUnion => {
                let Some(ParseValue::StructOrUnionSpecifier(index)) = returned else {
                    panic!("struct specifier returned an unexpected value: {returned:?}");
                };
                let token = self
                    .pending_type_specifier
                    .take()
                    .expect("struct-or-union child follows its keyword");
                // The child consumed the entire tag specifier. Merge it
                // into the normalized type set,
                // then reprocess the untouched token
                // that follows the child.
                self.specifiers
                    .type_specifiers
                    .make_struct_or_union(parser, context, index, token);
                {
                    let source_vectors = index.source_vectors;
                    self.source_vectors =
                        Some(self.source_vectors.map_or(source_vectors, |existing| {
                            context.merge_vectors(existing, source_vectors)
                        }));
                }
                self.consumed = true;
                self.phase = DeclarationSpecifiersPhase::Collect;
                return ParseAction::Continue;
            },
            | DeclarationSpecifiersPhase::AwaitEnum => {
                let Some(ParseValue::EnumSpecifier(EnumSpecifierResult {
                    index,
                    stopped_before_declaration,
                })) = returned
                else {
                    panic!("enum specifier returned an unexpected value: {returned:?}");
                };
                let token = self
                    .pending_type_specifier
                    .take()
                    .expect("enum child follows its keyword");
                // As above, the enum child owns its delimiters and
                // returns on the first token
                // belonging to this specifier sequence.
                self.specifiers
                    .type_specifiers
                    .make_enum(parser, context, index, token);
                {
                    let source_vectors = index.source_vectors;
                    self.source_vectors =
                        Some(self.source_vectors.map_or(source_vectors, |existing| {
                            context.merge_vectors(existing, source_vectors)
                        }));
                }
                self.consumed = true;
                if stopped_before_declaration {
                    self.specifiers.source_vectors = self.source_vectors.unwrap_or_default();
                    return ParseAction::Reduce(ParseValue::DeclarationSpecifiers(self.specifiers));
                }
                self.phase = DeclarationSpecifiersPhase::Collect;
                return ParseAction::Continue;
            },
            | DeclarationSpecifiersPhase::Collect => {
                debug_assert!(
                    returned.is_none(),
                    "this frame phase cannot receive a child value"
                );
            },
        }

        // EOF has two distinct failures. Keep them mutually exclusive
        // so one absent production does not cause a
        // cascade of specifier diagnostics.
        let Some(token) = token else {
            if !self.consumed {
                parser.report(
                    context,
                    ParserErrorType::UnexpectedEndBeforeDeclarationSpecifier,
                    None,
                );
            } else if self.specifiers.type_specifiers == TypeSpecifiers::Empty
                && !self.invalid_type_seen
            {
                parser.report(
                    context,
                    ParserErrorType::UnexpectedEndBeforeTypeSpecifier,
                    None,
                );
            }
            self.report_incomplete_complex(parser, context);
            self.specifiers.source_vectors = self.source_vectors.unwrap_or_default();
            return ParseAction::Reduce(ParseValue::DeclarationSpecifiers(self.specifiers));
        };

        if matches!(
            token.kind,
            TokenType::Keyword(KeywordTokenType::Struct | KeywordTokenType::Union)
        ) {
            // Keep the keyword for diagnostics/type-conflict
            // provenance; the child frame consumes
            // it as the first token it owns.
            self.pending_type_specifier = Some(token);
            self.phase = DeclarationSpecifiersPhase::AwaitStructOrUnion;
            let frame = StructOrUnionSpecifierFrame::new(parser.arena);
            return ParseAction::Push(ParseFrame::StructOrUnionSpecifier(
                parser.pools.struct_or_union_specifier(frame),
            ));
        }
        if token.kind == TokenType::Keyword(KeywordTokenType::Enum) {
            self.pending_type_specifier = Some(token);
            self.phase = DeclarationSpecifiersPhase::AwaitEnum;
            return ParseAction::Push(ParseFrame::EnumSpecifier(EnumSpecifierFrame::new(
                parser.arena,
            )));
        }

        if let Some(storage_class) = storage_class(token.kind) {
            if self.mode != SpecifierMode::Declaration {
                parser.report(
                    context,
                    ParserErrorType::DeclarationSpecifierNotAllowedHere(token.kind),
                    Some(token),
                );
                parser.merge_source(context, &mut self.source_vectors, token);
                self.consumed = true;
                return ParseAction::Consume;
            }
            if self.storage_seen {
                parser.report(
                    context,
                    ParserErrorType::StorageClassRedefinition(
                        self.specifiers
                            .storage_class
                            .expect("storage_seen implies a storage class"),
                        token.kind,
                    ),
                    Some(token),
                );
            }
            self.specifiers.storage_class = Some(storage_class);
            self.storage_seen = true;
            parser.merge_source(context, &mut self.source_vectors, token);
            self.consumed = true;
            return ParseAction::Consume;
        }

        if let TokenType::Keyword(keyword) = token.kind
            && let Some(specifier) = primitive_type_specifier(keyword)
        {
            let reported_before = context.pending_errors.len();
            self.apply_type_specifier(parser, context, token, specifier);
            self.type_conflict_seen |= context.pending_errors.len() != reported_before;
            parser.merge_source(context, &mut self.source_vectors, token);
            self.consumed = true;
            return ParseAction::Consume;
        }

        if let Some(qualifier) = type_qualifier(token.kind) {
            if self.mode != SpecifierMode::TypeName
                && self.specifiers.type_qualifiers.contains(qualifier)
            {
                report_duplicate_type_qualifier(parser, context, token, qualifier);
            }
            self.specifiers.type_qualifiers.insert(qualifier);
            parser.merge_source(context, &mut self.source_vectors, token);
            self.consumed = true;
            return ParseAction::Consume;
        }

        if token.kind == TokenType::Keyword(KeywordTokenType::Inline) {
            if self.mode != SpecifierMode::Declaration {
                parser.report(
                    context,
                    ParserErrorType::DeclarationSpecifierNotAllowedHere(token.kind),
                    Some(token),
                );
                parser.merge_source(context, &mut self.source_vectors, token);
                self.consumed = true;
                return ParseAction::Consume;
            }
            if self.specifiers.function_specifiers.is_inline {
                parser.report(context, ParserErrorType::InlineSpecifiedTwice, Some(token));
            }
            self.specifiers.function_specifiers.is_inline = true;
            parser.merge_source(context, &mut self.source_vectors, token);
            self.consumed = true;
            return ParseAction::Consume;
        }

        if token.kind == TokenType::Identifier
            && parser.scopes.is_typedef(token.contents)
            && (self.mode == SpecifierMode::TypeName
                || self.specifiers.type_specifiers == TypeSpecifiers::Empty
                || parser.typedef_name_continues_specifiers())
        {
            // A visible typedef spelling is still allowed to become the
            // declarator name. Consume it as a specifier only when no
            // type is present yet or lookahead
            // proves another declarator follows.
            self.specifiers.type_specifiers.make_typedef_name(
                parser,
                context,
                Identifier::from_token(token),
                token,
            );
            parser.merge_source(context, &mut self.source_vectors, token);
            self.consumed = true;
            return ParseAction::Consume;
        }

        // A non-typedef identifier in the empty type slot that is directly
        // followed by another declarator start or declaration-specifier
        // keyword cannot be the declarator itself: no declarator continues
        // with either. Diagnose it once as an unknown type name and let the
        // rest of the declaration follow, instead of misreading it as an
        // implicit-`int` name and then rejecting what comes after it.
        if token.kind == TokenType::Identifier
            && self.mode != SpecifierMode::TypeName
            && self.specifiers.type_specifiers == TypeSpecifiers::Empty
            && !self.invalid_type_seen
            && !parser.scopes.is_typedef(token.contents)
            && parser.cursor.following().is_some_and(|following| {
                following.kind == TokenType::Identifier
                    || following.kind == TokenType::Operator(OperatorTokenType::Asterisk)
                    || parser.declaration_starter(following)
            })
        {
            parser.report(context, ParserErrorType::UnknownTypeName, Some(token));
            self.invalid_type_seen = true;
            parser.merge_source(context, &mut self.source_vectors, token);
            self.consumed = true;
            return ParseAction::Consume;
        }

        // The first non-specifier belongs to the parent. Finalize
        // without consuming it, while reporting any
        // missing mandatory component. One diagnostic per missing piece: a
        // bare identifier is a name lacking its type, while any other token
        // means no declaration started here at all.
        if !self.consumed && token.kind != TokenType::Identifier {
            parser.report(
                context,
                ParserErrorType::EmptyDeclarationSpecifiers(token.kind),
                Some(token),
            );
        } else if self.specifiers.type_specifiers == TypeSpecifiers::Empty
            && !self.invalid_type_seen
        {
            parser.report(
                context,
                ParserErrorType::NoTypeSpecifiersInDeclarationSpecifiers(token.kind),
                Some(token),
            );
        }
        self.report_incomplete_complex(parser, context);
        self.specifiers.source_vectors = self.source_vectors.unwrap_or_default();
        ParseAction::Reduce(ParseValue::DeclarationSpecifiers(self.specifiers))
    }

    /// Diagnoses a finished list whose `_Complex` never received `float`,
    /// `double`, or `long double`.
    ///
    /// C99: §6.7.2 paragraph 2, pp. 99-100; PDF pp. 111-112 lists only
    /// `float _Complex`, `double _Complex`, and `long double _Complex`.
    fn report_incomplete_complex(&self, parser: &mut Parser<'tu, 'p>, context: &mut Context<'_>) {
        if !self.invalid_type_seen
            && !self.type_conflict_seen
            && matches!(
                self.specifiers.type_specifiers,
                TypeSpecifiers::Complex | TypeSpecifiers::ComplexLong
            )
        {
            parser.report(
                context,
                ParserErrorType::IncompleteComplexTypeSpecifier,
                self.complex_token,
            );
        }
    }

    fn apply_type_specifier(
        &mut self,
        parser: &mut Parser<'tu, 'p>,
        context: &mut Context<'_>,
        token: Token,
        specifier: PrimitiveTypeSpecifier,
    ) {
        let type_specifiers = &mut self.specifiers.type_specifiers;
        // Normalize order-independent keyword sequences into one canonical
        // TypeSpecifiers value while preserving specific conflict diagnostics.
        macro_rules! apply_once {
            ($is_duplicate:ident, $apply:ident) => {
                if type_specifiers.$is_duplicate() {
                    parser.report(
                        context,
                        ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                        Some(token),
                    );
                } else {
                    type_specifiers.$apply(parser, context, token);
                }
            };
        }

        match specifier {
            | PrimitiveTypeSpecifier::Signed => apply_once!(is_signed, make_signed),
            | PrimitiveTypeSpecifier::Unsigned => apply_once!(is_unsigned, make_unsigned),
            | PrimitiveTypeSpecifier::Int => apply_once!(is_int, make_int),
            | PrimitiveTypeSpecifier::Short => apply_once!(is_short, make_short),
            | PrimitiveTypeSpecifier::Long if type_specifiers.is_long_double() => {
                parser.report(
                    context,
                    ParserErrorType::LongLongDoubleSpecified,
                    Some(token),
                );
            },
            | PrimitiveTypeSpecifier::Long
                if type_specifiers.is_long() && type_specifiers.is_long_long() =>
            {
                parser.report(context, ParserErrorType::LongSpecifiedThrice, Some(token));
            },
            | PrimitiveTypeSpecifier::Long => type_specifiers.make_long(parser, context, token),
            | PrimitiveTypeSpecifier::Char => apply_once!(is_char, make_char),
            | PrimitiveTypeSpecifier::Float => apply_once!(is_float, make_float),
            | PrimitiveTypeSpecifier::Double if type_specifiers.is_double() => {
                parser.report(
                    context,
                    ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                    Some(token),
                );
            },
            | PrimitiveTypeSpecifier::Double if type_specifiers.is_long_long() => {
                parser.report(
                    context,
                    ParserErrorType::LongLongDoubleSpecified,
                    Some(token),
                );
            },
            | PrimitiveTypeSpecifier::Double => type_specifiers.make_double(parser, context, token),
            | PrimitiveTypeSpecifier::Void => apply_once!(is_void, make_void),
            | PrimitiveTypeSpecifier::Bool => apply_once!(is_bool, make_bool),
            | PrimitiveTypeSpecifier::Complex => {
                if self.complex_token.is_none() {
                    self.complex_token = Some(token);
                }
                apply_once!(is_complex, make_complex);
            },
            | PrimitiveTypeSpecifier::Imaginary => {
                self.invalid_type_seen = true;
                parser.report(
                    context,
                    ParserErrorType::UnsupportedImaginaryTypeSpecifier,
                    Some(token),
                );
            },
        }
    }
}

/// Classifies one storage-class-specifier keyword.
///
/// C99: §6.7.1, p. 98; PDF p. 110.
pub(super) fn storage_class(token: TokenType) -> Option<StorageClass> {
    match token {
        | TokenType::Keyword(KeywordTokenType::Auto) => Some(StorageClass::Auto),
        | TokenType::Keyword(KeywordTokenType::Register) => Some(StorageClass::Register),
        | TokenType::Keyword(KeywordTokenType::Static) => Some(StorageClass::Static),
        | TokenType::Keyword(KeywordTokenType::Extern) => Some(StorageClass::Extern),
        | TokenType::Keyword(KeywordTokenType::Typedef) => Some(StorageClass::Typedef),
        | _ => None,
    }
}

/// Classifies one type-qualifier keyword.
///
/// C99: §6.7.3, p. 108; PDF p. 120.
pub(super) fn type_qualifier(token: TokenType) -> Option<TypeQualifiers> {
    match token {
        | TokenType::Keyword(KeywordTokenType::Const) => Some(TypeQualifiers::CONST),
        | TokenType::Keyword(KeywordTokenType::Volatile) => Some(TypeQualifiers::VOLATILE),
        | TokenType::Keyword(KeywordTokenType::Restrict) => Some(TypeQualifiers::RESTRICT),
        | _ => None,
    }
}

pub(super) fn report_duplicate_type_qualifier(
    parser: &mut Parser<'_, '_>,
    context: &mut Context<'_>,
    token: Token,
    qualifier: TypeQualifiers,
) {
    let error_type = match qualifier {
        | TypeQualifiers::CONST => ParserErrorType::ConstSpecifiedTwice,
        | TypeQualifiers::VOLATILE => ParserErrorType::VolatileSpecifiedTwice,
        | TypeQualifiers::RESTRICT => ParserErrorType::RestrictSpecifiedTwice,
        | _ => unreachable!("one type qualifier is handled at a time"),
    };
    parser.report(context, error_type, Some(token));
}

/// Primitive keyword recognized while accumulating a C type-specifier set.
///
/// C99: normative type-specifiers are §6.7.2, pp. 99-100; PDF pp. 111-112.
/// `_Imaginary` comes from the keyword inventory in §6.4.1, p. 50; PDF p. 62
/// and is retained here only for the existing extension path.
#[derive(Debug, Clone, Copy)]
pub(super) enum PrimitiveTypeSpecifier {
    Signed,
    Unsigned,
    Int,
    Short,
    Long,
    Char,
    Float,
    Double,
    Void,
    Bool,
    Complex,
    Imaginary,
}

/// Classifies a keyword for [`PrimitiveTypeSpecifier`] accumulation.
///
/// C99: §6.7.2, pp. 99-100; PDF pp. 111-112; `_Imaginary` caveat as documented
/// on [`PrimitiveTypeSpecifier`].
fn primitive_type_specifier(keyword: KeywordTokenType) -> Option<PrimitiveTypeSpecifier> {
    match keyword {
        | KeywordTokenType::Signed => Some(PrimitiveTypeSpecifier::Signed),
        | KeywordTokenType::Unsigned => Some(PrimitiveTypeSpecifier::Unsigned),
        | KeywordTokenType::Int => Some(PrimitiveTypeSpecifier::Int),
        | KeywordTokenType::Short => Some(PrimitiveTypeSpecifier::Short),
        | KeywordTokenType::Long => Some(PrimitiveTypeSpecifier::Long),
        | KeywordTokenType::Char => Some(PrimitiveTypeSpecifier::Char),
        | KeywordTokenType::Float => Some(PrimitiveTypeSpecifier::Float),
        | KeywordTokenType::Double => Some(PrimitiveTypeSpecifier::Double),
        | KeywordTokenType::Void => Some(PrimitiveTypeSpecifier::Void),
        | KeywordTokenType::Bool => Some(PrimitiveTypeSpecifier::Bool),
        | KeywordTokenType::Complex => Some(PrimitiveTypeSpecifier::Complex),
        | KeywordTokenType::Imaginary => Some(PrimitiveTypeSpecifier::Imaginary),
        | _ => None,
    }
}
