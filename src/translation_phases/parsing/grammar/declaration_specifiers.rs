//! Recursive declaration-specifier grammar.
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
//! semantic analysis. Struct, union and enum specifiers recurse directly;
//! attribute and operand extensions run through their own families.

use crate::{
    configuration::{
        CStandard,
        Feature,
    },
    translation_phases::{
        SourceVectors,
        parsing::{
            Parser,
            declaration_syntax::{
                DeclarationSpecifiers,
                TypeQualifiers,
                TypeSpecifiers,
            },
            errors::ParserErrorType,
            extensions::{
                modern::{
                    ExtendedType,
                    SpecifierExtension,
                    SpecifierExtensionKind,
                    SyntaxOperand,
                    ThreadSpelling,
                },
                msvc::calling_convention,
            },
            grammar::{
                ProductionKind,
                enum_specifier::EnumSpecifierResult,
                modern::{
                    ModernKind,
                    ModernValue,
                },
                unexpected_return,
            },
            syntax::{
                Identifier,
                StorageClass,
            },
        },
        preprocessing::{
            KeywordTokenType,
            OperatorTokenType,
            Token,
            TokenType,
        },
    },
};

impl<'tu> Parser<'_, 'tu, '_> {
    /// Parses one declaration-specifier or specifier-qualifier sequence and
    /// returns before the first token that belongs to its parent.
    ///
    /// C99: §6.7 paragraph 1, p. 97; PDF p. 109; §6.7.2.1 paragraph 1,
    /// p. 101; PDF p. 113. `implicit_name_allowed` is false for parameter
    /// declarations, whose bare identifier is not an implicit-`int` name.
    pub(in crate::translation_phases::parsing) fn parse_declaration_specifiers(
        &mut self,
        mode: SpecifierMode,
        implicit_name_allowed: bool,
    ) -> Option<DeclarationSpecifiers<'tu>> {
        let entry = self.enter_production(ProductionKind::DeclarationSpecifiers)?;
        let mut list = SpecifierList::new(mode, implicit_name_allowed);
        let result = self.parse_specifier_list(&mut list);
        self.leave_production(entry);
        result
    }

    fn parse_specifier_list(
        &mut self,
        list: &mut SpecifierList<'tu>,
    ) -> Option<DeclarationSpecifiers<'tu>> {
        loop {
            // EOF has two distinct failures. Keep them mutually exclusive
            // so one absent production does not cause a cascade of
            // specifier diagnostics.
            let Some(token) = self.cursor.current() else {
                if !list.consumed {
                    self.report(
                        ParserErrorType::UnexpectedEndBeforeDeclarationSpecifier,
                        None,
                    );
                } else if list.specifiers.type_specifiers == TypeSpecifiers::Empty
                    && !list.invalid_type_seen
                {
                    self.report(ParserErrorType::UnexpectedEndBeforeTypeSpecifier, None);
                }
                list.report_incomplete_complex(self);
                return Some(list.finish());
            };

            // GNU extension: `__extension__` may lead a declaration or a
            // member declaration. Its owner (the declaration, parameter, or
            // member) restores the suppression depth when it ends.
            if matches!(token.kind, TokenType::Keyword(KeywordTokenType::Extension))
                && matches!(
                    list.mode,
                    SpecifierMode::Declaration | SpecifierMode::StructMember
                )
            {
                self.pedantic_suppression += 1;
                list.add_extension(
                    self,
                    SpecifierExtensionKind::ExtensionMarker,
                    token.source_vectors,
                );
                list.consumed = true;
                self.merge_source(&mut list.source_vectors, token);
                self.consume_token();
                continue;
            }
            if let TokenType::Keyword(
                keyword @ (KeywordTokenType::Int128 | KeywordTokenType::AutoType),
            ) = token.kind
            {
                list.apply_gnu_type(self, keyword, token);
                self.consume_token();
                continue;
            }
            if list.apply_float128(self, token) || list.apply_msvc(self, token) {
                self.consume_token();
                continue;
            }
            if self.attribute_starter(Some(token)) {
                let returned =
                    self.parse_modern(ModernKind::Attributes, None, self.hard_error_count)?;
                let ModernValue::Attributes(attributes) = returned else {
                    unexpected_return!("attribute protocol: {returned:?}")
                };
                list.add_extension(
                    self,
                    SpecifierExtensionKind::Attributes(attributes),
                    attributes.source_vectors,
                );
                list.consumed = true;
                continue;
            }
            if let TokenType::Keyword(keyword) = token.kind {
                if matches!(
                    keyword,
                    KeywordTokenType::Alignas
                        | KeywordTokenType::BitInt
                        | KeywordTokenType::Typeof
                        | KeywordTokenType::TypeofUnqual
                ) || keyword == KeywordTokenType::Atomic
                    && self.cursor.following().is_some_and(|x| {
                        matches!(
                            x.kind,
                            TokenType::Operator(OperatorTokenType::OpeningParenthesis)
                        )
                    })
                {
                    self.parse_specifier_operand(list, keyword, token)?;
                    continue;
                }
                if list.apply_keyword(self, keyword, token) {
                    self.consume_token();
                    continue;
                }
            }
            if matches!(
                token.kind,
                TokenType::Keyword(KeywordTokenType::Struct | KeywordTokenType::Union)
            ) {
                // The keyword stays for diagnostics and type-conflict
                // provenance; the child consumes it as the first token it
                // owns, then this list reprocesses the token after it.
                let index = self.parse_struct_or_union_specifier()?;
                list.specifiers
                    .type_specifiers
                    .make_struct_or_union(self, index, token);
                self.context
                    .merge_into(&mut list.source_vectors, index.source_vectors);
                list.consumed = true;
                continue;
            }
            if matches!(token.kind, TokenType::Keyword(KeywordTokenType::Enum)) {
                // As above, the enum child owns its delimiters and returns
                // on the first token belonging to this specifier sequence.
                let EnumSpecifierResult {
                    index,
                    stopped_before_declaration,
                } = self.parse_enum_specifier()?;
                list.specifiers
                    .type_specifiers
                    .make_enum(self, index, token);
                self.context
                    .merge_into(&mut list.source_vectors, index.source_vectors);
                list.consumed = true;
                if stopped_before_declaration {
                    return Some(list.finish());
                }
                continue;
            }
            if list.apply_storage_class(self, token) || list.apply_ordinary(self, token) {
                self.consume_token();
                continue;
            }
            list.finish_before(self, token);
            return Some(list.finish());
        }
    }

    /// Parses the parenthesized operand of `_Alignas`, `_BitInt`, `typeof`,
    /// `typeof_unqual`, or the `_Atomic(...)` type specifier.
    fn parse_specifier_operand(
        &mut self,
        list: &mut SpecifierList<'tu>,
        keyword: KeywordTokenType,
        token: Token,
    ) -> Option<()> {
        // C17 §6.7.5p2 (C23 (N3220) §6.7.6p2): an alignment specifier
        // belongs only to a declaration, a member declaration, or a
        // compound literal's type name. Its operand still parses.
        if keyword == KeywordTokenType::Alignas && list.mode == SpecifierMode::TypeName {
            self.report(
                ParserErrorType::DeclarationSpecifierNotAllowedHere(token.kind),
                Some(token),
            );
        }
        if keyword != KeywordTokenType::Alignas
            && list.specifiers.type_specifiers != TypeSpecifiers::Empty
            && !(keyword == KeywordTokenType::BitInt
                && matches!(
                    list.specifiers.type_specifiers,
                    TypeSpecifiers::Signed | TypeSpecifiers::Unsigned
                ))
        {
            list.specifiers
                .type_specifiers
                .report_conflict(self, token.contents, token);
        }
        let returned = self.parse_modern(
            ModernKind::Operand {
                type_only: keyword == KeywordTokenType::Atomic,
                constant:  matches!(
                    keyword,
                    KeywordTokenType::Alignas | KeywordTokenType::BitInt
                ),
            },
            None,
            self.hard_error_count,
        )?;
        let ModernValue::Operand(operand, source) = returned else {
            unexpected_return!("specifier operand protocol: {returned:?}")
        };
        if keyword == KeywordTokenType::Alignas {
            list.add_extension(self, SpecifierExtensionKind::Alignment(operand), source);
        } else {
            let kind = match (keyword, operand) {
                | (KeywordTokenType::Atomic, SyntaxOperand::Type(x)) => ExtendedType::Atomic(x),
                | (KeywordTokenType::BitInt, SyntaxOperand::Expression(width)) =>
                    ExtendedType::BitInt {
                        width,
                        signedness: list.signedness(),
                    },
                | (_, operand) => ExtendedType::Typeof {
                    operand,
                    unqualified: keyword == KeywordTokenType::TypeofUnqual,
                },
            };
            list.specifiers.type_specifiers = TypeSpecifiers::Extended(self.alloc_syntax(kind));
            self.context.merge_into(&mut list.source_vectors, source);
        }
        list.consumed = true;
        Some(())
    }
}

