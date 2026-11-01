//! Recursive struct and union specifier grammar.
//!
//! Translation phase 7 syntax analysis (§5.1.1.2, p. 10; PDF p. 22) of
//! `struct-or-union-specifier`, `struct-declaration-list`,
//! `struct-declaration`, `struct-declarator-list`, and `struct-declarator`
//! (C99: §6.7.2.1 paragraph 1, p. 101; PDF p. 113; §A.2.2, p. 412;
//! PDF p. 424), with the tag forms of §6.7.2.3, pp. 106-107;
//! PDF pp. 118-119.
//!
//! Tags and members have their own name spaces (§6.2.3 paragraph 1, p. 31;
//! PDF p. 43), so neither touches the scope stack. Diagnosed here: syntax
//! the grammar rejects, including an empty member list and a member
//! declaration without a declarator, which C99 does not allow. Left to
//! semantic analysis: the member constraints of §6.7.2.1 paragraphs 2-4,
//! p. 101; PDF p. 113, the named-member and layout rules of paragraphs 7-16,
//! pp. 102-103; PDF pp. 114-115, and the tag constraints of §6.7.2.3
//! paragraphs 1-2, p. 106; PDF p. 118. Member specifiers recurse directly;
//! member declarators, bit-field widths, attributes and assertions run
//! through their own families.

use crate::{
    configuration::Feature,
    translation_phases::{
        SourceVectors,
        parsing::{
            Parser,
            declaration_syntax::{
                DeclarationSpecifiers,
                Declarator,
                StructDeclaration,
                StructDeclarator,
                StructOrUnion,
                StructOrUnionSpecifier,
                TypeQualifiers,
                TypeSpecifiers,
            },
            errors::ParserErrorType,
            extensions::modern::{
                SpecifierExtension,
                SpecifierExtensionKind,
            },
            grammar::{
                ProductionKind,
                declaration_specifiers::SpecifierMode,
                declarators::DeclaratorMode,
                expression_operators::is_operator,
                expressions::{
                    ConstantExpressionResult,
                    ExpressionBoundary,
                },
                modern::{
                    ModernKind,
                    ModernValue,
                },
                statements::is_statement_keyword,
                unexpected_return,
            },
            recovery::{
                SynchronizationKind,
                SynchronizationSet,
            },
            syntax::Identifier,
        },
        preprocessing::{
            KeywordTokenType,
            OperatorTokenType,
            Token,
            TokenType,
        },
    },
    util::{
        arena_list::ArenaList,
        bump::ArenaVec,
    },
};

