use std::{
    fmt::{
        Debug,
        Display,
        Formatter,
        Result as FmtResult,
    },
    hash::Hash,
};

use super::{
    preprocessing::{
        CharacterTokenType,
        FloatTokenType,
        IntegerTokenType,
        KeywordTokenType,
        Preprocessor,
        Token,
        TokenType,
    },
    Context,
    ErrorSeverity,
    GetPosition,
    GetSeverity,
    GetSourceFileIndex,
    GetSourceVectors,
    SetPosition,
    SetSourceFileIndex,
    SourcePosition,
    SourceVectors,
    TranslationPhase,
};
use crate::{
    translation_phases::preprocessing::OperatorTokenType,
    util::{
        string_cache::StringCacheId,
        vector_slice::VectorSlice,
    },
};

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct Parser {
    pub(crate) preprocessor:  Preprocessor,
    pub(crate) types:         Vec<Type>,
    pub(crate) pending_token: Option<Token>,
    pub(crate) expressions:   Vec<Expression>,
    pub(crate) statements:    Vec<Statement>,
}

impl GetPosition for Parser {
    fn position(&self, context: &Context) -> SourcePosition {
        self.preprocessor.position(context)
    }
}

impl SetPosition for Parser {
    fn set_position(&mut self, context: &mut Context, position: SourcePosition) {
        self.preprocessor.set_position(context, position);
    }
}

impl GetSourceFileIndex for Parser {
    fn source_file_index(&self) -> u32 {
        self.preprocessor.source_file_index()
    }
}

impl SetSourceFileIndex for Parser {
    fn set_source_file_index(&mut self, context: &mut Context, source_file_index: u32) {
        self.preprocessor
            .set_source_file_index(context, source_file_index);
    }
}

mod type_qualifiers {
    bitfield::bitfield! {
        #[derive(PartialEq, Eq, Hash, Clone, Copy, Default)]
        pub struct TypeQualifiers(u8);
        impl Debug;
        pub is_const, set_is_const: 0;
        pub is_volatile, set_is_volatile: 1;
        pub is_restrict, set_is_restrict: 2;
    }
}

pub(crate) use type_qualifiers::TypeQualifiers;

mod type_specifiers {
    bitfield::bitfield! {
        #[derive(PartialEq, Eq, Hash, Clone, Copy, Default)]
        pub struct TypeSpecifiers(u16);
        impl Debug;
        pub is_short, set_is_short: 0;
        pub is_signed, set_is_signed: 1;
        pub is_unsigned, set_is_unsigned: 2;
        pub is_int, set_is_int: 3;
        pub is_float, set_is_float: 4;
        pub is_double, set_is_double: 5;
        pub is_void, set_is_void: 6;
        pub is_char, set_is_char: 7;
        pub is_bool, set_is_bool: 8;
        pub is_complex, set_is_complex: 9;
        pub is_long, set_is_long: 10;
        pub is_long_long, set_is_long_long: 11;
        pub is_long_double, set_is_long_double: 12;
    }
}

pub(crate) use type_specifiers::TypeSpecifiers;

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, Default)]
pub(crate) struct FunctionSpecifiers {
    pub(crate) is_inline: bool,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct DeclarationSpecifiers {
    pub(crate) storage_class:       StorageClass,
    pub(crate) type_qualifiers:     TypeQualifiers,
    pub(crate) type_specifiers:     TypeSpecifiers,
    pub(crate) function_specifiers: FunctionSpecifiers,
}

impl Default for DeclarationSpecifiers {
    fn default() -> Self {
        Self::new()
    }
}

impl DeclarationSpecifiers {
    const fn new() -> Self {
        Self {
            storage_class:       StorageClass::Auto,
            type_qualifiers:     TypeQualifiers(0),
            type_specifiers:     TypeSpecifiers(0),
            function_specifiers: FunctionSpecifiers { is_inline: false },
        }
    }
}

impl Parser {
    pub(crate) fn new(preprocessor: Preprocessor) -> Self {
        Self {
            preprocessor,
            types: Vec::new(),
            pending_token: None,
            expressions: Vec::new(),
            statements: Vec::new(),
        }
    }

    fn next_token(&mut self, context: &mut Context) -> Option<Token> {
        if let Some(token) = self.pending_token.take() {
            return Some(token);
        }
        self.preprocessor.next_item(context)
    }

    fn parse_statement(&mut self, context: &mut Context) -> Statement {
        todo!();
    }

    fn parse_expression(&mut self, context: &mut Context) -> Expression {
        todo!();
    }

    fn parse_struct_declaration(&mut self, context: &mut Context, token: Token) -> Type {
        todo!();
    }

