//! Typedef-sensitive declaration, type-name, and attribute lookahead.
//!
//! [`Parser::declaration_starter`] asks whether a token can start specifiers in
//! the current scope. The following scans distinguish declarations from
//! expressions and recognize declaration-shaped prefixes without consuming
//! input. They select grammar frames; they do not resolve types or check
//! redeclarations.
//!
//! C99: translation phase 7, §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22.
//! C99: declaration specifiers, §6.7 paragraph 1, p. 97; PDF p. 109;
//! typedef-name, §6.7.7 paragraph 1, p. 123; PDF p. 135; points of declaration,
//! §6.2.1 paragraph 7, p. 30; PDF p. 42. Attributes and vendor keywords are
//! extensions to these C99 productions.

use super::{
    Parser,
    frames::expression_operators::is_operator,
    syntax::Identifier,
};
use crate::translation_phases::preprocessing::{
    KeywordTokenType,
    OperatorTokenType,
    Token,
    TokenType,
};

impl Parser<'_, '_, '_> {
    /// Reports whether `token` can begin declaration specifiers in the current
    /// typedef environment.
    ///
    /// C99: declaration-specifiers are §6.7, p. 97; PDF p. 109, built from
    /// storage-class specifiers (§6.7.1, p. 98; PDF p. 110), type specifiers
    /// (§6.7.2, p. 99; PDF p. 111), type qualifiers (§6.7.3, p. 108;
    /// PDF p. 120), and `inline` (§6.7.4, p. 112; PDF p. 124); typedef-name
    /// is a type-specifier under §6.7.2, p. 99; PDF p. 111. `_Imaginary` is
    /// accepted here so the specifier frame can diagnose it. Later-standard
    /// specifiers, `_Static_assert`, attribute specifiers, and the GNU and
    /// MSVC specifier keywords also start one; a `[` counts only as described
    /// on [`Self::attribute_starter_in_lookahead`].
    pub(super) fn declaration_starter(&self, token: Token) -> bool {
        match token.kind {
            | TokenType::Keyword(
                KeywordTokenType::Alignas
                | KeywordTokenType::Atomic
                | KeywordTokenType::Noreturn
                | KeywordTokenType::ThreadLocal
                | KeywordTokenType::BitInt
                | KeywordTokenType::Decimal32
                | KeywordTokenType::Decimal64
                | KeywordTokenType::Decimal128
                | KeywordTokenType::Constexpr
                | KeywordTokenType::Int128
                | KeywordTokenType::Float128
                | KeywordTokenType::Int8
                | KeywordTokenType::Int16
                | KeywordTokenType::Int32
                | KeywordTokenType::Int64
                | KeywordTokenType::Ptr32
                | KeywordTokenType::Ptr64
                | KeywordTokenType::Unaligned
                | KeywordTokenType::W64
                | KeywordTokenType::Sptr
                | KeywordTokenType::Uptr
                | KeywordTokenType::AutoType
                | KeywordTokenType::Extension
                | KeywordTokenType::Typeof
                | KeywordTokenType::TypeofUnqual
                | KeywordTokenType::StaticAssert
                | KeywordTokenType::Auto
                | KeywordTokenType::Char
                | KeywordTokenType::Complex
                | KeywordTokenType::Const
                | KeywordTokenType::Double
                | KeywordTokenType::Enum
                | KeywordTokenType::Extern
                | KeywordTokenType::Float
                | KeywordTokenType::Imaginary
                | KeywordTokenType::Inline
                | KeywordTokenType::Forceinline
                | KeywordTokenType::Cdecl
                | KeywordTokenType::Stdcall
                | KeywordTokenType::Fastcall
                | KeywordTokenType::Vectorcall
                | KeywordTokenType::Thiscall
                | KeywordTokenType::Int
                | KeywordTokenType::Long
                | KeywordTokenType::Register
                | KeywordTokenType::Restrict
                | KeywordTokenType::Short
                | KeywordTokenType::Signed
                | KeywordTokenType::Static
                | KeywordTokenType::Struct
                | KeywordTokenType::Typedef
                | KeywordTokenType::Union
                | KeywordTokenType::Unsigned
                | KeywordTokenType::Void
                | KeywordTokenType::Volatile
                | KeywordTokenType::Bool,
            ) => true,
            | TokenType::Identifier => self.scopes.is_typedef(token.contents),
            | _ => self.attribute_starter_in_lookahead(token),
        }
    }

    /// Reports whether `token` can begin a `type-name`: a type specifier or
    /// qualifier, or a visible typedef name.
    ///
    /// C99: a `type-name` begins with a `specifier-qualifier-list` (§6.7.6
    /// paragraph 1, p. 122; PDF p. 134; §6.7.2.1 paragraph 1, p. 101;
    /// PDF p. 113).
    pub(super) fn type_name_starter(&self, token: Token) -> bool {
        match token.kind {
            | TokenType::Keyword(
                KeywordTokenType::Alignas
                | KeywordTokenType::Atomic
                | KeywordTokenType::BitInt
                | KeywordTokenType::Decimal32
                | KeywordTokenType::Decimal64
                | KeywordTokenType::Decimal128
                | KeywordTokenType::Int128
                | KeywordTokenType::Float128
                | KeywordTokenType::Int8
                | KeywordTokenType::Int16
                | KeywordTokenType::Int32
                | KeywordTokenType::Int64
                | KeywordTokenType::Ptr32
                | KeywordTokenType::Ptr64
                | KeywordTokenType::Unaligned
                | KeywordTokenType::W64
                | KeywordTokenType::Sptr
                | KeywordTokenType::Uptr
                | KeywordTokenType::AutoType
                | KeywordTokenType::Typeof
                | KeywordTokenType::TypeofUnqual
                | KeywordTokenType::Char
                | KeywordTokenType::Complex
                | KeywordTokenType::Const
                | KeywordTokenType::Double
                | KeywordTokenType::Enum
                | KeywordTokenType::Float
                | KeywordTokenType::Imaginary
                | KeywordTokenType::Int
                | KeywordTokenType::Long
                | KeywordTokenType::Restrict
                | KeywordTokenType::Short
                | KeywordTokenType::Signed
                | KeywordTokenType::Struct
                | KeywordTokenType::Union
                | KeywordTokenType::Unsigned
                | KeywordTokenType::Void
                | KeywordTokenType::Volatile
                | KeywordTokenType::Bool,
            ) => true,
            | TokenType::Identifier => self.scopes.is_typedef(token.contents),
            | _ => self.attribute_starter_in_lookahead(token),
        }
    }

    /// Reports whether the token after any leading GNU `__extension__`
    /// markers starts a declaration, so `__extension__ x` stays an
    /// expression.
    pub(super) fn extension_precedes_declaration(&self) -> bool {
        let mut offset = 0;
        let mut token = self.cursor.current();
        while token
            .is_some_and(|x| matches!(x.kind, TokenType::Keyword(KeywordTokenType::Extension)))
        {
            token = self.cursor.lookahead(offset);
            offset += 1;
        }
        token.is_some_and(|x| self.declaration_starter(x))
    }

    /// Reports whether a declaration, not a statement, follows the attribute
    /// specifiers (and `__extension__` markers) starting at the current token.
    pub(super) fn attributes_precede_declaration(&self) -> bool {
        let mut offset = 0;
        let mut token = self.cursor.current();
        loop {
            let parenthesized = token.is_some_and(|x| {
                matches!(
                    x.kind,
                    TokenType::Keyword(KeywordTokenType::Attribute | KeywordTokenType::Declspec)
                )
            });
            let mut depth = 0usize;
            let mut opened = false;
            while let Some(current) = token {
                match current.kind {
                    | TokenType::Operator(OperatorTokenType::OpeningParenthesis) if parenthesized =>
                    {
                        depth += 1;
                        opened = true;
                    },
                    | TokenType::Operator(OperatorTokenType::ClosingParenthesis) if parenthesized =>
                        depth = depth.saturating_sub(1),
                    | TokenType::Operator(OperatorTokenType::OpeningSquareBracket)
                        if !parenthesized =>
                    {
                        depth += 1;
                        opened = true;
                    },
                    | TokenType::Operator(OperatorTokenType::ClosingSquareBracket)
                        if !parenthesized =>
                        depth = depth.saturating_sub(1),
                    | TokenType::Operator(
                        OperatorTokenType::Semicolon | OperatorTokenType::ClosingCurlyBrace,
                    ) => return false,
                    | _ => {},
                }
                token = self.cursor.lookahead(offset);
                offset += 1;
                if opened && depth == 0 {
                    break;
                }
            }
            let Some(mut current) = token else {
                return false;
            };
            while matches!(
                current.kind,
                TokenType::Keyword(KeywordTokenType::Extension)
            ) {
                token = self.cursor.lookahead(offset);
                offset += 1;
                let Some(next) = token else { return false };
                current = next;
            }
            if matches!(
                current.kind,
                TokenType::Keyword(KeywordTokenType::Attribute | KeywordTokenType::Declspec)
            ) || matches!(
                current.kind,
                TokenType::Operator(OperatorTokenType::OpeningSquareBracket)
            ) && self.cursor.lookahead(offset).is_some_and(|x| {
                matches!(
                    x.kind,
                    TokenType::Operator(OperatorTokenType::OpeningSquareBracket)
                )
            }) {
                continue;
            }
            return self.declaration_starter(current);
        }
    }

    /// Reports whether `token`, the current token, begins an attribute
    /// specifier: `__attribute__`, `__declspec`, or `[[`.
    ///
    /// C23 (N3220): attribute-specifier is §6.7.13.2 paragraph 1,
    /// pp. 142-143; PDF pp. 155-156. `__attribute__` and `__declspec` are GNU
    /// and MSVC extensions.
    pub(super) fn attribute_starter(&self, token: Option<Token>) -> bool {
        token.is_some_and(|token| Self::attribute_starter_before(token, self.cursor.following()))
    }

    /// Reports whether `token`, followed by `next`, begins an attribute
    /// specifier. A `[` starts one only when another `[` follows it.
    fn attribute_starter_before(token: Token, next: Option<Token>) -> bool {
        match token.kind {
            | TokenType::Keyword(KeywordTokenType::Attribute | KeywordTokenType::Declspec) => true,
            | TokenType::Operator(OperatorTokenType::OpeningSquareBracket) =>
                next.is_some_and(|next| {
                    matches!(
                        next.kind,
                        TokenType::Operator(OperatorTokenType::OpeningSquareBracket)
                    )
                }),
            | _ => false,
        }
    }

    /// [`Self::attribute_starter_before`] for a token the caller may have
    /// read by lookahead. The token after a `[` is known only when the `[` is
    /// the current token or the one following it; at any later position the
    /// `[` is not taken as an attribute start.
    fn attribute_starter_in_lookahead(&self, token: Token) -> bool {
        if !matches!(
            token.kind,
            TokenType::Operator(OperatorTokenType::OpeningSquareBracket)
        ) {
            return Self::attribute_starter_before(token, None);
        }
        let next = if Some(token) == self.cursor.current() {
            self.cursor.following()
        } else if Some(token) == self.cursor.following() {
            self.cursor.lookahead(1)
        } else {
            None
        };
        Self::attribute_starter_before(token, next)
    }

    pub(super) fn declaration_recovery_starts_here(&mut self, token: Token) -> bool {
        if matches!(token.kind, TokenType::Keyword(KeywordTokenType::Extension)) {
            return self.extension_precedes_declaration();
        }
        self.declaration_starter(token)
            && (!matches!(token.kind, TokenType::Identifier)
                || self.typedef_name_continues_specifiers())
    }

    /// Returns whether the declaration starting at the current token
    /// declares one of `parameters`, judged from its first identifiers that are
    /// neither typedef names nor tags. Recovery uses this to tell an
    /// old-style parameter declaration from an unrelated declaration that
    /// follows a head missing its `;`. When the scan runs out of lookahead it
    /// answers yes, keeping the definition reading.
    pub(super) fn next_declaration_declares_one_of(&mut self, parameters: &[Identifier]) -> bool {
        const LOOKAHEAD: usize = 32;
        let mut after_tag_keyword = false;
        let mut depth = 0_usize;
        for index in 0..LOOKAHEAD {
            let token = if index == 0 {
                self.cursor.current()
            } else {
                self.cursor.lookahead(index - 1)
            };
            let Some(token) = token else {
                return false;
            };
            match token.kind {
                | TokenType::Keyword(
                    KeywordTokenType::Struct | KeywordTokenType::Union | KeywordTokenType::Enum,
                ) => after_tag_keyword = true,
                | TokenType::Identifier => {
                    let tag = std::mem::take(&mut after_tag_keyword);
                    if depth == 0 && !tag && !self.scopes.is_typedef(token.contents) {
                        return parameters
                            .iter()
                            .any(|parameter| parameter.name == token.contents);
                    }
                },
                | TokenType::Operator(OperatorTokenType::OpeningCurlyBrace) => {
                    after_tag_keyword = false;
                    depth += 1;
                },
                | TokenType::Operator(OperatorTokenType::ClosingCurlyBrace) => {
                    depth = depth.saturating_sub(1);
                },
                | TokenType::Operator(
                    OperatorTokenType::Semicolon
                    | OperatorTokenType::Comma
                    | OperatorTokenType::Equals,
                ) if depth == 0 => return false,
                | _ => after_tag_keyword = false,
            }
        }
        true
    }

    /// Resolves the declaration-specifier/declarator ambiguity after a visible
    /// typedef name using buffered lookahead.
    ///
    /// C99: typedef-name is §6.7.7, pp. 123-124; PDF pp. 135-136, and its
    /// declarator ambiguity is constrained by §6.7.5.3 paragraph 11,
    /// p. 119; PDF p. 131.
    pub(super) fn typedef_name_continues_specifiers(&mut self) -> bool {
        let Some(following) = self.cursor.following() else {
            return false;
        };
        matches!(following.kind, TokenType::Identifier)
            || is_operator(Some(following), OperatorTokenType::Asterisk)
            || self.parenthesized_declarator_follows_typedef()
            || self.declaration_starter(following)
    }

    /// Detects the parenthesized-pointer shape that forces a typedef spelling
    /// to remain a specifier rather than become the declarator name.
    ///
    /// C99: parenthesized direct-declarator and pointer are §6.7.5,
    /// p. 114; PDF p. 126; typedef-name is §6.7.7, pp. 123-124;
    /// PDF pp. 135-136.
    fn parenthesized_declarator_follows_typedef(&mut self) -> bool {
        let mut index = 0;
        while is_operator(
            self.cursor.lookahead(index),
            OperatorTokenType::OpeningParenthesis,
        ) {
            index += 1;
        }
        is_operator(self.cursor.lookahead(index), OperatorTokenType::Asterisk)
    }
}