impl<'tu, 'p> Parser<'_, 'tu, 'p> {
    /// Parses a struct-or-union specifier, including its optional tag and
    /// member declaration list, starting at its keyword.
    ///
    /// C99: structure and union specifiers, member declarations, and
    /// bit-fields are §6.7.2.1, pp. 101-104; PDF pp. 113-116.
    pub(in crate::translation_phases::parsing) fn parse_struct_or_union_specifier(
        &mut self,
    ) -> Option<&'tu StructOrUnionSpecifier<'tu>> {
        let entry = self.enter_production(ProductionKind::StructOrUnionSpecifier)?;
        let mut record = Record::new(self);
        let result = self
            .parse_record(&mut record)
            .map(|()| self.finish_record(&mut record));
        self.pools.struct_members.reclaim(&mut record.declarations);
        self.pools
            .struct_declarators
            .reclaim(&mut record.member_declarators);
        self.pools
            .source_vectors
            .reclaim(&mut record.source_vectors);
        self.leave_production(entry);
        result
    }

    fn parse_record(&mut self, record: &mut Record<'tu, 'p>) -> Option<()> {
        record.suppression_entry = self.pedantic_suppression;
        let Some(token) = self.cursor.current() else {
            self.report(ParserErrorType::ExpectedStructOrUnionKeyword(None), None);
            return Some(());
        };
        record.kind = match token.kind {
            | TokenType::Keyword(KeywordTokenType::Struct) => Some(StructOrUnion::Struct),
            | TokenType::Keyword(KeywordTokenType::Union) => Some(StructOrUnion::Union),
            | _ => {
                self.report(
                    ParserErrorType::ExpectedStructOrUnionKeyword(Some(token.kind)),
                    Some(token),
                );
                None
            },
        };
        record.source_vectors.push(token.source_vectors);
        self.consume_token();

        while self.attribute_starter(self.cursor.current()) {
            self.parse_record_attributes(record)?;
        }
        // The grammar allows either a tagged reference, a tagged definition,
        // or an anonymous definition. Record the tag first and decide
        // whether a body follows.
        let token = self.cursor.current();
        if let Some(token) = token
            && matches!(token.kind, TokenType::Identifier)
        {
            record.identifier = Some(Identifier::from_token(token));
            record.source_vectors.push(token.source_vectors);
            self.consume_token();
            if !is_operator(self.cursor.current(), OperatorTokenType::OpeningCurlyBrace) {
                return Some(());
            }
        } else if !is_operator(token, OperatorTokenType::OpeningCurlyBrace) {
            // C99 §6.7.2.1p1: `struct` and `union` take a tag, a body, or
            // both.
            self.report(
                ParserErrorType::StructOrUnionSpecifierWithoutNameAndBody(
                    token.map(|token| token.kind),
                ),
                token,
            );
            return Some(());
        }
        let token = self
            .cursor
            .current()
            .expect("opening-curly-brace token exists");
        record.body_started = true;
        record.pack_position = self.cursor.consumed;
        record.source_vectors.push(token.source_vectors);
        self.consume_token();
        self.parse_record_members(record)?;

        // Microsoft's `__declspec` after a record body is a declaration
        // specifier of the enclosing declaration, as in MSVC and Clang:
        // `struct S { int x; } __declspec(align(1)) s;` aligns `s`, not
        // `struct S`. GNU attributes there describe the record.
        // MSVC extension: Microsoft Learn, "declspec".
        while self.attribute_starter(self.cursor.current())
            && !self.cursor.current().is_some_and(|token| {
                matches!(token.kind, TokenType::Keyword(KeywordTokenType::Declspec))
            })
        {
            self.parse_record_attributes(record)?;
        }
        record.body_end_position = self.cursor.consumed;
        Some(())
    }

    /// Parses tag attributes before the tag or after the body.
    fn parse_record_attributes(&mut self, record: &mut Record<'tu, 'p>) -> Option<()> {
        let returned = self.parse_modern(ModernKind::Attributes, None, self.hard_error_count)?;
        let ModernValue::Attributes(x) = returned else {
            unexpected_return!("aggregate attributes protocol: {returned:?}")
        };
        record.attributes = Some(self.alloc_syntax(SpecifierExtension {
            kind:           SpecifierExtensionKind::Attributes(x),
            next:           record.attributes,
            source_vectors: x.source_vectors,
        }));
        record.source_vectors.push(x.source_vectors);
        Some(())
    }

    /// Parses member declarations through the body's `}`, which it owns.
    fn parse_record_members(&mut self, record: &mut Record<'tu, 'p>) -> Option<()> {
        loop {
            let token = self.cursor.current();
            if token.is_some_and(|x| {
                matches!(x.kind, TokenType::Keyword(KeywordTokenType::StaticAssert))
            }) {
                let returned =
                    self.parse_modern(ModernKind::Assertion, None, self.hard_error_count)?;
                let ModernValue::Assertion(assertion) = returned else {
                    unexpected_return!("member assertion protocol: {returned:?}")
                };
                record.declarations.push(StructDeclaration {
                    assertion:              Some(assertion),
                    extensions:             None,
                    type_qualifiers:        TypeQualifiers::empty(),
                    type_specifiers:        TypeSpecifiers::Empty,
                    struct_declarator_list: ArenaList::empty(),
                    source_vectors:         assertion.source_vectors,
                });
                record.source_vectors.push(assertion.source_vectors);
                continue;
            }
            if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
                let token = token.expect("closing-curly-brace token exists");
                // C99 §6.7.2.1p1: a struct-declaration-list has at least one
                // struct-declaration.
                if record.declarations.is_empty() {
                    self.extension(Feature::EmptyStructs, "empty struct or union", token);
                }
                record.source_vectors.push(token.source_vectors);
                self.consume_token();
                return Some(());
            }
            let Some(token) = token else {
                self.report(
                    ParserErrorType::ExpectedClosingCurlyBraceInStructDeclarationList(None),
                    None,
                );
                return Some(());
            };
            if is_operator(Some(token), OperatorTokenType::Semicolon) {
                // C99 §6.7.2.1p1: a struct-declaration begins with a
                // specifier-qualifier-list. GNU extension: an extra `;` in the
                // list declares nothing, so it does not make the list
                // nonempty.
                self.extension(
                    Feature::ExtraSemicolons,
                    "extra semicolon in a struct or union",
                    token,
                );
                record.source_vectors.push(token.source_vectors);
                self.consume_token();
                continue;
            }
            record.member_source = None;
            record.current_member_declarator_source = None;
            let specifiers =
                self.parse_declaration_specifiers(SpecifierMode::StructMember, true)?;
            if !self.parse_struct_declaration(record, specifiers)? {
                return Some(());
            }
        }
    }

    /// Parses the struct-declarator-list after one member's specifiers.
    /// Returns whether the member list continues; false means the body's
    /// `}` is missing and the specifier ends here.
    ///
    /// C99: §6.7.2.1 paragraph 1, p. 101; PDF p. 113.
    fn parse_struct_declaration(
        &mut self,
        record: &mut Record<'tu, 'p>,
        specifiers: DeclarationSpecifiers<'tu>,
    ) -> Option<bool> {
        record.member_specifiers = Some(specifiers);
        record.member_source = Some(specifiers.source_vectors);
        let token = self.cursor.current();
        if let Some(token) = token
            && matches!(
                token.kind,
                TokenType::Operator(OperatorTokenType::Semicolon)
            )
        {
            self.member_declares_nothing(specifiers, token);
            self.merge_source(&mut record.member_source, token);
            record.finish_member(self);
            self.consume_token();
            return Some(true);
        }
        // A leading colon is the unnamed-bit-field alternative; it
        // deliberately bypasses the named declarator child.
        // C99 §6.7.2.1p1 and p11.
        loop {
            if let Some(token) = self
                .cursor
                .current()
                .filter(|&token| is_operator(Some(token), OperatorTokenType::Colon))
            {
                self.merge_source(&mut record.member_source, token);
                self.merge_source(&mut record.current_member_declarator_source, token);
                record.member_declarator = None;
                self.consume_token();
                self.parse_bit_field_width(record)?;
            } else {
                let declarator = self.parse_declarator(DeclaratorMode::Named)?;
                record.member_declarator = declarator;
                if let Some(source_vectors) = declarator.map(|declarator| declarator.source_vectors)
                {
                    self.context
                        .merge_into(&mut record.member_source, source_vectors);
                    record.current_member_declarator_source = Some(source_vectors);
                }
                // Colon upgrades the completed member into a named bit-field.
                // Otherwise commit it as an ordinary member without consuming
                // the separator that follows.
                if let Some(token) = self
                    .cursor
                    .current()
                    .filter(|&token| is_operator(Some(token), OperatorTokenType::Colon))
                {
                    self.merge_source(&mut record.member_source, token);
                    self.merge_source(&mut record.current_member_declarator_source, token);
                    self.consume_token();
                    self.parse_bit_field_width(record)?;
                } else {
                    let source_vectors = record
                        .current_member_declarator_source
                        .take()
                        .unwrap_or_default();
                    record.member_declarators.push(StructDeclarator {
                        attributes: None,
                        declarator: record.member_declarator.take(),
                        bitfield_width: None,
                        source_vectors,
                    });
                }
            }
            match self.parse_after_struct_declarator(record)? {
                | DeclaratorListEnd::Comma => {},
                | DeclaratorListEnd::Member => return Some(true),
                | DeclaratorListEnd::Body => return Some(false),
            }
        }
    }

    /// C99 §6.7.2.1p1: a struct-declaration needs a struct-declarator-list;
    /// C99 has no anonymous members.
    fn member_declares_nothing(&mut self, specifiers: DeclarationSpecifiers<'tu>, token: Token) {
        let anonymous = matches!(
            specifiers.type_specifiers,
            TypeSpecifiers::StructOrUnion(x)
                if x.identifier.is_none() && x.struct_declaration_list.is_some()
        );
        // MSVC also accepts a typedef name of a structure or union; whether
        // the typedef names one is semantic.
        let ms_anonymous = matches!(specifiers.type_specifiers, TypeSpecifiers::TypedefName(_))
            || matches!(
                specifiers.type_specifiers,
                TypeSpecifiers::StructOrUnion(x) if x.identifier.is_some()
            );
        let ms_anonymous = ms_anonymous
            && self
                .context
                .configuration
                .accepts(Feature::MsAnonymousStructs);
        if ms_anonymous {
            self.extension(
                Feature::MsAnonymousStructs,
                "anonymous tagged struct or union member",
                token,
            );
        } else if anonymous {
            self.extension(
                Feature::AnonymousAggregates,
                "anonymous struct or union member",
                token,
            );
        } else {
            // C99: §6.7.2.1p1, p. 101; PDF p. 113. Clang accepts type-only
            // member declarations with -Wmissing-declarations; tagged records
            // do not become C11 anonymous members.
            self.report(ParserErrorType::MemberDeclaresNothing, Some(token));
        }
    }

    /// Parses the constant-expression bit-field width after its `:` and
    /// commits the member declarator.
    fn parse_bit_field_width(&mut self, record: &mut Record<'tu, 'p>) -> Option<()> {
        let ConstantExpressionResult {
            expression: index,
            recovered,
        } = self.parse_constant_expression(ExpressionBoundary::StructMember)?;
        record.width_recovered = recovered;
        let source_vectors = index.expression().source_vectors;
        self.context
            .merge_into(&mut record.member_source, source_vectors);
        self.context
            .merge_into(&mut record.current_member_declarator_source, source_vectors);
        let source_vectors = record
            .current_member_declarator_source
            .take()
            .unwrap_or_default();
        record.member_declarators.push(StructDeclarator {
            attributes: None,
            declarator: record.member_declarator.take(),
            bitfield_width: Some(index),
            source_vectors,
        });
        Some(())
    }

    /// Requires `,` or `;` after one struct declarator.
    fn parse_after_struct_declarator(
        &mut self,
        record: &mut Record<'tu, 'p>,
    ) -> Option<DeclaratorListEnd> {
        loop {
            let token = self.cursor.current();
            if self.attribute_starter(token) {
                let returned =
                    self.parse_modern(ModernKind::Attributes, None, self.hard_error_count)?;
                let ModernValue::Attributes(x) = returned else {
                    panic!("bit-field attribute child protocol");
                };
                if let Some(member) = record.member_declarators.last_mut() {
                    member.attributes = Some(self.alloc_syntax(SpecifierExtension {
                        kind:           SpecifierExtensionKind::Attributes(x),
                        next:           member.attributes,
                        source_vectors: x.source_vectors,
                    }));
                    member.source_vectors = self
                        .context
                        .merge_vectors(member.source_vectors, x.source_vectors);
                }
                self.context
                    .merge_into(&mut record.member_source, x.source_vectors);
                continue;
            }
            // `,` stays inside one struct-declarator-list; `;` commits the
            // whole struct-declaration. Enclosing delimiters are repaired and
            // reprocessed rather than swallowed here.
            let resuming_after_recovery =
                std::mem::take(&mut record.resuming_after_member_recovery);
            let width_recovered = std::mem::take(&mut record.width_recovered);
            if is_operator(token, OperatorTokenType::Comma) {
                let token = token.expect("comma token exists");
                self.merge_source(&mut record.member_source, token);
                self.consume_token();
                return Some(DeclaratorListEnd::Comma);
            }
            if is_operator(token, OperatorTokenType::Semicolon) {
                let token = token.expect("semicolon token exists");
                self.merge_source(&mut record.member_source, token);
                record.finish_member(self);
                self.consume_token();
                return Some(DeclaratorListEnd::Member);
            }
            if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
                if !resuming_after_recovery {
                    self.report(
                        ParserErrorType::ExpectedSemicolonBeforeClosingCurlyBraceInStructDeclaratorList,
                        token,
                    );
                }
                record.finish_member(self);
                return Some(DeclaratorListEnd::Member);
            }
            if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                if member_list_continues(self) {
                    // A stray `)` cannot end the member list; its `}`
                    // follows, so the member continues after it.
                    if !(resuming_after_recovery || width_recovered) {
                        self.report(
                            ParserErrorType::ExpectedCommaOrSemicolonInStructDeclaratorList(
                                token.map(|token| token.kind),
                            ),
                            token,
                        );
                    }
                    let token = token.expect("closing-parenthesis token exists");
                    self.merge_source(&mut record.member_source, token);
                    // The malformed terminator is diagnosed, so a `}` or
                    // following member that ends this one is not.
                    record.resuming_after_member_recovery = true;
                    self.consume_token();
                    continue;
                }
                self.report(
                    ParserErrorType::ExpectedClosingCurlyBraceInStructDeclarationList(
                        token.map(|token| token.kind),
                    ),
                    token,
                );
                record.finish_member(self);
                return Some(DeclaratorListEnd::Body);
            }
            if token.is_some_and(|token| self.declaration_starter(token)) {
                if !resuming_after_recovery {
                    self.report(
                        ParserErrorType::ExpectedCommaOrSemicolonInStructDeclaratorList(
                            token.map(|token| token.kind),
                        ),
                        token,
                    );
                }
                record.finish_member(self);
                return Some(DeclaratorListEnd::Member);
            }
            self.report(
                ParserErrorType::ExpectedCommaOrSemicolonInStructDeclaratorList(
                    token.map(|token| token.kind),
                ),
                token,
            );
            if token.is_none() {
                record.finish_member(self);
                return Some(DeclaratorListEnd::Body);
            }
            record.resuming_after_member_recovery = true;
            if let Some(recovered) = self.recover(SynchronizationSet {
                kind:   SynchronizationKind::StructMember,
                target: ProductionKind::StructOrUnionSpecifier,
            }) {
                record.source_vectors.push(recovered);
            }
        }
    }

    fn finish_record(&mut self, record: &mut Record<'tu, 'p>) -> &'tu StructOrUnionSpecifier<'tu> {
        self.pedantic_suppression = record.suppression_entry;
        let declaration_list = record
            .body_started
            .then(|| self.alloc_syntax_list(&mut record.declarations));
        let source_vectors = self.context.merge_vector_list(&record.source_vectors);
        self.alloc_syntax(StructOrUnionSpecifier {
            pack_position: record.pack_position,
            body_end_position: record.body_end_position,
            attributes: record.attributes,
            struct_or_union: record.kind.unwrap_or(StructOrUnion::Struct),
            identifier: record.identifier,
            struct_declaration_list: declaration_list,
            source_vectors,
        })
    }
}