    fn parse_enum_declaration(&mut self, context: &mut Context, token: Token) -> Type {
        todo!();
    }

    fn parse_type(&mut self, context: &mut Context) -> Type {
        todo!();
    }

    fn parse_identifier(
        &mut self,
        context: &mut Context,
        eof_message: &'static str,
        on_error: impl FnOnce(Token) -> ParserErrorType,
    ) -> Identifier {
        let Some(token) = self.next_token(context) else {
            let source_vectors =
                context.create_source_vectors(self.position(context), self.source_file_index(), 0);
            context.parser_error(ParserError {
                error_type: ParserErrorType::UnexpectedEndOfInput(eof_message),
                source_vectors,
            });
            return Identifier {
                name: context.string_cache.intern("<non-existent-identifier>"),
            };
        };
        match token.kind {
            | TokenType::Identifier => Identifier {
                name: token.contents,
            },
            | tt => {
                context.parser_error(ParserError {
                    error_type:     on_error(token),
                    source_vectors: token.source_vectors,
                });
                self.pending_token = Some(token);
                Identifier {
                    name: context.string_cache.intern("<non-existent-identifier>"),
                }
            },
        }
    }

    fn parse_typedef(&mut self, context: &mut Context, typedef: Token) -> Type {
        let referent_type = self.parse_type(context);
        let referent_type_index = self.types.len() - 1;

        let name =
            self.parse_identifier(context, "parsing typedef. Expected identifier.", |token| {
                ParserErrorType::ExpectedIdentifierInTypedef(token.kind)
            });
        match self.next_token(context) {
            | Some(token) if token.kind == TokenType::Operator(OperatorTokenType::Semicolon) => (),
            | Some(token) => {
                context.parser_error(ParserError {
                    error_type:     ParserErrorType::ExpectedSemicolonAfterTypedef(token.kind),
                    source_vectors: token.source_vectors,
                });
                self.pending_token = Some(token);
            },
            | None => {
                let source_vectors = context.create_source_vectors(
                    self.position(context),
                    self.source_file_index(),
                    0,
                );
                context.parser_error(ParserError {
                    error_type: ParserErrorType::UnexpectedEndOfInput(
                        "parsing typedef. Expected a semicolon.",
                    ),
                    source_vectors,
                });
            },
        }
        let t = Type {
            is_const:    false,
            is_volatile: false,
            kind:        TypeKind::Typedef {
                name,
                referent_type: TypeIndex(referent_type_index),
            },
        };
        self.types.push(t);
        t
    }

    fn parse_variable_declaration(&mut self, context: &mut Context) -> VariableDeclaration {
        todo!();
    }

    fn parse_function_definition(
        &mut self,
        context: &mut Context,
        name: Identifier,
        parameters: Option<VectorSlice<FunctionDefinitionArgument>>,
        return_type: Option<TypeIndex>,
    ) -> TopLevelStatement {
        match self.next_token(context) {
            | Some(token)
                if token.kind == TokenType::Operator(OperatorTokenType::OpeningCurlyBrace) =>
                (),
            | Some(token) if token.kind == TokenType::Operator(OperatorTokenType::Semicolon) => {
                return TopLevelStatement {
                    kind: TopLevelStatementType::FunctionDeclaration(FunctionDeclaration {
                        name,
                        parameters,
                        return_type,
                    }),
                };
            },
            | Some(token) => {
                context.parser_error(ParserError {
                    error_type: ParserErrorType::ExpectedSemicolonOrOpeningCurlyBraceAfterFunctionDeclaration(token.kind),
                    source_vectors: token.source_vectors,
                });
                self.pending_token = Some(token);
                return TopLevelStatement {
                    kind: TopLevelStatementType::FunctionDeclaration(FunctionDeclaration {
                        name,
                        parameters,
                        return_type,
                    }),
                };
            },
            | None => {
                let source_vectors = context.create_source_vectors(
                    self.position(context),
                    self.source_file_index(),
                    0,
                );
                context.parser_error(ParserError {
                    error_type: ParserErrorType::UnexpectedEndOfInput(
                        "parsing function definition. Expected a semicolon or an opening curly \
                         brace.",
                    ),
                    source_vectors,
                });
                return TopLevelStatement {
                    kind: TopLevelStatementType::FunctionDeclaration(FunctionDeclaration {
                        name,
                        parameters,
                        return_type,
                    }),
                };
            },
        }
        let statement_start_index: u32 = self
            .statements
            .len()
            .try_into()
            .expect("More than u32::MAX statements.");
        loop {
            match self.next_token(context) {
                | Some(token)
                    if token.kind == TokenType::Operator(OperatorTokenType::ClosingCurlyBrace) =>
                    break,
                | None => break,
                | Some(token) => {
                    self.pending_token = Some(token);
                    let statement = self.parse_statement(context);
                    self.statements.push(statement);
                },
            }
        }
        let length: u32 = self
            .statements
            .len()
            .try_into()
            .expect("More than u32::MAX statements.");
        let length = length - statement_start_index;
        TopLevelStatement {
            kind: TopLevelStatementType::FunctionDefinition(FunctionDefinition {
                declaration: FunctionDeclaration {
                    name,
                    parameters,
                    return_type,
                },
                statements:  VectorSlice::new(statement_start_index, length),
            }),
        }
    }