/// Accumulates one declaration-specifier or specifier-qualifier sequence.
///
/// C99: §6.7, p. 97; PDF p. 109; §6.7.2.1, p. 101; PDF p. 113.
#[expect(
    clippy::struct_excessive_bools,
    reason = "The booleans record independent facts about the specifier sequence."
)]
struct SpecifierList<'tu> {
    /// Grammar context limiting legal specifier families.
    mode:                  SpecifierMode,
    /// Accumulated normalized specifier result.
    specifiers:            DeclarationSpecifiers<'tu>,
    /// Whether at least one legal specifier has been consumed.
    consumed:              bool,
    implicit_name_allowed: bool,
    /// Whether a storage-class specifier has already appeared.
    storage_seen:          bool,
    /// Whether an invalid token occupied the mandatory type-specifier slot.
    invalid_type_seen:     bool,
    /// Whether a type specifier conflicted with the ones before it; the
    /// list is then already diagnosed once.
    type_conflict_seen:    bool,
    /// The `_Complex` keyword, retained to diagnose a list that never
    /// supplies its required real floating type.
    complex_token:         Option<Token>,
    /// Provenance accumulated across the complete specifier sequence.
    source_vectors:        Option<SourceVectors>,
}

impl<'tu> SpecifierList<'tu> {
    fn new(mode: SpecifierMode, implicit_name_allowed: bool) -> Self {
        Self {
            mode,
            specifiers: DeclarationSpecifiers::new(),
            consumed: false,
            implicit_name_allowed,
            storage_seen: false,
            invalid_type_seen: false,
            type_conflict_seen: false,
            complex_token: None,
            source_vectors: None,
        }
    }