/// How a struct-declarator ended.
#[derive(Clone, Copy)]
enum DeclaratorListEnd {
    /// A `,`: another struct-declarator follows.
    Comma,
    /// The member declaration ended; the member list continues.
    Member,
    /// The body's `}` is missing; the specifier ends here.
    Body,
}

/// One struct-or-union specifier under construction.
///
/// C99: §6.7.2.1, pp. 101-104; PDF pp. 113-116.
struct Record<'tu, 'p> {
    attributes: Option<&'tu SpecifierExtension<'tu>>,
    /// Keyword-selected aggregate kind.
    kind: Option<StructOrUnion>,
    /// Optional tag identifier.
    identifier: Option<Identifier>,
    /// Completed member declarations before arena insertion.
    declarations: ArenaVec<'p, StructDeclaration<'tu>>,
    /// Declarators belonging to the member declaration in progress.
    member_declarators: ArenaVec<'p, StructDeclarator<'tu>>,
    /// Specifiers shared by the member declarators in progress.
    member_specifiers: Option<DeclarationSpecifiers<'tu>>,
    /// Named declarator waiting for an optional bit-field width.
    member_declarator: Option<Declarator<'tu>>,
    /// Whether `{` was consumed, distinguishing a reference from a definition.
    body_started: bool,
    pack_position: usize,
    body_end_position: usize,
    /// Whether member synchronization just ran after a reported malformed
    /// struct-declarator terminator, so stopping before `}` or a following
    /// declaration finishes the member without a second diagnostic.
    resuming_after_member_recovery: bool,
    /// Whether the bit-field width before the current separator position
    /// already reported an error, so a stray `)` there is not diagnosed
    /// again.
    width_recovered: bool,
    /// Provenance accumulated across the complete tag specifier.
    source_vectors: ArenaVec<'p, SourceVectors>,
    /// Provenance accumulated for the member declaration in progress.
    member_source: Option<SourceVectors>,
    /// Provenance for the member declarator/bit-field currently being built.
    current_member_declarator_source: Option<SourceVectors>,
    /// `__extension__` suppression depth when the specifier began. Each
    /// member restores it, so a marker before one member covers only that
    /// member (GNU extension; C99 §5.1.1.3, p. 11; PDF p. 23).
    suppression_entry: usize,
}