    fn set_storage_class(
        &mut self,
        context: &mut Context,
        last_storage_class: StorageClass,
        new_storage_class: Token,
        storage_class_specified: bool,
    ) -> StorageClass {
        if storage_class_specified {
            context.parser_error(ParserError {
                error_type:     ParserErrorType::StorageClassRedefinition(
                    last_storage_class,
                    new_storage_class.kind,
                ),
                source_vectors: new_storage_class.source_vectors,
            });
        }
        match new_storage_class.kind {
            | TokenType::Keyword(KeywordTokenType::Auto) => StorageClass::Auto,
            | TokenType::Keyword(KeywordTokenType::Register) => StorageClass::Register,
            | TokenType::Keyword(KeywordTokenType::Static) => StorageClass::Static,
            | TokenType::Keyword(KeywordTokenType::Extern) => StorageClass::Extern,
            | TokenType::Keyword(KeywordTokenType::Typedef) => StorageClass::Typedef,
            | TokenType::Keyword(KeywordTokenType::Auto) => StorageClass::Auto,
            | _ => unreachable!("set_storage_class called with non-storage class token."),
        }
    }

    fn map_type_specifiers(specifier: TypeSpecifiers) -> ConflictingTypeSpecifier {
        if specifier.is_bool() {
            return ConflictingTypeSpecifier::Bool;
        }
        if specifier.is_char() {
            return ConflictingTypeSpecifier::Char;
        }
        if specifier.is_complex() {
            return ConflictingTypeSpecifier::Complex;
        }
        if specifier.is_double() {
            return ConflictingTypeSpecifier::Double;
        }
        if specifier.is_float() {
            return ConflictingTypeSpecifier::Float;
        }
        if specifier.is_int() {
            return ConflictingTypeSpecifier::Int;
        }
        if specifier.is_short() {
            return ConflictingTypeSpecifier::Short;
        }
        if specifier.is_void() {
            return ConflictingTypeSpecifier::Void;
        }
        if specifier.is_signed() {
            return ConflictingTypeSpecifier::Signed;
        }
        if specifier.is_unsigned() {
            return ConflictingTypeSpecifier::Unsigned;
        }
        if specifier.is_long() {
            return ConflictingTypeSpecifier::Long;
        }
        if specifier.is_long_long() {
            return ConflictingTypeSpecifier::LongLong;
        }

        unreachable!("map_type_specifiers called with empty type specifiers.");
    }