    fn finish(&mut self) -> DeclarationSpecifiers<'tu> {
        self.specifiers.source_vectors = self.source_vectors.unwrap_or_default();
        self.specifiers
    }

    /// The signedness a sized extended integer type inherits from a
    /// preceding `signed` or `unsigned`.
    fn signedness(&self) -> Option<bool> {
        match self.specifiers.type_specifiers {
            | TypeSpecifiers::Signed => Some(true),
            | TypeSpecifiers::Unsigned => Some(false),
            | _ => None,
        }
    }

    /// GNU `__int128` and `__auto_type`.
    fn apply_gnu_type(
        &mut self,
        parser: &mut Parser<'_, 'tu, '_>,
        keyword: KeywordTokenType,
        token: Token,
    ) {
        let extended = if keyword == KeywordTokenType::Int128 {
            let signedness = self.signedness();
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
        self.specifiers.type_specifiers = TypeSpecifiers::Extended(parser.alloc_syntax(extended));
        self.consumed = true;
        parser.merge_source(&mut self.source_vectors, token);
    }

    /// GCC Additional Floating Types: binary128 accepts _Complex in either
    /// order. Returns whether `token` was consumed.
    fn apply_float128(&mut self, parser: &mut Parser<'_, 'tu, '_>, token: Token) -> bool {
        if !matches!(token.kind, TokenType::Keyword(KeywordTokenType::Float128)) {
            return false;
        }
        let complex = self.specifiers.type_specifiers == TypeSpecifiers::Complex;
        if !matches!(
            self.specifiers.type_specifiers,
            TypeSpecifiers::Empty | TypeSpecifiers::Complex
        ) {
            self.specifiers
                .type_specifiers
                .report_conflict(parser, token.contents, token);
        }
        self.specifiers.type_specifiers =
            TypeSpecifiers::Extended(parser.alloc_syntax(ExtendedType::Float128 { complex }));
        self.consumed = true;
        parser.merge_source(&mut self.source_vectors, token);
        true
    }

    /// MSVC extensions to C99 §6.7.2, pp. 99-100; PDF pp. 111-112 and
    /// §6.7.5, p. 114; PDF p. 126. Width and ABI interpretation are deferred.
    /// Returns whether `token` was consumed.
    fn apply_msvc(&mut self, parser: &mut Parser<'_, 'tu, '_>, token: Token) -> bool {
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
            return true;
        }
        if let TokenType::Keyword(keyword) = token.kind
            && calling_convention(keyword)
        {
            self.add_extension(
                parser,
                SpecifierExtensionKind::MsModifier(keyword),
                token.source_vectors,
            );
            self.consumed = true;
            return true;
        }
        false
    }

    /// Thread storage, `constexpr`, `_Noreturn`, and decimal floating
    /// keywords. Returns whether `token` was consumed.
    fn apply_keyword(
        &mut self,
        parser: &mut Parser<'_, 'tu, '_>,
        keyword: KeywordTokenType,
        token: Token,
    ) -> bool {
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
                    Feature::C23Keywords,
                    "storage class in compound literal",
                    token,
                );
            }
            self.add_storage_extension(parser, keyword, token);
            self.consumed = true;
            return true;
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
            return true;
        }
        let decimal = match keyword {
            | KeywordTokenType::Decimal32 => ExtendedType::Decimal32,
            | KeywordTokenType::Decimal64 => ExtendedType::Decimal64,
            | KeywordTokenType::Decimal128 => ExtendedType::Decimal128,
            | _ => return false,
        };
        if self.specifiers.type_specifiers != TypeSpecifiers::Empty {
            self.specifiers
                .type_specifiers
                .report_conflict(parser, token.contents, token);
        }
        self.specifiers.type_specifiers = TypeSpecifiers::Extended(parser.alloc_syntax(decimal));
        parser.merge_source(&mut self.source_vectors, token);
        self.consumed = true;
        true
    }

    /// A storage-class specifier. Returns whether `token` was consumed.
    ///
    /// C99: §6.7.1 paragraphs 1-2, p. 98; PDF p. 110.
    fn apply_storage_class(&mut self, parser: &mut Parser<'_, 'tu, '_>, token: Token) -> bool {
        let Some(storage_class) = storage_class(token.kind) else {
            return false;
        };
        if self.mode == SpecifierMode::CompoundLiteral {
            parser.extension(
                Feature::C23Keywords,
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
            return true;
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
                && parser.context.configuration.standard() >= CStandard::C23
            {
                self.specifiers.auto_with_storage_class = true;
                if storage_class != StorageClass::Auto {
                    self.specifiers.storage_class = Some(storage_class);
                }
                parser.merge_source(&mut self.source_vectors, token);
                self.consumed = true;
                return true;
            }
            parser.report(
                ParserErrorType::StorageClassRedefinition(previous, token.kind),
                Some(token),
            );
        }
        self.report_storage_after_thread(parser, storage_class, token);
        // Replacing the storage class also discards its C23 `auto` pairing.
        self.specifiers.auto_with_storage_class = false;
        self.specifiers.storage_class = Some(storage_class);
        self.storage_seen = true;
        parser.merge_source(&mut self.source_vectors, token);
        self.consumed = true;
        true
    }

    /// Primitive type specifiers, qualifiers, `inline`, typedef names, and
    /// an unknown type name before a declarator. Returns whether `token`
    /// was consumed.
    fn apply_ordinary(&mut self, parser: &mut Parser<'_, 'tu, '_>, token: Token) -> bool {
        if let TokenType::Keyword(keyword) = token.kind
            && let Some(specifier) = primitive_type_specifier(keyword)
        {
            let reported_before = parser.context.pending_errors.len();
            self.apply_type_specifier(parser, token, specifier);
            self.type_conflict_seen |= parser.context.pending_errors.len() != reported_before;
            parser.merge_source(&mut self.source_vectors, token);
            self.consumed = true;
            return true;
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
            return true;
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
                    SpecifierExtensionKind::MsModifier(KeywordTokenType::Forceinline),
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
                return true;
            }
            if self.specifiers.function_specifiers.is_inline {
                parser.report(ParserErrorType::InlineSpecifiedTwice, Some(token));
            }
            self.specifiers.function_specifiers.is_inline = true;
            parser.merge_source(&mut self.source_vectors, token);
            self.consumed = true;
            return true;
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
            // declarator name. Consume it as a specifier only when no type
            // is present yet or lookahead proves another declarator follows.
            self.specifiers.type_specifiers.make_typedef_name(
                parser,
                Identifier::from_token(token),
                token,
            );
            parser.merge_source(&mut self.source_vectors, token);
            self.consumed = true;
            return true;
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
            return true;
        }
        false
    }

    /// Ends the list before `token`, which belongs to the parent, reporting
    /// any missing mandatory component. One diagnostic per missing piece: a
    /// bare identifier is a name lacking its type, while any other token
    /// means no declaration started here at all.
    ///
    /// C99 §6.7.2p2: at least one type specifier in each declaration, struct
    /// declaration, and type name.
    fn finish_before(&mut self, parser: &mut Parser<'_, 'tu, '_>, token: Token) {
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
                    && parser.context.configuration.standard() >= CStandard::C23
                {
                    self.specifiers.type_specifiers =
                        TypeSpecifiers::Extended(parser.alloc_syntax(ExtendedType::Inferred));
                } else {
                    parser.extension(Feature::ImplicitInt, "implicit int", token);
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

    /// Adds a thread storage-class specifier or `constexpr`, unless it
    /// repeats one already present.
    fn add_storage_extension(
        &mut self,
        parser: &mut Parser<'_, 'tu, '_>,
        keyword: KeywordTokenType,
        token: Token,
    ) {
        let kind = if keyword == KeywordTokenType::ThreadLocal {
            SpecifierExtensionKind::ThreadLocal(thread_spelling(parser, token))
        } else {
            SpecifierExtensionKind::Constexpr
        };
        if self.report_thread_conflict(parser, kind, token) {
            parser.merge_source(&mut self.source_vectors, token);
        } else {
            self.add_extension(parser, kind, token.source_vectors);
        }
    }

    /// Reports a storage class that cannot follow the thread specifier.
    /// C11: §6.7.1 paragraph 2, p. 109; PDF p. 127.
    fn report_storage_after_thread(
        &self,
        parser: &mut Parser<'_, 'tu, '_>,
        storage_class: StorageClass,
        token: Token,
    ) {
        if let Some(thread) = self.thread_spelling()
            && thread_conflicts(parser, storage_class)
        {
            parser.report(
                ParserErrorType::ThreadStorageConflict {
                    previous:    thread.spelling(),
                    conflicting: storage_class.spelling(),
                },
                Some(token),
            );
        }
    }

    /// The thread storage-class specifier already in the list, if any.
    fn thread_spelling(&self) -> Option<ThreadSpelling> {
        let mut extension = self.specifiers.extensions;
        while let Some(item) = extension {
            if let SpecifierExtensionKind::ThreadLocal(spelling) = item.kind {
                return Some(spelling);
            }
            extension = item.next;
        }
        None
    }

    /// Whether `constexpr` is already in the list.
    fn has_constexpr(&self) -> bool {
        let mut extension = self.specifiers.extensions;
        while let Some(item) = extension {
            if item.kind == SpecifierExtensionKind::Constexpr {
                return true;
            }
            extension = item.next;
        }
        false
    }

    /// Reports a thread storage-class specifier or `constexpr` that cannot
    /// join the specifiers before it. Returns whether `kind` repeats one
    /// already present, which then adds nothing.
    /// C11: §6.7.1 paragraph 2, p. 109; PDF p. 127. C23 (N3220): §6.7.2
    /// paragraph 2, p. 99; PDF p. 112.
    fn report_thread_conflict(
        &self,
        parser: &mut Parser<'_, 'tu, '_>,
        kind: SpecifierExtensionKind<'tu>,
        token: Token,
    ) -> bool {
        let previous = self.thread_spelling();
        let conflict = if let SpecifierExtensionKind::ThreadLocal(spelling) = kind {
            if let Some(previous) = previous
                && previous.repeats(spelling)
            {
                parser.report(
                    ParserErrorType::ThreadLocalSpecifiedTwice(spelling.spelling()),
                    Some(token),
                );
                return true;
            }
            previous
                .map(ThreadSpelling::spelling)
                .or_else(|| self.has_constexpr().then_some("constexpr"))
                .or_else(|| {
                    self.specifiers
                        .storage_class
                        .filter(|&storage_class| thread_conflicts(parser, storage_class))
                        .map(StorageClass::spelling)
                })
                .map(|previous| (previous, spelling.spelling()))
        } else {
            previous.map(|previous| (previous.spelling(), "constexpr"))
        };
        if let Some((previous, conflicting)) = conflict {
            parser.report(
                ParserErrorType::ThreadStorageConflict {
                    previous,
                    conflicting,
                },
                Some(token),
            );
        }
        false
    }

    fn add_extension(
        &mut self,
        parser: &mut Parser<'_, 'tu, '_>,
        kind: SpecifierExtensionKind<'tu>,
        source_vectors: SourceVectors,
    ) {
        self.specifiers.extensions = Some(parser.alloc_syntax(SpecifierExtension {
            kind,
            next: self.specifiers.extensions,
            source_vectors,
        }));
        parser
            .context
            .merge_into(&mut self.source_vectors, source_vectors);
    }

    /// Diagnoses a finished list whose `_Complex` never received `float`,
    /// `double`, or `long double`.
    ///
    /// C99: §6.7.2 paragraph 2, pp. 99-100; PDF pp. 111-112 lists only
    /// `float _Complex`, `double _Complex`, and `long double _Complex`.
    fn report_incomplete_complex(&self, parser: &mut Parser<'_, 'tu, '_>) {
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
        parser: &mut Parser<'_, 'tu, '_>,
        token: Token,
        specifier: PrimitiveTypeSpecifier,
    ) {
        let sign = matches!(
            specifier,
            PrimitiveTypeSpecifier::Signed | PrimitiveTypeSpecifier::Unsigned
        );
        let signed = matches!(specifier, PrimitiveTypeSpecifier::Signed);
        match self.specifiers.type_specifiers {
            | TypeSpecifiers::Extended(ExtendedType::MsInteger { width, signedness }) if sign => {
                if signedness.is_some() {
                    self.specifiers
                        .type_specifiers
                        .report_conflict(parser, token.contents, token);
                } else {
                    self.specifiers.type_specifiers =
                        TypeSpecifiers::Extended(parser.alloc_syntax(ExtendedType::MsInteger {
                            width:      *width,
                            signedness: Some(signed),
                        }));
                }
                return;
            },
            | TypeSpecifiers::Extended(ExtendedType::Int128 { signedness }) if sign => {
                if signedness.is_some() {
                    self.specifiers
                        .type_specifiers
                        .report_conflict(parser, token.contents, token);
                } else {
                    self.specifiers.type_specifiers =
                        TypeSpecifiers::Extended(parser.alloc_syntax(ExtendedType::Int128 {
                            signedness: Some(signed),
                        }));
                }
                return;
            },
            | TypeSpecifiers::Extended(ExtendedType::BitInt { width, signedness }) if sign => {
                if signedness.is_some() {
                    self.specifiers
                        .type_specifiers
                        .report_conflict(parser, token.contents, token);
                } else {
                    self.specifiers.type_specifiers =
                        TypeSpecifiers::Extended(parser.alloc_syntax(ExtendedType::BitInt {
                            width,
                            signedness: Some(signed),
                        }));
                }
                return;
            },
            | TypeSpecifiers::Extended(ExtendedType::Float128 { complex }) => {
                if matches!(specifier, PrimitiveTypeSpecifier::Complex) && !complex {
                    self.specifiers.type_specifiers = TypeSpecifiers::Extended(
                        parser.alloc_syntax(ExtendedType::Float128 { complex: true }),
                    );
                } else {
                    self.specifiers
                        .type_specifiers
                        .report_conflict(parser, token.contents, token);
                }
                return;
            },
            | _ => {},
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
                    parser.extension(Feature::LongLong, "long long", token);
                }
                type_specifiers.make_long(parser, token);
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

/// Grammar context controlling which specifier families are legal.
///
/// C99: declaration-specifiers are §6.7, p. 97; PDF p. 109, while
/// specifier-qualifier-list is §6.7.2.1, p. 101; PDF p. 113. A type name
/// takes a specifier-qualifier-list too, §6.7.6 paragraph 1, p. 122;
/// PDF p. 134.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::translation_phases::parsing) enum SpecifierMode {
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

/// Primitive keyword recognized while accumulating a C type-specifier set.
///
/// C99: normative type-specifiers are §6.7.2, pp. 99-100; PDF pp. 111-112.
/// `_Imaginary` comes from the keyword inventory in §6.4.1, p. 50; PDF p. 62
/// and is retained here only for the existing extension path. It is reserved
/// for imaginary types (§6.4.1 paragraph 2, p. 50; PDF p. 62), which only
/// informative annex G specifies; the specifier production rejects it.
#[derive(Debug, Clone, Copy)]
pub(in crate::translation_phases::parsing) enum PrimitiveTypeSpecifier {
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

/// How a thread storage-class keyword was written: `_Thread_local` keeps
/// the keyword's own spelling ID, and the aliases keep theirs.
/// C11: §6.7.1 paragraph 1, p. 109; PDF p. 127.
pub(in crate::translation_phases::parsing) fn thread_spelling(
    parser: &Parser<'_, '_, '_>,
    token: Token,
) -> ThreadSpelling {
    if token.contents == KeywordTokenType::ThreadLocal.cache_id() {
        ThreadSpelling::Reserved
    } else if parser.context.string_cache.at(token.contents) == "__thread" {
        ThreadSpelling::Gnu
    } else {
        ThreadSpelling::Keyword
    }
}

/// Whether a storage-class specifier cannot accompany thread storage:
/// only `static` and `extern` can, and from C23 `auto` too.
/// C11: §6.7.1 paragraph 2, p. 109; PDF p. 127. C23 (N3220): §6.7.2
/// paragraph 2, p. 99; PDF p. 112.
pub(in crate::translation_phases::parsing) fn thread_conflicts(
    parser: &Parser<'_, '_, '_>,
    storage_class: StorageClass,
) -> bool {
    match storage_class {
        | StorageClass::Static | StorageClass::Extern => false,
        | StorageClass::Auto => parser.context.configuration.standard() < CStandard::C23,
        | StorageClass::Register | StorageClass::Typedef => true,
    }
}

/// Classifies one storage-class-specifier keyword.
///
/// C99: §6.7.1 paragraph 1, p. 98; PDF p. 110. `typedef` is a storage-class
/// specifier for syntactic convenience only (paragraph 3, p. 98;
/// PDF p. 110).
pub(in crate::translation_phases::parsing) fn storage_class(
    token: TokenType,
) -> Option<StorageClass> {
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
pub(in crate::translation_phases::parsing) fn type_qualifier(
    token: TokenType,
) -> Option<TypeQualifiers> {
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
#[cold]
#[inline(never)]
pub(in crate::translation_phases::parsing) fn report_duplicate_type_qualifier(
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

/// Classifies a keyword for [`PrimitiveTypeSpecifier`] accumulation.
///
/// C99: §6.7.2, pp. 99-100; PDF pp. 111-112; `_Imaginary` caveat as documented
/// on [`PrimitiveTypeSpecifier`].
pub(in crate::translation_phases::parsing) fn primitive_type_specifier(
    keyword: KeywordTokenType,
) -> Option<PrimitiveTypeSpecifier> {
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