impl<'tu, 'p> Record<'tu, 'p> {
    fn new(parser: &mut Parser<'_, 'tu, 'p>) -> Self {
        let mut record = Self {
            attributes: None,
            kind: None,
            identifier: None,
            declarations: ArenaVec::new_in(parser.arena),
            member_declarators: ArenaVec::new_in(parser.arena),
            member_specifiers: None,
            member_declarator: None,
            body_started: false,
            pack_position: 0,
            body_end_position: 0,
            resuming_after_member_recovery: false,
            width_recovered: false,
            source_vectors: ArenaVec::new_in(parser.arena),
            member_source: None,
            current_member_declarator_source: None,
            suppression_entry: 0,
        };
        parser.pools.struct_members.lend(&mut record.declarations);
        parser
            .pools
            .struct_declarators
            .lend(&mut record.member_declarators);
        parser.pools.source_vectors.lend(&mut record.source_vectors);
        record
    }

    /// Commits the member declaration in progress and ends GNU
    /// `__extension__` suppression after checking member features.
    /// C99: flexible array members §6.7.2.1 paragraph 16, p. 103; PDF p. 115.
    fn finish_member(&mut self, parser: &mut Parser<'_, 'tu, 'p>) {
        for member in &self.member_declarators {
            if let Some(declarator) = member.declarator
                && declarator.is_unsized_array()
            {
                parser.extension_source(
                    Feature::FlexibleArrayMembers,
                    "flexible array member",
                    declarator.source_vectors,
                );
            }
        }
        parser.pedantic_suppression = self.suppression_entry;
        let start = parser.alloc_syntax_list(&mut self.member_declarators);
        let specifiers = self
            .member_specifiers
            .take()
            .expect("a struct member has specifiers");
        let source_vectors = self.member_source.take().unwrap_or_default();
        self.declarations.push(StructDeclaration {
            assertion: None,
            extensions: specifiers.extensions,
            type_qualifiers: specifiers.type_qualifiers,
            type_specifiers: specifiers.type_specifiers,
            struct_declarator_list: start,
            source_vectors,
        });
        self.source_vectors.push(source_vectors);
    }
}