    fn set_type_specifiers(
        &mut self,
        context: &mut Context,
        type_specifiers: &mut TypeSpecifiers,
        token: Token,
    ) {
        let TokenType::Keyword(keyword_token_type) = token.kind else {
            unreachable!("set_type_specifiers called with non-keyword token.");
        };
        match keyword_token_type {
            | KeywordTokenType::Int => {
                if type_specifiers.is_int() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                        source_vectors: token.source_vectors,
                    });
                } else if type_specifiers.is_void()
                    || type_specifiers.is_float()
                    || type_specifiers.is_double()
                    || type_specifiers.is_bool()
                    || type_specifiers.is_complex()
                    || type_specifiers.is_char()
                {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::ConflictingTypeSpecifiers(
                            Self::map_type_specifiers(*type_specifiers),
                            token.kind,
                        ),
                        source_vectors: token.source_vectors,
                    });
                }
                type_specifiers.set_is_int(true);
            },
            | KeywordTokenType::Long => {
                if type_specifiers.is_long_long() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::LongSpecifiedThrice,
                        source_vectors: token.source_vectors,
                    });
                } else if type_specifiers.is_long_double() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::LongLongDoubleSpecified,
                        source_vectors: token.source_vectors,
                    });
                } else if type_specifiers.is_void()
                    || type_specifiers.is_float()
                    || type_specifiers.is_bool()
                    || type_specifiers.is_complex()
                    || type_specifiers.is_char()
                {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::ConflictingTypeSpecifiers(
                            Self::map_type_specifiers(*type_specifiers),
                            token.kind,
                        ),
                        source_vectors: token.source_vectors,
                    });
                }
                if type_specifiers.is_double() {
                    type_specifiers.set_is_long_double(true);
                }
                if type_specifiers.is_long() {
                    type_specifiers.set_is_long_long(true);
                }
                type_specifiers.set_is_long(true);
            },
            | KeywordTokenType::Short => {
                if type_specifiers.is_short() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                        source_vectors: token.source_vectors,
                    });
                } else if type_specifiers.is_long()
                    || type_specifiers.is_void()
                    || type_specifiers.is_float()
                    || type_specifiers.is_double()
                    || type_specifiers.is_bool()
                    || type_specifiers.is_complex()
                    || type_specifiers.is_char()
                {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::ConflictingTypeSpecifiers(
                            Self::map_type_specifiers(*type_specifiers),
                            token.kind,
                        ),
                        source_vectors: token.source_vectors,
                    });
                }
                type_specifiers.set_is_short(true);
            },
            | KeywordTokenType::Signed => {
                if type_specifiers.is_signed() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                        source_vectors: token.source_vectors,
                    });
                } else if type_specifiers.is_unsigned() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::ConflictingTypeSpecifiers(
                            Self::map_type_specifiers(*type_specifiers),
                            token.kind,
                        ),
                        source_vectors: token.source_vectors,
                    });
                }
                type_specifiers.set_is_signed(true);
            },
            | KeywordTokenType::Unsigned => {
                if type_specifiers.is_unsigned() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                        source_vectors: token.source_vectors,
                    });
                } else if type_specifiers.is_signed() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::ConflictingTypeSpecifiers(
                            Self::map_type_specifiers(*type_specifiers),
                            token.kind,
                        ),
                        source_vectors: token.source_vectors,
                    });
                }
                type_specifiers.set_is_unsigned(true);
            },
            | KeywordTokenType::Float => {
                if type_specifiers.is_float() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                        source_vectors: token.source_vectors,
                    });
                } else if type_specifiers.is_void()
                    || type_specifiers.is_int()
                    || type_specifiers.is_double()
                    || type_specifiers.is_bool()
                    || type_specifiers.is_complex()
                    || type_specifiers.is_char()
                    || type_specifiers.is_long()
                    || type_specifiers.is_signed()
                    || type_specifiers.is_unsigned()
                {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::ConflictingTypeSpecifiers(
                            Self::map_type_specifiers(*type_specifiers),
                            token.kind,
                        ),
                        source_vectors: token.source_vectors,
                    });
                }
                type_specifiers.set_is_float(true);
            },
            | KeywordTokenType::Double => {
                if type_specifiers.is_double() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                        source_vectors: token.source_vectors,
                    });
                } else if type_specifiers.is_long_double() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::LongLongDoubleSpecified,
                        source_vectors: token.source_vectors,
                    });
                } else if type_specifiers.is_void()
                    || type_specifiers.is_int()
                    || type_specifiers.is_float()
                    || type_specifiers.is_bool()
                    || type_specifiers.is_complex()
                    || type_specifiers.is_char()
                    || type_specifiers.is_long()
                    || type_specifiers.is_signed()
                    || type_specifiers.is_unsigned()
                {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::ConflictingTypeSpecifiers(
                            Self::map_type_specifiers(*type_specifiers),
                            token.kind,
                        ),
                        source_vectors: token.source_vectors,
                    });
                }
                if type_specifiers.is_long() {
                    type_specifiers.set_is_long_double(true);
                }
                type_specifiers.set_is_double(true);
            },
            | KeywordTokenType::Void => {
                if type_specifiers.is_void() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                        source_vectors: token.source_vectors,
                    });
                } else if type_specifiers.is_int()
                    || type_specifiers.is_float()
                    || type_specifiers.is_double()
                    || type_specifiers.is_bool()
                    || type_specifiers.is_complex()
                    || type_specifiers.is_char()
                    || type_specifiers.is_long()
                    || type_specifiers.is_signed()
                    || type_specifiers.is_unsigned()
                {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::ConflictingTypeSpecifiers(
                            Self::map_type_specifiers(*type_specifiers),
                            token.kind,
                        ),
                        source_vectors: token.source_vectors,
                    });
                }
                type_specifiers.set_is_void(true);
            },
            | KeywordTokenType::Bool => {
                if type_specifiers.is_bool() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                        source_vectors: token.source_vectors,
                    });
                } else if type_specifiers.is_void()
                    || type_specifiers.is_int()
                    || type_specifiers.is_float()
                    || type_specifiers.is_double()
                    || type_specifiers.is_complex()
                    || type_specifiers.is_char()
                    || type_specifiers.is_long()
                    || type_specifiers.is_signed()
                    || type_specifiers.is_unsigned()
                {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::ConflictingTypeSpecifiers(
                            Self::map_type_specifiers(*type_specifiers),
                            token.kind,
                        ),
                        source_vectors: token.source_vectors,
                    });
                }
                type_specifiers.set_is_bool(true);
            },
            | KeywordTokenType::Complex => {
                if type_specifiers.is_complex() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                        source_vectors: token.source_vectors,
                    });
                } else if type_specifiers.is_void()
                    || type_specifiers.is_int()
                    || type_specifiers.is_float()
                    || type_specifiers.is_double()
                    || type_specifiers.is_bool()
                    || type_specifiers.is_char()
                    || type_specifiers.is_long()
                    || type_specifiers.is_signed()
                    || type_specifiers.is_unsigned()
                {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::ConflictingTypeSpecifiers(
                            Self::map_type_specifiers(*type_specifiers),
                            token.kind,
                        ),
                        source_vectors: token.source_vectors,
                    });
                }
                type_specifiers.set_is_complex(true);
            },
            | KeywordTokenType::Char => {
                if type_specifiers.is_char() {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::TypeSpecifierSpecifiedTwice(token.kind),
                        source_vectors: token.source_vectors,
                    });
                } else if type_specifiers.is_void()
                    || type_specifiers.is_int()
                    || type_specifiers.is_float()
                    || type_specifiers.is_double()
                    || type_specifiers.is_bool()
                    || type_specifiers.is_complex()
                    || type_specifiers.is_long()
                {
                    context.parser_error(ParserError {
                        error_type:     ParserErrorType::ConflictingTypeSpecifiers(
                            Self::map_type_specifiers(*type_specifiers),
                            token.kind,
                        ),
                        source_vectors: token.source_vectors,
                    });
                }
                type_specifiers.set_is_char(true);
            },
            | _ => unreachable!("set_type_specifiers called with non-type-specifier token."),
        }
    }

    fn parse_declaration_specifiers(&mut self, context: &mut Context) -> DeclarationSpecifiers {
        let mut specifiers = DeclarationSpecifiers::default();
        let mut storage_class_specified = false;
        loop {
            let Some(token) = self.next_token(context) else {
                if specifiers == DeclarationSpecifiers::default() {
                    return specifiers;
                }
                let source_vectors = context.create_source_vectors(
                    self.position(context),
                    self.source_file_index(),
                    0,
                );
                context.parser_error(ParserError {
                    error_type: ParserErrorType::UnexpectedEndOfInput(
                        "parsing declaration specifiers. Expected a type specifier.",
                    ),
                    source_vectors,
                });
                return specifiers;
            };
            match token.kind {
                | TokenType::Keyword(KeywordTokenType::Const) => {
                    if specifiers.type_qualifiers.is_const() {
                        context.parser_error(ParserError {
                            error_type:     ParserErrorType::ConstSpecifiedTwice,
                            source_vectors: token.source_vectors,
                        });
                    }
                    specifiers.type_qualifiers.set_is_const(true);
                },
                | TokenType::Keyword(KeywordTokenType::Volatile) => {
                    if specifiers.type_qualifiers.is_volatile() {
                        context.parser_error(ParserError {
                            error_type:     ParserErrorType::VolatileSpecifiedTwice,
                            source_vectors: token.source_vectors,
                        });
                    }
                    specifiers.type_qualifiers.set_is_volatile(true);
                },
                | TokenType::Keyword(KeywordTokenType::Restrict) => {
                    if specifiers.type_qualifiers.is_restrict() {
                        context.parser_error(ParserError {
                            error_type:     ParserErrorType::RestrictSpecifiedTwice,
                            source_vectors: token.source_vectors,
                        });
                    }
                    specifiers.type_qualifiers.set_is_restrict(true);
                },
                | TokenType::Keyword(KeywordTokenType::Inline) => {
                    if specifiers.function_specifiers.is_inline {
                        context.parser_error(ParserError {
                            error_type:     ParserErrorType::InlineSpecifiedTwice,
                            source_vectors: token.source_vectors,
                        });
                    }
                    specifiers.function_specifiers.is_inline = true;
                },
                | TokenType::Keyword(
                    KeywordTokenType::Static
                    | KeywordTokenType::Extern
                    | KeywordTokenType::Typedef
                    | KeywordTokenType::Auto
                    | KeywordTokenType::Register,
                ) => {
                    specifiers.storage_class = self.set_storage_class(
                        context,
                        specifiers.storage_class,
                        token,
                        storage_class_specified,
                    );
                    storage_class_specified = true;
                },
                | TokenType::Keyword(
                    KeywordTokenType::Int
                    | KeywordTokenType::Short
                    | KeywordTokenType::Long
                    | KeywordTokenType::Char
                    | KeywordTokenType::Signed
                    | KeywordTokenType::Unsigned
                    | KeywordTokenType::Float
                    | KeywordTokenType::Double
                    | KeywordTokenType::Void
                    | KeywordTokenType::Bool
                    | KeywordTokenType::Complex,
                ) => self.set_type_specifiers(context, &mut specifiers.type_specifiers, token),
                | _ => {
                    self.pending_token = Some(token);
                    return specifiers;
                },
            }
        }
    }

    fn parse_top_level_statement(&mut self, context: &mut Context) -> Option<TopLevelStatement> {
        let token = self.next_token(context)?;
        if token.kind == TokenType::Keyword(KeywordTokenType::Typedef) {
            return Some(TopLevelStatement {
                kind: TopLevelStatementType::TypeDeclaration(self.parse_typedef(context, token)),
            });
        }
        let type_ = self.parse_type(context);
        match type_.kind {
            | TypeKind::Function {
                name,
                parameters,
                return_type,
            } =>
                return Some(self.parse_function_definition(context, name, parameters, return_type)),
            | _ => {
                let expression = self.parse_expression(context);
                self.expressions.push(expression);
                Some(TopLevelStatement {
                    kind: TopLevelStatementType::VariableDefinition(VariableDefinition {
                        variable:    VariableDeclaration {
                            name:          //type_.,
                            todo!(),
                            var_type:      todo!(),
                            storage_class: todo!(),
                        },
                        initializer: todo!(),
                    }),
                })
            },
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
#[allow(clippy::enum_variant_names)]
pub(crate) enum State {
    ParsingTopLevelStatement,
    ParsingStatement,
    ParsingExpression,
    ParsingType,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct TopLevelStatement {
    pub(crate) kind: TopLevelStatementType,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum TopLevelStatementType {
    FunctionDefinition(FunctionDefinition),
    VariableDefinition(VariableDeclaration),
    FunctionDeclaration(FunctionDeclaration),
    TypeDeclaration(Type),
}

// A top level statement could be:
// * A function definition.
// * A global variable definition.
// * A function declaration.
// * A global variable declaration.
// * A type definition.

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct ExpressionIndex(usize);

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct StatementIndex(usize);

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct TypeIndex(usize);

#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) struct Statement {
    pub(crate) kind: StatementType,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) enum StatementType {
    Compound(Vec<Statement>),
    Expression(ExpressionIndex),
    If {
        condition_expression: ExpressionIndex,
        then_statement:       StatementIndex,
        else_statement:       Option<StatementIndex>,
    },
    While {
        condition_expression: ExpressionIndex,
        body_statement:       StatementIndex,
    },
    DoWhile {
        condition_expression: ExpressionIndex,
        body_statement:       StatementIndex,
    },
    For {
        initializer_statement: Option<StatementIndex>,
        condition_expression:  Option<ExpressionIndex>,
        post_expression:       Option<ExpressionIndex>,
        body_statement:        StatementIndex,
    },
    Return(ExpressionIndex),
    Break,
    Continue,
    Goto(Identifier),
    Label(Identifier, StatementIndex),
    Case(i64, StatementIndex),
    Default(StatementIndex),
    Null,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct Expression {
    pub(crate) result_type: TypeIndex,
    pub(crate) kind:        ExpressionType,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum ExpressionType {
    Conditional {
        condition_expression: ExpressionIndex,
        then_expression:      ExpressionIndex,
        else_expression:      ExpressionIndex,
    },
    Binary {
        operator:         BinaryOperator,
        left_expression:  ExpressionIndex,
        right_expression: ExpressionIndex,
    },
    Unary {
        operator:           UnaryOperator,
        operand_expression: ExpressionIndex,
    },
    Call {
        function_expression: ExpressionIndex,
        arguments:           VectorSlice<Expression>,
    },
    CompoundLiteral {
        var_type: Type,
        values:   VectorSlice<Expression>,
    },
    Identifier(Identifier),
    Constant(Constant),
    StringLiteral(StringCacheId),
    SizeofType(Type),
    SizeofExpr(ExpressionIndex),
    Cast {
        target_type:        Type,
        operand_expression: ExpressionIndex,
    },
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum Constant {
    Integer(IntegerTokenType),
    Float(FloatTokenType),
    Char(CharacterTokenType),
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum BinaryOperator {
    Multiplication,
    Division,
    Modulo,
    Addition,
    Subtraction,
    LeftShift,
    RightShift,
    LessThan,
    GreaterThan,
    LessThanOrEqual,
    GreaterThanOrEqual,
    Equal,
    NotEqual,
    BitwiseAnd,
    BitwiseXor,
    BitwiseOr,
    LogicalAnd,
    LogicalOr,
    Comma,
    Subscript,
    Assignment,
    MultiplicationAssignment,
    DivisionAssignment,
    ModuloAssignment,
    AdditionAssignment,
    SubtractionAssignment,
    LeftShiftAssignment,
    RightShiftAssignment,
    BitwiseAndAssignment,
    BitwiseXorAssignment,
    BitwiseOrAssignment,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum UnaryOperator {
    AddressOf,
    Indirection,
    Plus,
    Minus,
    BitwiseNot,
    LogicalNot,
    PreIncrement,
    PreDecrement,
    PostIncrement,
    PostDecrement,
    Cast,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct Identifier {
    pub(crate) name: StringCacheId,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct VariableDeclaration {
    pub(crate) name:          Identifier,
    pub(crate) var_type:      TypeIndex,
    pub(crate) storage_class: StorageClass,
    pub(crate) initializer:   Option<Expression>,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct VariableDefinition {
    pub(crate) variable:    VariableDeclaration,
    pub(crate) initializer: Expression,
}
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, Default)]
pub(crate) enum StorageClass {
    #[default]
    Auto,
    Register,
    Static,
    Extern,
    Typedef,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum PrimitiveType {
    Char,
    Short,
    Int,
    Long,
    LongLong,
    UnsignedChar,
    UnsignedShort,
    UnsignedInt,
    UnsignedLong,
    UnsignedLongLong,
    Float,
    Double,
    LongDouble,
    Void,
    Bool,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct FunctionDefinitionArgument {
    pub(crate) function_type: TypeIndex,
    pub(crate) name:          Option<Identifier>,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) struct Type {
    is_const:    bool,
    is_volatile: bool,
    kind:        TypeKind,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub(crate) enum TypeKind {
    Primitive(PrimitiveType),
    Pointer {
        pointee_type: TypeIndex,
    },
    Struct {
        name:   Option<Identifier>,
        fields: VectorSlice<VariableDeclaration>,
    },
    Union {
        name:   Identifier,
        fields: VectorSlice<VariableDeclaration>,
    },
    Typedef {
        name:          Identifier,
        referent_type: TypeIndex,
    },
    Enum {
        name:   Identifier,
        values: VectorSlice<EnumValue>,
    },
    Function {
        name:        Identifier,
        return_type: Option<TypeIndex>,
        /// None symbolizes a function with an unspecified number of arguments
        /// (i.e. `int f()`).
        parameters:  Option<VectorSlice<FunctionDefinitionArgument>>,
    },
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct EnumValue {
    pub(crate) name:  Identifier,
    pub(crate) value: i64,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct FunctionDeclaration {
    pub(crate) name:        Identifier,
    /// None symbolizes a function with an unspecified number of arguments
    /// (i.e. `int f()`).
    pub(crate) parameters:  Option<VectorSlice<FunctionDefinitionArgument>>,
    pub(crate) return_type: Option<TypeIndex>,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct FunctionDefinition {
    pub(crate) declaration: FunctionDeclaration,
    pub(crate) statements:  VectorSlice<Statement>,
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct ParserError {
    pub(crate) error_type:     ParserErrorType,
    pub(crate) source_vectors: SourceVectors,
}

impl Display for ParserError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.error_type)
    }
}

impl GetSeverity for ParserError {
    fn severity(&self) -> ErrorSeverity {
        self.error_type.severity()
    }
}

impl GetPosition for ParserError {
    fn position(&self, context: &Context) -> SourcePosition {
        self.source_vectors.position(context)
    }
}

impl GetSourceVectors for ParserError {
    fn source_vectors(&self, _context: &mut Context) -> SourceVectors {
        self.source_vectors
    }
}

impl std::error::Error for ParserError {}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum ConflictingTypeSpecifier {
    Short,
    Signed,
    Unsigned,
    Int,
    Float,
    Double,
    Void,
    Char,
    Bool,
    Complex,
    Long,
    LongLong,
}

impl Display for ConflictingTypeSpecifier {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            | ConflictingTypeSpecifier::Short => write!(f, "`short`"),
            | ConflictingTypeSpecifier::Signed => write!(f, "`signed`"),
            | ConflictingTypeSpecifier::Unsigned => write!(f, "`unsigned`"),
            | ConflictingTypeSpecifier::Int => write!(f, "`int`"),
            | ConflictingTypeSpecifier::Float => write!(f, "`float`"),
            | ConflictingTypeSpecifier::Double => write!(f, "`double`"),
            | ConflictingTypeSpecifier::Void => write!(f, "`void`"),
            | ConflictingTypeSpecifier::Char => write!(f, "`char`"),
            | ConflictingTypeSpecifier::Bool => write!(f, "`_Bool`"),
            | ConflictingTypeSpecifier::Complex => write!(f, "`_Complex`"),
            | ConflictingTypeSpecifier::Long => write!(f, "`long`"),
            | ConflictingTypeSpecifier::LongLong => write!(f, "`long long`"),
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum ParserErrorType {
    UnexpectedEndOfInput(&'static str),
    ExpectedIdentifierInTypedef(TokenType),
    ExpectedSemicolonAfterTypedef(TokenType),
    ExpectedSemicolonOrOpeningCurlyBraceAfterFunctionDeclaration(TokenType),
    StorageClassRedefinition(StorageClass, TokenType),
    ConstSpecifiedTwice,
    VolatileSpecifiedTwice,
    RestrictSpecifiedTwice,
    InlineSpecifiedTwice,
    ConflictingTypeSpecifiers(ConflictingTypeSpecifier, TokenType),
    TypeSpecifierSpecifiedTwice(TokenType),
    LongSpecifiedThrice,
    LongLongDoubleSpecified,
}

impl GetSeverity for ParserErrorType {
    fn severity(&self) -> ErrorSeverity {
        match self {
            | ParserErrorType::UnexpectedEndOfInput(..)
            | ParserErrorType::ExpectedIdentifierInTypedef(..)
            | ParserErrorType::ExpectedSemicolonOrOpeningCurlyBraceAfterFunctionDeclaration(..) =>
                ErrorSeverity::Error,
            | ParserErrorType::ExpectedSemicolonAfterTypedef(..)
            | ParserErrorType::StorageClassRedefinition(..)
            | ParserErrorType::ConstSpecifiedTwice
            | ParserErrorType::VolatileSpecifiedTwice
            | ParserErrorType::RestrictSpecifiedTwice
            | ParserErrorType::InlineSpecifiedTwice
            | ParserErrorType::ConflictingTypeSpecifiers(..)
            | ParserErrorType::TypeSpecifierSpecifiedTwice(..)
            | ParserErrorType::LongSpecifiedThrice
            | ParserErrorType::LongLongDoubleSpecified => ErrorSeverity::Warning,
        }
    }
}

impl Display for ParserErrorType {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            | ParserErrorType::UnexpectedEndOfInput(message) =>
                write!(f, "Unexpected end of input while {}!", message),
            | ParserErrorType::ExpectedIdentifierInTypedef(tt) => write!(
                f,
                "Expected an identifier in typedef, found instead {:?}!",
                tt
            ),
            | ParserErrorType::ExpectedSemicolonOrOpeningCurlyBraceAfterFunctionDeclaration(tt) =>
                write!(
                    f,
                    "Expected a semicolon or an opening curly brace after function declaration, \
                     found instead {:?}!",
                    tt
                ),
            | ParserErrorType::ExpectedSemicolonAfterTypedef(tt) => write!(
                f,
                "Expected a semicolon after typedef, found instead {:?}!",
                tt
            ),
            | ParserErrorType::StorageClassRedefinition(last, new) => write!(
                f,
                "Redefinition of storage class {:?} with {:?}!",
                last, new
            ),
            | ParserErrorType::ConstSpecifiedTwice =>
                write!(f, "`const` keyword specified twice in type declaration!"),
            | ParserErrorType::VolatileSpecifiedTwice =>
                write!(f, "`volatile` keyword specified twice in type declaration!"),
            | ParserErrorType::RestrictSpecifiedTwice =>
                write!(f, "`restrict` keyword specified twice in type declaration!"),
            | ParserErrorType::InlineSpecifiedTwice => write!(
                f,
                "`inline` keyword specified twice in function declaration or definition!"
            ),
            | ParserErrorType::ConflictingTypeSpecifiers(specifiers, tt) => write!(
                f,
                "Conflicting type specifiers {:?} and {:?}!",
                specifiers, tt
            ),
            | ParserErrorType::TypeSpecifierSpecifiedTwice(tt) =>
                write!(f, "Type specifier {:?} specified twice!", tt),
            | ParserErrorType::LongSpecifiedThrice =>
                write!(f, "`long` keyword specified thrice in type declaration!"),
            | ParserErrorType::LongLongDoubleSpecified => write!(
                f,
                "`long long` and `double` keywords specified together in type declaration!"
            ),
        }
    }
}

impl TranslationPhase for Parser {
    type Item = TopLevelStatement;

    fn next_item(&mut self, context: &mut Context) -> Option<Self::Item> {
        self.parse_top_level_statement(context)
    }
}
