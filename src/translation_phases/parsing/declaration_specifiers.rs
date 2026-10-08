//! Declaration-specifier frame and specifier classification helpers.
//!
//! Translation phase 7 syntax analysis (§5.1.1.2, p. 10; PDF p. 22) of
//! `declaration-specifiers` (C99: §6.7 paragraph 1, p. 97; PDF p. 109) and
//! `specifier-qualifier-list` (§6.7.2.1 paragraph 1, p. 101; PDF p. 113)
//! over the specifier families of §6.7.1 storage-class specifiers, p. 98;
//! PDF p. 110; §6.7.2 type specifiers, pp. 99-100; PDF pp. 111-112; §6.7.3
//! type qualifiers, p. 108; PDF p. 120; and §6.7.4 function specifiers,
//! p. 112; PDF p. 124; summarized in §A.2.2, pp. 411-413; PDF pp. 423-425.
//!
//! Diagnosed here: more than one storage-class specifier (§6.7.1
//! paragraph 2, p. 98; PDF p. 110); a type-specifier list outside the sets
//! of §6.7.2 paragraph 2, pp. 99-100; PDF pp. 111-112, including an empty
//! one; storage-class and function specifiers in a specifier-qualifier list;
//! and, as warnings only, repeated qualifiers and `inline`, which C99 accepts
//! (§6.7.3 paragraph 4, p. 108; PDF p. 120; §6.7.4 paragraph 5, p. 112;
//! PDF p. 124). A typedef-name is a type specifier only while the scope stack
//! classifies the identifier as one (§6.7.7, p. 123; PDF p. 135). Every other
//! §6.7.1-§6.7.4 constraint and the meaning of the specifiers are left to
//! semantic analysis.

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
    modern::{
        ExtendedType,
        ModernFrame,
        ModernKind,
        ModernValue,
        SpecifierExtension,
        SpecifierExtensionKind,
        SyntaxOperand,
    },
    struct_or_union::StructOrUnionSpecifierFrame,
    syntax::{
        Identifier,
        StorageClass,
    },
};
use crate::translation_phases::{
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
/// specifier-qualifier-list is §6.7.2.1, p. 101; PDF p. 113. A type name
/// takes a specifier-qualifier-list too, §6.7.6 paragraph 1, p. 122;
/// PDF p. 134.
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
    CompoundLiteral,
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
    implicit_name_allowed:  bool,
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
/// C99: the child alternatives are the `struct-or-union-specifier` and
/// `enum-specifier` type specifiers of §6.7.2 paragraph 1, p. 99;
/// PDF p. 111.
#[derive(Debug, Clone, Copy)]
pub(super) enum DeclarationSpecifiersPhase {
    /// Consume primitive, storage, qualifier, function, and typedef specifiers.
    Collect,
    AwaitModern(KeywordTokenType),
    AwaitAttributes,
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
            implicit_name_allowed: true,
            storage_seen: false,
            invalid_type_seen: false,
            type_conflict_seen: false,
            pending_type_specifier: None,
            complex_token: None,
            source_vectors: None,
        }
    }

    pub(super) fn parameter() -> Self {
        Self {
            implicit_name_allowed: false,
            ..Self::new(SpecifierMode::Declaration)
        }
    }

    pub(super) fn step(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
        returned: Option<ParseValue<'tu>>,
    ) -> ParseAction<'tu, 'p> {
        match self.phase {
            | DeclarationSpecifiersPhase::AwaitAttributes => {
                let Some(ParseValue::Modern(ModernValue::Attributes(attributes))) = returned else {
                    panic!("attribute protocol: {returned:?}")
                };
                self.add_extension(
                    parser,
                    SpecifierExtensionKind::Attributes(attributes),
                    attributes.source_vectors,
                );
                self.phase = DeclarationSpecifiersPhase::Collect;
                self.consumed = true;
                return ParseAction::Continue;
            },
            | DeclarationSpecifiersPhase::AwaitModern(keyword) => {
                let Some(ParseValue::Modern(ModernValue::Operand(operand, source))) = returned
                else {
                    panic!("specifier operand protocol: {returned:?}")
                };
                if keyword == KeywordTokenType::Alignas {
                    self.add_extension(parser, SpecifierExtensionKind::Alignment(operand), source);
                } else {
                    let kind = match (keyword, operand) {
                        | (KeywordTokenType::Atomic, SyntaxOperand::Type(x)) =>
                            ExtendedType::Atomic(x),
                        | (KeywordTokenType::BitInt, SyntaxOperand::Expression(width)) =>
                            ExtendedType::BitInt {
                                width,
                                signedness: match self.specifiers.type_specifiers {
                                    | TypeSpecifiers::Signed => Some(true),
                                    | TypeSpecifiers::Unsigned => Some(false),
                                    | _ => None,
                                },
                            },
                        | (_, operand) => ExtendedType::Typeof {
                            operand,
                            unqualified: keyword == KeywordTokenType::TypeofUnqual,
                        },
                    };
                    self.specifiers.type_specifiers =
                        TypeSpecifiers::Extended(parser.alloc_syntax(kind));
                    self.source_vectors = Some(
                        parser
                            .context
                            .merge_vectors(self.source_vectors.unwrap_or_default(), source),
                    );
                }
                self.phase = DeclarationSpecifiersPhase::Collect;
                self.consumed = true;
                return ParseAction::Continue;
            },
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
                    .make_struct_or_union(parser, index, token);
                {
                    let source_vectors = index.source_vectors;
                    self.source_vectors =
                        Some(self.source_vectors.map_or(source_vectors, |existing| {
                            parser.context.merge_vectors(existing, source_vectors)
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
                    .make_enum(parser, index, token);
                {
                    let source_vectors = index.source_vectors;
                    self.source_vectors =
                        Some(self.source_vectors.map_or(source_vectors, |existing| {
                            parser.context.merge_vectors(existing, source_vectors)
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
                    ParserErrorType::UnexpectedEndBeforeDeclarationSpecifier,
                    None,
                );
            } else if self.specifiers.type_specifiers == TypeSpecifiers::Empty
                && !self.invalid_type_seen
            {
                parser.report(ParserErrorType::UnexpectedEndBeforeTypeSpecifier, None);
            }
            self.report_incomplete_complex(parser);
            self.specifiers.source_vectors = self.source_vectors.unwrap_or_default();
            return ParseAction::Reduce(ParseValue::DeclarationSpecifiers(self.specifiers));
        };

        // GNU extension: `__extension__` may lead a declaration or a member
        // declaration. Its owner (the declaration, parameter, or member)
        // restores the suppression depth when it ends.
        if matches!(token.kind, TokenType::Keyword(KeywordTokenType::Extension))
            && matches!(
                self.mode,
                SpecifierMode::Declaration | SpecifierMode::StructMember
            )
        {
            parser.pedantic_suppression += 1;
            self.add_extension(
                parser,
                SpecifierExtensionKind::ExtensionMarker,
                token.source_vectors,
            );
            self.consumed = true;
            parser.merge_source(&mut self.source_vectors, token);
            return ParseAction::Consume;
        }
        if let TokenType::Keyword(
            keyword @ (KeywordTokenType::Int128 | KeywordTokenType::AutoType),
        ) = token.kind
        {
            let extended = if keyword == KeywordTokenType::Int128 {
                let signedness = match self.specifiers.type_specifiers {
                    | TypeSpecifiers::Signed => Some(true),
                    | TypeSpecifiers::Unsigned => Some(false),
                    | _ => None,
                };
                if !matches!(
                    self.specifiers.type_specifiers,
                    TypeSpecifiers::Empty | TypeSpecifiers::Signed | TypeSpecifiers::Unsigned
                ) {
                    self.specifiers
                        .type_specifiers
                        .report_conflict(parser, token.contents, token);
                }
                ExtendedType::Int128 { signedness }
            } else {
                if self.specifiers.type_specifiers != TypeSpecifiers::Empty {
                    self.specifiers
                        .type_specifiers
                        .report_conflict(parser, token.contents, token);
                }
                ExtendedType::AutoType
            };
            self.specifiers.type_specifiers =
                TypeSpecifiers::Extended(parser.alloc_syntax(extended));
            self.consumed = true;
            parser.merge_source(&mut self.source_vectors, token);
            return ParseAction::Consume;
        }
        if let Some(action) = self.step_msvc(parser, token) {
            return action;
        }
        if parser.attribute_starter(Some(token)) {
            self.phase = DeclarationSpecifiersPhase::AwaitAttributes;
            return ParseAction::Push(ParseFrame::Modern(parser.pools.modern(ModernFrame::new(
                parser.arena,
                ModernKind::Attributes,
                parser.hard_error_count,
            ))));
        }
        if let TokenType::Keyword(keyword) = token.kind {
            if matches!(
                keyword,
                KeywordTokenType::Alignas
                    | KeywordTokenType::BitInt
                    | KeywordTokenType::Typeof
                    | KeywordTokenType::TypeofUnqual
            ) || keyword == KeywordTokenType::Atomic
                && parser.cursor.following().is_some_and(|x| {
                    matches!(
                        x.kind,
                        TokenType::Operator(OperatorTokenType::OpeningParenthesis)
                    )
                })
            {
                // C17 §6.7.5p2 (C23 (N3220) §6.7.6p2): an alignment specifier
                // belongs only to a declaration, a member declaration, or a
                // compound literal's type name. Its operand still parses.
                if keyword == KeywordTokenType::Alignas && self.mode == SpecifierMode::TypeName {
                    parser.report(
                        ParserErrorType::DeclarationSpecifierNotAllowedHere(token.kind),
                        Some(token),
                    );
                }
                if keyword != KeywordTokenType::Alignas
                    && self.specifiers.type_specifiers != TypeSpecifiers::Empty
                    && !(keyword == KeywordTokenType::BitInt
                        && matches!(
                            self.specifiers.type_specifiers,
                            TypeSpecifiers::Signed | TypeSpecifiers::Unsigned
                        ))
                {
                    self.specifiers
                        .type_specifiers
                        .report_conflict(parser, token.contents, token);
                }
                self.phase = DeclarationSpecifiersPhase::AwaitModern(keyword);
                return ParseAction::Push(ParseFrame::Modern(parser.pools.modern(
                    ModernFrame::new(
                        parser.arena,
                        ModernKind::Operand {
                            type_only: keyword == KeywordTokenType::Atomic,
                            constant:  matches!(
                                keyword,
                                KeywordTokenType::Alignas | KeywordTokenType::BitInt
                            ),
                        },
                        parser.hard_error_count,
                    ),
                )));
            }
            if matches!(
                keyword,
                KeywordTokenType::ThreadLocal | KeywordTokenType::Constexpr
            ) {
                if !matches!(
                    self.mode,
                    SpecifierMode::Declaration | SpecifierMode::CompoundLiteral
                ) {
                    parser.report(
                        ParserErrorType::DeclarationSpecifierNotAllowedHere(token.kind),
                        Some(token),
                    );
                }
                if self.mode == SpecifierMode::CompoundLiteral {
                    parser.extension(
                        crate::configuration::Feature::C23Keywords,
                        "storage class in compound literal",
                        token,
                    );
                }
                self.add_extension(
                    parser,
                    if keyword == KeywordTokenType::ThreadLocal {
                        SpecifierExtensionKind::ThreadLocal
                    } else {
                        SpecifierExtensionKind::Constexpr
                    },
                    token.source_vectors,
                );
                self.consumed = true;
                return ParseAction::Consume;
            }
            if keyword == KeywordTokenType::Noreturn {
                if self.mode != SpecifierMode::Declaration {
                    parser.report(
                        ParserErrorType::DeclarationSpecifierNotAllowedHere(token.kind),
                        Some(token),
                    );
                }
                self.specifiers.function_specifiers.is_noreturn = true;
                parser.merge_source(&mut self.source_vectors, token);
                self.consumed = true;
                return ParseAction::Consume;
            }
            let decimal = match keyword {
                | KeywordTokenType::Decimal32 => Some(ExtendedType::Decimal32),
                | KeywordTokenType::Decimal64 => Some(ExtendedType::Decimal64),
                | KeywordTokenType::Decimal128 => Some(ExtendedType::Decimal128),
                | _ => None,
            };
            if let Some(decimal) = decimal {
                if self.specifiers.type_specifiers != TypeSpecifiers::Empty {
                    self.specifiers
                        .type_specifiers
                        .report_conflict(parser, token.contents, token);
                }
                self.specifiers.type_specifiers =
                    TypeSpecifiers::Extended(parser.alloc_syntax(decimal));
                parser.merge_source(&mut self.source_vectors, token);
                self.consumed = true;
                return ParseAction::Consume;
            }
        }
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
        if matches!(token.kind, TokenType::Keyword(KeywordTokenType::Enum)) {
            self.pending_type_specifier = Some(token);
            self.phase = DeclarationSpecifiersPhase::AwaitEnum;
            return ParseAction::Push(ParseFrame::EnumSpecifier(EnumSpecifierFrame::new(
                parser.arena,
            )));
        }

        if let Some(storage_class) = storage_class(token.kind) {
            if self.mode == SpecifierMode::CompoundLiteral {
                parser.extension(
                    crate::configuration::Feature::C23Keywords,
                    "storage class in compound literal",
                    token,
                );
            }
            if !matches!(
                self.mode,
                SpecifierMode::Declaration | SpecifierMode::CompoundLiteral
            ) {
                parser.report(
                    ParserErrorType::DeclarationSpecifierNotAllowedHere(token.kind),
                    Some(token),
                );
                parser.merge_source(&mut self.source_vectors, token);
                self.consumed = true;
                return ParseAction::Consume;
            }
            // C99 §6.7.1p2: at most one storage-class specifier. C23 lets
            // `auto` join any other one except `typedef`; `storage_class`
            // then keeps the other one (C23 (N3220) §6.7.2p2).
            if self.storage_seen {
                let previous = self
                    .specifiers
                    .storage_class
                    .expect("storage_seen implies a storage class");
                if !self.specifiers.auto_with_storage_class
                    && (previous == StorageClass::Auto) != (storage_class == StorageClass::Auto)
                    && previous != StorageClass::Typedef
                    && storage_class != StorageClass::Typedef
                    && parser.context.configuration.standard()
                        >= crate::configuration::CStandard::C23
                {
                    self.specifiers.auto_with_storage_class = true;
                    if storage_class != StorageClass::Auto {
                        self.specifiers.storage_class = Some(storage_class);
                    }
                    parser.merge_source(&mut self.source_vectors, token);
                    self.consumed = true;
                    return ParseAction::Consume;
                }
                parser.report(
                    ParserErrorType::StorageClassRedefinition(previous, token.kind),
                    Some(token),
                );
            }
            self.specifiers.storage_class = Some(storage_class);
            self.storage_seen = true;
            parser.merge_source(&mut self.source_vectors, token);
            self.consumed = true;
            return ParseAction::Consume;
        }

        if let TokenType::Keyword(keyword) = token.kind
            && let Some(specifier) = primitive_type_specifier(keyword)
        {
            let reported_before = parser.context.pending_errors.len();
            self.apply_type_specifier(parser, token, specifier);
            self.type_conflict_seen |= parser.context.pending_errors.len() != reported_before;
            parser.merge_source(&mut self.source_vectors, token);
            self.consumed = true;
            return ParseAction::Consume;
        }

        if let Some(qualifier) = type_qualifier(token.kind) {
            if self.mode != SpecifierMode::TypeName
                && self.specifiers.type_qualifiers.contains(qualifier)
            {
                report_duplicate_type_qualifier(parser, token, qualifier);
            }
            self.specifiers.type_qualifiers.insert(qualifier);
            parser.merge_source(&mut self.source_vectors, token);
            self.consumed = true;
            return ParseAction::Consume;
        }

        if matches!(
            token.kind,
            TokenType::Keyword(KeywordTokenType::Inline | KeywordTokenType::Forceinline)
        ) {
            if matches!(
                token.kind,
                TokenType::Keyword(KeywordTokenType::Forceinline)
            ) {
                self.add_extension(
                    parser,
                    SpecifierExtensionKind::MsModifier(match token.kind {
                        | TokenType::Keyword(k) => k,
                        | _ => unreachable!("modifier is keyword"),
                    }),
                    token.source_vectors,
                );
            }
            // C23 (N3220) §6.5.3.6p1: a compound literal takes only
            // storage-class specifiers before its type name, never function
            // specifiers.
            if self.mode != SpecifierMode::Declaration {
                parser.report(
                    ParserErrorType::DeclarationSpecifierNotAllowedHere(token.kind),
                    Some(token),
                );
                parser.merge_source(&mut self.source_vectors, token);
                self.consumed = true;
                return ParseAction::Consume;
            }
            if self.specifiers.function_specifiers.is_inline {
                parser.report(ParserErrorType::InlineSpecifiedTwice, Some(token));
            }
            self.specifiers.function_specifiers.is_inline = true;
            parser.merge_source(&mut self.source_vectors, token);
            self.consumed = true;
            return ParseAction::Consume;
        }

        // C99 §6.7.7p1: a typedef-name is an identifier the scope stack
        // currently binds as a typedef. §6.7.2p2 allows no other type
        // specifier beside it, so once a type is present the identifier is
        // normally the declarator, which may redeclare the name in an inner
        // scope (§6.2.1p4).
        if matches!(token.kind, TokenType::Identifier)
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
                Identifier::from_token(token),
                token,
            );
            parser.merge_source(&mut self.source_vectors, token);
            self.consumed = true;
            return ParseAction::Consume;
        }

        // A non-typedef identifier in the empty type slot that is directly
        // followed by another declarator start or declaration-specifier
        // keyword cannot be the declarator itself: no declarator continues
        // with either. Diagnose it once as an unknown type name and let the
        // rest of the declaration follow, instead of misreading it as an
        // implicit-`int` name and then rejecting what comes after it.
        // C99 has no implicit `int` (§6.7.2p2).
        if matches!(token.kind, TokenType::Identifier)
            && self.mode != SpecifierMode::TypeName
            && self.specifiers.type_specifiers == TypeSpecifiers::Empty
            && !self.invalid_type_seen
            && !parser.scopes.is_typedef(token.contents)
            && parser.cursor.following().is_some_and(|following| {
                matches!(following.kind, TokenType::Identifier)
                    || matches!(
                        following.kind,
                        TokenType::Operator(OperatorTokenType::Asterisk)
                    )
                    || parser.declaration_starter(following)
            })
        {
            parser.report(ParserErrorType::UnknownTypeName, Some(token));
            self.invalid_type_seen = true;
            parser.merge_source(&mut self.source_vectors, token);
            self.consumed = true;
            return ParseAction::Consume;
        }

        // The first non-specifier belongs to the parent. Finalize
        // without consuming it, while reporting any
        // missing mandatory component. One diagnostic per missing piece: a
        // bare identifier is a name lacking its type, while any other token
        // means no declaration started here at all.
        // C99 §6.7.2p2: at least one type specifier in each declaration,
        // struct declaration, and type name.
        let implicit_declarator = self.mode == SpecifierMode::Declaration
            && self.implicit_name_allowed
            && matches!(
                token.kind,
                TokenType::Identifier
                    | TokenType::Operator(
                        OperatorTokenType::Asterisk | OperatorTokenType::OpeningParenthesis
                    )
            );
        if !self.consumed && !implicit_declarator {
            parser.report(
                ParserErrorType::EmptyDeclarationSpecifiers(token.kind),
                Some(token),
            );
        } else if self.specifiers.type_specifiers == TypeSpecifiers::Empty
            && !self.invalid_type_seen
        {
            if self.mode == SpecifierMode::Declaration
                && self.has_only_attributes()
                && matches!(
                    token.kind,
                    TokenType::Operator(OperatorTokenType::Semicolon)
                )
                && self.specifiers.storage_class.is_none()
            {
            } else if self.mode == SpecifierMode::Declaration
                && (self.consumed || self.implicit_name_allowed)
            {
                if (self.specifiers.storage_class == Some(StorageClass::Auto)
                    || self.specifiers.auto_with_storage_class)
                    && parser.context.configuration.standard()
                        >= crate::configuration::CStandard::C23
                {
                    self.specifiers.type_specifiers =
                        TypeSpecifiers::Extended(parser.alloc_syntax(ExtendedType::Inferred));
                } else {
                    parser.extension(
                        crate::configuration::Feature::ImplicitInt,
                        "implicit int",
                        token,
                    );
                    self.specifiers.type_specifiers = TypeSpecifiers::Int;
                    self.specifiers.implicit_int = true;
                }
            } else {
                parser.report(
                    ParserErrorType::NoTypeSpecifiersInDeclarationSpecifiers(token.kind),
                    Some(token),
                );
            }
        }
        self.report_incomplete_complex(parser);
        self.specifiers.source_vectors = self.source_vectors.unwrap_or_default();
        ParseAction::Reduce(ParseValue::DeclarationSpecifiers(self.specifiers))
    }

    /// MSVC extensions to C99 §6.7.2, pp. 99-100; PDF pp. 111-112 and
    /// §6.7.5, p. 114; PDF p. 126. Width and ABI interpretation are deferred.
    fn step_msvc(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Token,
    ) -> Option<ParseAction<'tu, 'p>> {
        if let TokenType::Keyword(
            keyword @ (KeywordTokenType::Int8
            | KeywordTokenType::Int16
            | KeywordTokenType::Int32
            | KeywordTokenType::Int64),
        ) = token.kind
        {
            let signedness = match self.specifiers.type_specifiers {
                | TypeSpecifiers::Signed => Some(true),
                | TypeSpecifiers::Unsigned => Some(false),
                | TypeSpecifiers::Empty => None,
                | _ => {
                    self.specifiers
                        .type_specifiers
                        .report_conflict(parser, token.contents, token);
                    None
                },
            };
            let width = match keyword {
                | KeywordTokenType::Int8 => 8,
                | KeywordTokenType::Int16 => 16,
                | KeywordTokenType::Int32 => 32,
                | _ => 64,
            };
            self.specifiers.type_specifiers = TypeSpecifiers::Extended(
                parser.alloc_syntax(ExtendedType::MsInteger { width, signedness }),
            );
            self.consumed = true;
            parser.merge_source(&mut self.source_vectors, token);
            return Some(ParseAction::Consume);
        }
        if let TokenType::Keyword(keyword) = token.kind
            && super::msvc::calling_convention(keyword)
        {
            self.add_extension(
                parser,
                SpecifierExtensionKind::MsModifier(keyword),
                token.source_vectors,
            );
            self.consumed = true;
            return Some(ParseAction::Consume);
        }
        None
    }

    fn has_only_attributes(&self) -> bool {
        if self.specifiers.storage_class.is_some()
            || !self.specifiers.type_qualifiers.is_empty()
            || self.specifiers.function_specifiers.is_inline
            || self.specifiers.function_specifiers.is_noreturn
        {
            return false;
        }
        let mut extension = self.specifiers.extensions;
        if extension.is_none() {
            return false;
        }
        while let Some(item) = extension {
            if !matches!(item.kind, SpecifierExtensionKind::Attributes(_)) {
                return false;
            }
            extension = item.next;
        }
        true
    }

    fn add_extension(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        kind: SpecifierExtensionKind<'tu>,
        source_vectors: SourceVectors,
    ) {
        self.specifiers.extensions = Some(parser.alloc_syntax(SpecifierExtension {
            kind,
            next: self.specifiers.extensions,
            source_vectors,
        }));
        self.source_vectors = Some(
            parser
                .context
                .merge_vectors(self.source_vectors.unwrap_or_default(), source_vectors),
        );
    }

    /// Diagnoses a finished list whose `_Complex` never received `float`,
    /// `double`, or `long double`.
    ///
    /// C99: §6.7.2 paragraph 2, pp. 99-100; PDF pp. 111-112 lists only
    /// `float _Complex`, `double _Complex`, and `long double _Complex`.
    fn report_incomplete_complex(&self, parser: &mut Parser<'_, 'tu, 'p>) {
        if !self.invalid_type_seen
            && !self.type_conflict_seen
            && matches!(
                self.specifiers.type_specifiers,
                TypeSpecifiers::Complex | TypeSpecifiers::ComplexLong
            )
        {
            parser.report(
                ParserErrorType::IncompleteComplexTypeSpecifier,
                self.complex_token,
            );
        }
    }

    /// Folds one primitive type-specifier keyword into the accumulated set,
    /// diagnosing a list that leaves the sets of §6.7.2 paragraph 2.
    ///
    /// C99: §6.7.2 paragraph 2, pp. 99-100; PDF pp. 111-112. The keywords may
    /// appear in any order, intermixed with other specifiers.
    fn apply_type_specifier(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Token,
        specifier: PrimitiveTypeSpecifier,
    ) {
        if let TypeSpecifiers::Extended(ExtendedType::MsInteger { width, signedness }) =
            self.specifiers.type_specifiers
            && matches!(
                specifier,
                PrimitiveTypeSpecifier::Signed | PrimitiveTypeSpecifier::Unsigned
            )
        {
            if signedness.is_some() {
                self.specifiers
                    .type_specifiers
                    .report_conflict(parser, token.contents, token);
            } else {
                self.specifiers.type_specifiers =
                    TypeSpecifiers::Extended(parser.alloc_syntax(ExtendedType::MsInteger {
                        width:      *width,
                        signedness: Some(matches!(specifier, PrimitiveTypeSpecifier::Signed)),
                    }));
            }
            return;
        }
        if let TypeSpecifiers::Extended(ExtendedType::Int128 { signedness }) =
            self.specifiers.type_specifiers
            && matches!(
                specifier,
                PrimitiveTypeSpecifier::Signed | PrimitiveTypeSpecifier::Unsigned
            )
        {
            if signedness.is_some() {
                self.specifiers
                    .type_specifiers
                    .report_conflict(parser, token.contents, token);
            } else {
                self.specifiers.type_specifiers =
                    TypeSpecifiers::Extended(parser.alloc_syntax(ExtendedType::Int128 {
                        signedness: Some(matches!(specifier, PrimitiveTypeSpecifier::Signed)),
                    }));
            }
            return;
        }
        if let TypeSpecifiers::Extended(ExtendedType::BitInt { width, signedness }) =
            self.specifiers.type_specifiers
            && matches!(
                specifier,
                PrimitiveTypeSpecifier::Signed | PrimitiveTypeSpecifier::Unsigned
            )
        {
            let new_sign = matches!(specifier, PrimitiveTypeSpecifier::Signed);
            if signedness.is_some() {
                self.specifiers
                    .type_specifiers
                    .report_conflict(parser, token.contents, token);
            } else {
                self.specifiers.type_specifiers =
                    TypeSpecifiers::Extended(parser.alloc_syntax(ExtendedType::BitInt {
                        width,
                        signedness: Some(new_sign),
                    }));
            }
            return;
        }
        let type_specifiers = &mut self.specifiers.type_specifiers;
        // Normalize order-independent keyword sequences into one canonical
        // TypeSpecifiers value while preserving specific conflict diagnostics.
        macro_rules! apply_once {
            ($is_duplicate:ident, $apply:ident) => {
                if type_specifiers.$is_duplicate() {
                    parser.report(
                        ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                        Some(token),
                    );
                } else {
                    type_specifiers.$apply(parser, token);
                }
            };
        }

        match specifier {
            | PrimitiveTypeSpecifier::Signed => apply_once!(is_signed, make_signed),
            | PrimitiveTypeSpecifier::Unsigned => apply_once!(is_unsigned, make_unsigned),
            | PrimitiveTypeSpecifier::Int => apply_once!(is_int, make_int),
            | PrimitiveTypeSpecifier::Short => apply_once!(is_short, make_short),
            | PrimitiveTypeSpecifier::Long if type_specifiers.is_long_double() => {
                parser.report(ParserErrorType::LongLongDoubleSpecified, Some(token));
            },
            | PrimitiveTypeSpecifier::Long
                if type_specifiers.is_long() && type_specifiers.is_long_long() =>
            {
                parser.report(ParserErrorType::LongSpecifiedThrice, Some(token));
            },
            | PrimitiveTypeSpecifier::Long => {
                if type_specifiers.is_long() {
                    parser.extension(crate::configuration::Feature::LongLong, "long long", token);
                }
                self.specifiers.type_specifiers.make_long(parser, token);
            },
            | PrimitiveTypeSpecifier::Char => apply_once!(is_char, make_char),
            | PrimitiveTypeSpecifier::Float => apply_once!(is_float, make_float),
            | PrimitiveTypeSpecifier::Double if type_specifiers.is_double() => {
                parser.report(
                    ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                    Some(token),
                );
            },
            | PrimitiveTypeSpecifier::Double if type_specifiers.is_long_long() => {
                parser.report(ParserErrorType::LongLongDoubleSpecified, Some(token));
            },
            | PrimitiveTypeSpecifier::Double => type_specifiers.make_double(parser, token),
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
                    ParserErrorType::UnsupportedImaginaryTypeSpecifier,
                    Some(token),
                );
            },
        }
    }
}

/// Classifies one storage-class-specifier keyword.
///
/// C99: §6.7.1 paragraph 1, p. 98; PDF p. 110. `typedef` is a storage-class
/// specifier for syntactic convenience only (paragraph 3, p. 98;
/// PDF p. 110).
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
/// C99: §6.7.3 paragraph 1, p. 108; PDF p. 120.
pub(super) fn type_qualifier(token: TokenType) -> Option<TypeQualifiers> {
    match token {
        | TokenType::Keyword(KeywordTokenType::Ptr32) => Some(TypeQualifiers::PTR32),
        | TokenType::Keyword(KeywordTokenType::Ptr64) => Some(TypeQualifiers::PTR64),
        | TokenType::Keyword(KeywordTokenType::Unaligned) => Some(TypeQualifiers::UNALIGNED),
        | TokenType::Keyword(KeywordTokenType::W64) => Some(TypeQualifiers::W64),
        | TokenType::Keyword(KeywordTokenType::Sptr) => Some(TypeQualifiers::SPTR),
        | TokenType::Keyword(KeywordTokenType::Uptr) => Some(TypeQualifiers::UPTR),
        | TokenType::Keyword(KeywordTokenType::Atomic) => Some(TypeQualifiers::ATOMIC),
        | TokenType::Keyword(KeywordTokenType::Const) => Some(TypeQualifiers::CONST),
        | TokenType::Keyword(KeywordTokenType::Volatile) => Some(TypeQualifiers::VOLATILE),
        | TokenType::Keyword(KeywordTokenType::Restrict) => Some(TypeQualifiers::RESTRICT),
        | _ => None,
    }
}

/// Warns about a qualifier repeated in one list. C99 §6.7.3 paragraph 4,
/// p. 108; PDF p. 120, makes the repetition harmless, so this is a quality
/// diagnostic, not a constraint.
pub(super) fn report_duplicate_type_qualifier(
    parser: &mut Parser<'_, '_, '_>,
    token: Token,
    qualifier: TypeQualifiers,
) {
    let error_type = match qualifier {
        | TypeQualifiers::CONST => ParserErrorType::ConstSpecifiedTwice,
        | TypeQualifiers::VOLATILE => ParserErrorType::VolatileSpecifiedTwice,
        | TypeQualifiers::RESTRICT => ParserErrorType::RestrictSpecifiedTwice,
        | TypeQualifiers::ATOMIC
        | TypeQualifiers::PTR32
        | TypeQualifiers::PTR64
        | TypeQualifiers::UNALIGNED
        | TypeQualifiers::W64
        | TypeQualifiers::SPTR
        | TypeQualifiers::UPTR => return,
        | _ => unreachable!("one type qualifier is handled at a time"),
    };
    parser.report(error_type, Some(token));
}

/// Primitive keyword recognized while accumulating a C type-specifier set.
///
/// C99: normative type-specifiers are §6.7.2, pp. 99-100; PDF pp. 111-112.
/// `_Imaginary` comes from the keyword inventory in §6.4.1, p. 50; PDF p. 62
/// and is retained here only for the existing extension path. It is reserved
/// for imaginary types (§6.4.1 paragraph 2, p. 50; PDF p. 62), which only
/// informative annex G specifies; the frame rejects it.
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