/// Whether the body's `}` follows a stray `)` after a member declarator,
/// with only member declarations in between.
///
/// C99 §6.7.2.1p1: a struct-declaration-list holds member declarations,
/// each starting with a specifier or qualifier, so a `)` is stray when
/// the rest of the member and any member declarations lead to the list's
/// `}`. Inside an enclosing parenthesis, as in
/// `int f(struct S { int x ) int after;`, the `)` closes that
/// parenthesis and the body's `}` is missing. The scan is bounded so
/// repeated errors stay linear.
pub(in crate::translation_phases::parsing) fn member_list_continues(
    parser: &mut Parser<'_, '_, '_>,
) -> bool {
    const SCAN_LIMIT: usize = 64;
    // Parentheses and brackets, which never contain `;`.
    let mut groups = 0_u32;
    // Nested struct, union, or enum bodies.
    let mut braces = 0_u32;
    // The first tokens finish the current member; each later member
    // starts with a declaration specifier or qualifier.
    let mut member_start = false;
    let mut previous = None;
    let mut before_previous = None;
    for index in 0..SCAN_LIMIT {
        let Some(token) = parser.cursor.lookahead(index) else {
            return false;
        };
        let kind = token.kind;
        if is_statement_keyword(kind) {
            return false;
        }
        let top_level = groups == 0 && braces == 0;
        if top_level && member_start && !is_operator(Some(token), OperatorTokenType::Semicolon) {
            if !is_operator(Some(token), OperatorTokenType::ClosingCurlyBrace)
                && !parser.declaration_starter(token)
            {
                return false;
            }
            member_start = false;
        }
        match kind {
            | TokenType::Operator(OperatorTokenType::ClosingCurlyBrace) => {
                if groups > 0 {
                    return false;
                }
                if braces == 0 {
                    return true;
                }
                braces -= 1;
            },
            | TokenType::Operator(OperatorTokenType::Semicolon) => {
                if groups > 0 {
                    return false;
                }
                member_start = braces == 0;
            },
            | TokenType::Operator(OperatorTokenType::Equals) if groups == 0 => return false,
            | TokenType::Operator(OperatorTokenType::OpeningCurlyBrace) => {
                let opens_tag_body = |kind: Option<TokenType>| {
                    matches!(
                        kind,
                        Some(TokenType::Keyword(
                            KeywordTokenType::Struct
                                | KeywordTokenType::Union
                                | KeywordTokenType::Enum
                        ))
                    )
                };
                if groups == 0
                    && !(opens_tag_body(previous)
                        || matches!(previous, Some(TokenType::Identifier))
                            && opens_tag_body(before_previous))
                {
                    return false;
                }
                braces += 1;
            },
            | TokenType::Operator(
                OperatorTokenType::OpeningParenthesis | OperatorTokenType::OpeningSquareBracket,
            ) => groups += 1,
            | TokenType::Operator(
                OperatorTokenType::ClosingParenthesis | OperatorTokenType::ClosingSquareBracket,
            ) => {
                if groups == 0 {
                    return false;
                }
                groups -= 1;
            },
            | _ => {},
        }
        before_previous = previous;
        previous = Some(kind);
    }
    false
}
